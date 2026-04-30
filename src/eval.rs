//! Tree evaluator.
//!
//! Walks an [`Expr`] over a [`Value`] and yields a stream of results.
//! This is the correctness baseline; [`crate::stream`] handles a
//! narrow subset without allocating a Node tree and is checked
//! against this implementation by `stream_and_tree_agree`.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::ast::{attr, Node};
use crate::builtins;
use crate::error::RunError;
use crate::events::plain_text;
use crate::expr::{BinOp, CmpOp, Expr, Literal, ObjKey};
use crate::value::Value;

/// Scope: `$x` variables, user-defined functions, and filter-typed
/// parameters bound by those functions.
#[derive(Debug, Default, Clone)]
pub struct Env {
    bindings: BTreeMap<String, Value>,
    funcs: BTreeMap<String, Arc<UserFn>>,
    filters: BTreeMap<String, Arc<FilterClosure>>,
}

/// A `def name(params): body;` ready to be re-instantiated per call.
#[derive(Debug)]
pub(crate) struct UserFn {
    pub params: Vec<Arc<str>>,
    pub body: Expr,
}

/// Filter-typed argument bound at a call site. The expression has to
/// evaluate against the *caller's* environment, otherwise a recursive
/// `def f(n): ... f(n-1)` rebinds `n` to itself and the lookup loops
/// forever.
#[derive(Debug)]
pub(crate) struct FilterClosure {
    pub expr: Arc<Expr>,
    pub env: Env,
}

impl Env {
    /// Look up a `$x` binding.
    pub fn lookup(&self, name: &str) -> Option<&Value> {
        self.bindings.get(name)
    }

    /// Look up a user-defined function by name.
    pub(crate) fn lookup_func(&self, name: &str) -> Option<Arc<UserFn>> {
        self.funcs.get(name).cloned()
    }

    /// Look up a filter-typed parameter (a `def`'s argument).
    pub(crate) fn lookup_filter(&self, name: &str) -> Option<Arc<FilterClosure>> {
        self.filters.get(name).cloned()
    }

    /// Bind `name` to `value`. Chainable for `--arg` / `--argjson` /
    /// `as $x` rebinds.
    #[must_use]
    pub fn with(mut self, name: impl Into<String>, value: Value) -> Self {
        self.bindings.insert(name.into(), value);
        self
    }

    /// Register a user function.
    pub(crate) fn with_func(mut self, name: &str, f: Arc<UserFn>) -> Self {
        self.funcs.insert(name.to_string(), f);
        self
    }

    /// Bind a filter-typed parameter, capturing the env it should
    /// evaluate against.
    pub(crate) fn with_filter(mut self, name: &str, closure: Arc<FilterClosure>) -> Self {
        self.filters.insert(name.to_string(), closure);
        self
    }
}

type Stream = Box<dyn Iterator<Item = Result<Value, RunError>>>;

/// `a + b`. The `add` builtin folds with this.
pub(crate) fn apply_add(a: &Value, b: &Value) -> Result<Value, RunError> {
    apply_bin(a, BinOp::Add, b)
}

/// Total order matching jq's `sort`/`unique`.
pub(crate) fn value_cmp_for_sort(a: &Value, b: &Value) -> Ordering {
    value_cmp(a, b)
}

