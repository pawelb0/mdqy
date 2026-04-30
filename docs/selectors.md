# Markdown selectors

The selector layer is sugar on top of jq core. Every selector
rewrites into jq during parsing, so the evaluator only ever sees
primitives. That's why `h1:first > code:first | .literal` and its
longhand equivalent yield identical results.

## Kind shortcuts

Each of these is a zero-arg filter equivalent to
`.. | select(type == "<kind>")`:

```
headings  paragraphs  codeblocks (alias code)
links  images  items  lists  tables
blockquotes  footnotes
```

`h1` through `h6` filter headings by level:

```sh
mdqy 'h2'                # same as: headings | select(.level == 2)
mdqy 'h2 | .text'
```

## Pseudos

```
:first          same as first
:last           same as last
:nth(k)         zero-indexed. same as .[k]
:lang(rust)     for codeblocks. select(.lang == "rust")
:text("Install")  for headings. select(.text == "Install")
```

Pseudos chain:

```sh
mdqy 'h2:nth(1)'             # second H2
mdqy 'code:lang(bash):first'
```

## Hash sugar

`#` through `######` is shorthand for `section(...)`:

```sh
mdqy '# Install'                    # section("Install")
mdqy '## "Breaking changes"'        # section("Breaking changes")
mdqy '### Usage | .text'
```

`section(name)` finds the first heading whose `.text` matches
`name` case-insensitively and returns a synthetic `Section` node
whose children are the heading and everything after it up to (but
not including) the next heading of equal or shallower level.

`sections` streams one Section per heading in document order.
Nested sub-headings produce their own sections after their
enclosing one. `sections(N)` keeps only sections whose heading is
at level `N`:

```sh
mdqy 'sections | .text'                  # full text of every section
mdqy 'sections(3) | .text'               # full text of every H3 section
mdqy 'sections | select(.children[0].level == 3)'   # same, jq-style
```

## The `>` combinator

`a > b` scopes `b` inside the section produced by `a`. It's
context-sensitive. The parser only treats `>` as a combinator when
the right-hand side starts with a selector origin (`hN`, a known
selector name, `#`, or an ident followed by `:`). Otherwise `>`
stays a comparison operator.

The rewrite (roughly):

```
a > b   ≡   a | [headings] | .[0] | .text as $t | $__root | section($t) | b
```

Once `>` appears, the parser wraps the whole expression in
`. as $__root | ...` so `$__root` is the original input. Inside
the combinator, `section(...)` looks up from the document root,
independent of whatever `a` produced.

Examples:

```sh
mdqy 'h1:first > h2:nth(1) > code:first | .literal'
mdqy '# Install > codeblocks:lang(bash) | .literal'
mdqy '## Usage > links | .href'
```

## When to use what

`headings | select(.level == 2)` and `h2` compile to the same
plan. Selector forms are shorter at the call site; the jq forms
spell out the predicate.
