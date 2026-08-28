// Pure path helpers. No VSCode dependency, so they stay unit-testable.

/**
 * Glob covering everything under `buildDirPath`, relative to the workspace
 * root — the form `files.readonlyInclude` expects.
 *
 * A build directory outside the workspace can't be relativized, so its absolute
 * path is used unchanged; that still yields a usable glob.
 */
export function buildReadonlyGlob(rootPath: string, buildDirPath: string): string {
    const root = rootPath.replace(/\/+$/, "");
    const dir = buildDirPath.replace(/\/+$/, "");
    if (dir === root) return "**";
    const rel = dir.startsWith(root + "/") ? dir.slice(root.length + 1) : dir;
    return `${rel}/**`;
}
