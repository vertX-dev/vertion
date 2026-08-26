// Shape of the `vertion map --json` interface. Pure, so it stays unit-testable
// without a vscode host — the process spawning lives in `mapLine.ts`.

/** One translated reference, as `vertion map --json` reports it. */
export interface MapHit {
    /** `to-source` when the input was build output, `to-output` when it was source. */
    direction: "to-source" | "to-output";
    from: string;
    from_line: number;
    to: string;
    to_line: number;
    /** Present when the answer needs a caveat (line stripped, source drifted). */
    note?: string;
}

/**
 * Argv for translating one reference. The CLI picks the direction from the
 * path, so there's nothing to tell it beyond where we are.
 */
export function mapArgs(file: string, line: number): string[] {
    return ["map", `${file}:${line}`, "--json"];
}

/**
 * Parse the CLI's JSON. Returns null when it mapped nothing — which is a normal
 * outcome (the file isn't part of any build), not an error.
 */
export function parseMapOutput(stdout: string): MapHit | null {
    const trimmed = stdout.trim();
    if (!trimmed) return null;

    let parsed: unknown;
    try {
        parsed = JSON.parse(trimmed);
    } catch {
        throw new Error(`vertion map returned unparseable output: ${trimmed}`);
    }
    if (!Array.isArray(parsed) || parsed.length === 0) return null;

    const hit = parsed[0] as Partial<MapHit>;
    if (
        typeof hit.to !== "string" ||
        typeof hit.to_line !== "number" ||
        (hit.direction !== "to-source" && hit.direction !== "to-output")
    ) {
        throw new Error(`vertion map returned an unexpected shape: ${trimmed}`);
    }
    return hit as MapHit;
}

/**
 * What to call the jump in user-facing text, given which way it went.
 */
export function directionLabel(direction: MapHit["direction"]): string {
    return direction === "to-source" ? "source" : "build output";
}
