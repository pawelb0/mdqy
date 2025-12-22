# Transforms

mdqy's write path covers attribute-level edits. You can change a
link's href, strip a title, bump heading levels, and anything else
that lives in `Node::attrs`. Tree-shape edits (inserting a node,
reordering children) aren't wired up in v1. `walk` plus `|=`
covers the common rewrite cases.

## Operators

```
.attr |= f        update .attr by running f with its current value
del(.attr)        remove .attr entirely
walk(f)           post-order. recurse into children, then apply f
```

`|=` shapes:

```sh
(.. | select(type == "link")).href |= sub("http:"; "https:")
(h1).level |= (. + 1)
(codeblocks | select(.lang == "rust")).lang |= "rs"
```

`del` looks like a builtin call but carries mutation semantics,
so the write path intercepts it before the evaluator sees it.
`.attr` on the left of `|=` is handled the same way.

`walk(f)` runs `f` at every node, children first. Inside `f` you
can use the same mutation operators:

```sh
mdqy -U 'walk(if type == "heading" then .level |= (. + 1) else . end)' doc.md
mdqy -U 'walk(if type == "image" then del(.title) else . end)' doc.md
```

Conditions in `walk(f)` run through the read-only evaluator.
Mutation arms go through the walk-aware mini-interpreter in
`mutate::apply_walk_f`.

## Output modes

```
--dry-run       print a unified diff, exit 0 either way
-U              atomic in-place write (temp file + rename(2))
(default)       write transformed markdown to stdout
```

`--dry-run` and `-U` combine. `--dry-run` prints what `-U` would
do without touching the file.

```sh
mdqy --dry-run '(.. | select(type == "link")).href |= sub("http:"; "https:")' docs/
mdqy -U 'del((.. | select(type == "image")).title)' README.md
```

## How source bytes survive

Every `Node` parsed from the buffer carries a byte `Span`. The
serializer in `emit::md` walks the mutated tree. For any clean
subtree with a span, it calls
`out.extend_from_slice(&source[span])`, returning the original
bytes untouched. Mutations set `dirty = true` on the affected
node and every ancestor up to the root. Dirty subtrees get
re-emitted as pulldown events and fed to `pulldown-cmark-to-cmark`.

Consequences:

- A link href rewrite touches only the link span in the output.
  Surrounding paragraphs, fences, list markers, blank-line padding
  all stay verbatim.
- Regenerated spans go through `pulldown-cmark-to-cmark`, which
  normalises some style choices (emphasis marker, fence width).
  That normalisation is localised to dirty subtrees.
- Synthetic nodes (from `node(...)` or `section(...)`) have no
  span, so they always regenerate.

## Atomic writes

`-U` writes through `tempfile::NamedTempFile::persist`:

1. Create a temp file in the same directory as the target (same
   filesystem, so `rename(2)` is atomic and `EXDEV` can't bite).
2. Write the transformed bytes.
3. `fsync` the temp file.
4. Rename over the target.

If anything fails before the rename, the original is untouched.

## What's not supported

- `=`, the set-style assignment. Only `|=`, the update form.
- Structural mutation. You can't replace `.children` wholesale.
  `walk(f)` handles the cases that matter. A proper
  `.children |= [...]` path is a v2 task.
- Inserting siblings. Same reason.
- Mutation targets that don't resolve to `Node` values. v1
  attribute mutation requires node-valued selectors on the left of
  `|=` or `del`.

If one of these matters for your case, the tree evaluator already
has the pieces. What's missing is the plumbing between the
evaluator and the serializer.
