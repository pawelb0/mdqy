# Query language

jq core with markdown-specific filters layered on. If you know jq,
you already know most of this. The markdown bits live in
[selectors.md](selectors.md).

## Values

```
null          bool          number        string
array         object        node
```

Numbers are `f64` and round-trip as integers where they fit.
`type` returns the kind string for node values (`"heading"`,
`"link"`, `"code"`), which lets `select(type == "heading")` work
without a separate predicate.

Nodes expose their attributes as fields. `.level` on a heading,
`.href` on a link, `.lang` on a fenced code block. `.text` is the
flattened plaintext of the node and its descendants. `.children`
gives you the child stream.

## Core forms

```
.                 identity
.foo              field access
.[3]              array / child index
.[1:4]            slice
.[]               iterate
..                recurse (every node + descendant)
a | b             pipe. b runs with each result of a as input
a, b              comma. concatenate streams
a // b            b if a is null or empty
f?                suppress errors from f
```

Arithmetic `+ - * / %` and comparisons `== != < <= > >=` work as
in jq. `+` concatenates strings and arrays too. `null + x == x`.

Boolean: `and`, `or`, `not`. Short-circuiting.

## Constructors

```
[ a, b, c ]       array from the stream
{ foo: .bar }     object with computed values
{ level, text }   shorthand. same as { level: .level, text: .text }
```

## Conditionals

```
if cond then a
elif other then b
else c
end
```

`else` is optional; a false condition without one yields the input
unchanged.

## Variables

```
. as $root | ...              bind and continue
reduce .[] as $x (0; . + $x)  fold over a stream
foreach .[] as $x (0; . + $x; .)
```

`reduce` and `foreach` take `(init; update; extract?)`. `foreach`
emits on every step. `reduce` emits only at the end.

## User functions

```
def inc: . + 1;
def add(a; b): a + b;
def twice(f): f | f;
[1, 2, 3] | map(inc)
```

Filter parameters are passed by name, as in jq. `map(inc)` hands
`map` the `inc` filter to invoke on each element, rather than a
value.

## Error handling

```
try a                   yield empty instead of propagating
a ?                     same thing, postfix
```

`try` catches runtime errors from builtins (`.foo` on a number,
a regex that won't compile, an index out of range).

## Assignment

Use `|=`, the update form. `=` (set) is not implemented.

```
.href |= sub("http:"; "https:")
del(.title)
walk(if type == "heading" then .level |= (. + 1) else . end)
```

`walk(f)` is post-order. Children get `f` applied first, then the
parent. Inside `f`, the standard mutation operators work: `|=`,
`del`, and plain read-only expressions that return a node.

See [transforms.md](transforms.md) for the write path.

## Builtins

Markdown filters, documented in [selectors.md](selectors.md):

```
headings  codeblocks  code  links  images  items  lists
tables  paragraphs  blockquotes  footnotes  h1 .. h6
section(str)  section_re(rx)  sections  toc
text  anchor  frontmatter  node(kind; attrs; children)
```

Shape and collection:

```
type  length  keys  has(k)  empty  first  last
map(f)  select(f)  add  any(f)  all(f)  reverse
sort  sort_by(f)  unique  unique_by(f)  group_by(f)
min  max  min_by(f)  max_by(f)  range(m; n)
limit(n; f)  nth(n; f)
```

Strings and regex:

```
tostring  tonumber  tojson  fromjson
contains  startswith  endswith
split  join  ltrimstr  rtrimstr
ascii_downcase  ascii_upcase
test(rx)  sub(rx; repl)  gsub(rx; repl)
```

Paths and env:

```
paths  getpath  setpath
env  $ENV
```

## Examples

```sh
mdqy 'headings | .text'                 # every heading's plaintext
mdqy '.. | select(type == "link") | .href'
mdqy 'section("Install") | codeblocks | .lang'
mdqy 'toc' README.md
mdqy 'reduce (codeblocks | .lang // "none") as $l ({};
  setpath([$l]; (getpath([$l]) // 0) + 1))'
mdqy 'def h(n): headings | select(.level == n); h(2) | .text'
```
