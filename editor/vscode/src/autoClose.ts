import * as vscode from "vscode";
import { commentStyleFor } from "./extension";
import { detectMarker } from "./marker";
import { pairDocument } from "./pairing";

class AutoCloseProvider implements vscode.OnTypeFormattingEditProvider {
    provideOnTypeFormattingEdits(
        document: vscode.TextDocument,
        position: vscode.Position,
        ch: string,
        _options: vscode.FormattingOptions,
        _token: vscode.CancellationToken,
    ): vscode.TextEdit[] {
        if (ch !== "\n" || position.line === 0) return [];

        const style = commentStyleFor(document.languageId);
        const prevLineIndex = position.line - 1;
        const prevLineText = document.lineAt(prevLineIndex).text;
        const kind = detectMarker(prevLineText, style);

        // Only fire for versioned-with-star, ALL / EXC, or tag-only openers.
        const fires =
            (kind.kind === "Versioned" && kind.marker.hasStar) ||
            kind.kind === "All" ||
            kind.kind === "Exclude" ||
            kind.kind === "TagOnly";
        if (!fires) return [];

        // Skip if this marker is already paired (it's either a close or an
        // open that already has its partner).
        const pairing = pairDocument(document, style);
        const lineInfo = pairing.byLine.get(prevLineIndex);
        if (lineInfo && lineInfo.partnerLine !== null) return [];

        const indent = prevLineText.match(/^\s*/)?.[0] ?? "";
        const openContent = prevLineText.trim();
        const closeLine = indent + openContent;

        return [vscode.TextEdit.insert(position, "\n" + closeLine)];
    }
}

export function registerAutoClose(
    ctx: vscode.ExtensionContext,
    selector: vscode.DocumentSelector,
): void {
    ctx.subscriptions.push(
        vscode.languages.registerOnTypeFormattingEditProvider(
            selector,
            new AutoCloseProvider(),
            "\n",
        ),
    );
}