/// The dispatch. One match per [`Expr`] variant; helpers handle the
/// cases that aren't one-liners.
pub(crate) fn eval(expr: &Expr, input: Value, env: &Env) -> Stream {
    match expr {
        Expr::Identity => once(Ok(input)),
        Expr::RecurseAll => Box::new(RecurseAll { stack: vec![input] }),
        Expr::Field(name) => once(field(&input, name)),
        Expr::Index(idx) => index_stream(idx, input, env),
        Expr::Slice(lo, hi) => {
            let a = eval_int(lo.as_deref(), &input, env);
            let b = eval_int(hi.as_deref(), &input, env);
            once(a.and_then(|la| b.and_then(|hb| slice(&input, la, hb))))
        }
        Expr::Iterate => iterate(input),
        Expr::Pipe(l, r) => {
            let r = r.clone();
            let env = env.clone();
            Box::new(eval(l, input, &env.clone()).flat_map(move |x| match x {
                Ok(v) => eval(&r, v, &env),
                Err(e) => once(Err(e)),
            }))
        }
        Expr::Comma(a, b) => Box::new(eval(a, input.clone(), env).chain(eval(b, input, env))),
        Expr::Lit(l) => once(Ok(lit(l))),
        Expr::ArrayCtor(inner) => {
            let collected: Result<Vec<Value>, _> = eval(inner, input, env).collect();
            once(collected.map(|v| Value::Array(Arc::new(v))))
        }
        Expr::ObjectCtor(entries) => object(entries, input, env),
        Expr::Cmp(l, op, r) => cmp_stream(l, *op, r, input, env),
        Expr::Bin(l, op, r) => bin_stream(l, *op, r, input, env),
        Expr::Neg(x) => Box::new(eval(x, input, env).map(|r| r.and_then(neg))),
        Expr::Not(x) => Box::new(eval(x, input, env).map(|r| r.map(|v| Value::Bool(!v.truthy())))),
        Expr::If {
            branches,
            else_branch,
        } => if_stream(branches, else_branch.as_deref(), input, env),
        Expr::Var(name) => match env.lookup(name) {
            Some(v) => once(Ok(v.clone())),
            None => once(Err(RunError::Other(format!("${name} is not defined")))),
        },
        Expr::Call { name, args } => dispatch_call(name, args, input, env),
        Expr::Try(inner) => Box::new(eval(inner, input, env).filter_map(Result::ok).map(Ok)),
        Expr::As { bind, name, body } => {
            let body = body.clone();
            let name = name.clone();
            let env = env.clone();
            let outer = input.clone();
            Box::new(eval(bind, input, &env).flat_map(move |r| match r {
                Err(e) => once(Err(e)),
                Ok(v) => {
                    let bound = env.clone().with(name.as_ref(), v);
                    eval(&body, outer.clone(), &bound)
                }
            }))
        }
        Expr::Reduce {
            source,
            var,
            init,
            update,
        } => reduce_fold(source, var, init, update, None, input, env),
        Expr::Foreach {
            source,
            var,
            init,
            update,
            extract,
        } => reduce_fold(
            source,
            var,
            init,
            update,
            Some(extract.as_ref()),
            input,
            env,
        ),
        Expr::Def {
            name,
            params,
            body,
            rest,
        } => {
            let f = Arc::new(UserFn {
                params: params.clone(),
                body: (**body).clone(),
            });
            let new_env = env.clone().with_func(name, f);
            eval(rest, input, &new_env)
        }
        Expr::Assign(..) => once(Err(RunError::NotImplemented {
            feature: "mutation runs via Query::transform_bytes",
        })),
    }
}

// --- stream helpers ---------------------------------------------------------

fn once(r: Result<Value, RunError>) -> Stream {
    Box::new(std::iter::once(r))
}

fn type_err<S: Into<String>>(expected: S, got: &Value) -> RunError {
    RunError::Type {
        expected: expected.into(),
        got: got.type_name().into(),
    }
}

fn lit(l: &Literal) -> Value {
    match l {
        Literal::Null => Value::Null,
        Literal::Bool(b) => Value::Bool(*b),
        Literal::Number(n) => Value::Number(*n),
        Literal::String(s) => Value::String(Arc::from(s.as_ref())),
    }
}

// --- access -----------------------------------------------------------------

fn field(input: &Value, name: &str) -> Result<Value, RunError> {
    match input {
        Value::Null => Ok(Value::Null),
        Value::Object(m) => Ok(m.get(name).cloned().unwrap_or(Value::Null)),
        Value::Node(n) => Ok(node_field(n, name)),
        other => Err(type_err("object or node", other)),
    }
}

