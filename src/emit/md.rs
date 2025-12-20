//! Markdown serializer.
//!
//! Three paths:
//!
//! 1. Clean subtree with a span: byte-copy from the source buffer.
//!    Fence style, list markers, blank lines, reference-link form
//!    all survive. Nothing round-trips through the AST.
//! 2. Dirty subtree with a span: replace that exact byte range with
//!    bytes regenerated via `pulldown-cmark-to-cmark`. Normalisation
//!    stays local to the mutated region.
//! 3. No span (synthetic `Section`, `--merge` virtual root):
//!    regenerate every child in order.

use std::io;

use pulldown_cmark_to_cmark::cmark;

use crate::ast::{Node, Span};
use crate::error::RunError;
use crate::events::node_to_events_borrowed;
use crate::value::Value;

/// Top-level entry. Node values run through [`serialize`]; raw
/// strings print as-is; anything else falls back to JSON.
pub fn emit<W: io::Write>(writer: &mut W, source: &str, value: &Value) -> Result<(), RunError> {
    match value {
        Value::Node(n) => serialize(writer, source.as_bytes(), n),
        Value::String(s) => write_line(writer, s.as_bytes()),
        other => {
            let json = crate::emit::json::value_to_json(
                other,
                crate::emit::json::JsonOptions::COMPACT,
            );
            serde_json::to_writer(&mut *writer, &json).map_err(|e| RunError::Io(e.to_string()))?;
            writer.write_all(b"\n")?;
            Ok(())
        }
    }
}

/// Serialize a Node tree back to markdown using the strategy
/// described in the module docs.
pub fn serialize<W: io::Write>(writer: &mut W, source: &[u8], root: &Node) -> Result<(), RunError> {
    let Some(root_span) = root.span else {
        for child in &root.children {
            if let Value::Node(n) = child {
                serialize(writer, source, n)?;
            }
        }
        return Ok(());
    };

    let mut segments: Vec<(Span, Vec<u8>)> = Vec::new();
    collect_dirty_segments(root, &mut segments)?;
    let bounds = root_span.start.min(source.len())..root_span.end.min(source.len());

    if segments.is_empty() && !root.dirty {
        return write_line(writer, &source[bounds]);
    }

    segments.sort_by_key(|(s, _)| s.start);
    let mut pos = bounds.start;
    for (span, replacement) in segments {
        let seg_start = span.start.clamp(pos, bounds.end);
        let seg_end = span.end.clamp(seg_start, bounds.end);
        writer.write_all(&source[pos..seg_start])?;
        writer.write_all(&replacement)?;
        pos = seg_end;
    }
    write_line(writer, &source[pos..bounds.end])
}

/// Walk the tree collecting the minimal dirty subtrees. Stops at
/// the first dirty node on any path; its replacement already covers
/// anything dirty below it.
fn collect_dirty_segments(node: &Node, out: &mut Vec<(Span, Vec<u8>)>) -> Result<(), RunError> {
    if node.dirty {
        if let Some(span) = node.span {
            let events = node_to_events_borrowed(node);
            let mut buf = String::new();
            cmark(events.iter(), &mut buf).map_err(|e| RunError::Other(format!("cmark: {e}")))?;
            out.push((span, buf.into_bytes()));
            return Ok(());
        }
    }
    for child in &node.children {
        if let Value::Node(n) = child {
            collect_dirty_segments(n, out)?;
        }
    }
    Ok(())
}

fn write_line<W: io::Write>(writer: &mut W, bytes: &[u8]) -> Result<(), RunError> {
    writer.write_all(bytes)?;
    if !bytes.ends_with(b"\n") {
        writer.write_all(b"\n")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{attr, NodeKind};
    use crate::events::build_tree_from_source;

    #[test]
    fn identity_is_byte_exact_for_clean_tree() {
        let src = "# Title\n\nPara with **bold** and [link](https://example.com).\n\n```rust\nfn main() {}\n```\n";
        let mut out = Vec::new();
        serialize(&mut out, src.as_bytes(), &build_tree_from_source(src)).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), src);
    }

    #[test]
    fn dirty_subtree_regenerates_in_place() {
        let src = "Hello [docs](http://example.com).\n";
        let mut root = build_tree_from_source(src);
        walk_mut(&mut root, |n| {
            if n.kind == NodeKind::Link {
                n.attrs.insert(attr::HREF, Value::from("https://example.com"));
                n.dirty = true;
            }
        });
        let mut out = Vec::new();
        serialize(&mut out, src.as_bytes(), &root).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("https://example.com") && text.starts_with("Hello "), "{text}");
    }

    fn walk_mut(node: &mut Node, f: impl Fn(&mut Node) + Copy) {
        f(node);
        for child in &mut node.children {
            if let Value::Node(arc) = child {
                let mut cloned = (**arc).clone();
                walk_mut(&mut cloned, f);
                *arc = std::sync::Arc::new(cloned);
            }
        }
    }
}
