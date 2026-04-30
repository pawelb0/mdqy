# Quickstart

Work through this in a terminal with a markdown file handy. The
examples use `README.md`, but any markdown works.

## Install

```sh
cargo install --path . --features tty
```

Without `--features tty` you still get markdown and JSON output.
The `tty` feature pulls in `mdcat-ng` for terminal rendering.

## The shape of a query

```sh
mdqy '<EXPR>' [FILE ...]
```

If you pipe markdown in on stdin, the path argument is optional:

```sh
cat README.md | mdqy '.'
```

Identity. Prints the file back, byte-for-byte.

## Pull something out

The plaintext of each heading:

```sh
mdqy 'headings | .text' README.md
```

`headings` is a built-in filter that streams every heading node.
`.text` is the flattened plaintext on each one.

Link hrefs:

```sh
mdqy 'links | .href' README.md
```

Fence language tags:

```sh
mdqy 'codeblocks | .lang' README.md
```

## Filter

Only H2s:

```sh
mdqy 'headings | select(.level == 2) | .text' README.md
```

Or the short form:

```sh
mdqy 'h2 | .text' README.md
```

Only Rust code blocks:

```sh
mdqy 'code:lang(rust) | .literal' README.md
```

## Drill into a section

```sh
mdqy '# Install' README.md
```

`# Install` is sugar for `section("Install")`. It returns the
heading plus everything beneath it, up to the next heading of
equal or shallower level.

Grab every code fence inside the Install section:

```sh
mdqy '# Install > codeblocks | .literal' README.md
```

The `>` combinator scopes the right-hand side inside the section
the left side picked. Nest it:

```sh
mdqy '# Usage > ## "Query examples" > codeblocks:first | .literal' README.md
```

## Shape the output

An object per heading:

```sh
mdqy 'headings | {level, text, anchor}' README.md
```

`{level, text, anchor}` is jq shorthand for
`{level: .level, text: .text, anchor: .anchor}`.

Collect everything into one array:

```sh
mdqy '[headings | .text]' README.md
```

Group code blocks by language and count them:

```sh
mdqy 'reduce (codeblocks | .lang // "plain") as $l ({};
  setpath([$l]; (getpath([$l]) // 0) + 1))' README.md
```

## Switch output format

```sh
mdqy --output json 'headings' README.md
mdqy --output md   '# Install' README.md    # valid markdown
mdqy --output tty  '# Install' README.md    # needs the tty feature
mdqy --output text 'headings | .text' README.md
```

`auto` (the default) prints markdown for Node results, JSON for
scalars, and switches to `tty` when stdout is a terminal and the
feature is compiled in.

Without `--features tty` the markdown output still pipes into any
external renderer:

```sh
mdqy '# Install' README.md | mdcat -
mdqy '# Install' README.md | glow -
```

## Many files at once

```sh
mdqy 'headings | .text' docs/
```

Pass a directory, and mdqy walks it recursively, honouring
`.gitignore`, `.ignore`, and hidden-file rules.

Tag results with their source path:

```sh
mdqy --with-path 'headings | .text' docs/
```

Treat all files as one virtual document:

```sh
mdqy --merge 'codeblocks:lang(rust) | .literal' docs/
```

Collect all file roots into an array, then query once:

```sh
mdqy --slurp '[.[] | headings | .text] | add' docs/
```

## Rewrite in place

Preview first:

```sh
mdqy --dry-run '(.. | select(type == "link")).href |= sub("http:"; "https:")' README.md
```

That prints a unified diff. Nothing else changes. When you're
ready, swap `--dry-run` for `-U`:

```sh
mdqy -U '(.. | select(type == "link")).href |= sub("http:"; "https:")' README.md
```

`-U` writes atomically via a temp file in the same directory.
If it fails, the original is untouched.

Bump every heading one level deeper:

```sh
mdqy -U 'walk(if type == "heading" then .level |= (. + 1) else . end)' doc.md
```

Strip image titles across a docs tree:

```sh
mdqy -U 'walk(if type == "image" then del(.title) else . end)' docs/
```

Unchanged regions of the file are preserved verbatim; only the
mutated spans regenerate. See [transforms.md](transforms.md).

## Pipe into other tools

JSON output flattens each Node's attributes next to `kind` and
`children`, so standard jq pipelines work:

```sh
mdqy --output json 'codeblocks' README.md | jq '.lang' | sort -u
```

Raw output for shell loops:

```sh
mdqy --raw 'headings | .text' docs/ | while read -r h; do echo "# $h"; done
```

## See also

[language.md](language.md), [selectors.md](selectors.md),
[transforms.md](transforms.md), [architecture.md](architecture.md).
