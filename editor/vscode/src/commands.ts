import * as vscode from "vscode";
import { commentStyleFor } from "./extension";
import { isValidVersion } from "./marker";
import { pairDocument } from "./pairing";

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
    );
}
