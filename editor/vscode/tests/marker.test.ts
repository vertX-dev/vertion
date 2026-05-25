import { describe, expect, it } from "vitest";
import { detectMarker, isValidVersion, parseVersionTuple, compareVersionTuple } from "../src/marker";
import { pairLines } from "../src/pairing";

describe("detectMarker — Rust grammar parity", () => {
    it("parses a basic versioned open", () => {
        const k = detectMarker("//version 1.2 *", "//");
        expect(k.kind).toBe("Versioned");
        if (k.kind !== "Versioned") return;
        expect(k.marker.version).toBe("1.2");
        expect(k.marker.to).toBeNull();
        expect(k.marker.tags).toEqual([]);
        expect(k.marker.hasStar).toBe(true);
    });

    it("parses a versioned open with # comment", () => {
        const k = detectMarker("#version 1.2", "#");
        expect(k.kind).toBe("Versioned");
        if (k.kind !== "Versioned") return;
        expect(k.marker.version).toBe("1.2");
        expect(k.marker.hasStar).toBe(false);
    });

    it("parses tags", () => {
        const k = detectMarker("//version 1.2 [inventory,combat] *", "//");
        expect(k.kind).toBe("Versioned");
        if (k.kind !== "Versioned") return;
        expect(k.marker.tags).toEqual(["inventory", "combat"]);
    });

    it("parses ALL markers", () => {
        const k = detectMarker("//version ALL", "//");
        expect(k.kind).toBe("All");
    });

    it("ALL is case-insensitive", () => {
        expect(detectMarker("//version all", "//").kind).toBe("All");
        expect(detectMarker("//version All", "//").kind).toBe("All");
    });

    it("inline range without star", () => {
        const k = detectMarker("//version 1.3 2.0", "//");
        expect(k.kind).toBe("InlineRange");
        if (k.kind !== "InlineRange") return;
        expect(k.marker.version).toBe("1.3");
        expect(k.marker.to).toBe("2.0");
    });

    it("range block with star is Versioned", () => {
        const k = detectMarker("//version 1.3 2.0 *", "//");
        expect(k.kind).toBe("Versioned");
        if (k.kind !== "Versioned") return;
        expect(k.marker.version).toBe("1.3");
        expect(k.marker.to).toBe("2.0");
        expect(k.marker.hasStar).toBe(true);
    });

    it("range with tags", () => {
        const k = detectMarker("//version 1.3 2.0 [inventory,beta] *", "//");
        expect(k.kind).toBe("Versioned");
        if (k.kind !== "Versioned") return;
        expect(k.marker.to).toBe("2.0");
        expect(k.marker.tags).toEqual(["inventory", "beta"]);
    });

    it("range with from >= to is malformed", () => {
        const k = detectMarker("//version 2.0 1.3 *", "//");
        expect(k.kind).toBe("Malformed");
    });

    it("// version 2 of foo is malformed (trailing content)", () => {
        const k = detectMarker("// version 2 of foo", "//");
        expect(k.kind).toBe("Malformed");
    });

    it("//versionish is None (not a marker)", () => {
        const k = detectMarker("//versionish 1.2", "//");
        expect(k.kind).toBe("None");
    });

    it("missing version is malformed", () => {
        expect(detectMarker("//version", "//").kind).toBe("Malformed");
        expect(detectMarker("//version   ", "//").kind).toBe("Malformed");
    });

    it("unparseable version is malformed", () => {
        const k = detectMarker("//version notaversion *", "//");
        expect(k.kind).toBe("Malformed");
    });

    it("unterminated tag list is malformed", () => {
        const k = detectMarker("//version 1.2 [oops", "//");
        expect(k.kind).toBe("Malformed");
    });

    it("empty tag in list is malformed", () => {
        const k = detectMarker("//version 1.2 [,beta] *", "//");
        expect(k.kind).toBe("Malformed");
    });

    it("handles leading whitespace", () => {
        const k = detectMarker("    //version 1.2 *", "//");
        expect(k.kind).toBe("Versioned");
    });

    it("captures version span correctly", () => {
        const k = detectMarker("//version 1.2 *", "//");
        if (k.kind !== "Versioned") {
            expect.fail("expected Versioned");
            return;
        }
        const text = "//version 1.2 *";
        expect(text.slice(k.marker.versionSpan.start, k.marker.versionSpan.end)).toBe("1.2");
        expect(k.marker.starSpan?.start).toBe(text.indexOf("*"));
    });

    it("captures to span correctly", () => {
        const k = detectMarker("//version 1.3 2.0 *", "//");
        if (k.kind !== "Versioned") {
            expect.fail("expected Versioned");
            return;
        }
        const text = "//version 1.3 2.0 *";
        expect(k.marker.toSpan).not.toBeNull();
        if (!k.marker.toSpan) return;
        expect(text.slice(k.marker.toSpan.start, k.marker.toSpan.end)).toBe("2.0");
    });

    it("captures tag spans correctly", () => {
        const text = "//version 1.2 [inventory,combat] *";
        const k = detectMarker(text, "//");
        if (k.kind !== "Versioned") {
            expect.fail("expected Versioned");
            return;
        }
        expect(k.marker.tagSpans).toHaveLength(2);
        const [inv, comb] = k.marker.tagSpans;
        expect(text.slice(inv.start, inv.end)).toBe("inventory");
        expect(text.slice(comb.start, comb.end)).toBe("combat");
    });
});

