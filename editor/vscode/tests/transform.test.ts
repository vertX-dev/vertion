import { describe, expect, it } from "vitest";
import {
    computeDuplicate,
    computeSplit,
    isError,
    markerWithVersion,
} from "../src/transform";
import { detectMarker } from "../src/marker";

function lines(s: string): string[] {
    return s.split("\n");
}

describe("markerWithVersion", () => {
    function rewrite(text: string, spec: string): string {
        const k = detectMarker(text, "//");
        if (k.kind === "Malformed" || k.kind === "None") {
            throw new Error(`not a marker: ${text}`);
        }
        return markerWithVersion(text, k.marker, spec);
    }

    it("replaces a plain version", () => {
        expect(rewrite("//version 1.2 *", "2.0")).toBe("//version 2.0 *");
    });

    it("preserves tags, conditions and star spacing", () => {
        expect(rewrite("//version 1.2 [combat{hasAssets}] *", "2.0")).toBe(
            "//version 2.0 [combat{hasAssets}] *",
        );
        expect(rewrite("//version 1.2*", "2.0")).toBe("//version 2.0*");
    });

    it("collapses a range when given a single version", () => {
        expect(rewrite("//version 1.3 2.0 *", "2.5")).toBe("//version 2.5 *");
        expect(rewrite("//version 1.3 2.0*", "2.5")).toBe("//version 2.5*");
    });

    it("keeps a range when given two versions", () => {
        expect(rewrite("//version 1.2 *", "1.3 2.0")).toBe("//version 1.3 2.0 *");
    });

    it("inserts a version into a tag-only marker", () => {
        expect(rewrite("//version [wiki]", "2.0")).toBe("//version 2.0 [wiki]");
    });
});

describe("computeDuplicate", () => {
    it("inserts a copy under the new version", () => {
        const src = lines(
            ["//version 1.2 *", "function render() {", "  draw();", "}", "//version 1.2 *"].join(
                "\n",
            ),
        );
        const r = computeDuplicate(src, "//", 2, "2.0");
        if (isError(r)) {
            expect.fail(r.error);
            return;
        }
        expect(r.startLine).toBe(0);
        expect(r.endLine).toBe(4);
        expect(r.lines).toEqual([
            "//version 1.2 *",
            "function render() {",
            "  draw();",
            "}",
            "//version 1.2 *",
            "",
            "//version 2.0 *",
            "function render() {",
            "  draw();",
            "}",
            "//version 2.0 *",
        ]);
    });

    it("duplicates the innermost block when nested", () => {
        const src = lines(
            [
                "//version 1.0 *",
                "outer();",
                "//version 1.5 *",
                "inner();",
                "//version 1.5 *",
                "//version 1.0 *",
            ].join("\n"),
        );
        const r = computeDuplicate(src, "//", 3, "1.6");
        if (isError(r)) {
            expect.fail(r.error);
            return;
        }
        expect(r.startLine).toBe(2);
        expect(r.endLine).toBe(4);
        expect(r.lines).toEqual([
            "//version 1.5 *",
            "inner();",
            "//version 1.5 *",
            "",
            "//version 1.6 *",
            "inner();",
            "//version 1.6 *",
        ]);
    });

    it("errors outside any block", () => {
        const r = computeDuplicate(lines("plain();\nmore();"), "//", 0, "2.0");
        expect(isError(r)).toBe(true);
    });
});

