//! Builtin functions. One `invoke(name, ...)` dispatch.
//!
//! Two groups share the table: markdown helpers (`headings`,
//! `codeblocks`, `section`, ...) work on Node values built from the
//! parser; jq classics (`select`, `map`, `type`, `length`, `sub`,
//! `gsub`, ...) match the jq spec across every Value variant.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::Arc;

use regex::Regex;

use crate::ast::{attr, Node, NodeKind};
use crate::error::RunError;
use crate::eval::{self, Env};
use crate::events::plain_text;
use crate::expr::Expr;
use crate::value::Value;

type Stream = Box<dyn Iterator<Item = Result<Value, RunError>>>;

/// Invoke the builtin called `name`. Returns `None` if the name
/// isn't registered, so the caller can surface `unknown builtin`.
pub fn invoke(name: &str, args: &[Expr], input: Value, env: &Env) -> Option<Stream> {
    use NodeKind::{Code, FootnoteDef, Heading, Image, Item, Link, List, Paragraph, Quote, Table};
    Some(match name {
        "headings" => descendants(input, Heading),
        "paragraphs" => descendants(input, Paragraph),
        "codeblocks" | "code" => descendants(input, Code),
        "links" => descendants(input, Link),
        "images" => descendants(input, Image),
        "items" => descendants(input, Item),
        "lists" => descendants(input, List),
        "tables" => descendants(input, Table),
        "blockquotes" => descendants(input, Quote),
        "footnotes" => descendants(input, FootnoteDef),
        "rows" => descendants(input, NodeKind::Row),
        "cells" => cells_of(input),
        "headers" => headers_of(input),
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            headings_at(input, i64::from(name.as_bytes()[1] - b'0'))
        }
        "section" => return Some(section(args, input, env)),
        "sections" => return Some(sections(args, input, env)),
        "text" => one(text_of(&input)),
        "anchor" => one(anchor_of(&input)),
        "type" => ok(Value::from(input.type_name())),
        "length" => one(length_of(&input)),

        "empty" => Box::new(std::iter::empty()),
        "error" => error_builtin(args, input, env),
        "first" => first_or(args, input, env, true),
        "last" => first_or(args, input, env, false),
        "select" => select(args, input, env),
        "map" => map_builtin(args, input, env),
        "has" => has(args, input, env),
        "keys" => one(keys_of(&input)),
        "add" => one(add_all(&input)),
        "not" => ok(Value::Bool(!input.truthy())),
        "any" => any_or_all(args, input, env, true),
        "all" => any_or_all(args, input, env, false),
        "reverse" => one(reverse_of(input)),
        "sort" => one(sort_of(input)),
        "unique" => one(unique_of(input)),
        "tostring" => one(to_string_value(&input)),
        "tonumber" => one(to_number(&input)),

        "test" => one(regex_bool(args, &input, env, |re, s| re.is_match(s))),
        "sub" => one(regex_sub(args, &input, env, false)),
        "gsub" => one(regex_sub(args, &input, env, true)),
        "startswith" => one(str_pred(args, &input, env, |s, p| s.starts_with(p))),
        "endswith" => one(str_pred(args, &input, env, |s, p| s.ends_with(p))),
        "ascii_downcase" => one(ascii_case(&input, false)),
        "ascii_upcase" => one(ascii_case(&input, true)),
        "split" => one(split(args, &input, env)),
        "join" => one(join(args, &input, env)),
        "ltrimstr" => one(trim_side(args, &input, env, true)),
        "rtrimstr" => one(trim_side(args, &input, env, false)),
        "contains" => one(contains(args, &input, env)),
        "tojson" | "@json" => one(to_json(&input)),
        "fromjson" => one(from_json(&input)),
        "@uri" => one(format_uri(&input)),
        "@csv" => one(format_separated(&input, ',', true)),
        "@tsv" => one(format_separated(&input, '\t', false)),
        "@sh" => one(format_sh(&input)),
        "@html" => one(format_html(&input)),
        "env" | "$ENV" => one(Ok(env_as_value())),

        // Collection builtins that use a key function.
        "sort_by" => one(by_key_array(args, input, env, |items| {
            items.sort_by(|(ka, _), (kb, _)| eval::value_cmp_for_sort(ka, kb));
        })),
        "unique_by" => one(by_key_array(args, input, env, |items| {
            items.sort_by(|(ka, _), (kb, _)| eval::value_cmp_for_sort(ka, kb));
            items.dedup_by(|a, b| eval::value_cmp_for_sort(&a.0, &b.0).is_eq());
        })),
        "group_by" => one(group_by(args, input, env)),
        "min_by" => one(extreme_by(args, input, env, true)),
        "max_by" => one(extreme_by(args, input, env, false)),
        "min" => one(extreme(input, true)),
        "max" => one(extreme(input, false)),

        // Stream slicing.
        "range" => range(args, input, env),
        "limit" => limit(args, input, env),
        "nth" => one(nth(args, input, env)),

        // Paths.
        "paths" => paths(args, input, env),
        "getpath" => one(getpath(args, input, env)),
        "setpath" => one(setpath(args, input, env)),

        // Markdown helpers.
        "toc" => one(Ok(toc(&input))),
        "frontmatter" => one(Ok(frontmatter(&input))),
        "node" => one(build_node(args, input, env)),
        "walk" => walk_call(args, input, env),

        _ => return None,
    })
}

// --- helpers: stream constructors -------------------------------------------

fn ok(v: Value) -> Stream {
    Box::new(std::iter::once(Ok(v)))
}
fn err(e: RunError) -> Stream {
    Box::new(std::iter::once(Err(e)))
}
fn one(r: Result<Value, RunError>) -> Stream {
    Box::new(std::iter::once(r))
}

fn type_err(expected: &str, got: &Value) -> RunError {
    RunError::Type {
        expected: expected.into(),
        got: got.type_name().into(),
    }
}

/// First output of `expr` against `input`. Empty stream → `None`.
fn eval_first(expr: &Expr, input: &Value, env: &Env) -> Result<Option<Value>, RunError> {
    eval::eval(expr, input.clone(), env).next().transpose()
}