fn node_field(n: &Node, name: &str) -> Value {
    match name {
        "kind" => Value::from(n.kind.as_str()),
        "children" => Value::Array(Arc::new(n.children.clone())),
        "text" => Value::from(plain_text(&n.children)),
        "attrs" => {
            let m: BTreeMap<String, Value> = n
                .attrs
                .iter()
                .map(|(k, v)| ((*k).to_string(), v.clone()))
                .collect();
            Value::Object(Arc::new(m))
        }
        _ => n
            .attrs
            .get(attr::by_name(name).unwrap_or(""))
            .cloned()
            .unwrap_or(Value::Null),
    }
}

fn index_stream(idx: &Expr, input: Value, env: &Env) -> Stream {
    let idx = idx.clone();
    let env = env.clone();
    let host = input.clone();
    Box::new(eval(&idx, input, &env).map(move |r| r.and_then(|i| index(&host, &i))))
}

fn index(input: &Value, idx: &Value) -> Result<Value, RunError> {
    match (input, idx) {
        (Value::Null, _) => Ok(Value::Null),
        (Value::Array(a), Value::Number(n)) => Ok(at(a, *n)),
        (Value::Node(n), Value::Number(x)) => Ok(at(&n.children, *x)),
        (Value::Object(m), Value::String(s)) => {
            Ok(m.get(s.as_ref()).cloned().unwrap_or(Value::Null))
        }
        (Value::Node(n), Value::String(s)) => Ok(node_field(n, s)),
        (v, i) => Err(RunError::Type {
            expected: format!("index compatible with {}", v.type_name()),
            got: i.type_name().into(),
        }),
    }
}

fn at(arr: &[Value], n: f64) -> Value {
    let len = arr.len() as i64;
    let i = n as i64;
    let idx = if i < 0 { len + i } else { i };
    if (0..len).contains(&idx) {
        arr[idx as usize].clone()
    } else {
        Value::Null
    }
}

fn slice(input: &Value, lo: Option<i64>, hi: Option<i64>) -> Result<Value, RunError> {
    if let Value::String(s) = input {
        let chars: Vec<char> = s.chars().collect();
        let len = chars.len() as i64;
        let clamp = |x: i64| {
            let a = if x < 0 { len + x } else { x };
            a.clamp(0, len) as usize
        };
        let start = clamp(lo.unwrap_or(0));
        let end = clamp(hi.unwrap_or(len));
        let out: String = if start <= end {
            chars[start..end].iter().collect()
        } else {
            String::new()
        };
        return Ok(Value::from(out));
    }
    let arr: &[Value] = match input {
        Value::Array(a) => a,
        Value::Node(n) => &n.children,
        Value::Null => return Ok(Value::Null),
        other => return Err(type_err("array, node, or string", other)),
    };
    let len = arr.len() as i64;
    let clamp = |x: i64| {
        let a = if x < 0 { len + x } else { x };
        a.clamp(0, len) as usize
    };
    let start = clamp(lo.unwrap_or(0));
    let end = clamp(hi.unwrap_or(len));
    Ok(Value::Array(Arc::new(if start <= end {
        arr[start..end].to_vec()
    } else {
        Vec::new()
    })))
}

fn iterate(input: Value) -> Stream {
    if matches!(input, Value::Null) {
        return Box::new(std::iter::empty());
    }
    match children_of(&input) {
        Some(children) => Box::new(children.into_iter().map(Ok)),
        None => once(Err(type_err("array, object, or node", &input))),
    }
}

/// Direct children of `v`. Shared by `.[]` and `..`. Scalars return
/// `None` so callers can pick their own "no children" behaviour.
fn children_of(v: &Value) -> Option<Vec<Value>> {
    Some(match v {
        Value::Array(a) => (**a).clone(),
        Value::Object(m) => m.values().cloned().collect(),
        Value::Node(n) => n.children.clone(),
        _ => return None,
    })
}