describe("parseVersionTuple — semver padding parity", () => {
    it("pads single component", () => {
        const v = parseVersionTuple("1");
        expect(v).toEqual({ major: 1, minor: 0, patch: 0, pre: "", build: "" });
    });

    it("pads two components", () => {
        const v = parseVersionTuple("1.2");
        expect(v).toEqual({ major: 1, minor: 2, patch: 0, pre: "", build: "" });
    });

    it("accepts three components", () => {
        const v = parseVersionTuple("1.2.3");
        expect(v?.patch).toBe(3);
    });

    it("rejects leading zeros", () => {
        expect(isValidVersion("01.2")).toBe(false);
        expect(isValidVersion("1.02")).toBe(false);
    });

    it("compares 1.2 < 1.10", () => {
        const a = parseVersionTuple("1.2")!;
        const b = parseVersionTuple("1.10")!;
        expect(compareVersionTuple(a, b)).toBeLessThan(0);
    });

    it("rejects empty and garbage", () => {
        expect(isValidVersion("")).toBe(false);
        expect(isValidVersion("notaversion")).toBe(false);
    });
});

describe("pairLines — stack-pairing parity with parser.rs", () => {
    function lines(s: string): string[] {
        // Use \n splitting; trailing empty entry from a final \n is fine.
        return s.split("\n");
    }

    it("pairs a flat versioned block", () => {
        const src = lines("before\n//version 1.2 *\ninside\n//version 1.2 *\nafter");
        const r = pairLines(src, "//");
        expect(r.pairs).toHaveLength(1);
        expect(r.pairs[0].openLine).toBe(1);
        expect(r.pairs[0].closeLine).toBe(3);
        expect(r.unclosed).toHaveLength(0);
    });

    it("pairs nested versioned blocks", () => {
        const src = lines(
            [
                "a",
                "//version 1.5 *",
                "b",
                "//version 1.0 *",
                "c",
                "//version 1.0 *",
                "d",
                "//version 1.5 *",
                "e",
            ].join("\n"),
        );
        const r = pairLines(src, "//");
        expect(r.pairs).toHaveLength(2);
        const inner = r.pairs.find((p) => p.openMarker.version === "1.0");
        const outer = r.pairs.find((p) => p.openMarker.version === "1.5");
        expect(inner).toEqual(
            expect.objectContaining({ openLine: 3, closeLine: 5 }),
        );
        expect(outer).toEqual(
            expect.objectContaining({ openLine: 1, closeLine: 7 }),
        );
    });

    it("pairs range blocks by (version, to)", () => {
        const src = lines(
            [
                "//version 1.3 2.0 *",
                "in",
                "//version 1.3 2.0 *",
            ].join("\n"),
        );
        const r = pairLines(src, "//");
        expect(r.pairs).toHaveLength(1);
        expect(r.pairs[0].openMarker.to).toBe("2.0");
    });

    it("does NOT pair range with a plain versioned of same `from`", () => {
        const src = lines(
            [
                "//version 1.3 2.0 *",
                "in",
                "//version 1.3 *",
            ].join("\n"),
        );
        const r = pairLines(src, "//");
        expect(r.pairs).toHaveLength(0);
        expect(r.unclosed).toHaveLength(2);
    });

    it("pairs ALL blocks", () => {
        const src = lines("x\n//version ALL\nkept\n//version ALL\ny");
        const r = pairLines(src, "//");
        expect(r.pairs).toHaveLength(1);
        expect(r.pairs[0].kind).toBe("All");
        expect(r.pairs[0].openLine).toBe(1);
        expect(r.pairs[0].closeLine).toBe(3);
    });

    it("reports unclosed openers", () => {
        const src = lines("//version 1.2 *\ninside");
        const r = pairLines(src, "//");
        expect(r.pairs).toHaveLength(0);
        expect(r.unclosed).toHaveLength(1);
        expect(r.unclosed[0].marker.version).toBe("1.2");
    });

    it("reports unclosed range markers", () => {
        const src = lines("//version 1.3 2.0 *\ninside");
        const r = pairLines(src, "//");
        expect(r.unclosed).toHaveLength(1);
        expect(r.unclosed[0].marker.to).toBe("2.0");
    });

    it("byLine has partner info for paired lines", () => {
        const src = lines("//version 1.2 *\ninside\n//version 1.2 *");
        const r = pairLines(src, "//");
        expect(r.byLine.get(0)?.partnerLine).toBe(2);
        expect(r.byLine.get(2)?.partnerLine).toBe(0);
        expect(r.byLine.get(1)?.partnerLine).toBeNull();
    });

    it("inline range markers are not paired", () => {
        const src = lines("a\n//version 1.3 2.0\nb");
        const r = pairLines(src, "//");
        expect(r.pairs).toHaveLength(0);
        expect(r.unclosed).toHaveLength(0);
        expect(r.byLine.get(1)?.kind.kind).toBe("InlineRange");
    });
});
