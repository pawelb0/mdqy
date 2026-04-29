//! Convert between pulldown-cmark events and [`Node`] trees.
//!
//! Forward: [`build_tree_from_source`] runs the parser with byte
//! offsets and folds events into a Node tree.
//!
//! Reverse: [`node_to_events_borrowed`] (used by the md serializer)
//! and [`node_to_events_owned`] (used by the tty emitter) turn a
//! subtree back into an event stream, so `pulldown-cmark-to-cmark`
//! and `mdcat::push_tty` can consume it directly.
//!
//! [`options`] is the shared parse configuration. Matches
//! `mdcat::markdown_options()` so rendering and querying agree on
//! what the document is.

use std::ops::Range;
use std::sync::Arc;

use pulldown_cmark::{
    Alignment, CodeBlockKind, CowStr, Event, LinkType, MetadataBlockKind, Options, Parser, Tag,
    TagEnd,
};

use crate::ast::{attr, Node, NodeKind, Span};
use crate::value::Value;

/// CommonMark plus the GFM extensions mdqy supports. Exposed so
/// callers that build their own `Parser` stay in sync with us.
#[must_use]
pub fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_SMART_PUNCTUATION
        | Options::ENABLE_GFM
        | Options::ENABLE_DEFINITION_LIST
        | Options::ENABLE_WIKILINKS
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS
}

/// Parse markdown and build a tree. Every node gets a byte span.
#[must_use]
pub fn build_tree_from_source(source: &str) -> Node {
    let parser = Parser::new_ext(source, options());
    let mut root = build(parser.into_offset_iter());
    root.span = Some(Span {
        start: 0,
        end: source.len(),
    });
    attach_frontmatter(&mut root, source);
    root
}

/// Build a tree from an event iterator without offsets. All spans
/// come out `None`; use [`build_tree_from_source`] if the serializer
/// needs to byte-copy clean subtrees.
pub fn build_tree<'a, I: Iterator<Item = Event<'a>>>(events: I) -> Node {
    build(events.map(|e| (e, 0..0)))
}

/// Flat plaintext of a children slice.
///
/// Backs `.text` field access, the `text` field in JSON output, and
/// the stream runner's accumulator. Soft breaks become spaces, hard
/// breaks become `\n`.
#[must_use]
pub fn plain_text(children: &[Value]) -> String {
    let mut out = String::new();
    push_plain(&mut out, children);
    out
}

/// Borrowing event stream for `node` and its descendants. Feeds
/// straight into `pulldown-cmark-to-cmark`.
#[must_use]
pub fn node_to_events_borrowed(node: &Node) -> Vec<Event<'_>> {
    let mut out = Vec::new();
    emit_events(node, &mut out);
    out
}

/// Same as [`node_to_events_borrowed`] but with owned strings. The
/// tty emitter needs `Event<'static>` because its writer outlives
/// the source buffer.
#[cfg(feature = "tty")]
#[must_use]
pub fn node_to_events_owned(node: &Node) -> Vec<Event<'static>> {
    node_to_events_borrowed(node)
        .into_iter()
        .map(Event::into_static)
        .collect()
}

// ---- events to Node --------------------------------------------------------

