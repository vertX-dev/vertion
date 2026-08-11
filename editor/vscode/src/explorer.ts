// Explorer context-menu commands for `.vertion.<target>/` variant directories.
//
// All filesystem work goes through `vscode.workspace.fs` so it participates in
// VSCode's file watching, undo of file operations, and virtual filesystems.

import * as vscode from "vscode";
import {
    DEFAULT_STEM,
    describeVariant,
    isVariantError,
    parseVariantStem,
    targetExtension,
    targetNameFromVariantDir,
    variantDirName,
    VARIANT_PREFIX,
} from "./variantName";

function baseName(uri: vscode.Uri): string {
    const parts = uri.path.split("/");
    return parts[parts.length - 1] ?? "";
}

function parentUri(uri: vscode.Uri): vscode.Uri {
    return vscode.Uri.joinPath(uri, "..");
}

async function isDirectory(uri: vscode.Uri): Promise<boolean> {
    try {
        const stat = await vscode.workspace.fs.stat(uri);
        return stat.type === vscode.FileType.Directory;
    } catch {
        return false;
    }
}

async function exists(uri: vscode.Uri): Promise<boolean> {
    try {
        await vscode.workspace.fs.stat(uri);
        return true;
    } catch {
        return false;
    }
}

/** The nearest enclosing `.vertion.*` directory, or null. */
function findVariantDir(uri: vscode.Uri): vscode.Uri | null {
    let current = uri;
    for (let i = 0; i < 64; i++) {
        if (baseName(current).startsWith(VARIANT_PREFIX)) return current;
        const parent = parentUri(current);
        if (parent.path === current.path) return null;
        current = parent;
    }
    return null;
}

/** Prompt for a variant stem, validating it against the grammar as they type. */
async function promptForStem(
    prompt: string,
    value: string,
    taken: string[],
): Promise<string | undefined> {
    return vscode.window.showInputBox({
        prompt,
        value,
        valueSelection: [0, value.length],
        validateInput: (raw) => {
            const stem = raw.trim();
            if (stem.length === 0) return "Required";
            const parsed = parseVariantStem(stem);
            if (isVariantError(parsed)) return parsed.error;
            if (taken.includes(stem)) return `\`${stem}\` already exists here`;
            return null;
        },
    });
}

/** Existing variant stems in a directory, for collision checks and pickers. */
async function readVariantStems(
    dir: vscode.Uri,
    ext: string | null,
): Promise<{ stem: string; name: string; isDir: boolean }[]> {
    const out: { stem: string; name: string; isDir: boolean }[] = [];
    for (const [name, type] of await vscode.workspace.fs.readDirectory(dir)) {
        const isDir = type === vscode.FileType.Directory;
        let stem = name;
        if (!isDir && ext) {
            const suffix = `.${ext}`;
            if (!name.toLowerCase().endsWith(suffix.toLowerCase())) continue;
            stem = name.slice(0, name.length - suffix.length);
        }
        out.push({ stem, name, isDir });
    }
    return out;
}

function variantFileName(stem: string, ext: string | null): string {
    return ext ? `${stem}.${ext}` : stem;
}

// ---- Commands ------------------------------------------------------------

/** `logo.png` → `.vertion.logo.png/0.0.0.png`, keeping build output identical. */
async function convertToVariants(uri?: vscode.Uri): Promise<void> {
    if (!uri) return;
    const name = baseName(uri);
    if (findVariantDir(uri)) {
        vscode.window.showWarningMessage(
            "Vertion: this is already inside a variant directory.",
        );
        return;
    }

    const dirUri = vscode.Uri.joinPath(parentUri(uri), variantDirName(name));
    if (await exists(dirUri)) {
        vscode.window.showErrorMessage(
            `Vertion: ${variantDirName(name)} already exists.`,
        );
        return;
    }

    // `0.0.0` matches every build, so converting alone can't change the output.
    const stem = await promptForStem(
        `Version spec for the existing ${name} (0.0.0 matches every build)`,
        "0.0.0",
        [],
    );
    if (!stem) return;

    const isDir = await isDirectory(uri);
    const ext = isDir ? null : targetExtension(variantDirName(name));
    const target = vscode.Uri.joinPath(dirUri, variantFileName(stem.trim(), ext));

    await vscode.workspace.fs.createDirectory(dirUri);
    await vscode.workspace.fs.rename(uri, target);
    vscode.window.showInformationMessage(
        `Vertion: ${name} → ${variantDirName(name)}/${baseName(target)}`,
    );
}