// --- markdown filters --------------------------------------------------------

fn descendants(input: Value, kind: NodeKind) -> Stream {
    let mut out = Vec::new();
    collect(&input, kind, &mut out);
    Box::new(out.into_iter().map(Ok))
}

fn collect(v: &Value, kind: NodeKind, out: &mut Vec<Value>) {
    match v {
        Value::Node(n) => {
            if n.kind == kind {
                out.push(Value::Node(n.clone()));
            }
            n.children.iter().for_each(|c| collect(c, kind, out));
        }
        Value::Array(arr) => arr.iter().for_each(|c| collect(c, kind, out)),
        Value::Object(m) => m.values().for_each(|c| collect(c, kind, out)),
        _ => {}
    }
}

fn headings_at(input: Value, level: i64) -> Stream {
    let mut all = Vec::new();
    collect(&input, NodeKind::Heading, &mut all);
    Box::new(
        all.into_iter()
            .filter(move |v| matches!(v, Value::Node(n) if heading_level(n) == level))
            .map(Ok),
    )
}

fn heading_level(n: &Node) -> i64 {
    match n.attrs.get(attr::LEVEL) {
        Some(Value::Number(x)) => *x as i64,
        _ => 0,
    }
}

fn section(args: &[Expr], input: Value, env: &Env) -> Stream {
    if args.len() != 1 {
        return err(RunError::Other("section/1: expected one argument".into()));
    }
    let name = match eval_first(&args[0], &input, env) {
        Ok(Some(Value::String(s))) => s.to_string(),
        Ok(Some(other)) => return err(type_err("string", &other)),
        Ok(None) => return Box::new(std::iter::empty()),
        Err(e) => return err(e),
    };
    let Value::Node(root) = &input else {
        return err(type_err("node", &input));
    };
    let mut out = Vec::new();
    build_sections(root, &name, &mut out);
    Box::new(out.into_iter().map(Ok))
}

fn build_sections(node: &Node, name: &str, out: &mut Vec<Value>) {
    let mut i = 0;
    while i < node.children.len() {
        let Value::Node(heading) = &node.children[i] else {
            i += 1;
            continue;
        };
        if heading.kind != NodeKind::Heading {
            build_sections(heading, name, out);
            i += 1;
            continue;
        }
        if !plain_text(&heading.children).eq_ignore_ascii_case(name) {
            build_sections(heading, name, out);
            i += 1;
            continue;
        }
        let level = heading_level(heading);
        let mut section = Node::new(NodeKind::Section);
        section.children.push(Value::Node(heading.clone()));
        let mut j = i + 1;
        while let Some(Value::Node(sibling)) = node.children.get(j) {
            if sibling.kind == NodeKind::Heading && heading_level(sibling) <= level {
                break;
            }
            section.children.push(node.children[j].clone());
            j += 1;
        }
        out.push(Value::Node(Arc::new(section)));
        i = j;
    }
}

/// `sections` (no args) wraps every heading + its body into a Section
/// node and streams them in document order. `sections(N)` filters by
/// heading level. Body extends until the next heading at level <= the
/// section heading's.
fn sections(args: &[Expr], input: Value, env: &Env) -> Stream {
    let level_filter = match args.len() {
        0 => None,
        1 => match eval_first(&args[0], &input, env) {
            Ok(Some(Value::Number(n))) if n.fract() == 0.0 => Some(n as i64),
            Ok(Some(Value::Number(_))) => {
                return err(RunError::Other("sections: level must be an integer".into()));
            }
            Ok(Some(other)) => return err(type_err("number", &other)),
            Ok(None) => return Box::new(std::iter::empty()),
            Err(e) => return err(e),
        },
        _ => {
            return err(RunError::Other(
                "sections: expected 0 or 1 arguments".into(),
            ))
        }
    };
    let Value::Node(root) = &input else {
        return err(type_err("node", &input));
    };
    let mut out = Vec::new();
    build_all_sections(&root.children, level_filter, &mut out);
    Box::new(out.into_iter().map(Ok))
}

fn build_all_sections(children: &[Value], level_filter: Option<i64>, out: &mut Vec<Value>) {
    let mut i = 0;
    while i < children.len() {
        let Value::Node(node) = &children[i] else {
            i += 1;
            continue;
        };
        if node.kind != NodeKind::Heading {
            build_all_sections(&node.children, level_filter, out);
            i += 1;
            continue;
        }
        let level = heading_level(node);
        let mut j = i + 1;
        while let Some(Value::Node(sibling)) = children.get(j) {
            if sibling.kind == NodeKind::Heading && heading_level(sibling) <= level {
                break;
            }
            j += 1;
        }
        if level_filter.is_none_or(|want| want == level) {
            let mut section = Node::new(NodeKind::Section);
            section.children = children[i..j].to_vec();
            out.push(Value::Node(Arc::new(section)));
        }
        if i + 1 < j {
            build_all_sections(&children[i + 1..j], level_filter, out);
        }
        i = j;
    }
}

fn text_of(input: &Value) -> Result<Value, RunError> {
    match input {
        Value::Node(n) => Ok(Value::from(plain_text(&n.children))),
        Value::String(_) => Ok(input.clone()),
        other => Err(type_err("node or string", other)),
    }
}

fn anchor_of(input: &Value) -> Result<Value, RunError> {
    match input {
        Value::Node(n) if n.kind == NodeKind::Heading => {
            Ok(Value::from(slug::slugify(plain_text(&n.children))))
        }
        Value::String(s) => Ok(Value::from(slug::slugify(s.as_ref()))),
        other => Err(type_err("heading node or string", other)),
    }
}

fn length_of(v: &Value) -> Result<Value, RunError> {
    let n: i64 = match v {
        Value::Null => 0,
        Value::Array(a) => a.len() as i64,
        Value::Object(m) => m.len() as i64,
        Value::String(s) => s.chars().count() as i64,
        Value::Node(n) => n.children.len() as i64,
        Value::Bool(_) | Value::Number(_) => return Err(type_err("length-capable", v)),
    };
    Ok(Value::from(n))
}

