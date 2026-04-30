//! Stream-vs-tree dispatch.
//!
//! The stream predicate must accept only expressions tree mode also
//! handles correctly; `tests/queries.rs::stream_and_tree_agree`
//! enforces that.

use std::sync::Arc;

use crate::ast::NodeKind;
use crate::expr::{CmpOp, Expr, Literal};

/// Which runner handles the query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Stream,
    Tree,
}

/// Instructions for the stream runner. Accepts the shape
/// `KIND_CALL | [select(.level == N) |] .ATTR`, which covers the
/// common read queries: `headings | .text`, `codeblocks | .lang`,
/// `h1 | .text`, `headings | select(.level == 1) | .text`.
#[derive(Debug, Clone)]
pub struct StreamPlan {
    pub kind: NodeKind,
    /// If set, the element must match this heading level.
    pub level_eq: Option<i64>,
    pub emit: EmitKind,
}

/// What to emit for each matched element.
#[derive(Debug, Clone)]
pub enum EmitKind {
    /// Plaintext of descendants.
    Text,
    /// Plaintext of descendants, slugified GFM-style.
    Anchor,
    /// Scalar derived from the start-tag fields. See
    /// [`crate::stream`] for which attrs each `Tag` variant exposes.
    Attr(Arc<str>),
}

/// Pick the mode. Called once per compiled query; the result is
/// cached on `Query`.
#[must_use]
pub fn choose_mode(expr: &Expr) -> Mode {
    if has_mutation(expr) {
        return Mode::Tree;
    }
    if plan(expr).is_some() {
        Mode::Stream
    } else {
        Mode::Tree
    }
}

/// Build a [`StreamPlan`] if `expr` fits the stream-safe grammar.
#[must_use]
pub fn plan(expr: &Expr) -> Option<StreamPlan> {
    let stages = unfold_pipeline(expr);
    let (kind_stage, attr_stage) = match stages.as_slice() {
        [k, a] => (k, a),
        [k, mid, a] => {
            let constraint = extract_level_select(mid)?;
            let (kind, level) = kind_source(k)?;
            if kind != NodeKind::Heading || level.is_some_and(|l| l != constraint) {
                return None;
            }
            let emit = emit_for(kind, &as_field(a)?)?;
            return Some(StreamPlan {
                kind,
                level_eq: Some(constraint),
                emit,
            });
        }
        _ => return None,
    };
    let (kind, level_eq) = kind_source(kind_stage)?;
    let emit = emit_for(kind, &as_field(attr_stage)?)?;
    Some(StreamPlan {
        kind,
        level_eq,
        emit,
    })
}

/// `true` if `expr` matches the grammar `mutate::transform_bytes`
/// actually handles: `Assign`, `walk(...)`, `del(...)`, reachable
/// through `Pipe`/`Comma`. Anything else (assignments inside object
/// constructors, `if`-branches, `reduce`/`foreach` updates, function
/// args, etc.) targets local values and runs through eval instead.
/// Used by [`Query::is_read_only`](crate::Query::is_read_only) and by
/// the CLI to pick between query and transform paths.
#[must_use]
pub fn has_mutation(expr: &Expr) -> bool {
    match expr {
        Expr::Assign(..) => true,
        Expr::Call { name, .. } => matches!(name.as_ref(), "walk" | "del"),
        Expr::Pipe(a, b) | Expr::Comma(a, b) => has_mutation(a) || has_mutation(b),
        _ => false,
    }
}


fn unfold_pipeline(expr: &Expr) -> Vec<&Expr> {
    let mut out = Vec::new();
    unfold(expr, &mut out);
    out
}

fn unfold<'a>(expr: &'a Expr, out: &mut Vec<&'a Expr>) {
    if let Expr::Pipe(a, b) = expr {
        unfold(a, out);
        unfold(b, out);
    } else {
        out.push(expr);
    }
}

/// Match a kind-producing call like `headings`, `codeblocks`, or
/// `hN`. Returns `(kind, level)` where `level` is set only for `hN`.
fn kind_source(expr: &Expr) -> Option<(NodeKind, Option<i64>)> {
    let Expr::Call { name, args } = expr else {
        return None;
    };
    if !args.is_empty() {
        return None;
    }
    let n = name.as_ref();
    let kind = match n {
        "headings" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => NodeKind::Heading,
        "paragraphs" => NodeKind::Paragraph,
        "codeblocks" | "code" => NodeKind::Code,
        "links" => NodeKind::Link,
        "images" => NodeKind::Image,
        "items" => NodeKind::Item,
        "lists" => NodeKind::List,
        "tables" => NodeKind::Table,
        "blockquotes" => NodeKind::Quote,
        _ => return None,
    };
    let level = n.strip_prefix('h').and_then(|s| s.parse::<i64>().ok());
    Some((kind, level))
}

/// Match `select(.level == N)`.
fn extract_level_select(expr: &Expr) -> Option<i64> {
    let Expr::Call { name, args } = expr else {
        return None;
    };
    if name.as_ref() != "select" || args.len() != 1 {
        return None;
    }
    let Expr::Cmp(lhs, CmpOp::Eq, rhs) = &args[0] else {
        return None;
    };
    let (field, lit) = match (lhs.as_ref(), rhs.as_ref()) {
        (Expr::Field(f), Expr::Lit(Literal::Number(n)))
        | (Expr::Lit(Literal::Number(n)), Expr::Field(f)) => (f, *n),
        _ => return None,
    };
    (field.as_ref() == "level" && lit.fract() == 0.0 && (1.0..=6.0).contains(&lit))
        .then_some(lit as i64)
}

fn as_field(expr: &Expr) -> Option<Arc<str>> {
    if let Expr::Field(name) = expr {
        Some(name.clone())
    } else {
        None
    }
}

fn emit_for(kind: NodeKind, attr: &str) -> Option<EmitKind> {
    use NodeKind::{Code, Heading, Image, Link};
    let attr_value = || EmitKind::Attr(Arc::from(attr));
    match (attr, kind) {
        ("text", _) | ("literal", Code) => Some(EmitKind::Text),
        ("anchor", Heading) => Some(EmitKind::Anchor),
        ("level", Heading) | ("lang", Code) | ("alt", Image) | ("kind", _) => Some(attr_value()),
        ("href" | "title", Link | Image) => Some(attr_value()),
        _ => None,
    }
}
