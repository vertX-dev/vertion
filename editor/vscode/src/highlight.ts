import * as vscode from "vscode";
import { commentStyleFor } from "./extension";
import { pairDocument } from "./pairing";

const CONFIG_SECTION = "vertion.highlight";

let decorationType: vscode.TextEditorDecorationType | undefined;

function buildDecoration(): vscode.TextEditorDecorationType {
    const cfg = vscode.workspace.getConfiguration(CONFIG_SECTION);
    const bg = cfg.get<string>("backgroundColor", "#3a8bff33");
    const border = cfg.get<string>("borderColor", "#3a8bffaa");
    const borderWidth = cfg.get<string>("borderWidth", "1px");
    const opts: vscode.DecorationRenderOptions = {
        isWholeLine: true,
        backgroundColor: bg || undefined,
    };
    if (border && borderWidth && borderWidth !== "0") {
        opts.borderColor = border;
        opts.borderWidth = borderWidth;
        opts.borderStyle = "solid";
    }
    return vscode.window.createTextEditorDecorationType(opts);
}

function disposeDecoration(): void {
    if (decorationType) {
        decorationType.dispose();
        decorationType = undefined;
    }
}

function rebuildDecoration(): void {
    disposeDecoration();
    decorationType = buildDecoration();
}

function clearHighlights(editor: vscode.TextEditor): void {
    if (decorationType) editor.setDecorations(decorationType, []);
}

function updateHighlights(editor: vscode.TextEditor): void {
    if (!decorationType) return;
    const cfg = vscode.workspace.getConfiguration(CONFIG_SECTION);
    if (!cfg.get<boolean>("enabled", true)) {
        clearHighlights(editor);
        return;
    }
    const style = commentStyleFor(editor.document.languageId);
    const pairing = pairDocument(editor.document, style);
    const cursorLine = editor.selection.active.line;
    const info = pairing.byLine.get(cursorLine);
    if (
        !info ||
        info.partnerLine === null ||
        (info.kind.kind !== "Versioned" &&
            info.kind.kind !== "All" &&
            info.kind.kind !== "Exclude" &&
            info.kind.kind !== "TagOnly")
    ) {
        clearHighlights(editor);
        return;
    }
    const lineRange = (line: number): vscode.Range => {
        const text = editor.document.lineAt(line).text;
        return new vscode.Range(line, 0, line, text.length);
    };
    editor.setDecorations(decorationType, [
        lineRange(cursorLine),
        lineRange(info.partnerLine),
    ]);
}

export function registerHighlight(
    ctx: vscode.ExtensionContext,
    _selector: vscode.DocumentSelector,
): void {
    rebuildDecoration();
    ctx.subscriptions.push({ dispose: disposeDecoration });

    const refreshActive = (): void => {
        const editor = vscode.window.activeTextEditor;
        if (editor) updateHighlights(editor);
    };

    ctx.subscriptions.push(
        vscode.window.onDidChangeTextEditorSelection((e) =>
            updateHighlights(e.textEditor),
        ),
        vscode.window.onDidChangeActiveTextEditor((editor) => {
            if (editor) updateHighlights(editor);
        }),
        vscode.workspace.onDidChangeTextDocument((e) => {
            const editor = vscode.window.activeTextEditor;
            if (editor && e.document === editor.document) {
                updateHighlights(editor);
            }
        }),
        vscode.workspace.onDidChangeConfiguration((e) => {
            if (!e.affectsConfiguration(CONFIG_SECTION)) return;
            rebuildDecoration();
            refreshActive();
        }),
    );

    refreshActive();
}
