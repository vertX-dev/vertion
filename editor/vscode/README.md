# Vertion VSCode Extension

Editor support for [Vertion](https://github.com/vertX-dev/vertion) version markers.

## Features

- **Syntax highlighting** for marker lines (`//version 1.2 *`, `//version 1.3 2.0 *`, `//version 1.2 [tag] *`, `//version [wiki]`, `//version [stable{cond}]`, `//version [x{a}{!b}]`, `//version ALL`, `//version EXC`, and the `#` variants).
- **Auto-close on Enter** — typing an opener like `//version 1.2 *` and pressing Enter inserts a matching close line beneath the cursor. `ALL`, `EXC`, and tag-only openers also auto-close. Inline range markers (no `*`) do not auto-close.
- **Rename pair (F2)** — renaming the version, the upper bound, a tag, or a condition on one marker line updates its partner.
- **Pair highlight** — placing the cursor on an open or close marker highlights both lines. Colors are configurable (see Settings below).
- **Folding** — every paired block contributes a fold range that collapses through the close marker, so consecutive same-version blocks each fold cleanly.
- **Snippets** — `verb`, `vera`, `vere`, `vertt`, `vertc`, `vertn`, `vert`, `verrb`, `verr` for the marker shapes (slash variant); same prefixes with `#` for hash-comment languages.
- **Duplicate block** — copy the block under the cursor with a different version, ready to diverge.
- **Split block by version** — flatten a block containing nested version blocks into one standalone block per version variant.
- **Explorer context menu** — right-click any file or folder for a **Vertion** submenu that manages `.vertion.<target>/` variant directories.
- **Commands** — jump, wrap, duplicate, and split (Command Palette + rebindable keys).

## Explorer menu — per-version files

Right-click a file or folder in the explorer → **Vertion**:

| Item | Appears on | Does |
| --- | --- | --- |
| **Convert to Versioned File/Folder** | anything not already a variant | Creates `.vertion.<name>/` and moves the original in under a version spec you enter. Defaults to `0.0.0`, which matches every build, so converting alone never changes build output. |
| **Add Variant…** | a `.vertion.*` directory or anything inside one | Creates a new empty variant at the spec you enter, and opens it. |
| **Duplicate as New Version…** | a variant inside a `.vertion.*` directory | Copies it under a new spec, ready to diverge. |
| **Flatten to Plain File/Folder** | a `.vertion.*` directory | The inverse of convert: pick which variant survives, and it replaces the directory. The rest go to the trash (confirmed first). |

Whether a directory holds *file* or *folder* variants is taken from its own name — `.vertion.logo.png` declares the `png` extension, `.vertion.assets` has none and so holds folders.

Every prompt validates the spec against the same grammar the Rust builder uses (`src/variants.rs`), so `2.0.0`, `1.2.3e2.0.0`, `2.0.0-beta`, and `beta@!legacy` are all accepted and typos are rejected before anything is written.

## Commands & keybindings

| Command | Default keybinding | What it does |
| --- | --- | --- |
| `vertion.jumpToMatchingMarker` | `Ctrl+K Ctrl+M` (`Cmd+K Cmd+M` on macOS) | Move the cursor to the partner of the marker on the current line. |
| `vertion.wrapSelectionInBlock` | `Ctrl+K Ctrl+V` (`Cmd+K Cmd+V` on macOS) | Prompt for a version (or `ALL` / `EXC`) and wrap the selection in open/close markers. |
| `vertion.duplicateBlockWithVersion` | `Ctrl+K Ctrl+D` (`Cmd+K Cmd+D` on macOS) | Copy the block under the cursor beneath itself under a new version. Tags, conditions and `*` are preserved; only the version changes. |
| `vertion.splitBlockByVersion` | `Ctrl+K Ctrl+P` (`Cmd+K Cmd+P` on macOS) | Split a block containing nested version blocks into one standalone block per variant. |

### Split by version

Given a block whose body mixes base content with nested version blocks:

```js
//version 2.0.0 *
const someArray = [
'v1',
'v2',

//version 2.1.0 2.2.0*
'v3',
//version 2.1.0 2.2.0*

//version 2.3.0 *
'v4',
//version 2.3.0*
];
//version 2.0.0 *
```

splitting detects **3 variants** — the outer block alone, plus one per nested block — and rewrites it as three self-contained blocks, each with the nested markers dissolved:

```js
//version 2.0.0 *
const someArray = [
'v1',
'v2',
];
//version 2.0.0 *

//version 2.1.0 2.2.0*
const someArray = [
'v1',
'v2',
'v3',
];
//version 2.1.0 2.2.0*

//version 2.3.0 *
const someArray = [
'v1',
'v2',
'v4',
];
//version 2.3.0 *
```

Details:

- The cursor may be anywhere inside the block — including inside a nested child, in which case the command walks outward to the nearest block that actually has children.
- Blank lines directly adjacent to a nested block's markers are dropped, so removing a block doesn't leave a ragged gap.
- The open marker's exact text is reused for both ends of each generated block, which also normalizes a close marker written with different spacing.
- Deeper nesting rides along inside the variant it belongs to rather than multiplying variants.
- **Punctuation is not adjusted.** The trailing `,` after `'v2'` stays — fixing it up would need a real parser for each language. It remains valid JS.
- **Variants can overlap on build.** `//version 2.0.0 *` is cumulative, so a `-v 2.3.0` build emits both the 2.0.0 and 2.3.0 variants. Narrow variant 0 to a range (`//version 2.0.0 2.1.0 *`) if you need them mutually exclusive.

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
- Variant-filename parsing lives in `src/variantName.ts` and must mirror `../../src/variants.rs`.
- The comment-style table in `src/extension.ts` mirrors `../../src/config.rs::detect_comment_style`; unknown languages default to `//`.
