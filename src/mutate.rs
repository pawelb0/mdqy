//! Markdown write path. Eval mutates the tree; the serializer copies
//! clean spans verbatim and regenerates dirty ones.

use std::sync::Arc;

use crate::error::RunError;
use crate::eval::{self, Env};
use crate::events::build_tree_from_source;
use crate::expr::Expr;
use crate::value::Value;

/// Parse, mutate, serialise. Top-level entry for `--output md` and
/// `-U`. Eval runs `expr` against the parsed tree; the first output
/// must be a Node.
pub fn transform_bytes(expr: &Expr, source: &[u8]) -> Result<Vec<u8>, RunError> {
    let source_str =
        std::str::from_utf8(source).map_err(|e| RunError::Io(format!("source not utf-8: {e}")))?;
    let root = Arc::new(build_tree_from_source(source_str));
    let result = eval::eval(expr, Value::Node(root.clone()), &Env::default())
        .next()
        .transpose()?;
    let new_root = match result {
        Some(Value::Node(n)) => n,
        Some(other) => {
            return Err(RunError::Type {
                expected: "node".into(),
                got: other.type_name().into(),
            });
        }
        None => root,
    };
    let mut out = Vec::with_capacity(source.len());
    crate::emit::md::serialize(&mut out, source, &new_root)?;
    Ok(out)
}