fn cells_of(input: Value) -> Stream {
    match &input {
        Value::Node(n) if n.kind == NodeKind::Row => {
            let out: Vec<Value> = n
                .children
                .iter()
                .filter(|c| matches!(c, Value::Node(child) if child.kind == NodeKind::Cell))
                .cloned()
                .collect();
            Box::new(out.into_iter().map(Ok))
        }
        Value::Node(_) => descendants(input, NodeKind::Cell),
        _ => err(type_err("node", &input)),
    }
}

/// First-row cells of each Table in the input.
fn headers_of(input: Value) -> Stream {
    let mut tables = Vec::new();
    collect(&input, NodeKind::Table, &mut tables);
    let out: Vec<Value> = tables
        .iter()
        .filter_map(|t| match t {
            Value::Node(n) => Some(n),
            _ => None,
        })
        .filter_map(|table| {
            table.children.iter().find_map(|c| match c {
                Value::Node(n) if n.kind == NodeKind::Row => Some(n.clone()),
                _ => None,
            })
        })
        .flat_map(|row| {
            row.children
                .iter()
                .filter(|c| matches!(c, Value::Node(n) if n.kind == NodeKind::Cell))
                .cloned()
                .collect::<Vec<_>>()
        })
        .collect();
    Box::new(out.into_iter().map(Ok))
}

/// `error(msg)` raises a runtime error. With no argument, `input`
/// must already be a string. `try` / `?` can catch it.
fn error_builtin(args: &[Expr], input: Value, env: &Env) -> Stream {
    let msg = match args.first() {
        Some(expr) => match eval_first(expr, &input, env) {
            Ok(Some(Value::String(s))) => s.to_string(),
            Ok(Some(other)) => return err(type_err("string", &other)),
            Ok(None) => return Box::new(std::iter::empty()),
            Err(e) => return err(e),
        },
        None => match &input {
            Value::String(s) => s.to_string(),
            _ => return err(type_err("string", &input)),
        },
    };
    err(RunError::Other(msg))
}

// --- collections -------------------------------------------------------------

fn first_or(args: &[Expr], input: Value, env: &Env, first: bool) -> Stream {
    if args.is_empty() {
        return one(head_or_tail(&input, first));
    }
    let s = eval::eval(&args[0], input, env);
    let pick = if first { s.take(1).next() } else { s.last() };
    one(pick.unwrap_or(Ok(Value::Null)))
}

fn head_or_tail(v: &Value, first: bool) -> Result<Value, RunError> {
    let slice: &[Value] = match v {
        Value::Array(a) => a,
        Value::Node(n) => &n.children,
        Value::Null => return Ok(Value::Null),
        other => return Err(type_err("array or node", other)),
    };
    Ok(if first { slice.first() } else { slice.last() }
        .cloned()
        .unwrap_or(Value::Null))
}

fn select(args: &[Expr], input: Value, env: &Env) -> Stream {
    if args.len() != 1 {
        return err(RunError::Other("select/1 expects one argument".into()));
    }
    match eval_first(&args[0], &input, env) {
        Ok(Some(v)) if v.truthy() => ok(input),
        Ok(_) => Box::new(std::iter::empty()),
        Err(e) => err(e),
    }
}

fn map_builtin(args: &[Expr], input: Value, env: &Env) -> Stream {
    if args.len() != 1 {
        return err(RunError::Other("map/1 expects one argument".into()));
    }
    let items: Vec<Value> = match input {
        Value::Array(a) => (*a).clone(),
        Value::Node(n) => n.children.clone(),
        Value::Null => Vec::new(),
        other => return err(type_err("array or node", &other)),
    };
    let expr = args[0].clone();
    let mut out = Vec::with_capacity(items.len());
    for v in items {
        for r in eval::eval(&expr, v, env) {
            match r {
                Ok(x) => out.push(x),
                Err(e) => return err(e),
            }
        }
    }
    ok(Value::Array(Arc::new(out)))
}

fn has(args: &[Expr], input: Value, env: &Env) -> Stream {
    if args.len() != 1 {
        return err(RunError::Other("has/1 expects one argument".into()));
    }
    let present = match eval_first(&args[0], &input, env) {
        Ok(Some(Value::String(s))) => match &input {
            Value::Object(m) => m.contains_key(s.as_ref()),
            Value::Node(n) => {
                matches!(s.as_ref(), "kind" | "children" | "text" | "attrs")
                    || n.attrs.contains_key(&*s.to_string())
            }
            _ => false,
        },
        Ok(Some(Value::Number(n))) => {
            let len = match &input {
                Value::Array(a) => a.len() as isize,
                Value::Node(node) => node.children.len() as isize,
                _ => 0,
            };
            (0..len).contains(&(n as isize))
        }
        Ok(Some(other)) => return err(type_err("string or number key", &other)),
        Ok(None) => false,
        Err(e) => return err(e),
    };
    ok(Value::Bool(present))
}

fn keys_of(input: &Value) -> Result<Value, RunError> {
    let arr: Vec<Value> = match input {
        Value::Object(m) => m.keys().cloned().map(Value::from).collect(),
        Value::Array(a) => (0..a.len() as i64).map(Value::from).collect(),
        Value::Node(n) => {
            let mut ks: Vec<Value> = vec![
                Value::from("kind"),
                Value::from("children"),
                Value::from("text"),
            ];
            ks.extend(n.attrs.keys().map(|k| Value::from(*k)));
            ks
        }
        other => return Err(type_err("keyed value", other)),
    };
    Ok(Value::Array(Arc::new(arr)))
}

fn add_all(input: &Value) -> Result<Value, RunError> {
    let Value::Array(arr) = input else {
        return Err(type_err("array", input));
    };
    let mut iter = arr.iter().cloned();
    let Some(first) = iter.next() else {
        return Ok(Value::Null);
    };
    iter.try_fold(first, |a, b| eval::apply_add(&a, &b))
}

