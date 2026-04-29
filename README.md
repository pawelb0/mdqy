# mdqy

jq for markdown. Query markdown documents with a hybrid selector +
jq-style DSL. Rewrite them in place. Pretty render to a terminal.

## Install

Homebrew (macOS, Linux):

```sh
brew install pawelb0/tap/mdqy
```

Scoop (Windows):

```sh
scoop bucket add pawelb0 https://github.com/pawelb0/scoop-bucket
scoop install pawelb0/mdqy
```

Shell installer (Unix):

```sh
curl -sSfL https://github.com/pawelb0/mdqy/releases/latest/download/mdqy-installer.sh | sh
```

PowerShell installer (Windows):

```powershell
irm https://github.com/pawelb0/mdqy/releases/latest/download/mdqy-installer.ps1 | iex
```

From crates.io:

```sh
cargo install mdqy --features tty
```

Prebuilt binaries for every release: [github.com/pawelb0/mdqy/releases/latest](https://github.com/pawelb0/mdqy/releases/latest).
On macOS, direct downloads need `xattr -d com.apple.quarantine ./mdqy`
once before first run.

The `tty` feature pulls mdcat in for terminal rendering. Default
builds emit markdown or JSON.

## Documentation

- [quickstart](docs/quickstart.md): tutorial walkthrough. Start here.
- [language](docs/language.md): the jq-style query language.
- [selectors](docs/selectors.md): `hN`, `:pseudos`, `#` sugar, the `>` combinator.
- [transforms](docs/transforms.md): `|=`, `del`, `walk`, `-U`, `--dry-run`.
- [architecture](docs/architecture.md): pipeline, modules, the two evaluators, byte-exact round-trip.

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

`-U` writes in place via atomic rename. `--dry-run` prints a
unified diff and exits 0.

### Multi-file

```sh
mdqy 'headings | .text' docs/           # per-file by default
mdqy --slurp '[.[] | headings | .text] | length' docs/
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

See [docs.rs/mdqy](https://docs.rs/mdqy) for the full API.

## Tooling

```sh
cargo run --example gen_completions -- bash > mdqy.bash
cargo run --example gen_manpage > mdqy.1
cargo run --example export_schema --features schema-export > node.schema.json
```

## License

MPL-2.0. See `LICENSE`.
