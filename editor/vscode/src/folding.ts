import * as vscode from "vscode";
import { commentStyleFor } from "./extension";
import { pairDocument, foldRangesFromPairs } from "./pairing";

class FoldingProvider implements vscode.FoldingRangeProvider {
    provideFoldingRanges(
        document: vscode.TextDocument,
        _context: vscode.FoldingContext,
        _token: vscode.CancellationToken,
    ): vscode.FoldingRange[] {
        const style = commentStyleFor(document.languageId);
        const pairing = pairDocument(document, style);
        return foldRangesFromPairs(pairing.pairs).map(
            (r) =>
                new vscode.FoldingRange(
                    r.start,
                    r.end,
                    vscode.FoldingRangeKind.Region,
                ),
        );
    }
}

export function registerFolding(
    ctx: vscode.ExtensionContext,
    selector: vscode.DocumentSelector,
): void {
    ctx.subscriptions.push(
        vscode.languages.registerFoldingRangeProvider(
            selector,
            new FoldingProvider(),
        ),
    );
}
