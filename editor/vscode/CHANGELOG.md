# Changelog

All notable changes to the Vertion VSCode extension are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
The CLI has its own changelog:
[`CHANGELOG.md`](https://github.com/vertX-dev/vertion/blob/main/CHANGELOG.md).

## [1.0.0] — 2026-09-02

First marketplace release, and the point where the extension joins the CLI's
version line — it ships alongside Vertion 1.0.0.

### Added

- `capabilities.untrustedWorkspaces` is now declared explicitly. Highlighting,
  folding, snippets, and the editing commands work in any workspace.

### Changed

- `vertion.executablePath` is now `machine`-scoped and listed in
  `restrictedConfigurations`, so a workspace can no longer redirect the
  extension at a different binary. Set it in your user settings.

### Fixed

- Packaging always compiles first. Without a `vscode:prepublish` script the
  bundle took whatever happened to be in `out/`, which could ship stale code.
- Two files no longer ship in the extension: a leftover `vertion.toml` test
  fixture, and an orphaned `language-configuration.json` that no contribution
  referenced.

## [0.9.0] — 2026-08-26

### Added

- **Go to Matching Line (Source ↔ Build)** on `Ctrl+K Ctrl+L`, in the editor
  context menu and on the status bar item. Jumps between a build output line
  and the source line it came from, in either direction, by shelling out to
  `vertion map --json`. When a source line was stripped from the build, it
  reports the next surviving line rather than pointing somewhere wrong.
- A build-output guard: editing a file inside a build tree — detected by the
  `vertion.manifest.json` the builder writes there — now warns that the next
  build will overwrite it, and offers to jump to the source instead. Toggle with
  `vertion.warnOnBuildOutputEdit`.
- `vertion.executablePath`, for when `vertion` is not on `PATH`.

## [0.8.0] — 2026-08-11

### Added

- Tag priority awareness when several file variants match a build equally.

## [0.7.0] — 2026-08-06

### Added

- Explorer commands for per-version files and folders: **Convert to Versioned
  File/Folder**, **Add Variant**, **Duplicate as New Version**, and **Flatten to
  Plain File/Folder**, under a Vertion submenu in the explorer context menu.
- Variant name parsing and validation, so a badly named variant is caught before
  it silently loses a build.

## [0.5.0] — 2026-07-31

### Changed

- Condition marker handling brought in line with the CLI's revised semantics.

## [0.4.0] — 2026-07-31

### Added

- Support for tag conditions (`[tag{name}]`) in highlighting, pairing, and
  folding.

## [0.3.0] — 2026-07-24

### Added

- Tag support in markers, matching the CLI's tag handling for files and
  profiles.

## [0.2.0] — 2026-05-25

### Added

- Configurable keybindings.
- Configurable pair-highlight colours: `vertion.highlight.backgroundColor`,
  `borderColor`, and `borderWidth`, plus `vertion.highlight.enabled`.

## [0.1.0] — 2026-05-25

### Added

- Syntax highlighting for Vertion markers, injected into 22 host languages.
- Marker pair highlighting when the cursor rests on one.
- Folding of version blocks.
- Auto-close: typing an opening marker inserts its closing counterpart.
- Rename a marker and its pair together.
- Snippets, in both `//` and `#` comment styles.
- Commands: **Jump to Matching Marker**, **Wrap Selection in Version Block**,
  **Duplicate Block with New Version**, and **Split Block by Version**.
