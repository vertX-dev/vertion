# Vertion VSCode Extension

Editor support for [Vertion](https://github.com/vertX-dev/vertion) version markers.

## Features

- **Syntax highlighting** for marker lines (`//version 1.2 *`, `//version 1.3 2.0 *`, `//version 1.2 [tag] *`, `//version ALL`, `//version EXC`, and the `#` variants).
- **Auto-close on Enter** — typing an opener like `//version 1.2 *` and pressing Enter inserts a matching close line beneath the cursor. `ALL` and `EXC` openers also auto-close. Inline range markers (no `*`) do not auto-close.
- **Rename pair (F2)** — renaming the version, the upper bound, or a tag on one marker line updates its partner.
- **Pair highlight** — placing the cursor on an open or close marker highlights both lines. Colors are configurable (see Settings below).
- **Folding** — every paired block contributes a fold range that collapses through the close marker, so consecutive same-version blocks each fold cleanly.
- **Snippets** — `verb`, `vera`, `vere`, `vert`, `verrb`, `verr` for the marker shapes (slash variant); same prefixes with `#` for hash-comment languages.
- **Commands** — `Vertion: Jump to Matching Marker` and `Vertion: Wrap Selection in Version Block` (Command Palette + rebindable keys).

## Commands & keybindings

| Command | Default keybinding | What it does |
| --- | --- | --- |
| `vertion.jumpToMatchingMarker` | `Ctrl+K Ctrl+M` (`Cmd+K Cmd+M` on macOS) | Move the cursor to the partner of the marker on the current line. |
| `vertion.wrapSelectionInBlock` | `Ctrl+K Ctrl+V` (`Cmd+K Cmd+V` on macOS) | Prompt for a version (or `ALL` / `EXC`) and wrap the selection in open/close markers. |

Rebind either via VSCode's keyboard shortcuts editor (`Ctrl+K Ctrl+S`) — search for "Vertion".

## Settings

| Setting | Default | Description |
| --- | --- | --- |
| `vertion.highlight.enabled` | `true` | Highlight matching marker pairs when the cursor is on one. |
| `vertion.highlight.backgroundColor` | `#3a8bff33` | Background color applied to both lines of a matched pair. Any valid CSS color. |
| `vertion.highlight.borderColor` | `#3a8bffaa` | Border color applied to both lines of a matched pair. Leave empty to hide the border. |
| `vertion.highlight.borderWidth` | `1px` | CSS border width. Set to `0` to hide the border. |

## Install

Sideload the packaged `.vsix`:

```sh
npx vsce package         # writes vertion-<version>.vsix
code --install-extension vertion-*.vsix
```

## Develop

```sh
npm install
npm run compile          # tsc → out/
npm test                 # vitest tests for marker.ts + pairing.ts
```

Press F5 inside VSCode with `editor/vscode/` open to launch an Extension Development Host for live testing.

## Notes

- All marker parsing lives in `src/marker.ts` and must mirror the Rust grammar in `../../src/parser.rs`.
- The comment-style table in `src/extension.ts` mirrors `../../src/config.rs::detect_comment_style`; unknown languages default to `//`.