/** Create a new, empty variant in an existing variant directory. */
async function addVariant(uri?: vscode.Uri): Promise<void> {
    if (!uri) return;
    const dir = findVariantDir(uri);
    if (!dir) {
        vscode.window.showWarningMessage(
            "Vertion: not inside a `.vertion.*` variant directory.",
        );
        return;
    }
    const dirName = baseName(dir);
    // The directory name decides the shape: an extension means file variants.
    const ext = targetExtension(dirName);
    const existing = await readVariantStems(dir, ext);

    const stem = await promptForStem(
        `New variant for ${targetNameFromVariantDir(dirName)} (e.g. 2.0.0, 1.2.3e2.0.0, 2.0.0-beta)`,
        "",
        existing.map((e) => e.stem),
    );
    if (!stem) return;

    const target = vscode.Uri.joinPath(dir, variantFileName(stem.trim(), ext));
    if (ext === null) {
        await vscode.workspace.fs.createDirectory(target);
    } else {
        await vscode.workspace.fs.writeFile(target, new Uint8Array());
        await vscode.window.showTextDocument(target).then(undefined, () => undefined);
    }
    vscode.window.showInformationMessage(`Vertion: created ${baseName(target)}`);
}

/** Copy an existing variant to a new spec, ready to diverge. */
async function duplicateVariant(uri?: vscode.Uri): Promise<void> {
    if (!uri) return;
    const dir = findVariantDir(uri);
    if (!dir || dir.path === uri.path) {
        vscode.window.showWarningMessage(
            "Vertion: right-click a variant *inside* a `.vertion.*` directory to duplicate it.",
        );
        return;
    }
    const dirName = baseName(dir);
    const ext = targetExtension(dirName);
    const existing = await readVariantStems(dir, ext);
    const sourceName = baseName(uri);
    const sourceStem =
        ext && sourceName.toLowerCase().endsWith(`.${ext.toLowerCase()}`)
            ? sourceName.slice(0, sourceName.length - ext.length - 1)
            : sourceName;

    const stem = await promptForStem(
        `New version spec for the copy of ${sourceName}`,
        sourceStem,
        existing.map((e) => e.stem),
    );
    if (!stem) return;

    const target = vscode.Uri.joinPath(dir, variantFileName(stem.trim(), ext));
    await vscode.workspace.fs.copy(uri, target, { overwrite: false });
    vscode.window.showInformationMessage(
        `Vertion: ${sourceName} → ${baseName(target)}`,
    );
}

/** Collapse a variant directory back to a single plain file/folder. */
async function flattenVariants(uri?: vscode.Uri): Promise<void> {
    if (!uri) return;
    const dirName = baseName(uri);
    const targetName = targetNameFromVariantDir(dirName);
    if (!targetName || !(await isDirectory(uri))) {
        vscode.window.showWarningMessage(
            "Vertion: right-click a `.vertion.*` directory to flatten it.",
        );
        return;
    }
    const ext = targetExtension(dirName);
    const entries = await readVariantStems(uri, ext);
    if (entries.length === 0) {
        vscode.window.showWarningMessage("Vertion: this variant directory is empty.");
        return;
    }

    const picked = await vscode.window.showQuickPick(
        entries.map((e) => {
            const parsed = parseVariantStem(e.stem);
            return {
                label: e.name,
                description: isVariantError(parsed)
                    ? parsed.error
                    : describeVariant(parsed),
                entry: e,
            };
        }),
        { title: `Keep which variant as ${targetName}?`, placeHolder: "The rest are deleted" },
    );
    if (!picked) return;

    const confirm = await vscode.window.showWarningMessage(
        `Keep ${picked.label} as ${targetName} and delete ${dirName} with its other ${entries.length - 1} variant(s)?`,
        { modal: true },
        "Flatten",
    );
    if (confirm !== "Flatten") return;

    // Copy out first, then remove the directory — the two names never collide,
    // so a failure mid-way leaves the originals intact.
    const dest = vscode.Uri.joinPath(parentUri(uri), targetName);
    if (await exists(dest)) {
        vscode.window.showErrorMessage(`Vertion: ${targetName} already exists.`);
        return;
    }
    await vscode.workspace.fs.copy(
        vscode.Uri.joinPath(uri, picked.entry.name),
        dest,
        { overwrite: false },
    );
    await vscode.workspace.fs.delete(uri, { recursive: true, useTrash: true });
    vscode.window.showInformationMessage(`Vertion: flattened to ${targetName}`);
}

export function registerExplorerCommands(ctx: vscode.ExtensionContext): void {
    ctx.subscriptions.push(
        vscode.commands.registerCommand("vertion.convertToVariants", convertToVariants),
        vscode.commands.registerCommand("vertion.addVariant", addVariant),
        vscode.commands.registerCommand("vertion.duplicateVariant", duplicateVariant),
        vscode.commands.registerCommand("vertion.flattenVariants", flattenVariants),
    );
    // Referenced so the reserved stem stays in one place if it ever changes.
    void DEFAULT_STEM;
}