fn reduce_bool(
    input: &Value,
    combine: fn(bool, bool) -> bool,
    init: bool,
) -> Result<Value, RunError> {
    match input {
        Value::Array(a) => Ok(Value::Bool(
            a.iter().fold(init, |acc, v| combine(acc, v.truthy())),
        )),
        Value::Null => Ok(Value::Bool(init)),
        other => Err(type_err("array", other)),
    }
}

/// `any` / `all`. Zero args: truthy reduction over the array.
/// One arg: evaluate `f` per element and short-circuit on the first
/// hit (any) or miss (all).
fn any_or_all(args: &[Expr], input: Value, env: &Env, is_any: bool) -> Stream {
    if args.is_empty() {
        let combine: fn(bool, bool) -> bool = if is_any {
            |a, b| a || b
        } else {
            |a, b| a && b
        };
        return one(reduce_bool(&input, combine, !is_any));
    }
    if args.len() != 1 {
        return err(RunError::Other(
            "any/all: expected 0 or 1 arguments".into(),
        ));
    }
    let items: Vec<Value> = match input {
        Value::Array(a) => (*a).clone(),
        Value::Node(n) => n.children.clone(),
        Value::Null => Vec::new(),
        other => return err(type_err("array or node", &other)),
    };
    let mut acc = !is_any;
    for v in items {
        for r in eval::eval(&args[0], v, env) {
            match r {
                Ok(x) => {
                    let t = x.truthy();
                    if is_any && t {
                        return ok(Value::Bool(true));
                    }
                    if !is_any && !t {
                        return ok(Value::Bool(false));
                    }
                    acc = if is_any { acc || t } else { acc && t };
                }
                Err(e) => return err(e),
            }
        }
    }
    ok(Value::Bool(acc))
}

fn reverse_of(input: Value) -> Result<Value, RunError> {
    match input {
        Value::String(s) => Ok(Value::from(s.chars().rev().collect::<String>())),
        other => map_array(other, "array or string", |v| v.reverse()),
    }
}

fn sort_of(input: Value) -> Result<Value, RunError> {
    map_array(input, "array", |v| v.sort_by(eval::value_cmp_for_sort))
}

fn unique_of(input: Value) -> Result<Value, RunError> {
    map_array(input, "array", |v| {
        v.sort_by(eval::value_cmp_for_sort);
        v.dedup_by(|a, b| eval::value_cmp_for_sort(a, b).is_eq());
    })
}

/// Run `f` on a cloned copy of an array's contents. `Null` passes
/// through; other types get a type error using `expected` as the
/// expected-type label.
fn map_array(
    input: Value,
    expected: &str,
    f: impl FnOnce(&mut Vec<Value>),
) -> Result<Value, RunError> {
    match input {
        Value::Array(a) => {
            let mut v = (*a).clone();
            f(&mut v);
            Ok(Value::Array(Arc::new(v)))
        }
        Value::Null => Ok(Value::Null),
        other => Err(type_err(expected, &other)),
    }
}

#[allow(clippy::unnecessary_wraps)] // called via `one(...)` which takes Result.
fn to_string_value(v: &Value) -> Result<Value, RunError> {
    Ok(match v {
        Value::String(_) => v.clone(),
        Value::Null => Value::from("null"),
        Value::Bool(b) => Value::from(if *b { "true" } else { "false" }),
        Value::Number(n) if n.fract() == 0.0 && n.is_finite() => {
            Value::from((*n as i64).to_string())
        }
        Value::Number(n) => Value::from(n.to_string()),
        Value::Array(_) | Value::Object(_) => Value::from(json_string(v)),
        Value::Node(n) => Value::from(plain_text(&n.children)),
    })
}

fn json_string(v: &Value) -> String {
    let json = crate::emit::json::value_to_json(v, crate::emit::json::JsonOptions::COMPACT);
    serde_json::to_string(&json).unwrap_or_default()
}

fn to_number(v: &Value) -> Result<Value, RunError> {
    match v {
        Value::Number(_) => Ok(v.clone()),
        Value::String(s) => s
            .parse::<f64>()
            .map(Value::Number)
            .map_err(|_| RunError::Other(format!("tonumber: cannot parse `{s}`"))),
        other => Err(type_err("number or string", other)),
    }
}

// --- regex / string ops ------------------------------------------------------

fn regex_bool(
    args: &[Expr],
    input: &Value,
    env: &Env,
    f: impl Fn(&Regex, &str) -> bool,
) -> Result<Value, RunError> {
    let s = expect_string(input)?;
    let pattern = eval_string_arg(args.first(), input, env)?;
    let re = Regex::new(&pattern)?;
    Ok(Value::Bool(f(&re, s)))
}

fn regex_sub(args: &[Expr], input: &Value, env: &Env, all: bool) -> Result<Value, RunError> {
    if args.len() < 2 {
        return Err(RunError::Other(
            "sub/gsub: expected (pattern; replacement)".into(),
        ));
    }
    let s = expect_string(input)?;
    let pattern = eval_string_arg(Some(&args[0]), input, env)?;
    let repl = eval_string_arg(Some(&args[1]), input, env)?;
    let re = Regex::new(&pattern)?;
    let repl_str: &str = &repl;
    let out = if all {
        re.replace_all(s, repl_str).into_owned()
    } else {
        re.replace(s, repl_str).into_owned()
    };
    Ok(Value::from(out))
}

fn str_pred(
    args: &[Expr],
    input: &Value,
    env: &Env,
    f: impl Fn(&str, &str) -> bool,
) -> Result<Value, RunError> {
    let s = expect_string(input)?;
    let arg = eval_string_arg(args.first(), input, env)?;
    Ok(Value::Bool(f(s, &arg)))
}

fn ascii_case(input: &Value, upper: bool) -> Result<Value, RunError> {
    let s = expect_string(input)?;
    Ok(Value::from(if upper {
        s.to_ascii_uppercase()
    } else {
        s.to_ascii_lowercase()
    }))
}

