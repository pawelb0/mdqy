//! Event-stream evaluator.
//!
//! For the narrow class of queries that [`crate::analyze::plan`]
//! accepts, this runs straight off the `pulldown_cmark::Event`
//! iterator. No Node tree gets built, and allocations are O(matched
//! elements) instead of O(all nodes). Queries outside that class
//! fall through to [`crate::eval`], which builds the full tree.

use pulldown_cmark::{CodeBlockKind, Event, Tag, TagEnd};

use crate::analyze::{EmitKind, StreamPlan};
use crate::ast::NodeKind;
use crate::error::RunError;
use crate::expr::Expr;
use crate::value::Value;

type StreamItem = Result<Value, RunError>;

/// Run a stream-eligible query. `analyze::choose_mode` gates the
/// dispatch; the `analyze::plan` call here re-derives the plan and
/// fails loud if the two ever disagree.
pub fn run<'a, I>(
    expr: Expr,
    events: I,
) -> Box<dyn Iterator<Item = StreamItem> + 'a>
where
    I: Iterator<Item = Event<'a>> + 'a,
{
    let Some(plan) = crate::analyze::plan(&expr) else {
        return Box::new(std::iter::once(Err(RunError::NotImplemented {
            feature: "stream plan",
        })));
    };
    Box::new(StreamRunner::new(plan, events))
}

/// State machine driving the single-pass walk.
struct StreamRunner<I> {
    plan: StreamPlan,
    events: I,
    /// Nesting depth inside a matched element. `0` means we're not
    /// currently inside one.
    depth: usize,
    /// Text buffer for `Text` / `Anchor` emits.
    accum: String,
    /// Scalar captured at Start, drained at End. Bypasses the text
    /// buffer for attrs like `level` or `href` that come from the
    /// start tag itself.
    pending: Option<Value>,
    /// Values waiting to leave the iterator. Emitted at End, pulled
    /// one-per-`next()`.
    queued: Vec<Value>,
}

impl<'a, I> StreamRunner<I>
where
    I: Iterator<Item = Event<'a>>,
{
    fn new(plan: StreamPlan, events: I) -> Self {
        Self {
            plan,
            events,
            depth: 0,
            accum: String::new(),
            pending: None,
            queued: Vec::new(),
        }
    }

    fn feed(&mut self, event: Event<'a>) {
        let collecting = self.depth > 0 && self.needs_text();
        match event {
            Event::Start(tag) => self.on_start(tag),
            Event::End(end) => self.on_end(end),
            Event::Text(t) | Event::Code(t) if collecting => self.accum.push_str(&t),
            Event::SoftBreak if collecting => self.accum.push(' '),
            Event::HardBreak if collecting => self.accum.push('\n'),
            _ => {}
        }
    }

    fn needs_text(&self) -> bool {
        matches!(self.plan.emit, EmitKind::Text | EmitKind::Anchor)
    }

    fn on_start(&mut self, tag: Tag<'a>) {
        // Inside a matched element we only count nesting, so a nested
        // block's End doesn't close the outer match.
        if self.depth > 0 {
            if is_block(&tag) {
                self.depth += 1;
            }
            return;
        }

        let Some(kind) = tag_kind(&tag) else {
            return;
        };
        if kind != self.plan.kind {
            return;
        }

        if let Some(expected) = self.plan.level_eq {
            if kind != NodeKind::Heading || heading_level(&tag) != expected {
                return;
            }
        }

        self.depth = 1;
        self.accum.clear();
        self.pending = None;

        match &self.plan.emit {
            EmitKind::Text | EmitKind::Anchor => {}
            EmitKind::Attr(name) => {
                if let Some(v) = scalar_from_start(&tag, name) {
                    self.pending = Some(v);
                }
            }
        }
    }

    fn on_end(&mut self, _end: TagEnd) {
        if self.depth == 0 {
            return;
        }
        self.depth -= 1;
        if self.depth > 0 {
            return;
        }
        let emitted = match &self.plan.emit {
            EmitKind::Text => Some(Value::from(std::mem::take(&mut self.accum))),
            EmitKind::Anchor => Some(Value::from(slug::slugify(std::mem::take(&mut self.accum)))),
            EmitKind::Attr(_) => self.pending.take(),
        };
        if let Some(v) = emitted {
            self.queued.push(v);
        }
    }
}

impl<'a, I> Iterator for StreamRunner<I>
where
    I: Iterator<Item = Event<'a>>,
{
    type Item = StreamItem;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(v) = self.queued.pop() {
                return Some(Ok(v));
            }
            let event = self.events.next()?;
            self.feed(event);
        }
    }
}

// ---- tag helpers ------------------------------------------------------------

fn tag_kind(tag: &Tag<'_>) -> Option<NodeKind> {
    Some(match tag {
        Tag::Heading { .. } => NodeKind::Heading,
        Tag::Paragraph => NodeKind::Paragraph,
        Tag::CodeBlock(_) => NodeKind::Code,
        Tag::Link { .. } => NodeKind::Link,
        Tag::Image { .. } => NodeKind::Image,
        Tag::Item => NodeKind::Item,
        Tag::List(_) => NodeKind::List,
        Tag::Table(_) => NodeKind::Table,
        Tag::BlockQuote(_) => NodeKind::Quote,
        _ => return None,
    })
}

fn is_block(tag: &Tag<'_>) -> bool {
    matches!(
        tag,
        Tag::Paragraph
            | Tag::Heading { .. }
            | Tag::BlockQuote(_)
            | Tag::CodeBlock(_)
            | Tag::HtmlBlock
            | Tag::List(_)
            | Tag::Item
            | Tag::FootnoteDefinition(_)
            | Tag::Table(_)
            | Tag::TableHead
            | Tag::TableRow
            | Tag::TableCell
            | Tag::Link { .. }
            | Tag::Image { .. }
    )
}

fn heading_level(tag: &Tag<'_>) -> i64 {
    if let Tag::Heading { level, .. } = tag {
        i64::from(*level as u8)
    } else {
        0
    }
}

fn scalar_from_start(tag: &Tag<'_>, attr: &str) -> Option<Value> {
    match (tag, attr) {
        (Tag::Heading { level, .. }, "level") => Some(Value::from(i64::from(*level as u8))),
        (Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }, "href") => {
            Some(Value::from(dest_url.to_string()))
        }
        (Tag::Link { title, .. } | Tag::Image { title, .. }, "title") => Some(
            if title.is_empty() { Value::Null } else { Value::from(title.to_string()) },
        ),
        (Tag::CodeBlock(CodeBlockKind::Fenced(lang)), "lang") => {
            Some(Value::from(lang.to_string()))
        }
        (Tag::CodeBlock(CodeBlockKind::Indented), "lang") => Some(Value::Null),
        (_, "kind") => tag_kind(tag).map(|k| Value::from(k.as_str())),
        _ => None,
    }
}

// Contract: produce the same values as the tree evaluator for any
// query `analyze::plan` accepts. Enforced by
// `tests/queries.rs::stream_and_tree_agree`, not re-tested here.
