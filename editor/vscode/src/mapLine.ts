// Jump between a line in a build output file and the source line that produced
// it. Stripping a version block shifts everything below it, so the two line
// numbers diverge and an error reported against the build tree points at the
// wrong place in the source.
//
// The mapping is delegated to `vertion map` rather than reimplemented here: it
// depends on the full filter semantics (versions, ranges, tags, conditions,
// variant directories), and a second implementation would drift.

import * as path from "path";
import { execFile } from "child_process";
import { promisify } from "util";

import * as vscode from "vscode";

import { directionLabel, mapArgs, parseMapOutput, MapHit } from "./mapCli";

const execFileAsync = promisify(execFile);
const CONFIG_SECTION = "vertion";
const EXE_SETTING = "executablePath";

function executable(scope?: vscode.Uri): string {
    return vscode.workspace
        .getConfiguration(CONFIG_SECTION, scope)
        .get<string>(EXE_SETTING, "vertion");
}

/**
 * Translate one `file:line` through the CLI. `cwd` anchors the relative paths
 * the CLI prints, so the caller can resolve the result against it.
 */
async function runMap(
    exe: string,
    cwd: string,
    file: string,
    line: number,
): Promise<MapHit | null> {
    try {
        const { stdout } = await execFileAsync(exe, mapArgs(file, line), { cwd });
        return parseMapOutput(stdout);
    } catch (err) {
        const e = err as NodeJS.ErrnoException & { stderr?: string };
        if (e.code === "ENOENT") {
            throw new Error(
                `Vertion: \`${exe}\` not found on PATH — install it, or set ` +
                    `\`${CONFIG_SECTION}.${EXE_SETTING}\` to its full path.`,
            );
        }
        // A non-zero exit carries the CLI's own diagnostic, which is more
        // specific than anything we could say here.
        const detail = (e.stderr ?? e.message ?? "").trim();
        throw new Error(detail ? `Vertion: ${detail}` : `Vertion: ${e}`);
    }
}

/**
 * Reveal the counterpart of the cursor's line — the source line if we're in
 * build output, the built line if we're in source.
 */
export async function revealCounterpart(): Promise<void> {
    const editor = vscode.window.activeTextEditor;
    if (!editor || editor.document.uri.scheme !== "file") {
        vscode.window.showInformationMessage(
            "Vertion: open a file in a project or build tree first.",
        );
        return;
    }

    const file = editor.document.uri.fsPath;
    const line = editor.selection.active.line + 1;
    const folder = vscode.workspace.getWorkspaceFolder(editor.document.uri);
    const cwd = folder ? folder.uri.fsPath : path.dirname(file);

    let hit: MapHit | null;
    try {
        hit = await runMap(executable(editor.document.uri), cwd, file, line);
    } catch (err) {
        vscode.window.showErrorMessage((err as Error).message);
        return;
    }
    if (!hit) {
        vscode.window.showInformationMessage(
            "Vertion: this file isn't part of a build — nothing to map to.",
        );
        return;
    }

    const target = vscode.Uri.file(path.resolve(cwd, hit.to));
    const doc = await vscode.workspace.openTextDocument(target);
    const shown = await vscode.window.showTextDocument(doc, {
        preview: false,
    });
    // Clamp: the CLI works from the file on disk, which an unsaved buffer may
    // already have outgrown.
    const row = Math.min(hit.to_line - 1, Math.max(doc.lineCount - 1, 0));
    const pos = new vscode.Position(row, 0);
    shown.selection = new vscode.Selection(pos, pos);
    shown.revealRange(
        new vscode.Range(pos, pos),
        vscode.TextEditorRevealType.InCenter,
    );

    if (hit.note) {
        vscode.window.showWarningMessage(`Vertion: ${hit.note}`);
    } else {
        vscode.window.setStatusBarMessage(
            `Vertion: line ${line} → ${directionLabel(hit.direction)} line ${hit.to_line}`,
            4000,
        );
    }
}

export function registerMapCommands(ctx: vscode.ExtensionContext): void {
    ctx.subscriptions.push(
        vscode.commands.registerCommand("vertion.revealCounterpart", () =>
            revealCounterpart(),
        ),
    );
}
