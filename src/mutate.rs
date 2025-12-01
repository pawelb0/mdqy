//! Write path. Attribute-level mutation only, for now.
//!
//! Supported shapes:
//! ```text
//!   <SELECTOR> | .<attr> |= <f>    // or equivalently <SELECTOR>.<attr> |= <f>
//!   del(<SELECTOR>.<attr>)
//!   <mut1> | <mut2> | <mut3>       // pipe-chained
//! ```
//!
//! Structural mutation (`children |=`, `walk(f)`, insertions) is not
//! wired up yet. Attribute mutation alone covers the "rewrite all
//! http:// to https://" case, which is the headline use.
//!
//! The serializer in `emit::md` copies clean subtrees byte-for-byte
//! and only regenerates dirty ones, so a link rewrite touches just
//! the link span in the output file.

use std::collections::HashSet;
use std::sync::Arc;

use crate::ast::{attr, Node};
use crate::error::RunError;
use crate::eval::{self, Env};
use crate::events::build_tree_from_source;
use crate::expr::{AssignOp, Expr};
use crate::value::Value;

/// Parse, mutate, serialise. Top-level entry for `--output md`
/// and `-U`.
pub fn transform_bytes(expr: &Expr, source: &[u8]) -> Result<Vec<u8>, RunError> {
    let source_str = std::str::from_utf8(source)
        .map_err(|e| RunError::Io(format!("source not utf-8: {e}")))?;
    let root = build_tree_from_source(source_str);
    let mutated = apply_expr(expr, Arc::new(root))?;
    let mut out = Vec::with_capacity(source.len());
    crate::emit::md::serialize(&mut out, source, &mutated)?;
    Ok(out)
}

fn apply_expr(expr: &Expr, root: Arc<Node>) -> Result<Arc<Node>, RunError> {
    match expr {
        Expr::Identity => Ok(root),
        Expr::Pipe(a, b) => {
            let r1 = apply_expr(a, root)?;
            apply_expr(b, r1)
        }
        Expr::Assign(lhs, AssignOp::Update, rhs) => apply_update(lhs, rhs, root),
        Expr::Assign(_, AssignOp::Set, _) => Err(RunError::NotImplemented {
            feature: "`=` (use `|=`); v1 only supports update-style assignment",
        }),
        Expr::Delete(path) => apply_delete(path, root),
        // `del(...)` is parsed as a Call; intercept here because it has
        // mutation semantics, unlike every other builtin.
        Expr::Call { name, args } if name.as_ref() == "del" && args.len() == 1 => {
            apply_delete(&args[0], root)
        }
        // `walk(f)` likewise intercepts. Post-order: children recurse
        // first, then `f` runs against the updated node.
        Expr::Call { name, args } if name.as_ref() == "walk" && args.len() == 1 => {
            walk_tree(&args[0], root)
        }
        // Read-only subexpressions before a mutation are no-ops here.
        // Think `(headings) | ... |= f` where the `(headings)` side
        // exists for its results, not for mutation. Drop it.
        _ => Ok(root),
    }
}

/// Post-order walk. Children recurse first, then `f` runs at each
/// node. `f` can use mutation operators (`|=`, `del`) alongside
/// control flow; see [`apply_walk_f`] for the accepted shapes.
fn walk_tree(f: &Expr, node: Arc<Node>) -> Result<Arc<Node>, RunError> {
    let mut new_children = Vec::with_capacity(node.children.len());
    let mut descendant_mutated = false;
    for child in &node.children {
        if let Value::Node(arc) = child {
            let updated = walk_tree(f, arc.clone())?;
            descendant_mutated |= !Arc::ptr_eq(&updated, arc);
            new_children.push(Value::Node(updated));
        } else {
            new_children.push(child.clone());
        }
    }

    let mut current = (*node).clone();
    current.children = new_children;
    let current_arc = Arc::new(current);
    let mut updated = apply_walk_f(f, current_arc.clone())?;

    if descendant_mutated && Arc::ptr_eq(&updated, &current_arc) {
        // `f` left the node alone but we rebuilt children. Clone to
        // carry the new children through instead of dropping them.
        let mut n = (*current_arc).clone();
        n.dirty = true;
        updated = Arc::new(n);
    } else if !Arc::ptr_eq(&updated, &node) {
        let mut n = (*updated).clone();
        n.dirty = true;
        updated = Arc::new(n);
    }
    Ok(updated)
}