fn build<'a, I>(events: I) -> Node
where
    I: Iterator<Item = (Event<'a>, Range<usize>)>,
{
    let mut stack: Vec<Node> = vec![Node::new(NodeKind::Root)];

    for (event, range) in events {
        let span = Some(Span {
            start: range.start,
            end: range.end,
        });
        match event {
            Event::Start(tag) => {
                let mut node = start_node(&tag);
                node.span = span;
                stack.push(node);
            }
            Event::End(end) => {
                let mut node = stack.pop().expect("balanced start/end");
                if let Some(s) = node.span.as_mut() {
                    s.end = range.end;
                }
                finalize_node(&mut node, end);
                push_child(&mut stack, node);
            }
            Event::Text(t) => push_leaf(&mut stack, NodeKind::Text, Some(&t), span),
            Event::Code(t) => push_leaf(&mut stack, NodeKind::CodeInline, Some(&t), span),
            Event::Html(t) => push_leaf(&mut stack, NodeKind::Html, Some(&t), span),
            Event::InlineHtml(t) => push_leaf(&mut stack, NodeKind::HtmlInline, Some(&t), span),
            Event::InlineMath(t) | Event::DisplayMath(t) => {
                push_leaf(&mut stack, NodeKind::CodeInline, Some(&t), span);
            }
            Event::FootnoteReference(label) => {
                push_leaf(&mut stack, NodeKind::FootnoteRef, Some(&label), span);
            }
            Event::SoftBreak => push_leaf(&mut stack, NodeKind::BreakSoft, None, span),
            Event::HardBreak => push_leaf(&mut stack, NodeKind::BreakHard, None, span),
            Event::Rule => push_leaf(&mut stack, NodeKind::Rule, None, span),
            Event::TaskListMarker(checked) => {
                if let Some(parent) = stack.last_mut() {
                    if parent.kind == NodeKind::Item {
                        parent.attrs.insert(attr::CHECKED, Value::Bool(checked));
                    }
                }
            }
        }
    }

    debug_assert_eq!(stack.len(), 1, "tree must finish with only the root");
    stack.pop().expect("root node present")
}

/// Build a leaf node and append it under the current parent.
fn push_leaf(stack: &mut [Node], kind: NodeKind, value: Option<&str>, span: Option<Span>) {
    let mut n = match value {
        Some(t) => Node::new(kind).with_attr(attr::VALUE, t.to_string()),
        None => Node::new(kind),
    };
    n.span = span;
    push_child(stack, n);
}

fn push_child(stack: &mut [Node], node: Node) {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(Value::Node(Arc::new(node)));
    }
}

fn start_node(tag: &Tag<'_>) -> Node {
    // Tags with attributes handle themselves; everything else falls
    // through to `simple_kind(tag)` for a plain NodeKind.
    match tag {
        Tag::Heading { level, id, .. } => {
            let n = Node::new(NodeKind::Heading).with_attr(attr::LEVEL, i64::from(*level as u8));
            match id {
                Some(id) => n.with_attr(attr::ANCHOR, id.to_string()),
                None => n,
            }
        }
        Tag::CodeBlock(CodeBlockKind::Fenced(lang)) => {
            Node::new(NodeKind::Code).with_attr(attr::LANG, lang.to_string())
        }
        Tag::List(start) => {
            let mut n =
                Node::new(NodeKind::List).with_attr(attr::ORDERED, Value::Bool(start.is_some()));
            if let Some(s) = start {
                n = n.with_attr(attr::START, i64::try_from(*s).unwrap_or(0));
            }
            n
        }
        Tag::FootnoteDefinition(label) => {
            Node::new(NodeKind::FootnoteDef).with_attr(attr::VALUE, label.to_string())
        }
        Tag::Table(aligns) => {
            let arr: Vec<Value> = aligns
                .iter()
                .map(|a| Value::from(alignment_str(*a)))
                .collect();
            Node::new(NodeKind::Table).with_attr(attr::ALIGNS, Value::Array(Arc::new(arr)))
        }
        Tag::Link {
            link_type,
            dest_url,
            title,
            ..
        } => link_like(NodeKind::Link, *link_type, dest_url, title),
        Tag::Image {
            link_type,
            dest_url,
            title,
            ..
        } => link_like(NodeKind::Image, *link_type, dest_url, title),
        Tag::MetadataBlock(kind) => Node::new(NodeKind::Html).with_attr(
            attr::LANG,
            match kind {
                MetadataBlockKind::YamlStyle => "yaml",
                MetadataBlockKind::PlusesStyle => "toml",
            },
        ),
        _ => Node::new(simple_kind(tag)),
    }
}

