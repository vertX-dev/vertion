import { describe, expect, it } from "vitest";
import { foldRangesFromPairs, pairLines } from "../src/pairing";

describe("foldRangesFromPairs", () => {
    it("folds through the close marker (end = closeLine)", () => {
        const ranges = foldRangesFromPairs([{ openLine: 0, closeLine: 4 }]);
        expect(ranges).toEqual([{ start: 0, end: 4 }]);
    });

    it("skips empty blocks (open immediately followed by close)", () => {
        expect(foldRangesFromPairs([{ openLine: 2, closeLine: 3 }])).toEqual([]);
    });

    it("skips single-marker-gap blocks with nothing to collapse", () => {
        // closeLine === openLine + 1 → no body line → nothing to fold.
        expect(foldRangesFromPairs([{ openLine: 0, closeLine: 1 }])).toEqual([]);
    });

    it("gives each consecutive same-version block its own disjoint range", () => {
        // The reported bug: several same-version blocks in a row. Each must fold
        // through its own close marker so folded blocks don't leave dangling
        // close markers that look like un-collapsed content.
        const lines = [
            "//version 1.0 *",
            "a",
            "//version 1.0 *",
            "//version 1.0 *",
            "b",
            "//version 1.0 *",
            "//version 1.0 *",
            "c",
            "//version 1.0 *",
        ];
        const pairing = pairLines(lines, "//");
        const ranges = foldRangesFromPairs(pairing.pairs);
        expect(ranges).toEqual([
            { start: 0, end: 2 },
            { start: 3, end: 5 },
            { start: 6, end: 8 },
        ]);
        // Disjoint: no range's end reaches the next range's start.
        for (let i = 1; i < ranges.length; i++) {
            expect(ranges[i].start).toBeGreaterThan(ranges[i - 1].end);
        }
    });
});