fn expect_string(v: &Value) -> Result<&str, RunError> {
    if let Value::String(s) = v {
        Ok(s.as_ref())
    } else {
        Err(type_err("string", v))
    }
}

fn eval_string_arg(arg: Option<&Expr>, input: &Value, env: &Env) -> Result<String, RunError> {
    let expr = arg.ok_or_else(|| RunError::Other("missing string argument".into()))?;
    match eval_first(expr, input, env)? {
        Some(Value::String(s)) => Ok(s.to_string()),
        Some(other) => Err(type_err("string", &other)),
        None => Err(RunError::Other("argument stream was empty".into())),
    }
}

// ---- string / json helpers -------------------------------------------------

fn split(args: &[Expr], input: &Value, env: &Env) -> Result<Value, RunError> {
    let s = expect_string(input)?;
    let sep = eval_string_arg(args.first(), input, env)?;
    let parts: Vec<Value> = if sep.is_empty() {
        s.chars().map(|c| Value::from(c.to_string())).collect()
    } else {
        s.split(sep.as_str()).map(Value::from).collect()
    };
    Ok(Value::Array(Arc::new(parts)))
}

fn join(args: &[Expr], input: &Value, env: &Env) -> Result<Value, RunError> {
    let Value::Array(arr) = input else {
        return Err(type_err("array", input));
    };
    let sep = eval_string_arg(args.first(), input, env)?;
    let mut out = String::new();
    for (i, v) in arr.iter().enumerate() {
        if i > 0 {
            out.push_str(&sep);
        }
        match v {
            Value::String(s) => out.push_str(s),
            Value::Null => {}
            other => {
                use std::fmt::Write;
                let _ = write!(out, "{other:?}");
            }
        }
    }
    Ok(Value::from(out))
}

fn trim_side(args: &[Expr], input: &Value, env: &Env, left: bool) -> Result<Value, RunError> {
    let s = expect_string(input)?;
    let needle = eval_string_arg(args.first(), input, env)?;
    let trimmed = if left {
        s.strip_prefix(needle.as_str()).unwrap_or(s)
    } else {
        s.strip_suffix(needle.as_str()).unwrap_or(s)
    };
    Ok(Value::from(trimmed.to_string()))
}

fn contains(args: &[Expr], input: &Value, env: &Env) -> Result<Value, RunError> {
    match eval_first(&args[0], input, env)? {
        Some(needle) => Ok(Value::Bool(value_contains(input, &needle))),
        None => Ok(Value::Bool(false)),
    }
}

fn value_contains(haystack: &Value, needle: &Value) -> bool {
    match (haystack, needle) {
        (Value::String(a), Value::String(b)) => a.contains(b.as_ref()),
        (Value::Array(a), Value::Array(b)) => {
            b.iter().all(|nb| a.iter().any(|ha| value_contains(ha, nb)))
        }
        (Value::Object(a), Value::Object(b)) => b
            .iter()
            .all(|(k, v)| a.get(k).is_some_and(|av| value_contains(av, v))),
        (a, b) => eval::value_cmp_for_sort(a, b).is_eq(),
    }
}

#[allow(clippy::unnecessary_wraps)]
fn to_json(input: &Value) -> Result<Value, RunError> {
    Ok(Value::from(json_string(input)))
}

fn from_json(input: &Value) -> Result<Value, RunError> {
    let s = expect_string(input)?;
    let j: serde_json::Value =
        serde_json::from_str(s).map_err(|e| RunError::Other(format!("fromjson: {e}")))?;
    Ok(crate::emit::json::value_from_json(j))
}

/// `@uri`: percent-encode a string per RFC 3986 unreserved set.
fn format_uri(input: &Value) -> Result<Value, RunError> {
    let s = expect_string(input)?;
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if matches!(b, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            use std::fmt::Write as _;
            let _ = write!(out, "%{b:02X}");
        }
    }
    Ok(Value::from(out))
}

/// `@csv` / `@tsv`: join an array of scalars with a separator. Strings
/// get quoted (CSV style: embedded quotes doubled) when `quote` is
/// set; TSV passes strings through unchanged.
fn format_separated(input: &Value, sep: char, quote: bool) -> Result<Value, RunError> {
    let Value::Array(arr) = input else {
        return Err(type_err("array", input));
    };
    let mut out = String::new();
    for (i, v) in arr.iter().enumerate() {
        if i > 0 {
            out.push(sep);
        }
        match v {
            Value::String(s) if quote => {
                out.push('"');
                out.push_str(&s.replace('"', "\"\""));
                out.push('"');
            }
            Value::String(s) => out.push_str(s),
            Value::Number(n) if n.fract() == 0.0 && n.is_finite() => {
                use std::fmt::Write as _;
                let _ = write!(out, "{}", *n as i64);
            }
            Value::Number(n) => {
                use std::fmt::Write as _;
                let _ = write!(out, "{n}");
            }
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Null => {}
            other => return Err(type_err("scalar", other)),
        }
    }
    Ok(Value::from(out))
}

/// `@sh`: single-quote a string for safe shell pasting. Embedded
/// single quotes get closed, escaped, and reopened: `don't` ->
/// `'don'\''t'`. Arrays join with spaces.
fn format_sh(input: &Value) -> Result<Value, RunError> {
    fn escape_one(s: &str, out: &mut String) {
        out.push('\'');
        for ch in s.chars() {
            if ch == '\'' {
                out.push_str("'\\''");
            } else {
                out.push(ch);
            }
        }
        out.push('\'');
    }
    match input {
        Value::String(s) => {
            let mut out = String::new();
            escape_one(s, &mut out);
            Ok(Value::from(out))
        }
        Value::Array(arr) => {
            let mut out = String::new();
            for (i, v) in arr.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                let Value::String(s) = v else {
                    return Err(type_err("string element", v));
                };
                escape_one(s, &mut out);
            }
            Ok(Value::from(out))
        }
        other => Err(type_err("string or array", other)),
    }
}