/// Tag kinds that carry no attributes. Fallback for `start_node`.
fn simple_kind(tag: &Tag<'_>) -> NodeKind {
    match tag {
        Tag::BlockQuote(_) => NodeKind::Quote,
        Tag::CodeBlock(_) => NodeKind::Code,
        Tag::HtmlBlock => NodeKind::Html,
        Tag::Item => NodeKind::Item,
        Tag::TableHead | Tag::TableRow => NodeKind::Row,
        Tag::TableCell => NodeKind::Cell,
        Tag::Emphasis | Tag::Superscript | Tag::Subscript => NodeKind::Emphasis,
        Tag::Strong => NodeKind::Strong,
        Tag::Strikethrough => NodeKind::Strikethrough,
        Tag::DefinitionList | Tag::DefinitionListTitle | Tag::DefinitionListDefinition => {
            NodeKind::List
        }
        _ => NodeKind::Paragraph,
    }
}

fn link_like(kind: NodeKind, lt: LinkType, dest: &CowStr<'_>, title: &CowStr<'_>) -> Node {
    let mut n = Node::new(kind)
        .with_attr(attr::HREF, dest.to_string())
        .with_attr(attr::KIND_DETAIL, link_type_str(lt));
    if !title.is_empty() {
        n = n.with_attr(attr::TITLE, title.to_string());
    }
    n
}

fn finalize_node(node: &mut Node, _: TagEnd) {
    if node.kind == NodeKind::Heading && !node.attrs.contains_key(attr::ANCHOR) {
        let anchor = slug::slugify(plain_text(&node.children));
        node.attrs.insert(attr::ANCHOR, Value::from(anchor));
    }
    if node.kind == NodeKind::Code {
        let literal = plain_text(&node.children);
        node.attrs.insert(attr::LITERAL, Value::from(literal));
        node.children.clear();
    }
    if node.kind == NodeKind::Image {
        let alt = plain_text(&node.children);
        node.attrs.insert(attr::ALT, Value::from(alt));
    }
}

fn push_plain(out: &mut String, children: &[Value]) {
    for v in children {
        match v {
            Value::Node(n) => match n.kind {
                NodeKind::Text | NodeKind::CodeInline => {
                    if let Some(Value::String(s)) = n.attrs.get(attr::VALUE) {
                        out.push_str(s);
                    }
                }
                NodeKind::BreakSoft => out.push(' '),
                NodeKind::BreakHard => out.push('\n'),
                _ => push_plain(out, &n.children),
            },
            Value::String(s) => out.push_str(s),
            _ => {}
        }
    }
}

fn alignment_str(a: Alignment) -> &'static str {
    match a {
        Alignment::None => "none",
        Alignment::Left => "left",
        Alignment::Center => "center",
        Alignment::Right => "right",
    }
}

fn link_type_str(lt: LinkType) -> &'static str {
    match lt {
        LinkType::Inline => "inline",
        LinkType::Reference | LinkType::ReferenceUnknown => "reference",
        LinkType::Collapsed | LinkType::CollapsedUnknown => "collapsed",
        LinkType::Shortcut | LinkType::ShortcutUnknown => "shortcut",
        LinkType::Autolink => "autolink",
        LinkType::Email => "email",
        LinkType::WikiLink { .. } => "wikilink",
    }
}

// ---- Node to events --------------------------------------------------------

