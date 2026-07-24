import * as vscode from "vscode";
import { commentStyleFor } from "./extension";
import { detectMarker, Marker, MarkerSpan } from "./marker";
import { pairDocument } from "./pairing";

type RenameRole =
    | { kind: "version" }
    | { kind: "to" }
    | { kind: "tag"; index: number };

interface RenameTarget {
    span: MarkerSpan;
    role: RenameRole;
}

function findRenameTarget(marker: Marker, col: number): RenameTarget | null {
    const v = marker.versionSpan;
    if (col >= v.start && col <= v.end) {
        return { span: v, role: { kind: "version" } };
    }
    if (marker.toSpan && col >= marker.toSpan.start && col <= marker.toSpan.end) {
        return { span: marker.toSpan, role: { kind: "to" } };
    }
    for (let i = 0; i < marker.tagSpans.length; i++) {
        const s = marker.tagSpans[i];
        if (col >= s.start && col <= s.end) {
            return { span: s, role: { kind: "tag", index: i } };
        }
    }
    return null;
}

function findSpanForRole(marker: Marker, role: RenameRole): MarkerSpan | null {
    switch (role.kind) {
        case "version":
            return marker.versionSpan;
        case "to":
            return marker.toSpan;
        case "tag":
            return marker.tagSpans[role.index] ?? null;
    }
}

class RenameProvider implements vscode.RenameProvider {
    prepareRename(
        document: vscode.TextDocument,
        position: vscode.Position,
        _token: vscode.CancellationToken,
    ): vscode.Range {
        const style = commentStyleFor(document.languageId);
        const lineText = document.lineAt(position.line).text;
        const kind = detectMarker(lineText, style);
        if (
            kind.kind !== "Versioned" &&
            kind.kind !== "All" &&
            kind.kind !== "Exclude" &&
            kind.kind !== "InlineRange"
        ) {
            throw new Error("Not a Vertion marker token");
        }
        const target = findRenameTarget(kind.marker, position.character);
        if (!target) {
            throw new Error("Place the cursor on a version, upper bound, or tag");
        }
        return new vscode.Range(
            position.line,
            target.span.start,
            position.line,
            target.span.end,
        );
    }

    provideRenameEdits(
        document: vscode.TextDocument,
        position: vscode.Position,
        newName: string,
        _token: vscode.CancellationToken,
    ): vscode.WorkspaceEdit | undefined {
        const style = commentStyleFor(document.languageId);
        const lineText = document.lineAt(position.line).text;
        const kind = detectMarker(lineText, style);
        if (
            kind.kind !== "Versioned" &&
            kind.kind !== "All" &&
            kind.kind !== "Exclude" &&
            kind.kind !== "InlineRange"
        ) {
            return undefined;
        }
        const target = findRenameTarget(kind.marker, position.character);
        if (!target) return undefined;

        const edit = new vscode.WorkspaceEdit();
        edit.replace(
            document.uri,
            new vscode.Range(
                position.line,
                target.span.start,
                position.line,
                target.span.end,
            ),
            newName,
        );

        // Pair-aware rename: find the partner and apply the same edit there.
        const pairing = pairDocument(document, style);
        const info = pairing.byLine.get(position.line);
        if (info && info.partnerLine !== null) {
            const partnerInfo = pairing.byLine.get(info.partnerLine);
            if (
                partnerInfo &&
                (partnerInfo.kind.kind === "Versioned" ||
                    partnerInfo.kind.kind === "All" ||
                    partnerInfo.kind.kind === "Exclude")
            ) {
                const partnerSpan = findSpanForRole(
                    partnerInfo.kind.marker,
                    target.role,
                );
                if (partnerSpan) {
                    edit.replace(
                        document.uri,
                        new vscode.Range(
                            info.partnerLine,
                            partnerSpan.start,
                            info.partnerLine,
                            partnerSpan.end,
                        ),
                        newName,
                    );
                }
            }
        } else if (
            kind.kind === "Versioned" ||
            kind.kind === "All" ||
            kind.kind === "Exclude"
        ) {
            // Block-style marker with no partner — warn that only one side was edited.
            vscode.window.showWarningMessage(
                "Vertion: rename applied to a single line (no matching partner found).",
            );
        }

        return edit;
    }
}

export function registerRename(
    ctx: vscode.ExtensionContext,
    selector: vscode.DocumentSelector,
): void {
    ctx.subscriptions.push(
        vscode.languages.registerRenameProvider(selector, new RenameProvider()),
    );
}
