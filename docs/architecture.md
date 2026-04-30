# Architecture

## The pipeline

Source markdown hits `pulldown-cmark` with `ENABLE_OFFSET_ITER`.
Every event carries a byte span back to the original buffer. That
span travels through the tree and survives on every `Node` we
build. The write path reuses it to copy clean subtrees verbatim,
so unmodified bytes round-trip as-is.

The expression goes through its own small compiler. `lex`
tokenises, `parse` produces an `Expr` AST, and a selector-sugar
pass in the parser rewrites the CSS-ish forms (`h1:first`,
`# Install`, `>`) into jq primitives. Everything downstream only
knows about jq.

At run time we pick one of two evaluators. `analyze::plan` inspects
the `Expr` and, for a narrow class of heading-scoped read-only
queries, compiles a `StreamPlan` that consumes pulldown events
directly, without building a `Node` tree. Anything outside that
predicate walks the `Node` tree through `eval::eval`. The
differential test in `tests/queries.rs` asserts the two modes
produce the same values for every stream-eligible expression.

## Values

Seven variants, all cheap to clone:

```
Null | Bool | Number(f64) | String(Arc<str>)
Array(Arc<Vec<Value>>) | Object(Arc<BTreeMap<String, Value>>)
Node(Arc<Node>)
```

`Node::type_name()` returns `"heading"`, `"link"`, `"code"`, so
`select(type == "heading")` doubles as a kind predicate. Attribute
access (`.level`, `.href`, `.text`) looks up a canonical static
string key in `node.attrs`. `.children` projects the child `Value`
vector.

## Modules

| File | Role |
|---|---|
| `ast.rs` | `Node`, `NodeKind`, `Span`, canonical attr keys |
| `value.rs` | `Value` enum + type rules |
| `lex.rs` | Tokeniser; hand-rolled, single pass |
| `parse.rs` | Pratt + recursive-descent, desugars selectors inline |
| `expr.rs` | `Expr` AST |
| `events.rs` | pulldown ↔ Node, frontmatter extraction |
| `analyze.rs` | stream-vs-tree predicate + `StreamPlan` compiler |
| `stream.rs` | Event-stream evaluator |
| `eval.rs` | Tree evaluator + `Env` (bindings, user fns, filters) |
| `builtins.rs` | Filter registry (markdown + jq) |
| `mutate.rs` | Write path: `\|=`, `del`, `walk`, dirty propagation |
| `emit/md.rs` | Node → markdown via source slicing |
| `emit/json.rs` | Node/Value → JSON with flattened attrs |
| `emit/tty.rs` | Node → events → `mdcat::push_tty` |
| `walk.rs` | Directory traversal via `ignore` |
| `aggregate.rs` | `--per-file` / `--slurp` / `--merge` |
| `cli.rs` | Clap args + dispatch |
| `error.rs` | `CompileError` (source-caret) + `RunError` |

## Why two runners

The tree evaluator handles the whole language. The stream
evaluator optimises the common shallow read queries. Pulling every
`hN`, listing code fences, extracting link hrefs. For those a
10 MB document has no reason to land in an `Arc<Node>` graph. We
answer from the event iterator and stop.

Anything mutating, recursive, cross-stream, or variable-binding
falls back to tree mode. The predicate is conservative: a query
the stream runner can't handle correctly must fall back. The
differential test in `tests/queries.rs` keeps both runners in
sync.

## Spans and dirty bits

`Node::span` is `Option<Span>`. It's `Some` for anything parsed
from the buffer and `None` for anything synthesised (`node(...)`,
`section(...)` results, mutated subtrees). `Node::dirty` tracks
whether that span is still authoritative. A mutation flips it
true, and the serializer regenerates any dirty subtree from events
rather than trusting the stale span.

The contract `emit::md` enforces:

- Clean subtree with a span: `out.extend_from_slice(&source[span])`.
- Everything else: re-emit events via `node_to_events_borrowed` and
  hand them to `pulldown-cmark-to-cmark`.

Dirt propagates upward during mutation (see `walk_and_update` in
`mutate.rs`). A link-href rewrite dirties the link and every
ancestor, but siblings stay clean and the serializer byte-copies
them. Only the link's span regenerates.

## The tty feature

Terminal rendering goes through `mdcat-ng`, optional behind the
`tty` feature. For `Value::Node` results, `emit::tty` rebuilds an
`Event<'static>` stream via `node_to_events_owned` and hands it to
`mdcat::push_tty`. No markdown string is serialised and reparsed on
the way out.

Scalars print as plain lines. Auto-format picks `tty` when stdout
is a terminal and the feature is compiled in, `md` or `json`
otherwise.
