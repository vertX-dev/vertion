// Source of truth: `src/variants.rs`. This module must mirror that grammar
// exactly — any change to the Rust variant parser must land here in lockstep.
//
// Pure module. No VSCode dependency. Unit-tested via vitest.

import { compareVersionTuple, parseVersionTuple } from './marker';

/** Directories starting with this hold per-version variants of one target. */
export const VARIANT_PREFIX = '.vertion.';

/** Reserved stem for the variant used when nothing else matches. */
export const DEFAULT_STEM = '.vertion.default';

export interface VariantCondition {
    name: string;
    negated: boolean;
}

export interface VariantSpec {
    /** Lower bound, inclusive. `null` means "any version". */
    min: string | null;
    /** Upper bound, exclusive (the `e` separator). */
    max: string | null;
    tags: string[];
    conditions: VariantCondition[];
    isDefault: boolean;
}

export interface VariantError {
    error: string;
}

export function isVariantError<T>(v: T | VariantError): v is VariantError {
    return typeof v === 'object' && v !== null && 'error' in v;
}

/**
 * `1.2.3` or `1.2.3e2.0.0`. Returns null when the segment isn't a version at
 * all, so it can be re-read as a tag — `beta` contains an `e`, which is exactly
 * why this has to fail softly rather than throw.
 */
function parseVersionSegment(s: string): { min: string; max: string | null } | null {
    const e = s.indexOf('e');
    if (e >= 0) {
        const lo = s.slice(0, e);
        const hi = s.slice(e + 1);
        if (parseVersionTuple(lo) && parseVersionTuple(hi)) {
            return { min: lo, max: hi };
        }
    }
    return parseVersionTuple(s) ? { min: s, max: null } : null;
}

/**
 * Parse a variant stem (the filename without its extension).
 *
 * Grammar: `[<min>[e<max>]]` followed by `-<tag>[@<cond>...]` groups. The
 * leading `-` is only needed when something precedes, so a stem may start with
 * a tag directly (`beta`).
 */
export function parseVariantStem(stem: string): VariantSpec | VariantError {
    if (stem === DEFAULT_STEM) {
        return { min: null, max: null, tags: [], conditions: [], isDefault: true };
    }
    if (stem.length === 0) {
        return { error: 'empty variant name' };
    }

    const segments = stem.split('-');
    const spec: VariantSpec = {
        min: null,
        max: null,
        tags: [],
        conditions: [],
        isDefault: false,
    };

    const pending: string[] = [];
    const first = parseVersionSegment(segments[0]);
    if (first) {
        if (first.max) {
            const lo = parseVersionTuple(first.min)!;
            const hi = parseVersionTuple(first.max)!;
            if (compareVersionTuple(lo, hi) >= 0) {
                return { error: `version range \`${segments[0]}\` has min >= max` };
            }
        }
        spec.min = first.min;
        spec.max = first.max;
    } else {
        pending.push(segments[0]);
    }
    pending.push(...segments.slice(1));

    for (const group of pending) {
        if (group.length === 0) {
            return { error: `empty tag in \`${stem}\`` };
        }
        const parts = group.split('@');
        const tag = parts[0].trim();
        if (tag.length === 0) {
            return { error: `missing tag name before \`@\` in \`${stem}\`` };
        }
        spec.tags.push(tag);
        for (const raw of parts.slice(1)) {
            const body = raw.trim();
            const negated = body.startsWith('!');
            const name = (negated ? body.slice(1) : body).trim();
            if (name.length === 0) {
                return { error: `empty condition name in \`${stem}\`` };
            }
            if (name.includes('!') || name.includes('{') || name.includes('}')) {
                return { error: `invalid condition name \`${name}\` in \`${stem}\`` };
            }
            spec.conditions.push({ name, negated });
        }
    }
    return spec;
}

/** Human-readable summary, for quick-pick descriptions. */
export function describeVariant(spec: VariantSpec): string {
    if (spec.isDefault) return 'fallback when nothing else matches';
    const bits: string[] = [];
    if (spec.min && spec.max) bits.push(`${spec.min} ≤ build < ${spec.max}`);
    else if (spec.min) bits.push(`build ≥ ${spec.min}`);
    else bits.push('any version');
    if (spec.tags.length > 0) bits.push(`tag ${spec.tags.join(' or ')}`);
    for (const c of spec.conditions) {
        bits.push(c.negated ? `not ${c.name}` : c.name);
    }
    return bits.join(', ');
}

/** `.vertion.logo.png` → `logo.png`; anything else → null. */
export function targetNameFromVariantDir(dirName: string): string | null {
    if (!dirName.startsWith(VARIANT_PREFIX)) return null;
    const target = dirName.slice(VARIANT_PREFIX.length);
    return target.length > 0 ? target : null;
}

/** `logo.png` → `.vertion.logo.png`. */
export function variantDirName(targetName: string): string {
    return VARIANT_PREFIX + targetName;
}

/**
 * Extension the directory declares, without the dot — `.vertion.logo.png` → `png`.
 * Null means the target is a folder, so variants inside are folders too.
 */
export function targetExtension(dirName: string): string | null {
    const target = targetNameFromVariantDir(dirName);
    if (!target) return null;
    const dot = target.lastIndexOf('.');
    // A leading dot is part of the name (`.gitignore`), not an extension.
    if (dot <= 0 || dot === target.length - 1) return null;
    return target.slice(dot + 1);
}
