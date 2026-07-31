// Source of truth: `src/parser.rs` (`detect_marker`, `MarkerKind`) and
// `src/filter.rs` (`parse_version`). This module must mirror that grammar
// exactly — any change to the Rust parser must be reflected here in lockstep.
//
// Pure module. No VSCode dependency. Unit-tested via vitest.

export type CommentStyle = '//' | '#';

export interface MarkerSpan {
    /** Column index (UTF-16 code unit, matching VSCode positions). Inclusive. */
    start: number;
    /** Exclusive end column. */
    end: number;
    text: string;
}

/** One condition on a marker tag: `{cond}`, or negated `{!cond}`. */
export interface MarkerCondition {
    name: string;
    negated: boolean;
}

export interface Marker {
    /** Empty for tag-only markers (`//version [wiki]`), which carry no version. */
    version: string;
    /** Upper bound for range markers; null for single-version or ALL. */
    to: string | null;
    tags: string[];
    /** Conditions attached to this marker's tags (`[stable{a}{!b}]`). */
    conditions: MarkerCondition[];
    hasStar: boolean;
    /** Sub-token spans on the original line — used by rename + highlight. */
    versionSpan: MarkerSpan;
    toSpan: MarkerSpan | null;
    tagSpans: MarkerSpan[];
    /** Spans of the condition names inside `{...}`, for rename/highlight. */
    conditionSpans: MarkerSpan[];
    starSpan: MarkerSpan | null;
}

export type MarkerKind = { kind: 'Versioned'; marker: Marker } | { kind: 'All'; marker: Marker } | { kind: 'Exclude'; marker: Marker } | { kind: 'TagOnly'; marker: Marker } | { kind: 'InlineRange'; marker: Marker } | { kind: 'Malformed'; reason: string } | { kind: 'None' };

const KEYWORD = 'version';

/**
 * Parse a single line and return its marker classification.
 *
 * Mirrors `detect_marker` in `src/parser.rs`. Grammar (after the comment
 * prefix and optional whitespace):
 *
 *   `version` <ws> <v1> [<ws> <v2>] [<ws> `[tag1,tag2,...]`] [<ws> `*`] <ws>*
 *
 * - `<v1>` is `ALL` / `EXC` (case-insensitive) or a parseable version.
 * - `<v2>` is an optional upper bound (only valid when `<v1>` is a version).
 *   - With `*`: range block (open/close paired by stack).
 *   - Without `*`: inline range (applies to the next line only).
 */