/// `@html`: escape `<`, `>`, `&`, `"`, `'`.
fn format_html(input: &Value) -> Result<Value, RunError> {
    let s = expect_string(input)?;
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    Ok(Value::from(out))
}

fn env_as_value() -> Value {
    use std::collections::BTreeMap;
    let map: BTreeMap<String, Value> = std::env::vars().map(|(k, v)| (k, Value::from(v))).collect();
    Value::Object(Arc::new(map))
}

// ---- key-function collection helpers ---------------------------------------

/// Evaluate `f` on each array element to produce `(key, element)` pairs
/// and hand them to `reorder`. Result is an Array of the elements in
/// whatever order the reorder step leaves them in.
fn by_key_array(
    args: &[Expr],
    input: Value,
    env: &Env,
    reorder: impl FnOnce(&mut Vec<(Value, Value)>),
) -> Result<Value, RunError> {
    let key_fn = key_fn_arg(args, "sort_by/unique_by/group_by")?;
    let arr = expect_array(&input)?;
    let mut items: Vec<(Value, Value)> = arr
        .iter()
        .map(|v| eval_key(key_fn, v, env).map(|k| (k, v.clone())))
        .collect::<Result<_, _>>()?;
    reorder(&mut items);
    Ok(Value::Array(Arc::new(
        items.into_iter().map(|(_, v)| v).collect(),
    )))
}

fn group_by(args: &[Expr], input: Value, env: &Env) -> Result<Value, RunError> {
    let key_fn = key_fn_arg(args, "group_by")?;
    let arr = expect_array(&input)?;
    let mut items: Vec<(Value, Value)> = arr
        .iter()
        .map(|v| eval_key(key_fn, v, env).map(|k| (k, v.clone())))
        .collect::<Result<_, _>>()?;
    items.sort_by(|(ka, _), (kb, _)| eval::value_cmp_for_sort(ka, kb));

    let mut groups: Vec<Vec<Value>> = Vec::new();
    let mut last_key: Option<Value> = None;
    for (k, v) in items {
        if last_key
            .as_ref()
            .is_none_or(|prev| !eval::value_cmp_for_sort(prev, &k).is_eq())
        {
            groups.push(Vec::new());
            last_key = Some(k);
        }
        groups.last_mut().unwrap().push(v);
    }
    let out: Vec<Value> = groups
        .into_iter()
        .map(|g| Value::Array(Arc::new(g)))
        .collect();
    Ok(Value::Array(Arc::new(out)))
}

fn extreme_by(args: &[Expr], input: Value, env: &Env, least: bool) -> Result<Value, RunError> {
    let key_fn = key_fn_arg(args, "min_by/max_by")?;
    let arr = expect_array(&input)?;
    let pick = if least {
        Ordering::is_lt
    } else {
        Ordering::is_gt
    };
    let mut best: Option<(Value, Value)> = None;
    for v in arr {
        let k = eval_key(key_fn, v, env)?;
        if best
            .as_ref()
            .is_none_or(|(bk, _)| pick(eval::value_cmp_for_sort(&k, bk)))
        {
            best = Some((k, v.clone()));
        }
    }
    Ok(best.map_or(Value::Null, |(_, v)| v))
}

fn extreme(input: Value, least: bool) -> Result<Value, RunError> {
    let arr = expect_array(&input)?;
    let pick = if least {
        Ordering::is_lt
    } else {
        Ordering::is_gt
    };
    Ok(arr
        .iter()
        .skip(1)
        .fold(arr.first(), |best, v| match best {
            Some(b) if pick(eval::value_cmp_for_sort(v, b)) => Some(v),
            _ => best,
        })
        .cloned()
        .unwrap_or(Value::Null))
}

fn key_fn_arg<'a>(args: &'a [Expr], name: &str) -> Result<&'a Expr, RunError> {
    args.first()
        .ok_or_else(|| RunError::Other(format!("{name}: expected one argument")))
}

fn eval_key(f: &Expr, v: &Value, env: &Env) -> Result<Value, RunError> {
    eval_first(f, v, env).map(|o| o.unwrap_or(Value::Null))
}

fn expect_array(v: &Value) -> Result<&Vec<Value>, RunError> {
    match v {
        Value::Array(a) => Ok(a),
        other => Err(type_err("array", other)),
    }
}

// ---- stream slicing --------------------------------------------------------

/// First output of `expr` coerced to f64. `default` is used when the
/// argument's stream is empty.
fn eval_number(expr: &Expr, input: &Value, env: &Env, default: f64) -> Result<f64, RunError> {
    match eval_first(expr, input, env)? {
        Some(Value::Number(n)) => Ok(n),
        Some(other) => Err(type_err("number", &other)),
        None => Ok(default),
    }
}

/// `range(m; n)` yields integers `[m, n)`. `range(m; n; step)` strides.
fn range(args: &[Expr], input: Value, env: &Env) -> Stream {
    let nums: Result<Vec<f64>, _> = args
        .iter()
        .map(|a| match eval_first(a, &input, env)? {
            Some(Value::Number(n)) => Ok(n),
            Some(other) => Err(type_err("number", &other)),
            None => Err(RunError::Other("range: empty argument stream".into())),
        })
        .collect();
    let (start, stop, step) = match nums.as_deref() {
        Ok([n]) => (0.0, *n, 1.0),
        Ok([m, n]) => (*m, *n, 1.0),
        Ok([m, n, s]) => (*m, *n, *s),
        Ok(_) => return err(RunError::Other("range: 1..3 arguments".into())),
        Err(e) => return err(e.clone()),
    };
    if step == 0.0 {
        return err(RunError::Other("range: step cannot be zero".into()));
    }
    let mut values = Vec::new();
    let mut x = start;
    while (step > 0.0 && x < stop) || (step < 0.0 && x > stop) {
        values.push(Value::Number(x));
        x += step;
    }
    Box::new(values.into_iter().map(Ok))
}

