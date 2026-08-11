import { describe, expect, it } from "vitest";
import { buildReadonlyGlob } from "../src/paths";

describe("buildReadonlyGlob", () => {
    it("relativizes the build dir against the workspace root", () => {
        expect(buildReadonlyGlob("/w/proj", "/w/proj/build/1.2.0")).toBe(
            "build/1.2.0/**",
        );
    });

    it("handles a nested output root", () => {
        expect(buildReadonlyGlob("/w/proj", "/w/proj/out/dist/2.0.0")).toBe(
            "out/dist/2.0.0/**",
        );
    });

    it("tolerates trailing slashes", () => {
        expect(buildReadonlyGlob("/w/proj/", "/w/proj/build/1.0.0/")).toBe(
            "build/1.0.0/**",
        );
    });

    it("covers everything when the build dir is the workspace root", () => {
        expect(buildReadonlyGlob("/w/proj", "/w/proj")).toBe("**");
    });

    it("falls back to the absolute path when outside the workspace", () => {
        // An output folder configured outside the workspace can't be relativized;
        // returning the path unchanged still produces a usable glob.
        expect(buildReadonlyGlob("/w/proj", "/elsewhere/build/1.0.0")).toBe(
            "/elsewhere/build/1.0.0/**",
        );
    });
});