fn emit_events<'a>(node: &'a Node, out: &mut Vec<Event<'a>>) {
    use pulldown_cmark::{BlockQuoteKind, HeadingLevel as HL};

    let level = |n: &Node| -> HL {
        HL::try_from(heading_level_i64(n).clamp(1, 6) as usize).unwrap_or(HL::H1)
    };
    let val = |k| owned_cow(string_attr(node, k));
    let href_title = || (val(attr::HREF), val(attr::TITLE));
    // Leaf events return immediately; containers fall through to the
    // `(start, end)` tuple below.
    let (start, end) = match node.kind {
        NodeKind::Root | NodeKind::Section => return emit_children(node, out),
        NodeKind::Text => return out.push(Event::Text(val(attr::VALUE))),
        NodeKind::CodeInline => return out.push(Event::Code(val(attr::VALUE))),
        NodeKind::Html => return out.push(Event::Html(val(attr::VALUE))),
        NodeKind::HtmlInline => return out.push(Event::InlineHtml(val(attr::VALUE))),
        NodeKind::BreakSoft => return out.push(Event::SoftBreak),
        NodeKind::BreakHard => return out.push(Event::HardBreak),
        NodeKind::Rule => return out.push(Event::Rule),
        NodeKind::FootnoteRef => return out.push(Event::FootnoteReference(val(attr::VALUE))),
        NodeKind::Code => {
            out.push(Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(val(
                attr::LANG,
            )))));
            if let Some(Value::String(lit)) = node.attrs.get(attr::LITERAL) {
                out.push(Event::Text(CowStr::Boxed(lit.to_string().into_boxed_str())));
            }
            out.push(Event::End(TagEnd::CodeBlock));
            return;
        }
        NodeKind::Heading => {
            let lv = level(node);
            let tag = Tag::Heading {
                level: lv,
                id: None,
                classes: vec![],
                attrs: vec![],
            };
            (Some(tag), Some(TagEnd::Heading(lv)))
        }
        NodeKind::Paragraph => (Some(Tag::Paragraph), Some(TagEnd::Paragraph)),
        NodeKind::Quote => (
            Some(Tag::BlockQuote(None::<BlockQuoteKind>)),
            Some(TagEnd::BlockQuote(None)),
        ),
        NodeKind::List => {
            let ordered = matches!(node.attrs.get(attr::ORDERED), Some(Value::Bool(true)));
            let start = ordered.then(|| {
                if let Some(Value::Number(n)) = node.attrs.get(attr::START) {
                    u64::try_from(*n as i64).unwrap_or(1)
                } else {
                    1
                }
            });
            (Some(Tag::List(start)), Some(TagEnd::List(ordered)))
        }
        NodeKind::Item => (Some(Tag::Item), Some(TagEnd::Item)),
        NodeKind::Emphasis => (Some(Tag::Emphasis), Some(TagEnd::Emphasis)),
        NodeKind::Strong => (Some(Tag::Strong), Some(TagEnd::Strong)),
        NodeKind::Strikethrough => (Some(Tag::Strikethrough), Some(TagEnd::Strikethrough)),
        NodeKind::Link => {
            let (dest_url, title) = href_title();
            let tag = Tag::Link {
                link_type: LinkType::Inline,
                dest_url,
                title,
                id: CowStr::Borrowed(""),
            };
            (Some(tag), Some(TagEnd::Link))
        }
        NodeKind::Image => {
            let (dest_url, title) = href_title();
            let tag = Tag::Image {
                link_type: LinkType::Inline,
                dest_url,
                title,
                id: CowStr::Borrowed(""),
            };
            (Some(tag), Some(TagEnd::Image))
        }
        NodeKind::FootnoteDef => (
            Some(Tag::FootnoteDefinition(val(attr::VALUE))),
            Some(TagEnd::FootnoteDefinition),
        ),
        NodeKind::Table => (Some(Tag::Table(Vec::new())), Some(TagEnd::Table)),
        NodeKind::Row => (Some(Tag::TableRow), Some(TagEnd::TableRow)),
        NodeKind::Cell => (Some(Tag::TableCell), Some(TagEnd::TableCell)),
    };

    if let Some(tag) = start {
        out.push(Event::Start(tag));
    }
    emit_children(node, out);
    if let Some(e) = end {
        out.push(Event::End(e));
    }
}

fn emit_children<'a>(node: &'a Node, out: &mut Vec<Event<'a>>) {
    for child in &node.children {
        if let Value::Node(n) = child {
            emit_events(n, out);
        }
    }
}

fn owned_cow(s: String) -> CowStr<'static> {
    CowStr::Boxed(s.into_boxed_str())
}

fn string_attr(node: &Node, key: &'static str) -> String {
    match node.attrs.get(key) {
        Some(Value::String(s)) => s.to_string(),
        _ => String::new(),
    }
}

fn heading_level_i64(node: &Node) -> i64 {
    match node.attrs.get(attr::LEVEL) {
        Some(Value::Number(n)) => (*n as i64).clamp(1, 6),
        _ => 1,
    }
}

