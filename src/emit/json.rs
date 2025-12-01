//! JSON output.
//!
//! A Node serialises to `{ "kind": ..., <attr>: ..., "text": ...,
//! "children": [...] }`. Attrs sit at the top level (not nested) so a
//! downstream `jq` pipe can write `.level`, `.href`, ... directly.
//! `text` is the flattened plaintext of descendants; `children` is
//! omitted when empty; `span` appears only under `include_spans`.

use std::io;

use serde_json::{Map, Number, Value as J};

use crate::ast::Node;
use crate::error::RunError;
use crate::events::plain_text;
use crate::value::Value;

/// Flags from `--output json`. `compact` packs each result on one
/// line; `include_spans` adds the byte `span` object to every node.
#[derive(Debug, Clone, Copy, Default)]
pub struct JsonOptions {
    pub compact: bool,
    pub include_spans: bool,
}

/// Emit one value as JSON, followed by a newline.
pub fn emit<W: io::Write>(writer: &mut W, value: &Value, opts: JsonOptions) -> Result<(), RunError> {
    let json = value_to_json(value, opts);
    let result = if opts.compact {
        serde_json::to_writer(&mut *writer, &json)
    } else {
        serde_json::to_writer_pretty(&mut *writer, &json)
    };
    result.map_err(|e| RunError::Io(e.to_string()))?;
    writer.write_all(b"\n")?;
    Ok(())
}

/// Convert a `Value` to a `serde_json::Value` in the published shape.
#[must_use]
pub fn value_to_json(value: &Value, opts: JsonOptions) -> J {
    match value {
        Value::Null => J::Null,
        Value::Bool(b) => J::Bool(*b),
        Value::Number(n) => number_to_json(*n),
        Value::String(s) => J::String(s.to_string()),
        Value::Array(arr) => J::Array(arr.iter().map(|v| value_to_json(v, opts)).collect()),
        Value::Object(map) => J::Object(
            map.iter().map(|(k, v)| (k.clone(), value_to_json(v, opts))).collect(),
        ),
        Value::Node(node) => node_to_json(node, opts),
    }
}

fn node_to_json(node: &Node, opts: JsonOptions) -> J {
    let mut obj = Map::new();
    obj.insert("kind".into(), J::String(node.kind.as_str().into()));
    for (k, v) in &node.attrs {
        obj.insert((*k).to_string(), value_to_json(v, opts));
    }
    if !node.children.is_empty() {
        let text = plain_text(&node.children);
        if !text.is_empty() {
            obj.insert("text".into(), J::String(text));
        }
        obj.insert(
            "children".into(),
            J::Array(node.children.iter().map(|v| value_to_json(v, opts)).collect()),
        );
    }
    if opts.include_spans {
        if let Some(span) = node.span {
            let s: Map<String, J> = [("start", span.start), ("end", span.end)]
                .into_iter()
                .map(|(k, v)| (k.into(), J::Number(Number::from(v as u64))))
                .collect();
            obj.insert("span".into(), J::Object(s));
        }
    }
    J::Object(obj)
}

/// Integer-valued floats emit as JSON integers, so `.level == 1`
/// works downstream without writing `1.0`.
fn number_to_json(n: f64) -> J {
    if n.is_finite() && n.fract() == 0.0 && n >= i64::MIN as f64 && n <= i64::MAX as f64 {
        J::Number(Number::from(n as i64))
    } else {
        Number::from_f64(n).map_or(J::Null, J::Number)
    }
}
