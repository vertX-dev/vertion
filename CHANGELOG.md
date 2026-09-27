# Changelog

All notable changes to the Vertion CLI are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

The VSCode extension has its own changelog:
[`editor/vscode/CHANGELOG.md`](editor/vscode/CHANGELOG.md).

## [Unreleased]

### Added

- `[[files]]` entries can name a directory, gating every file beneath it.
  `path = "assets/v2"` with `version = "2.0"` keeps the whole folder out of
  earlier builds. Variant directories inside it are dropped by the path they
  produce, before a variant is chosen.
- `vertion validate` now reads `vertion.cfg`, when there is one, and warns
  about `[[files]]` entries that match no file. Such an entry gates nothing, so
  after a rename the file or directory shipped ungated with no sign anything
  was wrong. Entries the build would reject are reported as errors.

### Changed

- When several `[[files]]` entries cover a file, all of them must pass, the
  same way a nested marker block needs its parents to pass. Previously only the
  first entry for an exact path was used. A file entry can narrow what its
  directory allows, but can't bring a file back from an `EXC` directory.
- A `[[files]]` path naming the input directory itself (`""`, `"."`, `"./"`) is
  now an error. It used to match nothing, silently.

### Fixed

- `vertion build` (and `last`, `--auto`, `vertion include`, `vertion condition`)
  no longer re-serialises `vertion.cfg`. Each command now edits only the keys it
  owns: comments, alignment, table order and CRLF line endings are kept, and
  defaults such as `include = []` are no longer written into the file.

## [1.0.0] — 2026-09-02

First public release. Earlier tags (`v0.2.0`) existed only in git and were never
published to crates.io, so everything below is new to anyone installing Vertion
for the first time.

### Markers

- Version blocks in line comments, in both `//` and `#` comment styles, detected
  per file by extension.
- Range markers (`1.1 → 1.3`), `ALL` blocks, `EXC` exclusions, and inline
  single-line markers.
- Nesting, with an inner block only reachable when its parents pass.
- Tags (`[beta]`) restricting a block to builds that opt in with `--tag`.
- Conditions (`[tag{name}]`) gating a block on a named boolean. Unknown names
  never pass — negated or not — so a typo cannot silently include code.

### Filtering

- Cumulative (up to a version), range, and `ONLY` modes.
- A persisted include list for non-contiguous version sets, managed with
  `vertion include`.
- `--preserve-context` to keep surrounding structure when extracting.
- `--noc` to additionally strip whole-line comments.

### Per-version files

- `.vertion.<target>/` variant directories, choosing one file or folder per
  build from several candidates.
- Whole-file version assignment through `[[files]]`, for assets that cannot
  carry a comment marker.
- Tie-breaking by version, then `[project].tag_priority`, then specificity.

### Commands

- `build`, `last`, `extract`, `watch` — produce a filtered tree.
- `show`, `graph`, `validate`, `stats` — inspect markers without building.
- `init`, `include`, `condition` — manage configuration.
- `completions <shell>` and `man` — generate a shell completion script
  (bash, zsh, fish, PowerShell, elvish) or a roff man page. Both are derived
  from the same command tree the parser uses, so neither can drift from the
  real flags. Release archives ship them prebuilt.
- `map` — translate a line number between a build output file and its source,
  in either direction. Accepts `FILE:LINE`, `FILE:LINE:COL`, `--stdin` for
  piping a compiler or runtime stack trace through, and `--list`. The mapping is
  recomputed from the build manifest rather than stored, so it costs a build
  nothing and cannot go stale; a source edited since the build is detected and
  reported.

### Configuration

- `vertion.cfg` (TOML). A legacy `vertion.toml` is still read and written back
  to when present.
- Profiles bundling output, ignores, tags, increment level, and post-build
  commands.
- Conditions resolved once per build from `bool`, a `cmd` probe, or a
  machine-wide `global` switch.
- Post-build `run` and `run_here` command lists, receiving the build's facts as
  `VERTION_*` environment variables.

### Security

- `SECURITY.md` documents that `run`, `run_here`, and `[conditions.*].cmd`
  execute arbitrary shell, and that `vertion condition --list` resolves probes
  without building. Markers in source files are data and never reach a shell.
- Bumped `crossbeam-epoch` to 0.9.20, clearing RUSTSEC-2026-0204 (an invalid
  pointer dereference reachable through `rayon`). CI now runs `cargo audit` on
  every push and weekly.

### Requirements

- Rust **1.85** or newer. Earlier releases claimed 1.74, but that was never
  achievable: the dependency tree reaches edition-2024 crates, and the committed
  lockfile is v4. The claim is now tested by CI rather than asserted.

### Packaging

- The published crate excludes `editor/`, so the VSCode extension no longer
  ships inside it.
- `cargo binstall vertion` fetches a prebuilt release archive instead of
  compiling. Homebrew and Scoop manifests are templated in `dist/`.

[Unreleased]: https://github.com/vertX-dev/vertion/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/vertX-dev/vertion/releases/tag/v1.0.0