/// `limit(n; f)` keeps the first `n` results of `f`.
fn limit(args: &[Expr], input: Value, env: &Env) -> Stream {
    if args.len() != 2 {
        return err(RunError::Other("limit/2 expects (count; expr)".into()));
    }
    let n = match eval_number(&args[0], &input, env, 0.0) {
        Ok(n) => n as i64,
        Err(e) => return err(e),
    };
    if n <= 0 {
        return Box::new(std::iter::empty());
    }
    let taken: Vec<_> = eval::eval(&args[1], input, env).take(n as usize).collect();
    Box::new(taken.into_iter())
}

/// `nth(n; f)` returns the nth output of `f` (0-indexed).
fn nth(args: &[Expr], input: Value, env: &Env) -> Result<Value, RunError> {
    if args.len() != 2 {
        return Err(RunError::Other("nth/2 expects (index; expr)".into()));
    }
    let n = eval_number(&args[0], &input, env, 0.0)? as i64;
    if n < 0 {
        return Ok(Value::Null);
    }
    eval::eval(&args[1], input, env)
        .nth(n as usize)
        .unwrap_or(Ok(Value::Null))
}

// ---- paths -----------------------------------------------------------------

/// `paths` streams every non-empty path into `input` as an array of
/// keys/indices. Mirrors jq.
fn paths(args: &[Expr], input: Value, env: &Env) -> Stream {
    let mut out = Vec::new();
    collect_paths(&input, Vec::new(), &mut out);
    let mut keep: Vec<Vec<Value>> = Vec::with_capacity(out.len());
    if args.is_empty() {
        keep = out;
    } else if args.len() == 1 {
        for path in out {
            let Some(v) = get_at_path(&input, &path) else {
                continue;
            };
            match eval_first(&args[0], &v, env) {
                Ok(Some(r)) if r.truthy() => keep.push(path),
                Ok(_) => {}
                Err(e) => return err(e),
            }
        }
    } else {
        return err(RunError::Other("paths: expected 0 or 1 arguments".into()));
    }
    Box::new(keep.into_iter().map(|p| Ok(Value::Array(Arc::new(p)))))
}

fn get_at_path(input: &Value, path: &[Value]) -> Option<Value> {
    let mut cur = input.clone();
    for step in path {
        cur = match (&cur, step) {
            (Value::Array(a), Value::Number(n)) => {
                let i = *n as i64;
                if i < 0 || (i as usize) >= a.len() {
                    return None;
                }
                a[i as usize].clone()
            }
            (Value::Object(m), Value::String(k)) => m.get(k.as_ref())?.clone(),
            _ => return None,
        };
    }
    Some(cur)
}

fn collect_paths(v: &Value, prefix: Vec<Value>, out: &mut Vec<Vec<Value>>) {
    match v {
        Value::Array(a) => {
            for (i, child) in a.iter().enumerate() {
                let mut p = prefix.clone();
                p.push(Value::from(i as i64));
                out.push(p.clone());
                collect_paths(child, p, out);
            }
        }
        Value::Object(m) => {
            for (k, child) in m.iter() {
                let mut p = prefix.clone();
                p.push(Value::from(k.clone()));
                out.push(p.clone());
                collect_paths(child, p, out);
            }
        }
        _ => {}
    }
}

/// `getpath(path)` walks an array of keys/indices into `input`.
fn getpath(args: &[Expr], input: Value, env: &Env) -> Result<Value, RunError> {
    let path = match eval_first(&args[0], &input, env)? {
        Some(Value::Array(a)) => a,
        Some(other) => return Err(type_err("array of keys", &other)),
        None => return Ok(Value::Null),
    };
    let mut cur = input;
    for step in path.iter() {
        cur = match (&cur, step) {
            (Value::Object(m), Value::String(k)) => {
                m.get(k.as_ref()).cloned().unwrap_or(Value::Null)
            }
            (Value::Array(a), Value::Number(n)) => {
                let i = *n as i64;
                let idx = if i < 0 { a.len() as i64 + i } else { i };
                if idx < 0 || idx as usize >= a.len() {
                    Value::Null
                } else {
                    a[idx as usize].clone()
                }
            }
            _ => Value::Null,
        };
    }
    Ok(cur)
}

/// `setpath(path; value)` writes into a cloned `input`. Creates
/// missing intermediate containers as jq does.
fn setpath(args: &[Expr], input: Value, env: &Env) -> Result<Value, RunError> {
    if args.len() != 2 {
        return Err(RunError::Other("setpath/2 expects (path; value)".into()));
    }
    let path = match eval_first(&args[0], &input, env)? {
        Some(Value::Array(a)) => a,
        Some(other) => return Err(type_err("array of keys", &other)),
        None => return Ok(input),
    };
    let Some(value) = eval_first(&args[1], &input, env)? else {
        return Ok(input);
    };
    set_path_inner(input, path.as_ref(), value)
}

fn set_path_inner(root: Value, path: &[Value], value: Value) -> Result<Value, RunError> {
    let Some((head, tail)) = path.split_first() else {
        return Ok(value);
    };
    match head {
        Value::String(key) => {
            use std::collections::BTreeMap;
            let mut map: BTreeMap<String, Value> = match root {
                Value::Object(m) => (*m).clone(),
                Value::Null => BTreeMap::new(),
                other => return Err(type_err("object or null", &other)),
            };
            let child = map.remove(key.as_ref()).unwrap_or(Value::Null);
            let replaced = set_path_inner(child, tail, value)?;
            map.insert(key.to_string(), replaced);
            Ok(Value::Object(Arc::new(map)))
        }
        Value::Number(n) => {
            let mut arr: Vec<Value> = match root {
                Value::Array(a) => (*a).clone(),
                Value::Null => Vec::new(),
                other => return Err(type_err("array or null", &other)),
            };
            let i = *n as i64;
            let idx = if i < 0 {
                (arr.len() as i64 + i).max(0)
            } else {
                i
            } as usize;
            while arr.len() <= idx {
                arr.push(Value::Null);
            }
            arr[idx] = set_path_inner(arr[idx].clone(), tail, value)?;
            Ok(Value::Array(Arc::new(arr)))
        }
        other => Err(type_err("string or number path step", other)),
    }
}

