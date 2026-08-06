import * as vscode from "vscode";
import { commentStyleFor } from "./extension";
import { isValidVersion } from "./marker";
import { pairDocument } from "./pairing";
import {
    computeDuplicate,
    computeSplit,
    duplicateCursorOffset,
    isError,
} from "./transform";

async function jumpToMatchingMarker(): Promise<void> {
    const editor = vscode.window.activeTextEditor;
    if (!editor) return;
    const style = commentStyleFor(editor.document.languageId);
    const pairing = pairDocument(editor.document, style);
    const cur = editor.selection.active.line;
    const info = pairing.byLine.get(cur);
    if (!info || info.partnerLine === null) {
        vscode.window.setStatusBarMessage(
            "Vertion: no matching marker on this line",
            2000,
        );
        return;
    }
    const target = new vscode.Position(info.partnerLine, 0);
    editor.selection = new vscode.Selection(target, target);
    editor.revealRange(
        new vscode.Range(target, target),
        vscode.TextEditorRevealType.InCenterIfOutsideViewport,
    );
}

async function wrapSelectionInBlock(): Promise<void> {
    const editor = vscode.window.activeTextEditor;
    if (!editor) return;
    const sel = editor.selection;
    if (sel.isEmpty) {
        vscode.window.setStatusBarMessage(
            "Vertion: select code to wrap first",
            2000,
        );
        return;
    }
    const version = await vscode.window.showInputBox({
        prompt: "Version for the wrap block (or ALL / EXC)",
        value: "1.0",
        validateInput: (v) => {
            const t = v.trim().toUpperCase();
            if (t.length === 0) return "Required";
            if (t === "ALL" || t === "EXC") return null;
            return isValidVersion(v.trim()) ? null : "Not a valid version";
        },
    });
    if (!version) return;
    const trimmedVersion = version.trim();
    const upper = trimmedVersion.toUpperCase();
    const isKeyword = upper === "ALL" || upper === "EXC";

    const style = commentStyleFor(editor.document.languageId);
    const startLine = sel.start.line;
    // If the selection ends at column 0 of a line below the start, the user
    // typically meant to exclude that line — line-anchored wrap convention.
    const endLine =
        sel.end.character === 0 && sel.end.line > startLine
            ? sel.end.line - 1
            : sel.end.line;
    const indent =
        editor.document.lineAt(startLine).text.match(/^\s*/)?.[0] ?? "";
    const markerBody = isKeyword
        ? `${style}version ${upper}`
        : `${style}version ${trimmedVersion} *`;
    const openText = `${indent}${markerBody}`;
    const closeText = `${indent}${markerBody}`;

    await editor.edit((edit) => {
        const endPos = editor.document.lineAt(endLine).range.end;
        edit.insert(new vscode.Position(startLine, 0), openText + "\n");
        edit.insert(endPos, "\n" + closeText);
    });
}

/** All lines of the active document, for the pure transform helpers. */
function documentLines(document: vscode.TextDocument): string[] {
    const out: string[] = [];
    for (let i = 0; i < document.lineCount; i++) out.push(document.lineAt(i).text);
    return out;
}

/** Replace the inclusive line range `[startLine, endLine]` with `lines`. */
async function replaceLineRange(
    editor: vscode.TextEditor,
    startLine: number,
    endLine: number,
    lines: string[],
): Promise<void> {
    const range = new vscode.Range(
        startLine,
        0,
        endLine,
        editor.document.lineAt(endLine).text.length,
    );
    await editor.edit((edit) => edit.replace(range, lines.join("\n")));
}

async function duplicateBlockWithVersion(): Promise<void> {
    const editor = vscode.window.activeTextEditor;
    if (!editor) return;
    const style = commentStyleFor(editor.document.languageId);
    const cursorLine = editor.selection.active.line;

    const spec = await vscode.window.showInputBox({
        prompt: "Version for the duplicate (a version, a `from to` range, ALL or EXC)",
        value: "1.0",
        validateInput: (v) => {
            const t = v.trim();
            if (t.length === 0) return "Required";
            const upper = t.toUpperCase();
            if (upper === "ALL" || upper === "EXC") return null;
            const parts = t.split(/\s+/);
            if (parts.length > 2) return "At most two versions (`from to`)";
            return parts.every(isValidVersion) ? null : "Not a valid version";
        },
    });
    if (!spec) return;

    const lines = documentLines(editor.document);
    const result = computeDuplicate(lines, style, cursorLine, spec.trim());
    if (isError(result)) {
        vscode.window.setStatusBarMessage(`Vertion: ${result.error}`, 3000);
        return;
    }
    const offset = duplicateCursorOffset(lines, style, cursorLine);
    await replaceLineRange(editor, result.startLine, result.endLine, result.lines);

    // Land on the first body line of the copy so edits can start immediately.
    const target = new vscode.Position(result.startLine + offset, 0);
    editor.selection = new vscode.Selection(target, target);
    editor.revealRange(new vscode.Range(target, target));
}

async function splitBlockByVersion(): Promise<void> {
    const editor = vscode.window.activeTextEditor;
    if (!editor) return;
    const style = commentStyleFor(editor.document.languageId);
    const lines = documentLines(editor.document);
    const result = computeSplit(lines, style, editor.selection.active.line);
    if (isError(result)) {
        vscode.window.setStatusBarMessage(`Vertion: ${result.error}`, 3000);
        return;
    }

    const confirm = await vscode.window.showQuickPick(["Split", "Cancel"], {
        title: `Split into ${result.variantCount} blocks`,
        placeHolder: result.variantLabels.join("   |   "),
    });
    if (confirm !== "Split") return;

    await replaceLineRange(editor, result.startLine, result.endLine, result.lines);
    vscode.window.setStatusBarMessage(
        `Vertion: split into ${result.variantCount} blocks`,
        3000,
    );
}

export function registerCommands(ctx: vscode.ExtensionContext): void {
    ctx.subscriptions.push(
        vscode.commands.registerCommand(
            "vertion.jumpToMatchingMarker",
            jumpToMatchingMarker,
        ),
        vscode.commands.registerCommand(
            "vertion.wrapSelectionInBlock",
            wrapSelectionInBlock,
        ),
        vscode.commands.registerCommand(
            "vertion.duplicateBlockWithVersion",
            duplicateBlockWithVersion,
        ),
        vscode.commands.registerCommand(
            "vertion.splitBlockByVersion",
            splitBlockByVersion,
        ),
    );
}
