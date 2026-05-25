// Mirror of `parser.rs::process_file`'s stack-pairing rule. Walks a document
// once and returns pair/unclosed information so each provider can share the
// same view of the marker tree.
//
// Pure module (no vscode import). The document-shaped interface `DocLike`
// lets us drop a real vscode.TextDocument in at runtime without forcing
// tests to mock the editor.

import {
    CommentStyle,
    detectMarker,
    Marker,
    MarkerKind,
} from "./marker";

export type PairKind = "Versioned" | "All";

export interface PairedMarker {
    openLine: number;
    closeLine: number;
    openMarker: Marker;
    closeMarker: Marker;
    kind: PairKind;
}

export interface UnclosedMarker {
    line: number;
    marker: Marker;
    kind: PairKind;
}

export interface LineMarkerInfo {
    line: number;
    kind: MarkerKind;
    /** When the line is part of a pair, the partner line number. */
    partnerLine: number | null;
}

export interface PairingResult {
    pairs: PairedMarker[];
    unclosed: UnclosedMarker[];
    /** Marker info indexed by line for O(1) lookup. */
    byLine: Map<number, LineMarkerInfo>;
}

interface StackEntry {
    line: number;
    marker: Marker;
    kind: PairKind;
}

export function pairLines(lines: string[], style: CommentStyle): PairingResult {
    const stack: StackEntry[] = [];
    const pairs: PairedMarker[] = [];
    const byLine = new Map<number, LineMarkerInfo>();

    for (let i = 0; i < lines.length; i++) {
        const kind = detectMarker(lines[i], style);
        let partnerLine: number | null = null;

        switch (kind.kind) {
            case "Versioned": {
                const m = kind.marker;
                const top = stack[stack.length - 1];
                if (
                    top &&
                    top.kind === "Versioned" &&
                    top.marker.version === m.version &&
                    top.marker.to === m.to
                ) {
                    stack.pop();
                    pairs.push({
                        openLine: top.line,
                        closeLine: i,
                        openMarker: top.marker,
                        closeMarker: m,
                        kind: "Versioned",
                    });
                    partnerLine = top.line;
                    const openInfo = byLine.get(top.line);
                    if (openInfo) openInfo.partnerLine = i;
                } else {
                    stack.push({ line: i, marker: m, kind: "Versioned" });
                }
                break;
            }
            case "All": {
                const m = kind.marker;
                const top = stack[stack.length - 1];
                if (
                    top &&
                    top.kind === "All" &&
                    top.marker.version.toUpperCase() === "ALL"
                ) {
                    stack.pop();
                    pairs.push({
                        openLine: top.line,
                        closeLine: i,
                        openMarker: top.marker,
                        closeMarker: m,
                        kind: "All",
                    });
                    partnerLine = top.line;
                    const openInfo = byLine.get(top.line);
                    if (openInfo) openInfo.partnerLine = i;
                } else {
                    stack.push({ line: i, marker: m, kind: "All" });
                }
                break;
            }
            default:
                break;
        }

        byLine.set(i, { line: i, kind, partnerLine });
    }

    const unclosed: UnclosedMarker[] = stack.map((s) => ({
        line: s.line,
        marker: s.marker,
        kind: s.kind,
    }));

    return { pairs, unclosed, byLine };
}

// ---- Document-aware cache ----------------------------------------------
//
// Providers run frequently (cursor moves, edits, folding rebuilds). Repeating
// the full-document walk for every call wastes work, so cache by document
// uri + version.

export interface DocLike {
    uri: { toString(): string };
    version: number;
    lineCount: number;
    lineAt(line: number): { text: string };
}

interface CacheEntry {
    docVersion: number;
    result: PairingResult;
}

const cache = new Map<string, CacheEntry>();

export function pairDocument(doc: DocLike, style: CommentStyle): PairingResult {
    const key = doc.uri.toString();
    const cached = cache.get(key);
    if (cached && cached.docVersion === doc.version) return cached.result;
    const lines: string[] = [];
    for (let i = 0; i < doc.lineCount; i++) {
        lines.push(doc.lineAt(i).text);
    }
    const result = pairLines(lines, style);
    cache.set(key, { docVersion: doc.version, result });
    return result;
}

export function invalidatePairingCache(uri?: string): void {
    if (uri) cache.delete(uri);
    else cache.clear();
}
