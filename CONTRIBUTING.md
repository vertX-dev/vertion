# Contributing to Vertion

Thanks for taking the time. Bug reports and small focused pull requests are both
welcome; if you're planning something large, open an issue first so we can agree
on the shape before you write it.

## Getting set up

Vertion needs **Rust 1.85 or newer** — that floor is real, not aspirational, and
CI enforces it. The dependency tree reaches edition-2024 crates, so older
toolchains cannot build the project at all.

```sh
git clone https://github.com/vertX-dev/vertion.git
cd vertion
cargo build
cargo test
```

The VSCode extension is a separate Node project:

```sh
cd editor/vscode
npm ci
npm run compile
npm test
```

## Before you open a pull request

Run what CI runs, so you find out here rather than there:

```sh
cargo fmt --all
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

CI additionally runs a `cargo check` on Rust 1.85, `cargo audit`, and the
extension's build and tests. All five jobs must pass.

If you touched anything public in `src/lib.rs`'s modules — `config`, `filter`,
`linemap`, `parser` — note that the crate carries `#![warn(missing_docs)]`, and
CI treats warnings as errors. Every public item needs a doc comment.

## Tests

Three suites, and which one you want depends on what you changed:

| Where | What it covers |
| --- | --- |
| `src/**` unit tests | Filtering logic in isolation: marker parsing, filter modes, line maps |
| `tests/cli.rs` | The real binary end to end — argument parsing, exit codes, an actual build tree on disk |
| `editor/vscode/tests` | Extension logic, in vscode-free modules only |

A change to marker or filter semantics usually wants a unit test. A change to
argument handling, exit codes, or output layout wants a `tests/cli.rs` test.

The extension deliberately mirrors some Rust logic in TypeScript (`marker.ts`
against `parser.rs`). If you change one, check whether the other needs the same
change — the tests will not tell you.

Note that vitest cannot resolve `vscode` imports, so anything you want to test
must live in a module that doesn't import it. `pairing.ts`, `paths.ts`, and
`mapCli.ts` are the existing examples.

## Style

Match the file you're editing. A few things that hold throughout:

- Comments explain *why*, not *what*. If a line needs a comment to say what it
  does, the line is usually the problem.
- Error messages are lowercase and go to stderr as `error: <message>`.
- Paths are printed relative to the working directory where that helps, and
  normalized to the platform's separator.

## Security

Please don't file security problems as public issues — see
[SECURITY.md](SECURITY.md), which also explains why `run`, `run_here`, and
`[conditions.*].cmd` executing arbitrary shell is intended behaviour rather
than a vulnerability.

## Commits and releases

Branch off `main`. Keep the subject line short and imperative.

Releases are cut by tagging `vX.Y.Z`, which must match `version` in
`Cargo.toml` — the release workflow fails the build if the two disagree. Tagging
builds binaries for five targets and attaches them to a **draft** release;
publishing to crates.io and the Marketplace stays manual.

User-visible changes belong in [CHANGELOG.md](CHANGELOG.md), and extension
changes in [editor/vscode/CHANGELOG.md](editor/vscode/CHANGELOG.md).
