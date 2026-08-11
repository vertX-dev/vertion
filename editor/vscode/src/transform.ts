// Block-rewriting transforms: duplicate a block under a new version, and split
// a block that contains nested version blocks into one standalone block per
// version variant.
//
// Pure module (no vscode import) so the line-level logic is unit-testable.

import { CommentStyle, Marker } from "./marker";
import { pairLines, PairedMarker } from "./pairing";

export interface TransformError {
    error: string;
}

export function isError<T>(r: T | TransformError): r is TransformError {
    return typeof r === "object" && r !== null && "error" in r;
}

/** A whole-document replacement of the line range `[startLine, endLine]`. */
export interface LineEdit {
    startLine: number;
    endLine: number;
    lines: string[];
}

// ---- Block lookup --------------------------------------------------------

function contains(outer: PairedMarker, inner: PairedMarker): boolean {
    return (
        outer.openLine < inner.openLine && inner.closeLine < outer.closeLine
    );
}

/** Blocks containing `line`, innermost first. */
function enclosingBlocks(pairs: PairedMarker[], line: number): PairedMarker[] {
    return pairs
        .filter((p) => p.openLine <= line && line <= p.closeLine)
        .sort((a, b) => b.openLine - a.openLine);
}

/** Blocks nested directly inside `block` (no intermediate ancestor). */
export function directChildren(
    pairs: PairedMarker[],
    block: PairedMarker,
): PairedMarker[] {
    const inside = pairs.filter((p) => contains(block, p));
    return inside
        .filter(
            (p) =>
                !inside.some(
                    (q) => q !== p && contains(block, q) && contains(q, p),
                ),
        )
        .sort((a, b) => a.openLine - b.openLine);
}

// ---- Duplicate -----------------------------------------------------------

/**
 * Rewrite a marker line to carry `spec` as its version, preserving everything
 * else (tags, conditions, `*`, spacing) by splicing over the version span.
 *
 * `spec` may be a single version, a range (`1.3 2.0`), or `ALL`/`EXC`.
 */
export function markerWithVersion(
    lineText: string,
    marker: Marker,
    spec: string,
): string {
    const start = marker.versionSpan.start;
    // Replacing through `to` collapses a range marker when a single version is given.
    const end = (marker.toSpan ?? marker.versionSpan).end;
    // Tag-only markers have an empty version span sitting at the `[`, so the
    // inserted version needs its own separating space.
    const pad = marker.version === "" ? " " : "";
    return lineText.slice(0, start) + spec + pad + lineText.slice(end);
}

/**
 * Duplicate the block under `cursorLine`, inserting a copy directly beneath it
 * with `spec` as the new version.
 */
export function computeDuplicate(
    lines: string[],
    style: CommentStyle,
    cursorLine: number,
    spec: string,
): LineEdit | TransformError {
    const pairing = pairLines(lines, style);
    const enclosing = enclosingBlocks(pairing.pairs, cursorLine);
    if (enclosing.length === 0) {
        return { error: "Place the cursor inside a version block to duplicate it." };
    }
    const block = enclosing[0];
    const openText = lines[block.openLine];
    const newMarker = markerWithVersion(openText, block.openMarker, spec);
    const body = lines.slice(block.openLine + 1, block.closeLine);

    // Original block, then a blank separator, then the copy.
    const out = [
        ...lines.slice(block.openLine, block.closeLine + 1),
        "",
        newMarker,
        ...body,
        newMarker,
    ];
    return { startLine: block.openLine, endLine: block.closeLine, lines: out };
}

/** Line offset (from the edit's start) of the first body line of the copy. */
export function duplicateCursorOffset(
    lines: string[],
    style: CommentStyle,
    cursorLine: number,
): number {
    const pairing = pairLines(lines, style);
    const block = enclosingBlocks(pairing.pairs, cursorLine)[0];
    if (!block) return 0;
    // original block (closeLine-openLine+1) + blank + new open marker
    return block.closeLine - block.openLine + 3;
}

// ---- Split ---------------------------------------------------------------

export interface SplitResult extends LineEdit {
    /** Number of blocks produced (1 base variant + one per nested block). */
    variantCount: number;
    /** Marker text of each produced block, for the confirmation message. */
    variantLabels: string[];
}

function isBlank(s: string): boolean {
    return s.trim() === "";
}

function trimBlanks(out: string[]): void {
    while (out.length > 0 && isBlank(out[out.length - 1])) out.pop();
}

/**
 * Body of one variant: the target block's own content, with every nested block
 * removed except `variant`, whose markers are dissolved and content inlined.
 *
 * Blank lines directly adjacent to a nested block's markers are dropped — they
 * were separating the block, and leaving them behind creates ragged gaps.
 */
function variantBody(
    lines: string[],
    block: PairedMarker,
    children: PairedMarker[],
    variant: PairedMarker | null,
): string[] {
    const out: string[] = [];
    let i = block.openLine + 1;
    while (i < block.closeLine) {
        const child = children.find(
            (c) => c.openLine <= i && i <= c.closeLine,
        );
        if (!child) {
            out.push(lines[i]);
            i++;
            continue;
        }
        trimBlanks(out);
        if (child === variant) {
            const inner = lines.slice(child.openLine + 1, child.closeLine);
            while (inner.length > 0 && isBlank(inner[0])) inner.shift();
            while (inner.length > 0 && isBlank(inner[inner.length - 1])) inner.pop();
            out.push(...inner);
        }
        i = child.closeLine + 1;
        while (i < block.closeLine && isBlank(lines[i])) i++;
    }
    trimBlanks(out);
    while (out.length > 0 && isBlank(out[0])) out.shift();
    return out;
}

/**
 * Split the nearest enclosing block that has nested version blocks into one
 * standalone block per variant: the base content alone, then base content plus
 * each nested block's content inlined under that block's own marker.
 */
export function computeSplit(
    lines: string[],
    style: CommentStyle,
    cursorLine: number,
): SplitResult | TransformError {
    const pairing = pairLines(lines, style);
    const enclosing = enclosingBlocks(pairing.pairs, cursorLine);
    if (enclosing.length === 0) {
        return { error: "Place the cursor inside a version block to split it." };
    }
    // Walk outward to the first block that actually contains nested blocks, so
    // the command works with the cursor anywhere — including inside a child.
    let block: PairedMarker | undefined;
    let children: PairedMarker[] = [];
    for (const candidate of enclosing) {
        const kids = directChildren(pairing.pairs, candidate);
        if (kids.length > 0) {
            block = candidate;
            children = kids;
            break;
        }
    }
    if (!block) {
        return {
            error: "No nested version blocks here — nothing to split.",
        };
    }

    const variants: (PairedMarker | null)[] = [null, ...children];
    const out: string[] = [];
    const labels: string[] = [];
    for (const variant of variants) {
        // Reuse the open marker verbatim for both ends, which also normalizes a
        // close marker that was written with different spacing.
        const markerText = lines[(variant ?? block).openLine];
        if (out.length > 0) out.push("");
        out.push(markerText, ...variantBody(lines, block, children, variant), markerText);
        labels.push(markerText.trim());
    }

    return {
        startLine: block.openLine,
        endLine: block.closeLine,
        lines: out,
        variantCount: variants.length,
        variantLabels: labels,
    };
}