export function detectMarker(line: string, style: CommentStyle): MarkerKind {
    // 1. Skip leading whitespace.
    const trimStart = leadingWhitespaceLength(line);
    let cursor = trimStart;

    // 2. Strip comment prefix.
    if (!line.startsWith(style, cursor)) {
        return { kind: 'None' };
    }
    cursor += style.length;

    // 3. Skip whitespace, then keyword.
    cursor = skipWhitespace(line, cursor);
    if (!line.startsWith(KEYWORD, cursor)) {
        return { kind: 'None' };
    }
    cursor += KEYWORD.length;

    // 4. Require whitespace or EOL right after the keyword.
    if (cursor >= line.length) {
        return malformed('missing version, `ALL`/`EXC`, or `[tags]` after `version`');
    }
    if (!isWhitespace(line.charAt(cursor))) {
        return { kind: 'None' };
    }
    cursor = skipWhitespace(line, cursor);
    if (cursor >= line.length) {
        return malformed('missing version, `ALL`/`EXC`, or `[tags]` after `version`');
    }

    // 5. First token: ALL, EXC, or a version — omitted entirely for tag-only
    //    markers, where a `[tag]` list follows the keyword directly.
    const tagOnly = line.charAt(cursor) === '[';
    let versionToken = '';
    let versionSpan: MarkerSpan = { start: cursor, end: cursor, text: '' };
    let isAll = false;
    let isExc = false;
    if (!tagOnly) {
        const tokenStart = cursor;
        cursor = findTokenEnd(line, cursor);
        versionToken = line.slice(tokenStart, cursor);
        versionSpan = { start: tokenStart, end: cursor, text: versionToken };
        cursor = skipWhitespace(line, cursor);

        const upper = versionToken.toUpperCase();
        isAll = upper === 'ALL';
        isExc = upper === 'EXC';
        if (!isAll && !isExc && !isValidVersion(versionToken)) {
            return malformed(`unparseable version \`${versionToken}\``);
        }
    }
    const isKeyword = isAll || isExc;

    // 6. Optional second version token (`to`) — only for a real version (not ALL/EXC/tag-only).
    let toSpan: MarkerSpan | null = null;
    if (!tagOnly && !isKeyword && cursor < line.length && line.charAt(cursor) !== '[' && line.charAt(cursor) !== '*') {
        const nextStart = cursor;
        const nextEnd = findTokenEnd(line, cursor);
        const nextToken = line.slice(nextStart, nextEnd);
        // Only consume as `to` if it parses as a version. Otherwise leave for
        // the trailing-content check to flag (preserves Rust behavior for
        // `// version 2 of foo`).
        if (isValidVersion(nextToken)) {
            toSpan = { start: nextStart, end: nextEnd, text: nextToken };
            cursor = skipWhitespace(line, nextEnd);
        }
    }

    // 7. Optional [tags], each optionally carrying a `{condition}`.
    const tagSpans: MarkerSpan[] = [];
    const conditionSpans: MarkerSpan[] = [];
    const conditionList: MarkerCondition[] = [];
    if (cursor < line.length && line.charAt(cursor) === '[') {
        const openBracket = cursor;
        const closeBracket = line.indexOf(']', openBracket + 1);
        if (closeBracket < 0) {
            return malformed('unterminated `[` tag list');
        }
        // Split tag body on commas, tracking column ranges.
        let tagSearch = openBracket + 1;
        while (tagSearch <= closeBracket) {
            const commaOrEnd = nextCommaOrEnd(line, tagSearch, closeBracket);
            const raw = line.slice(tagSearch, commaOrEnd);
            const lead = raw.length - raw.replace(/^\s+/, '').length;
            const trail = raw.length - raw.replace(/\s+$/, '').length;
            const trimmed = raw.slice(lead, raw.length - trail);
            if (trimmed.length === 0) {
                return malformed('empty tag in list');
            }
            const entryStart = tagSearch + lead;
            const parsed = parseTagEntry(trimmed, entryStart);
            if ('error' in parsed) {
                return malformed(parsed.error);
            }
            tagSpans.push(parsed.tag);
            for (const c of parsed.conditions) {
                conditionSpans.push(c.span);
                conditionList.push({ name: c.span.text, negated: c.negated });
            }
            tagSearch = commaOrEnd + 1; // step past the comma
        }
        cursor = skipWhitespace(line, closeBracket + 1);
    }

    // 8. Optional `*`.
    let starSpan: MarkerSpan | null = null;
    if (cursor < line.length && line.charAt(cursor) === '*') {
        starSpan = { start: cursor, end: cursor + 1, text: '*' };
        cursor = skipWhitespace(line, cursor + 1);
    }

    // 9. Trailing content is malformed.
    if (cursor < line.length) {
        const trailing = line.slice(cursor).replace(/\s+$/, '');
        return malformed(`unexpected trailing content \`${trailing}\``);
    }

    // 10. Range marker semantics: from < to (else malformed).
    if (toSpan) {
        const fromV = parseVersionTuple(versionToken);
        const toV = parseVersionTuple(toSpan.text);
        if (fromV && toV && compareVersionTuple(fromV, toV) >= 0) {
            return malformed(`range marker has from >= to (${versionToken} >= ${toSpan.text})`);
        }
    }

    const hasStar = starSpan !== null;
    const marker: Marker = {
        version: versionToken,
        to: toSpan ? toSpan.text : null,
        tags: tagSpans.map((t) => t.text),
        conditions: conditionList,
        hasStar,
        versionSpan,
        toSpan,
        tagSpans,
        conditionSpans,
        starSpan,
    };
    if (tagOnly) {
        return { kind: 'TagOnly', marker };
    }
    if (isAll) {
        return { kind: 'All', marker };
    }
    if (isExc) {
        return { kind: 'Exclude', marker };
    }
    if (marker.to !== null && !hasStar) {
        return { kind: 'InlineRange', marker };
    }
    return { kind: 'Versioned', marker };
}