/// Parse any leading `---`/`+++` metadata block as YAML or TOML
/// and attach it to `root.attrs[FRONTMATTER]`. Parse failure leaves
/// the attr unset; the rest of the pipeline treats that as `null`.
fn attach_frontmatter(root: &mut Node, source: &str) {
    // pulldown-cmark emits MetadataBlock anywhere `---...---` appears;
    // we only honour it as frontmatter when it's the first block.
    let Some(Value::Node(metadata)) = root.children.first().cloned() else {
        return;
    };
    if metadata.kind != NodeKind::Html {
        return;
    }
    let Some(Value::String(flavour)) = metadata.attrs.get(attr::LANG) else {
        return;
    };
    let Some(span) = metadata.span else { return };
    let body = &source[span.start..span.end.min(source.len())];
    let inner = strip_fences(body);
    let parsed = match flavour.as_ref() {
        "yaml" => serde_yaml::from_str::<serde_json::Value>(inner).ok(),
        "toml" => toml::from_str::<serde_json::Value>(inner).ok(),
        _ => None,
    };
    if let Some(json) = parsed {
        root.attrs
            .insert(attr::FRONTMATTER, crate::emit::json::value_from_json(json));
    }
}

/// Drop the first and last lines of a `---\n...\n---\n` block.
fn strip_fences(body: &str) -> &str {
    let trimmed = body.trim_matches('\n');
    let after_open = trimmed.split_once('\n').map_or("", |(_, rest)| rest);
    after_open
        .rsplit_once('\n')
        .map_or(after_open, |(prefix, _)| prefix)
}

#[cfg(test)]
mod tests {
    //! Pin the Node attribute schema that downstream modules rely on
    //! (heading level/anchor, code lang/literal, item checked,
    //! link href/kind_detail). End-to-end queries live in
    //! `tests/queries.rs`.

    use super::*;

    /// Parse `source` and hand the first child Node to `check`.
    fn first_node<F: FnOnce(&Node)>(source: &str, check: F) {
        let root = build_tree_from_source(source);
        match &root.children[0] {
            Value::Node(n) => check(n),
            _ => panic!("expected a Node in children[0]"),
        }
    }

    #[test]
    fn build_tree_populates_expected_attrs() {
        first_node("# Hello World\n", |n| {
            assert!(
                matches!(n.attrs.get(attr::LEVEL), Some(Value::Number(x)) if (*x - 1.0).abs() < 1e-9)
            );
            assert!(
                matches!(n.attrs.get(attr::ANCHOR), Some(Value::String(s)) if s.as_ref() == "hello-world")
            );
        });
        first_node("```rust\nfn main() {}\n```\n", |n| {
            assert!(
                matches!(n.attrs.get(attr::LANG), Some(Value::String(s)) if s.as_ref() == "rust")
            );
            assert!(
                matches!(n.attrs.get(attr::LITERAL), Some(Value::String(s)) if s.as_ref() == "fn main() {}\n")
            );
        });
        first_node("- [x] done\n- [ ] open\n", |list| {
            let items: Vec<_> = list
                .children
                .iter()
                .filter_map(|v| {
                    if let Value::Node(n) = v {
                        Some(n)
                    } else {
                        None
                    }
                })
                .collect();
            assert!(matches!(
                items[0].attrs.get(attr::CHECKED),
                Some(Value::Bool(true))
            ));
            assert!(matches!(
                items[1].attrs.get(attr::CHECKED),
                Some(Value::Bool(false))
            ));
        });
        first_node("See [docs](https://example.com).\n", |para| {
            let link = para
                .children
                .iter()
                .find_map(|c| match c {
                    Value::Node(n) if n.kind == NodeKind::Link => Some(n),
                    _ => None,
                })
                .expect("link present");
            assert!(
                matches!(link.attrs.get(attr::HREF), Some(Value::String(s)) if s.as_ref() == "https://example.com")
            );
            assert!(
                matches!(link.attrs.get(attr::KIND_DETAIL), Some(Value::String(s)) if s.as_ref() == "inline")
            );
        });
    }
}