fn eval_int(expr: Option<&Expr>, input: &Value, env: &Env) -> Result<Option<i64>, RunError> {
    let Some(e) = expr else { return Ok(None) };
    match eval(e, input.clone(), env).next() {
        Some(Ok(Value::Number(n))) => Ok(Some(n as i64)),
        Some(Ok(other)) => Err(type_err("integer", &other)),
        Some(Err(err)) => Err(err),
        None => Ok(None),
    }
}

// --- compare / binary -------------------------------------------------------

/// Comparison: evaluate both sides, compare, emit a bool. No
/// short-circuit.
fn cmp_stream(l: &Expr, op: CmpOp, r: &Expr, input: Value, env: &Env) -> Stream {
    cross(l, r, input, env, move |lv, rv| {
        Ok(Value::Bool(match op {
            CmpOp::Eq => value_cmp(&lv, &rv).is_eq(),
            CmpOp::Ne => !value_cmp(&lv, &rv).is_eq(),
            CmpOp::Lt => value_cmp(&lv, &rv).is_lt(),
            CmpOp::Le => value_cmp(&lv, &rv).is_le(),
            CmpOp::Gt => value_cmp(&lv, &rv).is_gt(),
            CmpOp::Ge => value_cmp(&lv, &rv).is_ge(),
        }))
    })
}

/// Cross-product of two streams, combined via `f`.
fn cross<F>(l: &Expr, r: &Expr, input: Value, env: &Env, f: F) -> Stream
where
    F: Fn(Value, Value) -> Result<Value, RunError> + Clone + 'static,
{
    let env = env.clone();
    let r = r.clone();
    let outer = input.clone();
    Box::new(eval(l, input, &env).flat_map(move |x| match x {
        Err(e) => once(Err(e)),
        Ok(lv) => {
            let (env, outer, f) = (env.clone(), outer.clone(), f.clone());
            Box::new(eval(&r, outer, &env).map(move |y| y.and_then(|rv| f(lv.clone(), rv))))
                as Stream
        }
    }))
}

fn value_cmp(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        (Value::Number(x), Value::Number(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (Value::String(x), Value::String(y)) => x.as_ref().cmp(y.as_ref()),
        (Value::Array(x), Value::Array(y)) => x
            .iter()
            .zip(y.iter())
            .map(|(a, b)| value_cmp(a, b))
            .find(|o| !o.is_eq())
            .unwrap_or_else(|| x.len().cmp(&y.len())),
        _ => type_rank(a).cmp(&type_rank(b)),
    }
}

fn type_rank(v: &Value) -> u8 {
    match v {
        Value::Null => 0,
        Value::Bool(false) => 1,
        Value::Bool(true) => 2,
        Value::Number(_) => 3,
        Value::String(_) => 4,
        Value::Array(_) => 5,
        Value::Object(_) => 6,
        Value::Node(_) => 7,
    }
}

/// `l op r` as an `Expr::Bin`. Short-circuits `and`, `or`, and `//`
/// so the RHS runs only when the LHS doesn't settle the result.
fn bin_stream(l: &Expr, op: BinOp, r: &Expr, input: Value, env: &Env) -> Stream {
    let env = env.clone();
    let r = r.clone();
    let outer = input.clone();
    if matches!(op, BinOp::Alt) {
        // jq spec: `a // b` emits the non-null/non-false outputs of
        // `a`. If `a` yields none (empty stream or all null/false),
        // emit `b` once against the original input.
        let mut keep: Vec<Result<Value, RunError>> = Vec::new();
        let mut had_truthy = false;
        for x in eval(l, input, &env) {
            match x {
                Err(e) => return once(Err(e)),
                Ok(v) if !matches!(v, Value::Null | Value::Bool(false)) => {
                    had_truthy = true;
                    keep.push(Ok(v));
                }
                Ok(_) => {}
            }
        }
        if had_truthy {
            return Box::new(keep.into_iter());
        }
        return eval(&r, outer, &env);
    }
    Box::new(eval(l, input, &env).flat_map(move |x| match x {
        Err(e) => once(Err(e)),
        Ok(lv) => {
            if let Some(v) = short_circuit(&lv, op) {
                return once(Ok(v));
            }
            let (env, outer) = (env.clone(), outer.clone());
            Box::new(eval(&r, outer, &env).map(move |y| y.and_then(|rv| apply_bin(&lv, op, &rv))))
                as Stream
        }
    }))
}

/// Decide on `and`/`or` from the LHS alone, or `None` to fall through
/// and evaluate the RHS.
fn short_circuit(lv: &Value, op: BinOp) -> Option<Value> {
    match op {
        BinOp::And if !lv.truthy() => Some(Value::Bool(false)),
        BinOp::Or if lv.truthy() => Some(Value::Bool(true)),
        _ => None,
    }
}

fn apply_bin(a: &Value, op: BinOp, b: &Value) -> Result<Value, RunError> {
    let arith: fn(f64, f64) -> f64 = match op {
        BinOp::And => return Ok(Value::Bool(a.truthy() && b.truthy())),
        BinOp::Or => return Ok(Value::Bool(a.truthy() || b.truthy())),
        BinOp::Alt => return Ok(b.clone()),
        BinOp::Add => return add(a, b),
        BinOp::Sub => |x, y| x - y,
        BinOp::Mul => |x, y| x * y,
        BinOp::Div => |x, y| x / y,
        BinOp::Mod => |x, y| x % y,
    };
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => Ok(Value::Number(arith(*x, *y))),
        _ => Err(RunError::Type {
            expected: "number".into(),
            got: format!("{} op {}", a.type_name(), b.type_name()),
        }),
    }
}