function malformed(reason: string): MarkerKind {
    return { kind: 'Malformed', reason };
}

interface ParsedTagEntry {
    tag: MarkerSpan;
    conditions: { span: MarkerSpan; negated: boolean }[];
}

/**
 * Parse one `[tag]` entry: a tag name followed by zero or more `{[!]condition}`
 * groups — `stable`, `stable{a}`, `stable{a}{!b}`.
 *
 * Mirrors `parse_tag_entry` in `src/parser.rs`. `entryStart` is the column of
 * `entry[0]` on the original line, so returned spans are absolute.
 */
function parseTagEntry(entry: string, entryStart: number): ParsedTagEntry | { error: string } {
    const brace = entry.indexOf('{');
    const rawName = brace >= 0 ? entry.slice(0, brace) : entry;
    const name = rawName.trim();
    if (name.length === 0) {
        return { error: 'missing tag name before `{`' };
    }
    if (name.includes('}')) {
        return { error: `stray \`}\` in tag \`${name}\` (conditions are written \`tag{name}\`)` };
    }
    const nameOffset = rawName.indexOf(name);
    const tag: MarkerSpan = {
        start: entryStart + nameOffset,
        end: entryStart + nameOffset + name.length,
        text: name,
    };

    const conditions: { span: MarkerSpan; negated: boolean }[] = [];
    let i = brace >= 0 ? brace : entry.length;
    while (i < entry.length) {
        while (i < entry.length && isWhitespace(entry.charAt(i))) i++;
        if (i >= entry.length) break;
        if (entry.charAt(i) !== '{') {
            return { error: `unexpected \`${entry.slice(i).trim()}\` after conditions on tag \`${name}\`` };
        }
        const close = entry.indexOf('}', i + 1);
        if (close < 0) {
            return { error: `unterminated \`{\` condition on tag \`${name}\`` };
        }
        const body = entry.slice(i + 1, close);
        const trimmedBody = body.trim();
        const negated = trimmedBody.startsWith('!');
        const condName = (negated ? trimmedBody.slice(1) : trimmedBody).trim();
        if (condName.length === 0) {
            return { error: `empty condition name on tag \`${name}\`` };
        }
        if (condName.includes('!') || condName.includes('{') || condName.includes('}')) {
            return { error: `invalid condition name \`${condName}\` on tag \`${name}\`` };
        }
        const condStart = entryStart + i + 1 + body.indexOf(condName);
        conditions.push({
            span: { start: condStart, end: condStart + condName.length, text: condName },
            negated,
        });
        i = close + 1;
    }
    return { tag, conditions };
}

function isWhitespace(ch: string): boolean {
    return /\s/.test(ch);
}

function leadingWhitespaceLength(s: string): number {
    let i = 0;
    while (i < s.length && isWhitespace(s.charAt(i))) i++;
    return i;
}

function skipWhitespace(s: string, from: number): number {
    let i = from;
    while (i < s.length && isWhitespace(s.charAt(i))) i++;
    return i;
}

function findTokenEnd(s: string, from: number): number {
    let i = from;
    while (i < s.length) {
        const c = s.charAt(i);
        if (isWhitespace(c) || c === '[') return i;
        i++;
    }
    return i;
}

function nextCommaOrEnd(s: string, from: number, end: number): number {
    let i = from;
    while (i < end) {
        if (s.charAt(i) === ',') return i;
        i++;
    }
    return end;
}

