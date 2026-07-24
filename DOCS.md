# Vertion — Full Reference

Complete reference for every marker, CLI flag, and config field. For installation and a quick tour, see [README.md](README.md); this file is the exhaustive spec.

## Table of contents

- [1. Config file (`vertion.cfg`)](#1-config-file-vertioncfg)
- [2. Marker syntax](#2-marker-syntax)
- [3. Filter modes](#3-filter-modes)
- [4. CLI reference](#4-cli-reference)
- [5. Feature deep dives](#5-feature-deep-dives)
- [6. Build output](#6-build-output)
- [7. Exit codes](#7-exit-codes)

---

## 1. Config file (`vertion.cfg`)

TOML syntax. Created by `vertion init`. Read from the current working directory only (no upward search).

- Default filename: **`vertion.cfg`**.
- Legacy filename: `vertion.toml` is still read (and written back to) if `vertion.cfg` doesn't exist and `vertion.toml` does — existing projects keep working without renaming anything. `vertion init` always creates the new `.cfg` name.
- `show`, `graph`, `validate`, and `stats` do **not** read this file — they take everything from CLI flags.

### Full schema

```toml
[project]
version = "1.2.0"              # required. Used as the implicit filter for `build` with no -v.
input   = "./src"               # default: "./src"
output  = "./build"             # default: "./build"
ignore  = ["./build", "./node_modules"]   # default: [] (init seeds these two)

[build]
increment = "minor"             # "major" | "minor" | "patch". Default: "minor".

[last]                          # written automatically after every successful build/extract/watch build.
version    = "1.1.0"            # upper-bound version of the last build
dev        = false
auto       = false              # whether --auto was used
mode       = "cumulative"       # "cumulative" | "range" | "only" | "include" | ""
range_from = ""                 # set only when mode == "range"
tags       = []                 # tag filter used
profile    = ""                 # profile name used, if any (see note below)
wrap       = ""                 # "temp" | "perm" | ""
wrap_name  = ""

[profiles.prod]                 # any number of named tables under [profiles.*]
input     = "./src"             # optional override
output    = "./build/prod"      # optional override
ignore    = ["tests", "debug"]  # optional override — REPLACES [project].ignore, not merged
increment = "minor"             # optional override
run       = ["npm install", "npm run build"]   # optional post-build commands
tags      = ["combat"]          # optional default tag filter (CLI --tag replaces it)
wrap      = "temp"              # optional: "temp" | "perm"
wrap_name = ".vertion_wrap"     # optional

# Zero or more. A non-contiguous version set, managed via `vertion include`.
[[include]]
from = "1.1"
to   = "1.1"                    # from == to → a single exact version

[[include]]
from = "1.5"
to   = "1.8"                    # from < to → an inclusive range

# Zero or more. Whole-file version assignment for files that can't carry
# in-line comment markers (images, binaries, most JSON/CSV).
[[files]]
path    = "assets/logo.png"     # relative to the effective input dir
version = "2.0"                 # semver-ish string, or the literal "EXC"
tags    = ["ui"]                # optional; filtered like in-code block tags

[[files]]
path    = "assets/wip.psd"
version = "EXC"                 # excluded from every build, regardless of filter
```

### `[project]` (required)

| Field | Type | Default | Notes |
|---|---|---|---|
| `version` | string | — (required) | Semver-ish (`"1"`, `"1.2"`, `"1.2.3"` all valid — missing components pad with `.0`). Used as the filter when `vertion build` is run with no `-v`. |
| `input` | path | `"./src"` | |
| `output` | path | `"./build"` | Root; a per-version subfolder is created beneath it. |
| `ignore` | array of paths | `[]` | Paths under here are skipped entirely by `build`/`last`/`extract`/`watch`. |

### `[build]`

| Field | Type | Default | Notes |
|---|---|---|---|
| `increment` | `"major"` \| `"minor"` \| `"patch"` | `"minor"` | Bump level used by `--auto` when not overridden with `-M`/`-m`/`-P`. |

### `[last]` (auto-written, read by `vertion last`)

Written after every successful `build`/`last`. You normally don't hand-edit this.

**Gotcha:** `vertion last` does **not** automatically reapply `[last].profile` — it only records which profile was used for reference. Pass `-p <name>` again on the `last` invocation if you want the same profile; otherwise `last` resolves paths from bare `[project]` (or whichever `-p` you pass this time). `tags` and `dev` **are** restored automatically when the corresponding CLI flag isn't given; `wrap`/`wrap_name` are also restored automatically unless `--wrap` is passed explicitly on the `last` invocation or a profile is in effect.

### `[profiles.NAME]`

Selected with `-p NAME` / `--profile NAME` on `build`, `last`, `watch`, or `extract`. Never auto-selected.

| Field | Type | Merge behavior |
|---|---|---|
| `input` | path | Overrides `[project].input` if set. |
| `output` | path | Overrides `[project].output` if set. |
| `ignore` | array of paths | **Replaces** `[project].ignore` entirely if non-empty (not merged). |
| `increment` | string | Overrides `[build].increment` if set (validated). |
| `run` | array of strings | Post-build shell commands. CLI `-r`/`--run` fully replaces this list (no merge) when given. |
| `tags` | array of strings | Default tag filter for builds using this profile. CLI `-t`/`--tag` **replaces** this list entirely when given (no merge). |
| `wrap` | `"temp"` \| `"perm"` | Default wrap mode for this profile. |
| `wrap_name` | string | Default wrap folder name for this profile. |

Resolution order for input/output/ignore/increment/tags/wrap: **CLI flag > profile field > `[project]`/`[build]` default.** `-n`/`--ignore` values passed on the CLI are *appended* to whatever `ignore` list the profile/project step produced; `-t`/`--tag` and `-r`/`--run` instead *replace* the profile's list.

### `[[include]]`

An array of version ranges managed with `vertion include` (see [§4.12](#412-vertion-include)) — don't hand-edit unless you know the format. Used as the filter with `vertion build --include` / `vertion last` (when `last.mode == "include"`). The build's effective version set is the **union** of all entries.

| Field | Type |
|---|---|
| `from` | version string |
| `to` | version string (must be `>= from`) |

### `[[files]]`

Whole-file version gating for files that can't hold comment markers. See [§5.9](#59-file-level-versioning-files).

| Field | Type | Notes |
|---|---|---|
| `path` | string | Relative to the effective input directory. Leading `./` and `\` are normalized away, so `"assets/x.png"`, `"./assets/x.png"`, and Windows-style `"assets\x.png"` all match the same entry. No globs/wildcards — exact path only. |
| `version` | string | A version, evaluated against the active filter exactly like an in-code block. Or the literal `"EXC"` (case-insensitive) to exclude the file from **every** build unconditionally. |
| `tags` | array of strings | Optional. Filtered by the active `--tag` set with the same OR-logic as in-code block tags: an untagged file always passes; a tagged file is kept only if it shares a tag. Ignored when `version = "EXC"`. |

---

## 2. Marker syntax

### Grammar

After the line's comment prefix (`//` or `#`, see table below) and any leading whitespace:

```
version <ws> <token1> [<ws> <token2>] [<ws> [tag1,tag2,...]] [<ws> *]
```

- The keyword `version` is **case-sensitive** and must be lowercase. `//Version 1.2` is not recognized as a marker (it's just an ordinary comment).
- `<token1>` is either a version (`"1"`, `"1.2"`, `"1.2.3"`), or one of the keywords `ALL` / `EXC` (case-insensitive detection, but keep casing consistent — see gotcha below).
- `<token2>` (optional) is a second version, only valid when `<token1>` is a real version — this turns the marker into a **range** marker.
- `[tags]` (optional) is a comma-separated tag list in square brackets.
- Trailing `*` marks a **block** (paired open/close via a stack). Without `*`, a two-version marker is an **inline range** that applies to the next content line only.

### Comment prefix by file extension

| Prefix | Extensions |
|---|---|
| `//` | `js jsx ts tsx rs cpp cc cxx c h hpp java cs go kt swift scala php` |
| `#` | `py sh bash zsh rb yaml yml toml pl r` |
| `//` (default) | any other/unknown extension, including files with no extension |

### Version block

```js
//version 1.2 *
  ...code included when the active filter matches 1.2...
//version 1.2 *
```

The same syntax opens and closes — a block is closed by a subsequent marker with the identical `(version, to)` pair sitting on top of the parser's stack. The trailing `*` is optional on plain single-version blocks but conventional (helps eyeballing open vs. close in a diff).

### `ALL` block

```js
//version ALL
  ...always included, regardless of filter...
//version ALL
```

Always passes every filter mode. Still subject to `--tag` filtering if the block itself carries tags (an untagged `ALL` block always passes the tag filter too).

### `EXC` block

```js
//version EXC
  ...always excluded, regardless of filter...
//version EXC
```

The inverse of `ALL` — content inside is dropped from **every** build (`cumulative`, `range`, `only`, `extract`, `include`), no matter how wide the version window is. An `EXC` ancestor forces exclusion of everything nested inside it, even if an inner block's own version would otherwise pass (the ancestor rule applies to `EXC` the same way it does to any failing block). Useful for commenting out debug scaffolding, secrets, or WIP code that should never ship.

### Range block

```js
//version 1.3 2.0 *
  ...included when: from <= build_upper < to...
//version 1.3 2.0 *
```

- Condition: **`from <= build_upper < to`** — lower bound inclusive, upper bound exclusive.
- `build_upper` is the active filter's effective ceiling: the target version for `cumulative`/`extract`, the range's `to` for `range` mode, the max `to` across all `[[include]]` entries for `include` mode.
- **Always skipped in `ONLY` mode** — a range describes a window of versions, which has no meaning when you're asking for one exact version.
- The trailing `*` is **required** here (a two-version marker without `*` is parsed as an inline range instead — different meaning).

### Inline range marker

```js
//version 1.3 2.0
doSomethingFun();
```

No `*`, so this applies to exactly the next content line (not a block) — same `from <= build_upper < to` condition. Also skipped entirely in `ONLY` mode.

### Tags

```js
//version 1.2 [combat,inventory] *
  ...
//version 1.2 *
```

- `--tag NAME` (repeatable) filters by OR-logic: a tagged block is kept only if it shares **at least one** tag (case-insensitive) with the ones passed on the CLI.
- **Untagged blocks always pass** the tag filter — `--tag` only constrains blocks that themselves carry tags, it never hides plain version blocks.
- Tags can be combined with ranges: `//version 1.3 2.0 [beta] *`.

### Nesting & the ancestor rule

Every block in the nesting chain must **independently** satisfy the filter. If an outer block fails (including an `EXC` ancestor), everything inside it is discarded regardless of what the inner blocks say:

```js
//version 1.5 *
  //version 1.0 *
    // this line requires BOTH 1.5 AND 1.0 to pass — not just the innermost
  //version 1.0 *
//version 1.5 *
```

### Pairing rules & gotchas

- **Plain version/range blocks** close on an exact string match of `(version, to)` against the top of the open-block stack — `"1.2"` and `"1.20"` are different strings and won't pair with each other, and neither will `"1.2"` vs. `"1.2.0"`. Type the close marker identically to the open marker.
- **`ALL`/`EXC` blocks** close on a case-insensitive match against the literal keyword, not against each other's exact text — so `//version ALL` will close a block opened with `//version all`. Still, **use consistent casing** (uppercase `ALL`/`EXC` is the convention) to avoid confusing `vertion show`/`vertion graph` output, which prints back whatever casing you used to open the block.
- A malformed marker (bad version, unterminated `[`, unexpected trailing content, `from >= to` on a range) is reported with file + line number and a reason, and the line itself is stripped from output like any other marker line.
- Unclosed blocks are reported as warnings (or hard errors under `--strict` / `vertion validate`).

### File-level versioning (`[[files]]`)

For files that can't hold in-line comments — images, most binaries, JSON/CSV you don't want littered with marker lines — assign a version in the config instead (see [§1](#1-config-file-vertioncfg)):

```toml
[[files]]
path    = "assets/logo.png"
version = "2.0"

[[files]]
path    = "assets/wip.psd"
version = "EXC"
```

- Evaluated with the exact same `FilterMode` logic as an in-code block (`version_matches`), so `cumulative`/`range`/`only`/`extract`/`include` all apply correctly.
- `version = "EXC"` excludes the file from every build unconditionally, mirroring an in-code `EXC` block.
- A file not listed here is unaffected by `[[files]]` — it just goes through the normal path (parsed for markers if it's text, copied through untouched if it's binary or has no markers).

---

## 3. Filter modes

| Mode | CLI form | Included versions |
|---|---|---|
| Cumulative | `-v 1.2` | base + everything `<= 1.2` |
| Range | `-v 1.1 1.3` | base + everything `>= 1.1 && <= 1.3` |
| Only | `-v 1.2 ONLY` | base + exactly `== 1.2` (range/inline-range markers always skipped) |
| Extract | `vertion extract 1.2` | **only** blocks whose version exactly equals `1.2` (base excluded unless `--preserve-context`) |
| Include | `--include` (uses `[[include]]`) | base + union of all `[[include]]` entries |

"Base" = unmarked lines + passing `ALL` blocks. `EXC` blocks/files are excluded under every mode above, with no exception (not even `--include` with a wide-enough range).

`ONLY` is matched case-insensitively (`only`, `Only`, `ONLY` all work).

---

## 4. CLI reference

### Command list

| Command | Alias | Purpose |
|---|---|---|
| `vertion build` | `b` | Build a filtered output tree |
| `vertion last` | `l` | Rebuild using the previous build's saved settings |
| `vertion watch` | `w` | Watch the input directory, rebuild on change |
| `vertion extract` | `e` | Pull out only the blocks matching one version |
| `vertion show` | `s` | List version blocks in a single file |
| `vertion graph` | `g` | Tree view of version nesting in a single file |
| `vertion validate` | `V` | Scan the project for marker errors |
| `vertion stats` | `S` | Project-wide marker statistics |
| `vertion init` | — | Create `vertion.cfg` |
| `vertion include` | — | Manage the persisted `[[include]]` list |

Run `vertion --help` or `vertion <command> --help` for the live version of any of this.

### Shared build flags (`build`, `last`, `watch`)

These three subcommands share one flag set. Not every flag applies to every one of them — see the per-command notes below.

| Long | Short | Value | Default | Description |
|---|---|---|---|---|
| `--version-spec` | `-v` | 1–2 values | — | `<version>`, `<from> <to>`, or `<version> ONLY`. Omit entirely to use `[project].version` (cumulative). |
| `--input` | `-I` | path | `[project].input` / profile | Input directory. |
| `--output` | `-o` | path | `[project].output` / profile | Output root (a per-version subfolder is created under it). |
| `--ignore` | `-n` | path (repeatable) | `[project].ignore` / profile, plus these appended | Paths to skip. |
| `--tag` | `-t` | string (repeatable) | — | OR-logic tag filter. |
| `--profile` | `-p` | name | — | Named profile from `vertion.cfg`. Never auto-selected. |
| `--dev` | `-d` | flag | off | Build to a timestamped subfolder instead of overwriting. |
| `--strict` | `-q` | flag | off | Treat warnings (malformed markers, unclosed blocks) as a hard build failure. Files are still written before the error is returned. |
| `--auto` | `-a` | flag | off | After a successful build, bump `[project].version` by one increment step. Illegal with `ONLY` or `--include`. |
| `--major` | `-M` | flag | — | Force `--auto`'s bump to major. |
| `--minor` | `-m` | flag | — | Force `--auto`'s bump to minor. |
| `--patch` | `-P` | flag | — | Force `--auto`'s bump to patch. |
| `--no-progress` | — | flag | off | Suppress the progress bar (auto-suppressed already when stderr isn't a TTY). |
| `--no-comments` | `--noc` | flag | off | Strip whole-line comments from output (see [§5.4](#54---no-comments---noc)). |
| `--include` | `-i` | flag | off | Use the union of all `[[include]]` entries as the filter. Illegal together with `-v` or `--auto`. |
| `--run` | `-r` | string (repeatable) | profile's `run` list | Shell command to run in the output folder after a successful build. Fully replaces the profile's `run` list when given (no merge). |
| `--wrap` | — | 0–2 values: `[MODE] [NAME]` | off | Copy project files into an intermediate folder first (see [§5.2](#52---wrap)). `MODE` must be `temp` or `perm` if given; `NAME` only makes sense alongside an explicit `MODE`. |
| `--force` | — | flag | off | Allow an input path outside the project root (prints a warning instead of erroring). Does **not** bypass the output-inside-input check — use `--wrap` for that. |

Only one of `-M`/`-m`/`-P` should be given; if none are given, the increment level falls back to the profile's/`[build].increment`.

Only one of `--auto` and `--include` combinations apply per the illegal-combo rules in [§4.3](#43-vertion-build).

### 4.3. `vertion build`

```
vertion build [-v VERSION [TO|ONLY]] [-I INPUT] [-o OUTPUT] [-n IGNORE]... [-t TAG]...
              [-p PROFILE] [-d] [-q] [-a] [-M|-m|-P] [--no-progress] [--noc]
              [-i] [-r RUN]... [--wrap [MODE] [NAME]] [--force]
```

- No `-v` and no `--include` → uses `[project].version` as a cumulative filter.
- Illegal combinations (hard errors before anything is written):
  - `--include` together with `-v`
  - `--include` together with `--auto`
  - `--auto` together with `ONLY`
- On success, `[last]` is always updated (mode, version, tags, dev, profile, wrap); `--auto` additionally bumps and saves `[project].version`.

### 4.4. `vertion last`

Same flags and behavior as `build`, but the filter comes from `[last]` instead of `-v`/`[project].version`:

- `[last].mode == "include"` → replays the `--include` union.
- Otherwise replays cumulative/range/only using `[last].version` / `[last].range_from`.
- Errors if there's no recorded `[last]` state yet.
- `--auto` is illegal here if the last build used `ONLY`.
- **Restored automatically** from `[last]` unless overridden on the CLI: `tags` (OR'd — CLI `-t` fully replaces if given), `dev` (OR'd — once a build is `dev`, `last` without an explicit override stays `dev`), `wrap`/`wrap_name`.
- **Not restored automatically:** `--profile`. Pass `-p NAME` again on the `last` invocation if you want the same profile — otherwise paths resolve from bare `[project]` (or whatever `-p` you give this time).

### 4.5. `vertion watch`

```
vertion watch [-v VERSION [TO|ONLY]] [-I INPUT] [-o OUTPUT] [-n IGNORE]... [-t TAG]...
              [-p PROFILE] [-d] [-q] [--no-progress] [--noc] [-i] [-r RUN]...
```

Runs an initial build, then rebuilds (whole-tree, 300ms debounced) on every filesystem change under the input directory, printing a timestamped divider before each rebuild attempt. `Ctrl+C` to stop.

- Accepts the full shared flag set syntactically, but **`--auto`, `-M`/`-m`/`-P`, `--wrap`, and `--force` have no effect under `watch`** — they're silently ignored (the version-bump/wrap machinery only runs for one-shot `build`/`last`).
- `--include` **does** work as a filter source under `watch`, but the `--include` + `-v` illegal-combination check (which `build` enforces) is **not** enforced here — combining them is undefined rather than a clean error.
- `[last]` is **not** updated by `watch`.
- `-r`/`--run` commands re-run after every successful rebuild. A failing command is reported but does **not** stop the watcher (unlike `build`, where a failing run command fails the whole invocation).

### 4.6. `vertion extract`

```
vertion extract VERSION [-c] [-I INPUT] [-o OUTPUT] [-n IGNORE]... [-t TAG]...
                 [-p PROFILE] [-q]
```

| Long | Short | Value | Default |
|---|---|---|---|
| (positional) | — | version | required |
| `--preserve-context` | `-c` | flag | off |
| `--input` | `-I` | path | `.` (falls through to profile/`[project].input` if left at default) |
| `--output` | `-o` | path | `./build` (same fallback behavior) |
| `--ignore` | `-n` | path (repeatable) | — |
| `--tag` | `-t` | string (repeatable) | — |
| `--profile` | `-p` | name | — |
| `--strict` | `-q` | flag | off |

- Pulls out only the content of blocks whose version **exactly equals** `VERSION` (single-version blocks only — `m.to.is_none()`).
- Base (unmarked) lines, `ALL` blocks, and range/inline-range blocks are **excluded** unless `-c`/`--preserve-context` is given, in which case they're kept alongside the extracted content.
- `EXC` blocks/files are never extracted, with or without `--preserve-context`.
- No `--dev`, `--auto`, `--run`, `--wrap`, or `--no-comments` — extract is always a plain, immediate, single-shot pull.
- Does not touch `[last]`.

### 4.7. `vertion show`

```
vertion show FILE [-T]
```

Prints an indented list of every block in `FILE` with its line range — `[base]` regions, `[version X]`, `[ALL]`, range blocks as `[version X → Y]`, inline ranges as `[inline X → Y]`. `-T`/`--tags` additionally prints each block's tags. **Ignores any version filter** — shows every marker in the file structurally, regardless of what a build would keep.

### 4.8. `vertion graph`

```
vertion graph FILE
```

Same data as `show`, rendered as a `├── / └── / │` tree instead of an indented list. Also filter-agnostic.

### 4.9. `vertion validate`

```
vertion validate [-q] [-I INPUT] [-n IGNORE]...
```

Scans every file under `INPUT` (default `.`) for marker problems — does **not** read `vertion.cfg` at all, this is pure CLI. Reports:

- Malformed markers (bad version, unterminated tag list, `from >= to` on a range, trailing junk).
- Unclosed blocks (open marker with no matching close).
- Mismatched range closes (same version, different `to` reopened/closed inconsistently).
- Duplicate-sibling warnings (same `(version, to)` already open higher on the stack — the close will pair with the wrong block).

`-q`/`--strict` promotes all warnings to errors. Exits non-zero if any errors remain.

### 4.10. `vertion stats`

```
vertion stats [-I INPUT] [-n IGNORE]... [-j]
```

Also pure-CLI, no config file involved. Reports: files scanned, files with markers, total blocks, tagged blocks, deepest nesting, average nesting, version distribution, tag distribution, and the top 5 files by block count. `-j`/`--json` emits the same data as JSON (field names: `files_scanned`, `files_with_markers`, `total_blocks`, `tagged_blocks`, `deepest_nesting`, `average_nesting`, `version_distribution`, `tag_distribution`, `top_files_by_blocks`).

### 4.11. `vertion init`

```
vertion init
```

Creates `vertion.cfg` in the current directory with commented defaults (see [§1](#1-config-file-vertioncfg)). Errors if `vertion.cfg` **or** legacy `vertion.toml` already exists.

### 4.12. `vertion include`

```
vertion include [VERSION [+ OFFSET]] [-s] [-r FROM TO]
```

Manages `[[include]]`. Exactly one mode per invocation:

| Form | Effect |
|---|---|
| `vertion include 1.2` | Add exact entry `1.2 → 1.2` |
| `vertion include 1.2 + 4` | Add a forward range: bump applied at the same dot-level as typed. `1.2` (1 dot) + 4 → `1.2 → 1.6`. `1` (0 dots) + 2 → major bump. `1.2.3` (2 dots) + 2 → patch bump. |
| `vertion include --show` / `-s` | List all saved entries (`from` alone if `from == to`, else `from → to`) |
| `vertion include --remove FROM TO` / `-r FROM TO` | Exact match on `(FROM, TO)` deletes the entry. If `FROM` matches an entry's `from` and `TO` falls inside it, the entry is trimmed (`from` moves up to `TO`) instead of deleted. Otherwise errors — no partial match. |

Adding a duplicate of an existing entry is a no-op (reported, not an error). Every add/remove rewrites the whole `[[include]]` array in `vertion.cfg`.

---

## 5. Feature deep dives

### 5.1. `--auto` / auto-increment

After a **successful** build (not `extract`, not `watch`), `--auto` computes `autoincrement(filter.upper(), level)` and writes it to `[project].version` — this becomes the *next* build's default target, it has no effect on the *current* build's output. `level` is `-M`/`-m`/`-P` if given, else the profile's/`[build].increment` (default `minor`). Illegal with `ONLY` (a single exact version has no natural "next" step) and with `--include`.

### 5.2. `--wrap`

Copies the effective input tree into an intermediate folder (default name `.vertion_wrap`, directly under the project root) before building, so you can safely point `--input` at the project root itself without the output folder colliding with it (normally output-inside-input is a hard error).

| Form | Mode | Name |
|---|---|---|
| `--wrap` | `temp` | `.vertion_wrap` |
| `--wrap perm` | `perm` | `.vertion_wrap` |
| `--wrap temp myname` | `temp` | `myname` |
| `--wrap perm myname` | `perm` | `myname` |

The first token, if given, **must** be `temp` or `perm` — there's no way to give just a custom name and keep the default mode; write `--wrap temp myname` explicitly.

- `temp`: the wrap folder is deleted after the build, success or failure.
- `perm`: left in place for inspection.
- The wrap step always additionally excludes the resolved output directory and `vertion.cfg` itself, on top of your `--ignore`/`[project].ignore` list. The wrap directory is cleared (not merged) at the start of each wrap.
- Resolution priority: CLI `--wrap` > `[last]` (for `vertion last` only) > profile's `wrap`/`wrap_name`.

### 5.3. Path safety checks

Run before anything touches disk:

1. **Input outside the project root** (resolved input, made absolute, doesn't start with the cwd) → hard error, unless `--force` (then a warning is printed and the build proceeds).
2. **Output inside the effective build input** (post-wrap, if wrapping) → hard error: "output path is inside input path. Use --wrap to isolate project files before building." `--force` does **not** bypass this one — only `--wrap` does, because wrapping makes the actual build input a sibling folder, not an ancestor of the output.

### 5.4. `--no-comments` / `--noc`

Strips whole-line comments (any line whose first non-whitespace characters are the file's comment prefix) from build output. Available on `build`, `last`, `watch`. Not available on `extract`.

- **Whole-line only.** Trailing/inline comments (`doStuff(); // note`) and `//` occurring inside string literals are left untouched — stripping those safely needs a real language lexer.
- Applies independently of version markers: a file with zero `//version` markers but ordinary comments will now be rewritten (previously it would have been a straight byte-for-byte copy).

### 5.5. Post-build `--run` commands

Sequential shell commands executed in the build's **output** directory (the per-version subfolder, e.g. `./build/1.2.0`), not the input or project root. `cmd /C` on Windows, `sh -c` elsewhere; stdout/stderr stream live. Stops at the first non-zero exit and fails the whole `build`/`last` invocation (files remain on disk; only the command's own exit status propagates). Under `watch`, a failing run command is reported but does not stop the watcher — it retries on the next rebuild.

### 5.6. `--dev` builds

Instead of overwriting `<output_root>/<version>/`, writes to a timestamped `<output_root>/<version>_YYYY-MM-DD_HH-MM/` folder, so repeated dev builds don't clobber each other.

### 5.7. `--strict` / validation issue types

Malformed markers and unclosed blocks are warnings by default (printed, build continues) and hard errors under `-q`/`--strict` (build/validate fails, but for `build` the files are still written to disk before the error is returned). `vertion validate` additionally reports mismatched range closes and duplicate-sibling opens, which are promoted to errors the same way under `--strict`.

### 5.8. Color output

Success/warning/error lines are colorized (green/yellow/red) when stderr is a real terminal. Respects `NO_COLOR` (any value disables color).

---

## 6. Build output

```
<output_root>/
  <version>/                      ← plain build, e.g. -v 1.2 or -v 1.1 1.2 (upper bound)
    ...filtered files, mirroring input's structure...
    vertion.manifest.json
  <version>_YYYY-MM-DD_HH-MM/     ← --dev build
```

`vertion.manifest.json` (also what `--json`-style tooling should parse) mirrors the CLI summary:

```json
{
  "files_processed": 182,
  "files_modified": 73,
  "files_copied": 109,
  "lines_stripped": 14283,
  "time_ms": 328,
  "output": "/abs/path/to/build/1.2.0",
  "version": "1.2.0",
  "mode": "cumulative",
  "warnings": []
}
```

Files with no markers (and, under `--no-comments`, no comments either) are copied byte-for-byte; everything else is rewritten.

---

## 7. Exit codes

`0` on success, `1` on any error — including `--strict` build failures, `vertion validate` finding uncleared errors, a failing `--run` command, or any of the illegal-flag-combination checks. Errors are printed to stderr as `error: <message>`.
