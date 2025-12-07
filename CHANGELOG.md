# Changelog

## Unreleased

jq-style query language: pipes, filters, comparisons, arithmetic,
object and array ctors, `if`, `as $x`, `reduce`, `foreach`, user
`def`, `try`.

Markdown selectors layered on top. `headings`, `codeblocks`, `links`,
etc. as zero-arg filters. `hN` for heading level. Pseudos `:first`,
`:last`, `:nth(k)`, `:lang(x)`, `:text(x)`. Hash shorthand `# Title`
and `# "Multi word"` for `section()`. Combinator `>` scopes the RHS
into the LHS's section.

Two runners. A single-pass event-stream evaluator for heading-scoped
reads; a tree evaluator for everything else. They match on every
stream-eligible query (checked in tests).

Transforms: `|=` and `del` on attributes, `walk(f)` with `|=` inside
`f`, `-U` atomic in-place writes, `--dry-run` unified diffs. Byte-exact
source slicing means clean subtrees round-trip verbatim; only mutated
spans regenerate.

Frontmatter: `---` YAML or `+++` TOML parsed into the root, reachable
via the `frontmatter` builtin.

Multi-file: positional args, globs, recursive directory walk via the
`ignore` crate (same rules as ripgrep), `--per-file` / `--slurp` /
`--merge` aggregation.

Output: markdown, JSON (flattened Node schema), text. Auto mode
selects markdown or mdcat TTY rendering based on stdout. TTY is
behind the `tty` feature so default builds stay small.

Other builtins: `sort_by`, `group_by`, `unique_by`, `min_by`,
`max_by`, `min`, `max`, `range`, `limit`, `nth`, `paths`, `getpath`,
`setpath`, `split`, `join`, `ltrimstr`, `rtrimstr`, `contains`,
`tojson`, `fromjson`, `env`, `toc`, `node`.

CLI: `--arg`, `--argjson`, `-n`, `-R`, `--stdin`, `--with-path`,
`--no-color`, `--from-file`. Compile errors point at the column
with a caret.

Tooling: `cargo run --example gen_completions -- <shell>`,
`gen_manpage`, `export_schema` (feature-gated).
