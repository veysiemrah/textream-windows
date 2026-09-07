/**
 * `.textream` file dialogs.
 *
 * The native picker lives here, in the webview — it is the one piece of this
 * feature that is a UI concern rather than a file-format one. Everything about
 * what a `.textream` file actually contains stays in Rust (`document.rs`),
 * reachable through `saveScriptFile`/`openScriptFile` in `./api`.
 */

import { save, open, confirm } from "@tauri-apps/plugin-dialog";
import * as api from "./api";

let cachedExtension: string | null = null;
const LAST_DOCUMENT_DIRECTORY_KEY = "textream:lastDocumentDirectory";

async function extension(): Promise<string> {
  cachedExtension ??= await api.scriptFileExtension();
  return cachedExtension;
}

async function filter() {
  const ext = await extension();
  return [{ name: "Textream Script", extensions: [ext] }];
}

function lastDocumentDirectory(): string | undefined {
  try {
    return localStorage.getItem(LAST_DOCUMENT_DIRECTORY_KEY) || undefined;
  } catch {
    return undefined;
  }
}

function parentDirectory(path: string): string | null {
  const slash = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  if (slash <= 0) return null;
  return path.slice(0, slash);
}

function rememberDocumentPath(path: string) {
  const directory = parentDirectory(path);
  if (!directory) return;
  try {
    localStorage.setItem(LAST_DOCUMENT_DIRECTORY_KEY, directory);
  } catch {
    // Remembering the picker location is a convenience, never a reason for a
    // successful open/save to fail.
  }
}

/**
 * Opens a native picker with the last successfully used script directory when
 * possible. A stale location (deleted drive/folder, unplugged share) is retried
 * without `defaultPath`, matching the system picker fallback behavior.
 */
async function withRememberedDirectory<T>(
  pick: (defaultPath?: string) => Promise<T>,
): Promise<T> {
  const remembered = lastDocumentDirectory();
  if (!remembered) return pick();

  try {
    return await pick(remembered);
  } catch {
    return pick();
  }
}

/**
 * Prompts for a destination and writes the script there.
 *
 * @returns `true` if the file was written, `false` if the user cancelled.
 */
export async function saveScript(script: string): Promise<boolean> {
  const ext = await extension();
  const filters = await filter();
  const path = await withRememberedDirectory((directory) =>
    save({
      title: "Save script",
      defaultPath: directory
        ? `${directory}\\Untitled.${ext}`
        : `Untitled.${ext}`,
      filters,
    }),
  );
  if (!path) return false;
  await api.saveScriptFile(path, script);
  rememberDocumentPath(path);
  return true;
}

/**
 * Prompts for a `.textream` file and returns its script, flattened to one
 * continuous page.
 *
 * Asks for confirmation first when `currentScript` is non-empty, since opening
 * replaces whatever is in the editor and there is no undo for that here — the
 * script is autosaved continuously, so the one being replaced is not
 * recoverable once this returns.
 *
 * @returns The loaded script, or `null` if the user cancelled at either step.
 */
export async function openScript(
  currentScript: string,
): Promise<string | null> {
  if (currentScript.trim() !== "") {
    const proceed = await confirm(
      "Opening a script replaces the one you're editing now.",
      { title: "Replace current script?", kind: "warning" },
    );
    if (!proceed) return null;
  }

  const filters = await filter();
  const path = await withRememberedDirectory((defaultPath) =>
    open({
      title: "Open script",
      defaultPath,
      filters,
      multiple: false,
      directory: false,
    }),
  );
  if (!path || Array.isArray(path)) return null;
  const script = await api.openScriptFile(path);
  rememberDocumentPath(path);
  return script;
}
