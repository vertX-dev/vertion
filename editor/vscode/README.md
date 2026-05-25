# Vertion VSCode Extension

Editor support for [Vertion](https://github.com/vertX-dev/vertion) version markers.

## Features

- **Syntax highlighting** for marker lines (`//version 1.2 *`, `//version 1.3 2.0 *`, `//version 1.2 [tag] *`, `//version ALL`, and the `#` variants).
- **Auto-close on Enter** — typing an opener like `//version 1.2 *` and pressing Enter inserts a matching close line beneath the cursor. ALL openers also auto-close. Inline range markers (no `*`) do not auto-close.
- **Rename pair (F2)** — renaming the version, the upper bound, or a tag on one marker line updates its partner.
- **Bracket-match highlight** — placing the cursor on an open or close marker highlights both lines.
- **Folding** — every paired block contributes a fold range.
- **Snippets** — `verb`, `vera`, `vert`, `verrb`, `verr` for the five marker shapes (slash variant); same prefixes with `#` for hash-comment languages.

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
