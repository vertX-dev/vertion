import { describe, expect, it } from "vitest";

import { directionLabel, mapArgs, parseMapOutput } from "../src/mapCli";

describe("mapArgs", () => {
    it("builds a FILE:LINE reference the CLI understands", () => {
        expect(mapArgs("src/game.rs", 57)).toEqual([
            "map",
            "src/game.rs:57",
            "--json",
        ]);
    });

    it("leaves a Windows path intact — the CLI splits from the right", () => {
        expect(mapArgs("C:\\proj\\src\\game.rs", 4)).toEqual([
            "map",
            "C:\\proj\\src\\game.rs:4",
            "--json",
        ]);
    });
});

describe("parseMapOutput", () => {
    const hit = {
        direction: "to-source",
        from: "build/1.0.0/game.rs",
        from_line: 4,
        to: "src/game.rs",
        to_line: 11,
    };

    it("returns the first hit", () => {
        expect(parseMapOutput(JSON.stringify([hit]))).toEqual(hit);
    });

    it("carries a note through", () => {
        const withNote = { ...hit, note: "line 8 was stripped from this build" };
        expect(parseMapOutput(JSON.stringify([withNote]))?.note).toBe(
            "line 8 was stripped from this build",
        );
    });

    it("treats an empty result as nothing to map, not an error", () => {
        expect(parseMapOutput("[]")).toBeNull();
        expect(parseMapOutput("   ")).toBeNull();
    });

    it("throws on output that isn't JSON", () => {
        expect(() => parseMapOutput("error: no build found")).toThrow(
            /unparseable/,
        );
    });

    it("throws when a hit is missing the fields we navigate by", () => {
        expect(() => parseMapOutput(JSON.stringify([{ to: "src/a.rs" }]))).toThrow(
            /unexpected shape/,
        );
        expect(() =>
            parseMapOutput(
                JSON.stringify([{ ...hit, direction: "sideways" }]),
            ),
        ).toThrow(/unexpected shape/);
    });
});

describe("directionLabel", () => {
    it("names the side that was jumped to", () => {
        expect(directionLabel("to-source")).toBe("source");
        expect(directionLabel("to-output")).toBe("build output");
    });
});
