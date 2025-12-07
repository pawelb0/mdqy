# mdqy

jq for markdown. Query markdown documents with a hybrid selector +
jq-style DSL. Rewrite them in place. Pretty render to a terminal.

## Install

From source:

```
cargo install --path . --features tty
```

The `tty` feature pulls mdcat in so results can render to the
terminal. Skip it if you only need JSON / markdown output. Default
build stays at ~15 crates.

## Usage

```
mdqy '<EXPR>' [PATH...]
```

Paths can be files or directories. Directories walk recursively and
honour `.gitignore`, `.ignore`, and hidden-file rules.

### Query examples

```sh
mdqy '.' README.md                       # identity, byte-exact
mdqy 'headings | .text' README.md        # each heading, plaintext
mdqy 'headings | select(.level == 1) | .text' docs/
mdqy 'h1:first | .text' README.md        # first H1
mdqy 'codeblocks | .lang' README.md      # languages of fenced blocks
mdqy 'links | .href' README.md
mdqy 'section("Install")' README.md      # the Install section back out
mdqy '# Install > codeblocks:first | .literal' tutorial.md  # combinator
mdqy '.. | select(type == "heading")' README.md
```

### Output format

Default is `auto`:

- stdout is a terminal + `tty` feature compiled in → render
- stdout is piped → raw markdown (Node results) or JSON (scalars)

Override with `--output md | json | tty | text`.

### Transforms

```sh
mdqy --dry-run \
     '(.. | select(type == "link")).href |= sub("http:"; "https:")' \
     README.md

mdqy -U 'del((.. | select(type == "image")).title)' docs/
```

`-U` writes in place (atomic rename). `--dry-run` prints a unified
diff and exits 0 whether or not anything changed.

### Multi-file

```sh
mdqy 'headings | .text' docs/           # per-file by default
mdqy --slurp '.[].headings | length' docs/
mdqy --merge 'codeblocks | select(.lang == "rust")' docs/
```

## Library use

```rust
use mdqy::{parse, Query};

let q = Query::compile("headings | .text")?;
for v in q.run_tree(&parse(source)) {
    println!("{:?}", v?);
}
```

See `docs.rs` for the full API.

## Tooling

Generate shell completions:

```sh
cargo run --example gen_completions -- bash > mdqy.bash
cargo run --example gen_completions -- zsh  > _mdqy
cargo run --example gen_completions -- fish > mdqy.fish
```

Emit a man page:

```sh
cargo run --example gen_manpage > mdqy.1
```

Export the Node JSON schema:

```sh
cargo run --example export_schema --features schema-export > node.schema.json
```

## License

MPL-2.0. See `LICENSE`.