/// Mini-interpreter for `walk(f)`'s body. The eval in `crate::eval`
/// rejects `Assign`/`Delete` at compile time, so walk brings its own
/// mutation-aware walker. Read-only forms delegate to the normal
/// evaluator and expect a Node-valued result.
fn apply_walk_f(f: &Expr, node: Arc<Node>) -> Result<Arc<Node>, RunError> {
    match f {
        Expr::Identity => Ok(node),
        Expr::Pipe(a, b) => {
            let mid = apply_walk_f(a, node)?;
            apply_walk_f(b, mid)
        }
        Expr::If { branches, else_branch } => {
            for (cond, then_branch) in branches {
                match eval::eval(cond, Value::Node(node.clone()), &Env::default()).next() {
                    Some(Ok(v)) if v.truthy() => return apply_walk_f(then_branch, node),
                    Some(Err(e)) => return Err(e),
                    _ => {}
                }
            }
            match else_branch.as_deref() {
                Some(e) => apply_walk_f(e, node),
                None => Ok(node),
            }
        }
        Expr::Assign(lhs, AssignOp::Update, rhs) => {
            let (_, attr_name) = split_attr_lhs(lhs)?;
            let targets: HashSet<usize> = [Arc::as_ptr(&node) as usize].into_iter().collect();
            walk_and_update(node, &targets, &attr_name, &Op::Update(rhs))
        }
        Expr::Call { name, args } if name.as_ref() == "del" && args.len() == 1 => {
            let (_, attr_name) = split_attr_lhs(&args[0])?;
            let targets: HashSet<usize> = [Arc::as_ptr(&node) as usize].into_iter().collect();
            walk_and_update(node, &targets, &attr_name, &Op::Delete)
        }
        _ => {
            // Read-only path: eval against the node and demand a
            // Node in return. `.` and chained field access land here.
            match eval::eval(f, Value::Node(node.clone()), &Env::default()).next() {
                Some(Ok(Value::Node(n))) => Ok(n),
                Some(Ok(Value::Null)) => Ok(node),
                Some(Ok(other)) => Err(RunError::Type {
                    expected: "node".into(),
                    got: other.type_name().into(),
                }),
                Some(Err(e)) => Err(e),
                None => Ok(node),
            }
        }
    }
}

#[allow(dead_code)]
/// Conservative identity: same kind, same-length children, each child
/// shares an `Arc` with its counterpart. Not a deep equals; good
/// enough to detect "`f` didn't touch this node" without requiring
/// `Value` to implement `PartialEq` (it can't, `f64` blocks it).
fn nodes_equal(a: &Node, b: &Node) -> bool {
    a.kind == b.kind
        && a.children.len() == b.children.len()
        && a.children.iter().zip(b.children.iter()).all(|(x, y)| match (x, y) {
            (Value::Node(xn), Value::Node(yn)) => Arc::ptr_eq(xn, yn),
            _ => false,
        })
}

fn apply_update(lhs: &Expr, rhs: &Expr, root: Arc<Node>) -> Result<Arc<Node>, RunError> {
    apply_attr_op(lhs, root, Op::Update(rhs))
}

fn apply_delete(path: &Expr, root: Arc<Node>) -> Result<Arc<Node>, RunError> {
    apply_attr_op(path, root, Op::Delete)
}

enum Op<'a> {
    Update(&'a Expr),
    Delete,
}