// ---- markdown: toc ---------------------------------------------------------

/// `toc` yields `[{level, text, anchor}, ...]` for every heading.
fn toc(input: &Value) -> Value {
    let mut headings = Vec::new();
    collect_headings(input, &mut headings);
    let entries: Vec<Value> = headings.into_iter().map(heading_to_entry).collect();
    Value::Array(Arc::new(entries))
}

fn collect_headings(v: &Value, out: &mut Vec<Arc<Node>>) {
    if let Value::Node(n) = v {
        if n.kind == NodeKind::Heading {
            out.push(n.clone());
        }
        for c in &n.children {
            collect_headings(c, out);
        }
    }
}

/// `node(obj)` lifts an object of shape
/// `{kind, <attrs>..., children?}` into a freshly constructed Node.
/// Unknown `.kind` strings default to `paragraph`. The result is
/// dirty so serialize regenerates it.
fn build_node(args: &[Expr], input: Value, env: &Env) -> Result<Value, RunError> {
    let source = match args.first() {
        Some(arg) => eval_first(arg, &input, env)?.unwrap_or(Value::Null),
        None => input,
    };
    let Value::Object(map) = source else {
        return Err(type_err("object", &source));
    };
    let kind = match map.get("kind") {
        Some(Value::String(s)) => NodeKind::from_name(s).unwrap_or(NodeKind::Paragraph),
        _ => NodeKind::Paragraph,
    };
    let mut node = Node::new(kind);
    for (k, v) in map.iter() {
        if k == "kind" || k == "children" {
            continue;
        }
        if let Some(key) = attr::by_name(k) {
            node = node.with_attr(key, v.clone());
        }
    }
    if let Some(Value::Array(arr)) = map.get("children") {
        node.children.clone_from(arr);
    }
    node.dirty = true;
    Ok(Value::Node(Arc::new(node)))
}

/// `frontmatter` returns the parsed YAML/TOML metadata block, or
/// `null` when the document has none.
fn frontmatter(input: &Value) -> Value {
    match input {
        Value::Node(n) => n
            .attrs
            .get(attr::FRONTMATTER)
            .cloned()
            .unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

fn heading_to_entry(node: Arc<Node>) -> Value {
    use std::collections::BTreeMap;
    let mut obj: BTreeMap<String, Value> = BTreeMap::new();
    if let Some(level) = node.attrs.get(attr::LEVEL).cloned() {
        obj.insert("level".into(), level);
    }
    obj.insert("text".into(), Value::from(plain_text(&node.children)));
    if let Some(a) = node.attrs.get(attr::ANCHOR).cloned() {
        obj.insert("anchor".into(), a);
    }
    Value::Object(Arc::new(obj))
}

/// `walk(f)` recursively applies `f` to every value in the input,
/// bottom-up. Mirrors jq's stdlib `def walk(f): ...` but handles
/// `Value::Node` by walking children and preserving Arc identity for
/// untouched subtrees so the byte-exact serialiser keeps clean spans.
fn walk_call(args: &[Expr], input: Value, env: &Env) -> Stream {
    if args.len() != 1 {
        return err(RunError::Other("walk/1: expected one argument".into()));
    }
    match walk_value(input, &args[0], env) {
        Ok(v) => ok(v),
        Err(e) => err(e),
    }
}

fn walk_value(v: Value, f: &Expr, env: &Env) -> Result<Value, RunError> {
    let inner = match &v {
        Value::Array(a) => {
            let mut new_arr: Vec<Value> = Vec::with_capacity(a.len());
            let mut changed = false;
            for c in a.iter() {
                let new_c = walk_value(c.clone(), f, env)?;
                if !value_id_eq(&new_c, c) {
                    changed = true;
                }
                new_arr.push(new_c);
            }
            if changed {
                Value::Array(Arc::new(new_arr))
            } else {
                v.clone()
            }
        }
        Value::Object(m) => {
            let mut new_map: BTreeMap<String, Value> = BTreeMap::new();
            let mut changed = false;
            for (k, val) in m.iter() {
                let new_v = walk_value(val.clone(), f, env)?;
                if !value_id_eq(&new_v, val) {
                    changed = true;
                }
                new_map.insert(k.clone(), new_v);
            }
            if changed {
                Value::Object(Arc::new(new_map))
            } else {
                v.clone()
            }
        }
        Value::Node(n) => {
            let mut new_children: Vec<Value> = Vec::with_capacity(n.children.len());
            let mut changed = false;
            for c in &n.children {
                let new_c = walk_value(c.clone(), f, env)?;
                if !value_id_eq(&new_c, c) {
                    changed = true;
                }
                new_children.push(new_c);
            }
            if changed {
                let mut new_node = (**n).clone();
                new_node.children = new_children;
                // Children changed; the original span no longer
                // describes the right bytes. Dirty so the serialiser
                // regenerates from events rather than copying source.
                new_node.dirty = true;
                Value::Node(Arc::new(new_node))
            } else {
                v.clone()
            }
        }
        _ => v.clone(),
    };
    let result = eval::eval(f, inner.clone(), env)
        .next()
        .transpose()?
        .ok_or_else(|| RunError::Other("walk: filter produced no output".into()))?;
    if let (Value::Node(orig), Value::Node(after)) = (&inner, &result) {
        if !Arc::ptr_eq(orig, after) && !after.dirty {
            let mut clone = (**after).clone();
            clone.dirty = true;
            return Ok(Value::Node(Arc::new(clone)));
        }
    }
    Ok(result)
}

fn value_id_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Node(x), Value::Node(y)) => Arc::ptr_eq(x, y),
        (Value::Array(x), Value::Array(y)) => Arc::ptr_eq(x, y),
        (Value::Object(x), Value::Object(y)) => Arc::ptr_eq(x, y),
        _ => eval::value_cmp_for_sort(a, b).is_eq(),
    }
}