describe("computeSplit", () => {
    // The reference case: one outer block whose body mixes base content with
    // two nested blocks, split into three standalone per-version blocks.
    const SRC = [
        "//version 2.0.0 *",
        "const someArray = [",
        "'v1',",
        "'v2',",
        "",
        "//version 2.1.0 2.2.0*",
        "'v3',",
        "//version 2.1.0 2.2.0*",
        "",
        "//version 2.3.0 *",
        "'v4',",
        "//version 2.3.0*",
        "];",
        "//version 2.0.0 *",
    ];

    it("splits into one block per version variant", () => {
        const r = computeSplit(SRC, "//", 3);
        if (isError(r)) {
            expect.fail(r.error);
            return;
        }
        expect(r.variantCount).toBe(3);
        expect(r.startLine).toBe(0);
        expect(r.endLine).toBe(13);
        expect(r.lines).toEqual([
            "//version 2.0.0 *",
            "const someArray = [",
            "'v1',",
            "'v2',",
            "];",
            "//version 2.0.0 *",
            "",
            "//version 2.1.0 2.2.0*",
            "const someArray = [",
            "'v1',",
            "'v2',",
            "'v3',",
            "];",
            "//version 2.1.0 2.2.0*",
            "",
            "//version 2.3.0 *",
            "const someArray = [",
            "'v1',",
            "'v2',",
            "'v4',",
            "];",
            "//version 2.3.0 *",
        ]);
    });

    it("works with the cursor inside a nested child", () => {
        // Line 6 is `'v3',` — inside the 2.1.0 child, which has no children of
        // its own, so the command walks outward to the 2.0.0 block.
        const r = computeSplit(SRC, "//", 6);
        if (isError(r)) {
            expect.fail(r.error);
            return;
        }
        expect(r.variantCount).toBe(3);
        expect(r.startLine).toBe(0);
    });

    it("normalizes a close marker written with different spacing", () => {
        const r = computeSplit(SRC, "//", 3);
        if (isError(r)) {
            expect.fail(r.error);
            return;
        }
        // Source closed 2.3.0 with `//version 2.3.0*`; output uses the open form.
        const emitted = r.lines.filter((l) => l.includes("2.3.0"));
        expect(emitted).toEqual(["//version 2.3.0 *", "//version 2.3.0 *"]);
    });

    it("preserves indentation of body lines", () => {
        const src = [
            "  //version 1.0 *",
            "  base();",
            "  //version 1.5 *",
            "    nested();",
            "  //version 1.5 *",
            "  //version 1.0 *",
        ];
        const r = computeSplit(src, "//", 1);
        if (isError(r)) {
            expect.fail(r.error);
            return;
        }
        expect(r.lines).toEqual([
            "  //version 1.0 *",
            "  base();",
            "  //version 1.0 *",
            "",
            "  //version 1.5 *",
            "  base();",
            "    nested();",
            "  //version 1.5 *",
        ]);
    });

    it("keeps grandchildren nested inside the variant they belong to", () => {
        const src = [
            "//version 1.0 *",
            "base();",
            "//version 1.5 *",
            "mid();",
            "//version 1.7 *",
            "deep();",
            "//version 1.7 *",
            "//version 1.5 *",
            "//version 1.0 *",
        ];
        const r = computeSplit(src, "//", 1);
        if (isError(r)) {
            expect.fail(r.error);
            return;
        }
        // 1.0 and 1.5 are the variants; 1.7 rides along inside the 1.5 one.
        expect(r.variantCount).toBe(2);
        expect(r.lines).toEqual([
            "//version 1.0 *",
            "base();",
            "//version 1.0 *",
            "",
            "//version 1.5 *",
            "base();",
            "mid();",
            "//version 1.7 *",
            "deep();",
            "//version 1.7 *",
            "//version 1.5 *",
        ]);
    });

    it("errors when the block has no nested blocks", () => {
        const src = ["//version 1.0 *", "just();", "code();", "//version 1.0 *"];
        const r = computeSplit(src, "//", 1);
        expect(isError(r)).toBe(true);
    });

    it("errors outside any block", () => {
        const r = computeSplit(lines("plain();"), "//", 0);
        expect(isError(r)).toBe(true);
    });

    it("handles hash-comment languages", () => {
        const src = [
            "#version 1.0 *",
            "base = 1",
            "#version 1.5 *",
            "extra = 2",
            "#version 1.5 *",
            "#version 1.0 *",
        ];
        const r = computeSplit(src, "#", 1);
        if (isError(r)) {
            expect.fail(r.error);
            return;
        }
        expect(r.variantCount).toBe(2);
        expect(r.lines).toEqual([
            "#version 1.0 *",
            "base = 1",
            "#version 1.0 *",
            "",
            "#version 1.5 *",
            "base = 1",
            "extra = 2",
            "#version 1.5 *",
        ]);
    });
});
