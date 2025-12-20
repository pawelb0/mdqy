//! Expression AST. Parser builds it, evaluator and stream runner walk it.

use std::sync::Arc;

/// `==`, `!=`, `<`, `<=`, `>`, `>=`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp { Eq, Ne, Lt, Le, Gt, Ge }

/// Arithmetic, boolean, and jq's `//` default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp { Add, Sub, Mul, Div, Mod, And, Or, Alt }

/// `=` and `|=`. Parser accepts both; the mutator only implements
/// `|=` today and errors on `=`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp { Set, Update }

#[derive(Debug, Clone)]
pub enum Literal {
    Null,
    Bool(bool),
    Number(f64),
    String(Arc<str>),
}

/// Compiled expression.
///
/// Boxed children keep the enum a small fixed size. Every runtime
/// walker matches this shape, so the variant set is the authoritative
/// grammar.
#[derive(Debug, Clone)]
pub enum Expr {
    Identity,
    RecurseAll,
    Field(Arc<str>),
    Index(Box<Expr>),
    Slice(Option<Box<Expr>>, Option<Box<Expr>>),
    Iterate,
    Pipe(Box<Expr>, Box<Expr>),
    Comma(Box<Expr>, Box<Expr>),
    Lit(Literal),
    ArrayCtor(Box<Expr>),
    ObjectCtor(Vec<(ObjKey, Expr)>),
    Cmp(Box<Expr>, CmpOp, Box<Expr>),
    Bin(Box<Expr>, BinOp, Box<Expr>),
    Neg(Box<Expr>),
    Not(Box<Expr>),
    If { branches: Vec<(Expr, Expr)>, else_branch: Option<Box<Expr>> },
    Var(Arc<str>),
    Call { name: Arc<str>, args: Vec<Expr> },
    Try(Box<Expr>),
    Assign(Box<Expr>, AssignOp, Box<Expr>),
    /// `expr as $x | rest`. Binds each output of `expr` to `$x`
    /// inside `rest`.
    As { bind: Box<Expr>, name: Arc<str>, body: Box<Expr> },
    /// `reduce SRC as $x (INIT; UPDATE)`. Folds `SRC` into one value,
    /// starting at `INIT` and replacing the accumulator with each
    /// `UPDATE` output.
    Reduce {
        source: Box<Expr>,
        var: Arc<str>,
        init: Box<Expr>,
        update: Box<Expr>,
    },
    /// `foreach SRC as $x (INIT; UPDATE; EXTRACT)`. Same fold as
    /// `Reduce`, but yields `EXTRACT(acc)` at every step instead of
    /// only the final accumulator.
    Foreach {
        source: Box<Expr>,
        var: Arc<str>,
        init: Box<Expr>,
        update: Box<Expr>,
        extract: Box<Expr>,
    },
    /// `def name(params): body;`. User-defined function. Each `param`
    /// is available inside `body` as a filter bound to the
    /// corresponding call-site argument.
    Def {
        name: Arc<str>,
        params: Vec<Arc<str>>,
        body: Box<Expr>,
        rest: Box<Expr>,
    },
}

/// Key of an object-ctor entry.
#[derive(Debug, Clone)]
pub enum ObjKey {
    Ident(Arc<str>),
    Str(Arc<str>),
    Expr(Expr),
}