/// `a + b`. Numbers add, strings concat, arrays extend. Null on
/// either side is identity.
fn add(a: &Value, b: &Value) -> Result<Value, RunError> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => Ok(Value::Number(x + y)),
        (Value::String(x), Value::String(y)) => Ok(Value::String(Arc::from(format!("{x}{y}")))),
        (Value::Array(x), Value::Array(y)) => {
            let mut out = (**x).clone();
            out.extend_from_slice(y);
            Ok(Value::Array(Arc::new(out)))
        }
        (Value::Null, v) | (v, Value::Null) => Ok(v.clone()),
        _ => Err(RunError::Type {
            expected: "matching numeric/string/array operands".into(),
            got: format!("{} + {}", a.type_name(), b.type_name()),
        }),
    }
}

fn neg(v: Value) -> Result<Value, RunError> {
    if let Value::Number(n) = v {
        Ok(Value::Number(-n))
    } else {
        Err(type_err("number", &v))
    }
}

// --- control flow -----------------------------------------------------------

fn if_stream(
    branches: &[(Expr, Expr)],
    else_branch: Option<&Expr>,
    input: Value,
    env: &Env,
) -> Stream {
    for (cond, then_branch) in branches {
        match eval(cond, input.clone(), env).next() {
            Some(Ok(v)) if v.truthy() => return eval(then_branch, input, env),
            Some(Err(e)) => return once(Err(e)),
            _ => {}
        }
    }
    match else_branch {
        Some(e) => eval(e, input, env),
        None => once(Ok(input)),
    }
}

