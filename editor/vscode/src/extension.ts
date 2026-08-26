import * as vscode from "vscode";
import { CommentStyle } from "./marker";
import { invalidatePairingCache } from "./pairing";
import { registerAutoClose } from "./autoClose";
import { registerRename } from "./rename";
import { registerFolding } from "./folding";
import { registerHighlight } from "./highlight";
import { registerCommands } from "./commands";
import { registerExplorerCommands } from "./explorer";
import { registerBuildGuard } from "./buildGuard";
import { registerMapCommands } from "./mapLine";

// Language → comment style. Mirrors `src/config.rs::detect_comment_style`.
// Unknown languages default to `//` (matches Rust default).
const LANGUAGE_TABLE: ReadonlyArray<{ id: string; style: CommentStyle }> = [
    { id: "javascript", style: "//" },
    { id: "javascriptreact", style: "//" },
    { id: "typescript", style: "//" },
    { id: "typescriptreact", style: "//" },
    { id: "rust", style: "//" },
    { id: "c", style: "//" },
    { id: "cpp", style: "//" },
    { id: "java", style: "//" },
    { id: "csharp", style: "//" },
    { id: "go", style: "//" },
    { id: "kotlin", style: "//" },
    { id: "swift", style: "//" },
    { id: "scala", style: "//" },
    { id: "php", style: "//" },
    { id: "python", style: "#" },
    { id: "shellscript", style: "#" },
    { id: "ruby", style: "#" },
    { id: "yaml", style: "#" },
    { id: "toml", style: "#" },
    { id: "perl", style: "#" },
    { id: "r", style: "#" },
];

export function commentStyleFor(languageId: string): CommentStyle {
    const found = LANGUAGE_TABLE.find((l) => l.id === languageId);
    return found ? found.style : "//";
}

export const DOCUMENT_SELECTOR: vscode.DocumentSelector = LANGUAGE_TABLE.map(
    (l) => ({ language: l.id }),
);

export function activate(ctx: vscode.ExtensionContext): void {
    registerAutoClose(ctx, DOCUMENT_SELECTOR);
    registerRename(ctx, DOCUMENT_SELECTOR);
    registerFolding(ctx, DOCUMENT_SELECTOR);
    registerHighlight(ctx, DOCUMENT_SELECTOR);
    registerCommands(ctx);
    registerExplorerCommands(ctx);
    registerBuildGuard(ctx);
    registerMapCommands(ctx);

    ctx.subscriptions.push(
        vscode.workspace.onDidCloseTextDocument((doc) => {
            invalidatePairingCache(doc.uri.toString());
        }),
    );
}

export function deactivate(): void {
    invalidatePairingCache();
}
