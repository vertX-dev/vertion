// Warns when you edit a file that lives inside a Vertion build output folder,
// where the next build will silently overwrite your changes.
//
// Detection uses the build manifest rather than the configured output path: the
// builder writes `vertion.manifest.json` into every output folder it produces,
// so an ancestor carrying that file *is* build output — no matter whether the
// path came from `[project].output`, a profile, `--output`, or a `--dev`
// timestamped directory.

import * as vscode from "vscode";

import { buildReadonlyGlob } from "./paths";

const MANIFEST_NAME = "vertion.manifest.json";
const CONFIG_SECTION = "vertion";
const WARN_SETTING = "warnOnBuildOutputEdit";

/** How far up to walk before giving up, if there's no workspace boundary. */
const MAX_DEPTH = 64;

/** Cache of directory path → the build root containing it (or null). */
const cache = new Map<string, vscode.Uri | null>();

async function fileExists(uri: vscode.Uri): Promise<boolean> {
    try {
        await vscode.workspace.fs.stat(uri);
        return true;
    } catch {
        return false;
    }
}

/**
 * The build output folder containing `uri`, or null. Walks up from the file
 * looking for a manifest, stopping at the workspace folder boundary.
 */
export async function findBuildRoot(
    uri: vscode.Uri,
): Promise<vscode.Uri | null> {
    if (uri.scheme !== "file") return null;
    const folder = vscode.workspace.getWorkspaceFolder(uri);
    const stopAt = folder?.uri.path;

    let dir = vscode.Uri.joinPath(uri, "..");
    for (let i = 0; i < MAX_DEPTH; i++) {
        const key = dir.path;
        const cached = cache.get(key);
        if (cached !== undefined) return cached;

        const found = (await fileExists(vscode.Uri.joinPath(dir, MANIFEST_NAME)))
            ? dir
            : null;
        cache.set(key, found);
        if (found) return found;

        if (stopAt && key === stopAt) return null;
        const parent = vscode.Uri.joinPath(dir, "..");
        if (parent.path === key) return null;
        dir = parent;
    }
    return null;
}

// ---- UI ------------------------------------------------------------------

/** Documents already warned about, so we nag at most once per file per session. */
const warned = new Set<string>();

let statusItem: vscode.StatusBarItem | undefined;

function warningEnabled(): boolean {
    return vscode.workspace
        .getConfiguration(CONFIG_SECTION)
        .get<boolean>(WARN_SETTING, true);
}

async function updateStatusBar(editor: vscode.TextEditor | undefined): Promise<void> {
    if (!statusItem) return;
    if (!editor) {
        statusItem.hide();
        return;
    }
    const root = await findBuildRoot(editor.document.uri);
    if (!root) {
        statusItem.hide();
        return;
    }
    statusItem.text = "$(warning) Vertion build output";
    statusItem.tooltip =
        `This file is inside ${root.path.split("/").pop()}, a Vertion build output folder.\n` +
        "Edits here are overwritten by the next build — change the source instead.";
    statusItem.backgroundColor = new vscode.ThemeColor(
        "statusBarItem.warningBackground",
    );
    statusItem.show();
}

async function warnOnEdit(doc: vscode.TextDocument): Promise<void> {
    if (!warningEnabled()) return;
    const key = doc.uri.toString();
    if (warned.has(key)) return;

    const root = await findBuildRoot(doc.uri);
    if (!root) return;
    // Mark before awaiting the dialog, so a burst of keystrokes shows one popup.
    warned.add(key);

    const choice = await vscode.window.showWarningMessage(
        `Vertion: you're editing build output — the next build will overwrite ${doc.uri.path.split("/").pop()}. Edit the source file instead.`,
        "Make Folder Read-Only",
        "Don't Warn Again",
    );
    if (choice === "Don't Warn Again") {
        await vscode.workspace
            .getConfiguration(CONFIG_SECTION)
            .update(WARN_SETTING, false, vscode.ConfigurationTarget.Global);
        return;
    }
    if (choice === "Make Folder Read-Only") {
        const folder = vscode.workspace.getWorkspaceFolder(doc.uri);
        if (!folder) return;
        const glob = buildReadonlyGlob(folder.uri.path, root.path);
        const files = vscode.workspace.getConfiguration("files", folder.uri);
        const current = files.get<Record<string, boolean>>("readonlyInclude", {});
        await files.update(
            "readonlyInclude",
            { ...current, [glob]: true },
            vscode.ConfigurationTarget.Workspace,
        );
        vscode.window.showInformationMessage(
            `Vertion: \`${glob}\` is now read-only in this workspace.`,
        );
    }
}

export function registerBuildGuard(ctx: vscode.ExtensionContext): void {
    statusItem = vscode.window.createStatusBarItem(
        vscode.StatusBarAlignment.Right,
        100,
    );
    ctx.subscriptions.push(statusItem);

    ctx.subscriptions.push(
        vscode.workspace.onDidChangeTextDocument((e) => {
            if (e.contentChanges.length > 0) void warnOnEdit(e.document);
        }),
        vscode.window.onDidChangeActiveTextEditor((editor) => {
            void updateStatusBar(editor);
        }),
        // A build replaces the manifest; drop the cache so a folder that just
        // became (or stopped being) output is re-detected.
        vscode.workspace.onDidCreateFiles(() => cache.clear()),
        vscode.workspace.onDidDeleteFiles(() => cache.clear()),
    );

    void updateStatusBar(vscode.window.activeTextEditor);
}