// ---- Version parsing ----------------------------------------------------
//
// Mirrors `parse_version` in `src/filter.rs`: pad to MAJOR.MINOR.PATCH and
// validate per semver 1.0 grammar. We don't bring in the full semver crate;
// for our purposes (parse + compare) a tuple is enough.

interface VersionTuple {
    major: number;
    minor: number;
    patch: number;
    pre: string;
    build: string;
}

const VERSION_RE = /^([0-9]+)\.([0-9]+)\.([0-9]+)(?:-([0-9A-Za-z.-]+))?(?:\+([0-9A-Za-z.-]+))?$/;

export function parseVersionTuple(raw: string): VersionTuple | null {
    const s = raw.trim();
    if (s.length === 0) return null;
    // Pad to three components based on dot count, matching Rust.
    const dots = (s.match(/\./g) || []).length;
    let padded = s;
    if (dots === 0) padded = `${s}.0.0`;
    else if (dots === 1) padded = `${s}.0`;
    const m = VERSION_RE.exec(padded);
    if (!m) return null;
    // semver forbids leading zeros on numeric components (e.g. "01.2.3").
    if (hasLeadingZero(m[1]) || hasLeadingZero(m[2]) || hasLeadingZero(m[3])) {
        return null;
    }
    const major = Number(m[1]);
    const minor = Number(m[2]);
    const patch = Number(m[3]);
    if (!Number.isSafeInteger(major) || !Number.isSafeInteger(minor) || !Number.isSafeInteger(patch)) {
        return null;
    }
    return {
        major,
        minor,
        patch,
        pre: m[4] ?? '',
        build: m[5] ?? '',
    };
}

function hasLeadingZero(s: string): boolean {
    return s.length > 1 && s.charAt(0) === '0';
}

export function isValidVersion(raw: string): boolean {
    return parseVersionTuple(raw) !== null;
}

export function compareVersionTuple(a: VersionTuple, b: VersionTuple): number {
    if (a.major !== b.major) return a.major - b.major;
    if (a.minor !== b.minor) return a.minor - b.minor;
    if (a.patch !== b.patch) return a.patch - b.patch;
    // Pre-release: empty (no pre) ranks higher than any pre-release.
    if (a.pre === '' && b.pre !== '') return 1;
    if (a.pre !== '' && b.pre === '') return -1;
    if (a.pre === b.pre) return 0;
    return comparePreRelease(a.pre, b.pre);
}

function comparePreRelease(a: string, b: string): number {
    const as = a.split('.');
    const bs = b.split('.');
    const n = Math.min(as.length, bs.length);
    for (let i = 0; i < n; i++) {
        const ai = as[i];
        const bi = bs[i];
        const aNum = /^[0-9]+$/.test(ai);
        const bNum = /^[0-9]+$/.test(bi);
        if (aNum && bNum) {
            const diff = Number(ai) - Number(bi);
            if (diff !== 0) return diff;
        } else if (aNum) {
            return -1; // numeric ranks lower than alphanumeric
        } else if (bNum) {
            return 1;
        } else if (ai !== bi) {
            return ai < bi ? -1 : 1;
        }
    }
    return as.length - bs.length;
}

// ---- Pair-key helper ----------------------------------------------------
//
// Mirrors the stack-pairing rule in `parser.rs::process_file`: two markers
// pair when their (version, to) tuples are equal. ALL pairs on the literal
// "ALL" (case-insensitive).

export function markerPairKey(marker: Marker, kind: 'Versioned' | 'All' | 'Exclude' | 'TagOnly'): string {
    if (kind === 'Exclude') return 'EXC';
    if (kind === 'All') return 'ALL';
    // Tag-only markers have no version, so they pair on their tag+condition list.
    if (kind === 'TagOnly') {
        const conds = marker.conditions.map((c) => (c.negated ? `!${c.name}` : c.name)).join(',');
        return `T:${marker.tags.join(',')}|${conds}`;
    }
    return `V:${marker.version} ${marker.to ?? ''}`;
}
