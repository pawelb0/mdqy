# Changelog

## [Unreleased]

First development cut. Not yet on crates.io; mdcat integration runs
off a path dependency.

### Added

- Hybrid DSL: jq core (pipes, filters, mutation, control flow) plus
  selector pseudos (`h1:first`, `:nth(k)`, `:lang(x)`, `:text(x)`).
- Query runners: stream fast path for heading-scoped queries, tree
  evaluator for everything else. Differential test keeps them in
  sync.
- Multi-file input: positional files + directory walk via `ignore`,
  with `--per-file`, `--slurp`, `--merge` aggregation.
- Markdown output with byte-exact source slicing; clean subtrees
  round-trip verbatim.
- Attribute mutation: `|=`, `del(...)`, with atomic `-U` writes and
  `--dry-run` unified diffs.
- Optional `tty` feature: render through mdcat's `push_tty` when
  stdout is a terminal, markdown otherwise.
- Auto-detect output format based on `stdout.is_terminal()`.
- `def name(params): body;` user-defined functions, including
  filter-typed parameters.
- `reduce SRC as $x (INIT; UPDATE)` and
  `foreach SRC as $x (INIT; UPDATE; EXTRACT)`.
- `expr as $x | rest` bindings.
- YAML + TOML frontmatter parsed into `root.attrs.frontmatter`; new
  `frontmatter` builtin exposes it.
- Extra builtins: `sort_by`, `group_by`, `unique_by`, `min_by`,
  `max_by`, `min`, `max`, `range`, `limit`, `nth`, `paths`,
  `getpath`, `setpath`, `split`, `join`, `ltrimstr`, `rtrimstr`,
  `contains`, `tojson`, `fromjson`, `env`, `toc`.
- `--arg`, `--argjson`, `-n`, `-R`, `--stdin`, `--with-path`,
  `--no-color` flags wired.
- `CompileError::render(source)` shows a caret under the offending
  column.
- Shell completions, man page, and JSON schema emitted via
  `cargo run --example ...`.
- `walk(f)` honours `|=` and `del` inside `f`, routing mutation at
  the current node.
- `node(obj)` builds a Node from `{kind, <attrs>, children}`.
- Selector sugar: `# Title` and `# "Multi word"` shorthand for
  `section("...")`; bare `#..######` picks up the matching `hN`.
- `[]` literal is the empty array.
- mdcat path dependency swapped for the published `mdcat-ng` on
  crates.io.
