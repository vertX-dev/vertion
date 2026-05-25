import * as vscode from "vscode";
import { commentStyleFor } from "./extension";
import { pairDocument } from "./pairing";

class FoldingProvider implements vscode.FoldingRangeProvider {
    provideFoldingRanges(
        document: vscode.TextDocument,
        _context: vscode.FoldingContext,
        _token: vscode.CancellationToken,
    ): vscode.FoldingRange[] {
        const style = commentStyleFor(document.languageId);
        const pairing = pairDocument(document, style);
        const ranges: vscode.FoldingRange[] = [];
        for (const p of pairing.pairs) {
            if (p.closeLine <= p.openLine + 1) continue;
            ranges.push(
                new vscode.FoldingRange(
                    p.openLine,
                    p.closeLine - 1,
                    vscode.FoldingRangeKind.Region,
                ),
            );
        }
        return ranges;
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
