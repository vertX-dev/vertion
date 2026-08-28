# Vertion

[![CI](https://github.com/vertX-dev/vertion/actions/workflows/ci.yml/badge.svg)](https://github.com/vertX-dev/vertion/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

A CLI tool that filters source files by version markers and writes a build tree containing only the code relevant to a chosen version (or range).

> Full syntax reference (every marker form, every CLI flag, the complete `vertion.cfg` schema): [DOCS.md](DOCS.md).

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
vertion init                                 # create vertion.cfg
vertion build -v 1.2                         # cumulative up to 1.2
vertion build -v 1.1 1.3                     # range
vertion build -v 1.2 ONLY                    # exactly 1.2 + base
vertion build -v 1.2 --noc                   # also strip whole-line comments
vertion extract 1.2 --preserve-context       # only 1.2 blocks + base
vertion show src/foo.js --tags               # list version blocks
vertion graph src/foo.js                     # tree view
vertion validate --strict                    # check markers project-wide
vertion stats                                # marker stats
vertion watch -v 1.2                         # rebuild on file change
vertion last                                 # rebuild with previous settings
vertion map build/1.2.0/app.js:57            # which source line is that?

# Conditions gating `[tag{cond}]` markers (project cfg, or -G for the global one)
vertion condition --add imagesInStable --bool false
vertion condition --add hasAssets --cmd "test -d assets/img"   # a hook
vertion condition --add apiReleased --global-ref apiReleased   # wait on a shared switch
vertion condition --set imagesInStable --bool true
vertion condition --list                     # all conditions + resolved values
vertion condition --hooks                    # only the command-backed ones

# Persisted include list (non-contiguous version sets)
vertion include 1.1                          # add exact version
vertion include 1.5 + 3                      # add range 1.5 → 1.8
vertion include --show                       # list entries
vertion include --remove 1.5 1.7             # trim or delete an entry
vertion build --include                      # build union of all entries

# Post-build commands (also re-run after every `watch` rebuild)
vertion build -v 1.2 --run "npm install" --run "npm run build"
vertion watch -v 1.2 --run "npm run build"   # rebuild + re-run on every change
vertion build -v 1.2 --run "make" --run-here  # run in the invocation dir, not ./build/1.2
# For a per-command mix, set both `run` and `run_here` on a profile (see config below)

# Every spawned command gets the build's facts as VERTION_* env vars, so a
# downstream tool can find the tree that was just built (see "Build environment").
vertion build -v 1.2 --run "unified update-local --profile dev"

# Wrap: copy project files into an intermediate folder before building.
# Lets you safely point `-I .` at the project root without colliding with output.
vertion build -v 1.2 -I . --wrap              # temp wrap (default), cleaned up after build
vertion build -v 1.2 -I . --wrap perm         # keep the wrap dir for inspection
vertion build -v 1.2 -I . --wrap temp my_dir  # custom wrap folder name

# Path safety: input outside the project root is a hard error.
# Use --force to override (prints a warning).
vertion build -v 1.2 -I /some/other/tree --force
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

//version EXC                         // always EXcluded (dropped from every build)
  ...code...
//version EXC

//version [wiki]                      // tag-only: no version, selected by tag alone
  ...code...
//version [wiki]

//version [stable{imagesInStable}]    // tag gated by a named condition
  ...code...
//version [stable{imagesInStable}]

//version [x{!legacy}]                // `!` negates
//version [z{a}{!b}]                  // chain groups: a AND NOT b

//version 1.3 2.0 *                   // range block: from <= build_upper < to
  ...code...
//version 1.3 2.0 *

//version 1.3 2.0                     // inline range: applies to next line only
doSomethingFun();
```

- Same syntax for open and close — Vertion pairs them via a stack (matched on `(version, to)`).
- Trailing `*` is optional but recommended on single-version markers; **required** on range blocks (two versions + `*`). Two versions **without** `*` is an inline range. The `*` may be glued to the version (`//version 1.2*`).
- Range marker condition: `from <= build_upper < to` (lower inclusive, upper exclusive). Range markers are skipped entirely in `ONLY` mode.
- Nesting rule: every block in the chain must independently pass the filter.
- `ALL` blocks are always kept; `EXC` blocks are always dropped (an `EXC` ancestor excludes everything inside it, regardless of filter).
- Tag-only markers drop the version entirely — the tag is the selector. They pair by tag list, so `[a]` never closes `[b]`.
- **Tags are opt-in:** a tagged block ships only when one of its tags is active (`--tag`, else the profile's `tags`, else `[project].default_tags`). With none set, all tagged content is skipped. `*` is a wildcard admitting every tag. Untagged blocks are never affected.
- A tag may carry one or more `{condition}` groups defined in `[conditions.*]`; every condition on a marker must hold, in every filter mode. `{!name}` negates. An unknown name never passes (even negated) and warns. Manage them with `vertion condition` (see below).
- `--no-comments` (`--noc`) strips whole-line comments from the built output. Trailing/inline comments and `//` inside strings are left alone.

## Per-version files (`.vertion.<target>/`)

For files that differ wholesale between versions — images, binaries, generated data — store every version together in a directory named after the output file. The build picks one; no renaming step, no config entry.

```text
assets/.vertion.logo.png/
    0.0.0.png              # fallback, any version
    1.2.3e2.0.0.png        # 1.2.3 <= build < 2.0.0  (`e` = exclusive max)
    2.0.0.png              # from 2.0.0 onward
    2.0.0-beta.png         # >= 2.0.0 and tag `beta`
    2.0.0-beta@ready.png   # ... and condition `ready`
    .vertion.default.png   # used when nothing matches
```

`vertion build -v 2.5 --tag beta` writes `assets/logo.png` from `2.0.0-beta.png`. Highest version wins; then `[project].tag_priority`; then the more specific variant (more tags/conditions). Every variant must share the extension declared by the directory name. `.vertion.assets/` does the same for whole folders. See [DOCS.md](DOCS.md#59b-variant-directories-vertiontarget) for the full grammar.

## Config (`vertion.cfg`)

```toml
[project]
version = "1.2"
input   = "./src"
output  = "./build"
ignore  = ["./build", "./node_modules"]
default_tags = []        # tags active when --tag isn't given.
                         # [] = skip all tagged content; ["*"] = allow every tag.
tag_priority = []        # tie-breaker when several file variants match equally,
                         # e.g. ["beta", "combat"] — earlier entries win.

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
run       = ["npm install", "npm run build"]   # post-build cmds, run in the OUTPUT folder
run_here  = ["git add build"]                  # post-build cmds, run in the INVOCATION dir

# Non-contiguous version set (used with `vertion build --include`).
# Manage with `vertion include` / `vertion include --remove`.
[[include]]
from = "1.1"
to   = "1.1"

[[include]]
from = "1.5"
to   = "1.8"

# Whole-file version assignments for files that can't carry comment markers
# (images, JSON, binaries). Path is relative to the input dir.
[[files]]
path = "assets/logo.png"
version = "2.0"

[[files]]
path = "config/data.json"
version = "1.0"

[[files]]
path = "assets/wip.psd"
version = "EXC"          # always excluded, like an EXC block

[[files]]
path = "assets/new-ui.png"
version = "1.0"
conditions = ["!legacy"] # gated like a marker's {cond}; "!" negates

# Named conditions for `[tag{name}]` markers. Precedence: cmd > global > bool.
[conditions.imagesInStable]
bool = false             # manual project switch

[conditions.apiReleased]
global = "apiReleased"   # defer to ~/.vertion/vertion.cfg; false until defined there
bool   = false

[conditions.hasAssets]
cmd = "test -d assets/img"   # exit 0 = true, re-evaluated each build
```

Use a profile with `--profile prod`. `--auto` increments `[project].version` after a successful build (illegal with `ONLY`, `--include`, or `--last ONLY`).

`[[files]]` assigns a version to a whole file. The file is excluded from the build when its version fails the active filter (e.g. `logo.png` above is dropped from any build below `2.0`); otherwise it copies as-is. Use `version = "EXC"` to exclude a file from every build. Applies to `build`, `extract`, and `watch`.

> The config file is `vertion.cfg` (TOML syntax). A legacy `vertion.toml` is still read and written back to if present, so existing projects keep working — rename it to `vertion.cfg` when convenient.

## Debugging a build (`vertion map`)

Stripping a version block shifts every line below it, so a line number from a
build tree doesn't point at the same place in your source. `vertion map`
translates between the two — the direction is inferred from the path.

```sh
vertion map build/1.2.0/game.rs:57:9   # a built line -> the source line
vertion map src/game.rs:112            # and back the other way
vertion map --list build/1.2.0/game.rs # the whole map for one file

# Or pipe a whole stack trace / build log through it, and every recognized
# file reference is rewritten to point at the source. Unrecognized text is
# passed through untouched.
cargo run 2>&1 | vertion map --stdin
node build/1.2.0/app.js 2>&1 | vertion map --stdin
```

It costs a build nothing: the map is recomputed on demand from the source file
plus the settings recorded in `vertion.manifest.json`, so there's no sidecar to
write and nothing that can go stale. If you edit the source after building,
vertion notices the line counts no longer agree and says so.

Mapping *forward* from a line the build stripped has no exact answer — vertion
reports the next surviving line and flags it.

The VSCode extension exposes the same jump on **Ctrl+K Ctrl+L**.

## Build environment

Every command Vertion spawns — profile `run`, CLI `--run`, and `[conditions.*].cmd` probes — receives the build's facts as environment variables. A downstream tool can then locate the tree Vertion just produced instead of hardcoding a version that goes stale on the next bump:

```toml
# unified.cfg, in the project root — never touched when the version changes
behavior_pack = "${VERTION_OUTPUT:-./src}/BP"
```

| Variable | Value |
|---|---|
| `VERTION_ROOT` | Project root — the directory holding `vertion.cfg` |
| `VERTION_OUTPUT` | The versioned build folder just written |
| `VERTION_OUTPUT_ROOT` | `VERTION_OUTPUT`'s parent — the configured `output`, resolved |
| `VERTION_INPUT` | Input directory actually built from (the wrap folder under `--wrap`) |
| `VERTION_VERSION` | Version the build was filtered at, e.g. `2.5.0` |
| `VERTION_VERSION_DIR` | Leaf folder name — carries the timestamp suffix under `--dev` |
| `VERTION_PROFILE` | Active profile name; empty when none |
| `VERTION_MODE` | `cumulative` \| `range` \| `only` \| `include` |
| `VERTION_TAGS` | Active tag filter, comma-joined; empty when none |
| `VERTION_DEV` | `1` under `--dev`, else `0` |

Paths are absolute and normalized. The values do **not** depend on `--run-here` — only `cwd` does — so a command can always reach both the project and the build output. Under `watch` they're recomputed per rebuild.

One sharp edge: `cmd.exe` expands an empty-valued variable exactly like an undefined one, so `--profile "%VERTION_PROFILE%"` in a `run` line passes the literal `%VERTION_PROFILE%` when no profile is active. Name the profile outright in `run` commands; the empty string arrives intact for tools that read the environment themselves.

Full details in [DOCS.md §5.11](DOCS.md#511-build-environment-vertion_).

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
