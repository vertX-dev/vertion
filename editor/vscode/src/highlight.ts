import * as vscode from "vscode";
import { commentStyleFor } from "./extension";
import { pairDocument } from "./pairing";

class HighlightProvider implements vscode.DocumentHighlightProvider {
    provideDocumentHighlights(
        document: vscode.TextDocument,
        position: vscode.Position,
        _token: vscode.CancellationToken,
    ): vscode.DocumentHighlight[] {
        const style = commentStyleFor(document.languageId);
        const pairing = pairDocument(document, style);
        const info = pairing.byLine.get(position.line);
        if (!info || info.partnerLine === null) return [];
        if (info.kind.kind !== "Versioned" && info.kind.kind !== "All") {
            return [];
        }
        const thisLine = document.lineAt(position.line).text;
        const partnerLine = document.lineAt(info.partnerLine).text;
        return [
            new vscode.DocumentHighlight(
                new vscode.Range(position.line, 0, position.line, thisLine.length),
                vscode.DocumentHighlightKind.Text,
            ),
            new vscode.DocumentHighlight(
                new vscode.Range(
                    info.partnerLine,
                    0,
                    info.partnerLine,
                    partnerLine.length,
                ),
                vscode.DocumentHighlightKind.Text,
            ),
        ];
    }
}

export function registerHighlight(
    ctx: vscode.ExtensionContext,
    selector: vscode.DocumentSelector,
): void {
    ctx.subscriptions.push(
        vscode.languages.registerDocumentHighlightProvider(
            selector,
            new HighlightProvider(),
        ),
    );
}
