import { describe, expect, it } from "vitest";
import {
    DEFAULT_STEM,
    describeVariant,
    isVariantError,
    parseVariantStem,
    targetExtension,
    targetNameFromVariantDir,
    variantDirName,
} from "../src/variantName";

function ok(stem: string) {
    const r = parseVariantStem(stem);
    if (isVariantError(r)) {
        expect.fail(`${stem}: ${r.error}`);
        throw new Error("unreachable");
    }
    return r;
}

describe("parseVariantStem — parity with variants.rs", () => {
    it("parses a plain version", () => {
        const s = ok("2.0.0");
        expect(s.min).toBe("2.0.0");
        expect(s.max).toBeNull();
        expect(s.tags).toEqual([]);
    });

    it("parses a version range", () => {
        const s = ok("1.2.3e2.0.0");
        expect(s.min).toBe("1.2.3");
        expect(s.max).toBe("2.0.0");
    });

    it("reads a bare tag even when it contains an `e`", () => {
        // `beta` must not be mistaken for a `b`e`ta` range.
        const s = ok("beta");
        expect(s.min).toBeNull();
        expect(s.tags).toEqual(["beta"]);
    });

    it("parses version + tags + conditions", () => {
        const s = ok("2.0.0-beta@a@b");
        expect(s.min).toBe("2.0.0");
        expect(s.tags).toEqual(["beta"]);
        expect(s.conditions).toEqual([
            { name: "a", negated: false },
            { name: "b", negated: false },
        ]);
    });

    it("parses multiple tags and negated conditions", () => {
        expect(ok("2.0.0-beta-combat").tags).toEqual(["beta", "combat"]);
        expect(ok("beta@!legacy").conditions).toEqual([
            { name: "legacy", negated: true },
        ]);
    });

    it("flags the reserved default stem", () => {
        expect(ok(DEFAULT_STEM).isDefault).toBe(true);
    });

    it("rejects malformed stems", () => {
        for (const bad of ["", "2.0.0e1.0.0", "2.0.0-", "2.0.0-@cond", "beta@"]) {
            expect(isVariantError(parseVariantStem(bad)), bad).toBe(true);
        }
    });
});

describe("directory naming", () => {
    it("round-trips target names", () => {
        expect(variantDirName("logo.png")).toBe(".vertion.logo.png");
        expect(targetNameFromVariantDir(".vertion.logo.png")).toBe("logo.png");
        expect(targetNameFromVariantDir("logo.png")).toBeNull();
        expect(targetNameFromVariantDir(".vertion.")).toBeNull();
    });

    it("derives the declared extension, or null for folders", () => {
        expect(targetExtension(".vertion.logo.png")).toBe("png");
        expect(targetExtension(".vertion.assets")).toBeNull();
        // A dotfile target has no extension — the dot is part of its name.
        expect(targetExtension(".vertion..gitignore")).toBeNull();
    });
});

describe("describeVariant", () => {
    it("summarizes windows and tags in words", () => {
        expect(describeVariant(ok("1.2.3e2.0.0"))).toContain("1.2.3 ≤ build < 2.0.0");
        expect(describeVariant(ok("2.0.0"))).toContain("build ≥ 2.0.0");
        expect(describeVariant(ok("beta"))).toContain("any version");
        expect(describeVariant(ok("2.0.0-beta@!legacy"))).toContain("not legacy");
        expect(describeVariant(ok(DEFAULT_STEM))).toContain("fallback");
    });
});