fn apply_attr_op(path: &Expr, root: Arc<Node>, op: Op<'_>) -> Result<Arc<Node>, RunError> {
    let (selector, attr_name) = split_attr_lhs(path)?;
    let targets = collect_target_ptrs(&selector, &root)?;
    if targets.is_empty() {
        return Ok(root);
    }
    walk_and_update(root, &targets, &attr_name, &op)
}

/// Split a mutation target into `(selector, attribute)`.
///
/// The parser produces `<SELECTOR>.<attr>` as
/// `Pipe(SELECTOR, Field(attr))`, so we only handle that shape plus
/// the bare-field case. Anything else is rejected.
fn split_attr_lhs(expr: &Expr) -> Result<(Expr, String), RunError> {
    match expr {
        Expr::Field(name) => Ok((Expr::Identity, name.to_string())),
        Expr::Pipe(sel, tail) => match tail.as_ref() {
            Expr::Field(name) => Ok((sel.as_ref().clone(), name.to_string())),
            _ => Err(RunError::NotImplemented {
                feature: "mutation target must end in `.<attr>` (v1 scope)",
            }),
        },
        _ => Err(RunError::NotImplemented {
            feature: "mutation target shape not supported in v1",
        }),
    }
}

/// Evaluate `selector` against the root; collect the `Arc::as_ptr`
/// of every Node it yields. Non-Node outputs are a type error.
fn collect_target_ptrs(selector: &Expr, root: &Arc<Node>) -> Result<HashSet<usize>, RunError> {
    let env = Env::default();
    let stream = eval::eval(selector, Value::Node(root.clone()), &env);
    let mut ptrs = HashSet::new();
    for r in stream {
        match r? {
            Value::Node(n) => {
                ptrs.insert(Arc::as_ptr(&n) as usize);
            }
            _ => {
                // Non-Node target; unsupported for v1 attribute mutation.
                return Err(RunError::NotImplemented {
                    feature: "mutation target must resolve to Node values (v1 scope)",
                });
            }
        }
    }
    Ok(ptrs)
}

/// Clone the subtree rooted at `node`, applying `op` at every node
/// whose pointer is in `targets`. Ancestors stay clean (not marked
/// dirty). The serializer walks the tree picking the minimal dirty
/// subtrees to regenerate, so leaving ancestors clean is what keeps
/// unrelated output bytes untouched.
fn walk_and_update(
    node: Arc<Node>,
    targets: &HashSet<usize>,
    attr_name: &str,
    op: &Op<'_>,
) -> Result<Arc<Node>, RunError> {
    let is_target = targets.contains(&(Arc::as_ptr(&node) as usize));

    let mut new_children = Vec::with_capacity(node.children.len());
    let mut descendant_mutated = false;
    for child in &node.children {
        if let Value::Node(arc) = child {
            let updated = walk_and_update(arc.clone(), targets, attr_name, op)?;
            descendant_mutated |= !Arc::ptr_eq(&updated, arc);
            new_children.push(Value::Node(updated));
        } else {
            new_children.push(child.clone());
        }
    }

    if !is_target && !descendant_mutated {
        return Ok(node);
    }

    let mut new_node = (*node).clone();
    new_node.children = new_children;

    if is_target {
        let Some(key) = attr::by_name(attr_name) else {
            return Err(RunError::Other(format!(
                "unknown attribute `{attr_name}`; only canonical attrs are supported"
            )));
        };
        match op {
            Op::Delete => {
                new_node.attrs.remove(key);
            }
            Op::Update(rhs) => {
                let current = new_node.attrs.get(key).cloned().unwrap_or(Value::Null);
                let replacement = eval::eval(rhs, current, &Env::default())
                    .next()
                    .transpose()?
                    .unwrap_or(Value::Null);
                new_node.attrs.insert(key, replacement);
            }
        }
        new_node.dirty = true;
    }
    Ok(Arc::new(new_node))
}

