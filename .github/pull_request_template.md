<!--
Thanks for the patch. Fill in what applies and delete the rest — this is a
prompt, not a form to complete exhaustively.
-->

## What this changes

<!-- One or two sentences. If it fixes an issue, "Fixes #123" here. -->

## Why

<!-- The problem behind the change. For a bug, what went wrong; for a feature,
     what wasn't possible before. -->

## How it was verified

<!-- What you actually ran, and anything you checked by hand. `watch`, the
     variant directories, and the extension have thin automated coverage, so
     manual verification genuinely matters there. -->

- [ ] `cargo fmt --all`
- [ ] `cargo clippy --locked --all-targets -- -D warnings`
- [ ] `cargo test --locked`
- [ ] `npm test` in `editor/vscode` (only if the extension changed)

## Checklist

- [ ] Tests added or updated, in the suite that fits the change
      (unit for filtering logic, `tests/cli.rs` for CLI behaviour)
- [ ] Public items in `config`, `filter`, `linemap`, or `parser` are documented
      — the crate is `#![warn(missing_docs)]` and CI denies warnings
- [ ] User-visible changes noted in `CHANGELOG.md`
      (or `editor/vscode/CHANGELOG.md`)
- [ ] Docs updated if behaviour, flags, or config changed
      (`README.md` for the tour, `DOCS.md` for the reference)
- [ ] Marker or filter semantics changed in Rust → checked whether
      `editor/vscode/src/marker.ts` needs the same change
