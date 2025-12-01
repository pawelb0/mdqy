//! Pick stream mode vs tree mode for a compiled expression.
//!
//! The stream predicate is narrow on purpose. Anything accepted here
//! must also run correctly under tree mode, and
//! `tests/queries.rs::stream_and_tree_agree` checks that exact
//! property on every expression we label stream-eligible.

use std::sync::Arc;

use crate::ast::NodeKind;
use crate::expr::{CmpOp, Expr, Literal};

/// Which runner handles the query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Stream,
    Tree,
}

/// Instructions for the stream runner.
///
/// We only accept the narrow shape `KIND_CALL | [select(.level == N) |] .ATTR`.
/// That's the hot path: `headings | .text`, `codeblocks | .lang`,
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
    // Flatten the pipeline left to right.
    let stages = unfold_pipeline(expr);
    if stages.len() < 2 {
        return None;
    }

    // Stage 0: kind source (`headings`, `codeblocks`, ..., `hN`).
    let (kind, mut level_eq) = stage0(stages[0])?;

    // Optional middle stage: `select(.level == N)`.
    let emit_stage_idx = if stages.len() == 3 {
        let level_constraint = extract_level_select(stages[1])?;
        // Only Headings carry a level; conflicting constraints are rejected.
        if kind != NodeKind::Heading {
            return None;
        }
        match (level_eq, level_constraint) {
            (Some(a), b) if a != b => return None,
            _ => level_eq = Some(level_constraint),
        }
        2
    } else if stages.len() == 2 {
        1
    } else {
        return None;
    };

    // Final stage: `.ATTR`, where ATTR is a recognised field/attr.
    let attr = as_field(stages[emit_stage_idx])?;
    let emit = emit_for(kind, &attr)?;
    Some(StreamPlan {
        kind,
        level_eq,
        emit,
    })
}

/// `true` if `expr` (or anything inside it) would mutate the tree.
/// Used by [`Query::is_read_only`](crate::Query::is_read_only) and by
/// the CLI to pick between query and transform paths.
#[must_use]
pub fn has_mutation(expr: &Expr) -> bool {
    match expr {
        Expr::Assign(..) | Expr::Delete(..) => true,
        Expr::Call { name, args } => {
            matches!(name.as_ref(), "walk" | "del") || args.iter().any(has_mutation)
        }
        Expr::Pipe(a, b) | Expr::Comma(a, b) | Expr::Cmp(a, _, b) | Expr::Bin(a, _, b) => {
            has_mutation(a) || has_mutation(b)
        }
        Expr::Neg(x) | Expr::Not(x) | Expr::Try(x) | Expr::ArrayCtor(x) | Expr::Index(x) => {
            has_mutation(x)
        }
        Expr::Slice(a, b) => [a, b].iter().any(|x| x.as_deref().is_some_and(has_mutation)),
        Expr::If { branches, else_branch } => {
            branches.iter().any(|(c, t)| has_mutation(c) || has_mutation(t))
                || else_branch.as_deref().is_some_and(has_mutation)
        }
        Expr::As { bind, body, .. } => has_mutation(bind) || has_mutation(body),
        Expr::ObjectCtor(entries) => entries.iter().any(|(_, v)| has_mutation(v)),
        Expr::Reduce { source, init, update, .. } => {
            has_mutation(source) || has_mutation(init) || has_mutation(update)
        }
        Expr::Foreach { source, init, update, extract, .. } => {
            has_mutation(source) || has_mutation(init) || has_mutation(update) || has_mutation(extract)
        }
        Expr::Def { body, rest, .. } => has_mutation(body) || has_mutation(rest),
        Expr::Identity
        | Expr::RecurseAll
        | Expr::Field(_)
        | Expr::Iterate
        | Expr::Lit(_)
        | Expr::Var(_) => false,
    }
}

// ---- helpers ----------------------------------------------------------------

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

/// Match the first pipeline stage: a kind-producing call like
/// `headings`, `codeblocks`, or `hN`. Returns `(kind, level)` where
/// `level` is set only for `hN`.
fn stage0(expr: &Expr) -> Option<(NodeKind, Option<i64>)> {
    let Expr::Call { name, args } = expr else { return None };
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

/// Match an optional middle stage of the form `select(.level == N)`.
fn extract_level_select(expr: &Expr) -> Option<i64> {
    let Expr::Call { name, args } = expr else { return None };
    if name.as_ref() != "select" || args.len() != 1 {
        return None;
    }
    let Expr::Cmp(lhs, CmpOp::Eq, rhs) = &args[0] else { return None };
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
