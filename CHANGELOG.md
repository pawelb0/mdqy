# Changelog

## 0.1.4

- Move `|=`, `=`, `del`, and `walk` to the eval path so non-`-U`
  pipelines see mutations on local values.
- Fix `//` to match jq on empty / null / false LHS.
- Apply the predicate in `any(f)`, `all(f)`, and `paths(f)`.
- Fix `as` binding to consume only the preceding term.
- Allow chained comparisons via left-association.
- Make `split("")` yield single characters.
- Add string slicing (`.[lo:hi]` on strings).
- Fan out object construction across value streams.
- Extend `as $x` body through subsequent pipes.
- Recognise `not` as a 0-ary postfix filter.
- Resolve mutation paths through `..` and `select(f)`.
- Require `-U` or `--dry-run` to enter the transform path.
- Stress harness expanded to 647 cases across 22 sections.

## 0.1.3

- Preserve `aligns` and `TableHead` on table round-trip.

## 0.1.2

- Embed vhs demo gif in README.
- Document brew, scoop, and installer channels.

## 0.1.1

- Wire cargo-dist for multi-platform release.
- Add scoop and crate publish workflows.
- Drop musl target (mdcat-ng curl pulls openssl-sys).

## 0.1.0

Initial release.