fn object(entries: &[(ObjKey, Expr)], input: Value, env: &Env) -> Stream {
    let mut combos: Vec<Vec<(String, Value)>> = vec![Vec::new()];
    for (k, v_expr) in entries {
        let key = match k {
            ObjKey::Ident(s) | ObjKey::Str(s) => s.to_string(),
            ObjKey::Expr(e) => match eval(e, input.clone(), env).next() {
                Some(Ok(Value::String(s))) => s.to_string(),
                Some(Ok(other)) => return once(Err(type_err("string key", &other))),
                Some(Err(e)) => return once(Err(e)),
                None => return Box::new(std::iter::empty()),
            },
        };
        let values: Vec<Value> = match eval(v_expr, input.clone(), env).collect() {
            Ok(vs) => vs,
            Err(e) => return once(Err(e)),
        };
        if values.is_empty() {
            return Box::new(std::iter::empty());
        }
        let mut next = Vec::with_capacity(combos.len() * values.len());
        for combo in &combos {
            for v in &values {
                let mut nc = combo.clone();
                nc.push((key.clone(), v.clone()));
                next.push(nc);
            }
        }
        combos = next;
    }
    let out: Vec<Result<Value, RunError>> = combos
        .into_iter()
        .map(|kv| {
            let mut m = BTreeMap::new();
            for (k, v) in kv {
                m.insert(k, v);
            }
            Ok(Value::Object(Arc::new(m)))
        })
        .collect();
    Box::new(out.into_iter())
}

// --- recursive descent ------------------------------------------------------

struct RecurseAll {
    stack: Vec<Value>,
}

impl Iterator for RecurseAll {
    type Item = Result<Value, RunError>;

    fn next(&mut self) -> Option<Self::Item> {
        let v = self.stack.pop()?;
        // Push children in reverse so pre-order walks match source.
        if let Some(mut kids) = children_of(&v) {
            kids.reverse();
            self.stack.extend(kids);
        }
        Some(Ok(v))
    }
}

/// Call dispatch. Filter-typed parameters (from `def`) win first,
/// then user-defined functions, then the builtin registry.
fn dispatch_call(name: &Arc<str>, args: &[Expr], input: Value, env: &Env) -> Stream {
    if args.is_empty() {
        if let Some(filter) = env.lookup_filter(name) {
            return eval(&filter.expr, input, &filter.env);
        }
    }
    if let Some(f) = env.lookup_func(name) {
        if args.len() != f.params.len() {
            return once(Err(RunError::Other(format!(
                "{name}: expected {} arg(s), got {}",
                f.params.len(),
                args.len()
            ))));
        }
        // Capture caller's env so each filter argument evaluates in
        // the scope it was passed from, not the callee's scope.
        let caller_env = env.clone();
        let mut new_env = env.clone();
        for (p, a) in f.params.iter().zip(args.iter()) {
            let closure = Arc::new(FilterClosure {
                expr: Arc::new(a.clone()),
                env: caller_env.clone(),
            });
            new_env = new_env.with_filter(p.as_ref(), closure);
        }
        return eval(&f.body, input, &new_env);
    }
    builtins::invoke(name, args, input, env)
        .unwrap_or_else(|| once(Err(RunError::Other(format!("unknown builtin `{name}`")))))
}

/// Shared body for `reduce` and `foreach`. `extract == None` means
/// reduce (yield only the final accumulator); `Some(e)` means foreach
/// (yield `e(acc)` per iteration).
fn reduce_fold(
    source: &Expr,
    var: &Arc<str>,
    init: &Expr,
    update: &Expr,
    extract: Option<&Expr>,
    input: Value,
    env: &Env,
) -> Stream {
    let first_or_null = |expr, val, env: &Env| -> Result<Value, RunError> {
        eval(expr, val, env)
            .next()
            .transpose()
            .map(|o| o.unwrap_or(Value::Null))
    };
    let items: Vec<Value> = match eval(source, input.clone(), env).collect::<Result<_, _>>() {
        Ok(v) => v,
        Err(e) => return once(Err(e)),
    };
    let mut acc = match first_or_null(init, input, env) {
        Ok(v) => v,
        Err(e) => return once(Err(e)),
    };
    let mut out = Vec::new();
    for item in items {
        let bound = env.clone().with(var.as_ref(), item);
        acc = match first_or_null(update, acc, &bound) {
            Ok(v) => v,
            Err(e) => return once(Err(e)),
        };
        if let Some(e) = extract {
            out.push(first_or_null(e, acc.clone(), &bound));
        }
    }
    if extract.is_none() {
        out.push(Ok(acc));
    }
    Box::new(out.into_iter())
}
