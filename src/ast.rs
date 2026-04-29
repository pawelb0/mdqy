//! Node tree: kinds, spans, attribute keys.

use std::collections::BTreeMap;

use crate::value::Value;

/// Every node kind the tree builder emits.
///
/// One-to-one with pulldown-cmark block and inline variants, plus one
/// synthetic kind (`Section`) that the `section(...)` builtin returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Root,
    Heading,
    Paragraph,
    Code,
    Quote,
    List,
    Item,
    Table,
    Row,
    Cell,
    Link,
    Image,
    Emphasis,
    Strong,
    Strikethrough,
    Text,
    CodeInline,
    Html,
    HtmlInline,
    BreakSoft,
    BreakHard,
    Rule,
    FootnoteRef,
    FootnoteDef,
    /// Produced by `section(name)`.
    Section,
}

impl NodeKind {
    /// Lowercase name used in JSON output and `.kind == "..."` tests.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        KIND_NAMES
            .iter()
            .find(|(_, k)| *k == self)
            .map_or("unknown", |(s, _)| *s)
    }

    /// Inverse of [`Self::as_str`]. `None` for unknown names.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        KIND_NAMES.iter().find(|(s, _)| *s == name).map(|(_, k)| *k)
    }
}

/// Canonical (name, variant) table. One row per `NodeKind`, used by
/// both `as_str` and `from_name` so they can't drift.
const KIND_NAMES: &[(&str, NodeKind)] = &[
    ("root", NodeKind::Root),
    ("heading", NodeKind::Heading),
    ("paragraph", NodeKind::Paragraph),
    ("code", NodeKind::Code),
    ("quote", NodeKind::Quote),
    ("list", NodeKind::List),
    ("item", NodeKind::Item),
    ("table", NodeKind::Table),
    ("row", NodeKind::Row),
    ("cell", NodeKind::Cell),
    ("link", NodeKind::Link),
    ("image", NodeKind::Image),
    ("emphasis", NodeKind::Emphasis),
    ("strong", NodeKind::Strong),
    ("strikethrough", NodeKind::Strikethrough),
    ("text", NodeKind::Text),
    ("code_inline", NodeKind::CodeInline),
    ("html", NodeKind::Html),
    ("html_inline", NodeKind::HtmlInline),
    ("break_soft", NodeKind::BreakSoft),
    ("break_hard", NodeKind::BreakHard),
    ("rule", NodeKind::Rule),
    ("footnote_ref", NodeKind::FootnoteRef),
    ("footnote_def", NodeKind::FootnoteDef),
    ("section", NodeKind::Section),
];

/// Byte range into the parsed source. Used by the md serializer to
/// copy clean subtrees verbatim.
#[derive(Debug, Clone, Copy, serde::Serialize)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

/// One markdown node.
#[derive(Debug, Clone)]
pub struct Node {
    pub kind: NodeKind,
    pub attrs: BTreeMap<&'static str, Value>,
    pub children: Vec<Value>,
    pub span: Option<Span>,
    /// Set by the mutation path. A dirty node's span is stale, and the
    /// serializer regenerates the subtree instead of byte-slicing it.
    pub dirty: bool,
}

impl Node {
    /// Empty node of `kind`. No span, no children, no attrs.
    #[must_use]
    pub fn new(kind: NodeKind) -> Self {
        Self {
            kind,
            attrs: BTreeMap::new(),
            children: Vec::new(),
            span: None,
            dirty: false,
        }
    }

    /// Consuming builder. `Node::new(kind).with_attr(K, v).with_attr(...)`.
    #[must_use]
    pub fn with_attr(mut self, key: &'static str, value: impl Into<Value>) -> Self {
        self.attrs.insert(key, value.into());
        self
    }
}

/// Canonical attribute keys.
///
/// The tree stores attrs keyed by `&'static str`, so the parser,
/// evaluator, and emitter can't drift on the string form. Callers that
/// take user input (e.g. `.foo` field access) go through [`by_name`].
pub mod attr {
    pub const LEVEL: &str = "level";
    pub const ANCHOR: &str = "anchor";
    pub const LANG: &str = "lang";
    pub const LITERAL: &str = "literal";
    pub const HREF: &str = "href";
    pub const TITLE: &str = "title";
    pub const ALT: &str = "alt";
    pub const ORDERED: &str = "ordered";
    pub const START: &str = "start";
    pub const TIGHT: &str = "tight";
    pub const CHECKED: &str = "checked";
    pub const ALIGNS: &str = "aligns";
    pub const VALUE: &str = "value";
    /// Link flavour: `"inline"`, `"reference"`, `"autolink"`, or `"email"`.
    pub const KIND_DETAIL: &str = "kind_detail";
    pub const FRONTMATTER: &str = "frontmatter";

    /// Look up the canonical key for a user-facing name.
    #[must_use]
    pub fn by_name(name: &str) -> Option<&'static str> {
        Some(match name {
            "level" => LEVEL,
            "anchor" => ANCHOR,
            "lang" => LANG,
            "literal" => LITERAL,
            "href" => HREF,
            "title" => TITLE,
            "alt" => ALT,
            "ordered" => ORDERED,
            "start" => START,
            "tight" => TIGHT,
            "checked" => CHECKED,
            "aligns" => ALIGNS,
            "value" => VALUE,
            "kind_detail" => KIND_DETAIL,
            "frontmatter" => FRONTMATTER,
            _ => return None,
        })
    }

    /// Schema for a canonical attr. The mutation path uses this to
    /// reject writes that would render unobservably (for example, a
    /// string into `level`, which the heading emitter ignores).
    #[must_use]
    pub fn expected_type(key: &'static str) -> &'static str {
        match key {
            LEVEL | START => "number",
            ORDERED | TIGHT | CHECKED => "boolean",
            ALIGNS => "array",
            FRONTMATTER => "any",
            _ => "string",
        }
    }
}
