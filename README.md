# Vertion

[![CI](https://github.com/vertX-dev/vertion/actions/workflows/ci.yml/badge.svg)](https://github.com/vertX-dev/vertion/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

A CLI tool that filters source files by version markers and writes a build tree containing only the code relevant to a chosen version (or range).

## Install

### From source

```sh
git clone https://github.com/vertX-dev/vertion.git
cd vertion
cargo install --path .
```

`cargo install` puts the `vertion` binary in `~/.cargo/bin` (already on `PATH` for a standard Rust toolchain install).

### Build without installing

```sh
cargo build --release
./target/release/vertion --help
```

Requires Rust **1.74** or newer.

## Quick start

```sh
vertion init                                 # create vertion.toml
vertion build -v 1.2                         # cumulative up to 1.2
vertion build -v 1.1 1.3                     # range
vertion build -v 1.2 ONLY                    # exactly 1.2 + base
vertion extract 1.2 --preserve-context       # only 1.2 blocks + base
vertion show src/foo.js --tags               # list version blocks
vertion graph src/foo.js                     # tree view
vertion validate --strict                    # check markers project-wide
vertion stats                                # marker stats
vertion watch -v 1.2                         # rebuild on file change
vertion last                                 # rebuild with previous settings

# Persisted include list (non-contiguous version sets)
vertion include 1.1                          # add exact version
vertion include 1.5 + 3                      # add range 1.5 → 1.8
vertion include --show                       # list entries
vertion include --remove 1.5 1.7             # trim or delete an entry
vertion build --include                      # build union of all entries

# Post-build commands
vertion build -v 1.2 --run "npm install" --run "npm run build"
```

Run `vertion --help` (or `vertion <subcommand> --help`) for the full flag list — every long flag has a short alias (e.g. `-b` for `build`, `-v` for version, `-I` for `--input`).

## Marker syntax

```js
//version 1.2 *                       // open block (// languages)
//version 1.2 [combat,inventory] *    // open block with tags
  ...code...
//version 1.2 *                       // close block

#version 1.2 *                        # open block (# languages)
  ...code...
#version 1.2 *                        # close block

//version ALL                         // always included
```

- Same syntax for open and close — Vertion pairs them via a stack.
- Trailing `*` is optional but recommended (it disambiguates from regular `// version 2 ...` comments).
- Nesting rule: every block in the chain must independently pass the filter.

## Config (`vertion.toml`)

```toml
[project]
version = "1.2"
input   = "./src"
output  = "./build"
ignore  = ["./build", "./node_modules"]

[build]
increment = "minor"          # major | minor | patch

[last]                       # written automatically after each build
version    = "1.1"
mode       = "cumulative"
dev        = false

[profiles.prod]
output    = "./build/prod"
ignore    = ["tests", "debug"]
increment = "minor"
run       = ["npm install", "npm run build"]   # post-build commands

# Non-contiguous version set (used with `vertion build --include`).
# Manage with `vertion include` / `vertion include --remove`.
[[include]]
from = "1.1"
to   = "1.1"

[[include]]
from = "1.5"
to   = "1.8"
```

Use a profile with `--profile prod`. `--auto` increments `[project].version` after a successful build (illegal with `ONLY`, `--include`, or `--last ONLY`).

## Performance notes

Vertion processes files in parallel via [rayon], with a live progress bar ([indicatif]) that auto-hides on non-TTY stderr or with `--no-progress`.

### Windows: exclude the output directory from Defender

Windows Defender's real-time protection scans every newly written file. On large projects (10k+ files) this can dominate build time — typical example: a 10k-file build that runs in **4 seconds** without Defender can take **2 minutes** with it on, because MsMpEng.exe pegs a CPU core scanning the freshly written output.

Add an exclusion (PowerShell as Administrator):

```powershell
Add-MpPreference -ExclusionPath "C:\path\to\your\project\build"
# Or, exclude the binary itself so its writes are never scanned:
Add-MpPreference -ExclusionProcess "vertion.exe"
```

Remove later with `Remove-MpPreference -ExclusionPath ...`.

Other things that help on any OS:

- Build to a local SSD — not a network share, HDD, or sync folder (OneDrive, Dropbox, Google Drive all hook file writes and add overhead similar to Defender).
- Keep `--ignore` lists tight so Vertion doesn't walk huge dependency trees (`node_modules`, `target`, `.git`, etc.).

## Development

```sh
cargo test           # unit + integration tests
cargo fmt            # format
cargo clippy         # lint
```

CI runs `fmt --check`, `clippy -D warnings`, build, and tests on Linux, macOS, and Windows.

## License

[MIT](LICENSE) © vertX

[rayon]: https://docs.rs/rayon
[indicatif]: https://docs.rs/indicatif
