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

async function extension(): Promise<string> {
  cachedExtension ??= await api.scriptFileExtension();
  return cachedExtension;
}

async function filter() {
  const ext = await extension();
  return [{ name: "Textream Script", extensions: [ext] }];
}

/**
 * Prompts for a destination and writes the script there.
 *
 * @returns `true` if the file was written, `false` if the user cancelled.
 */
export async function saveScript(script: string): Promise<boolean> {
  const ext = await extension();
  const path = await save({
    title: "Save script",
    defaultPath: `Untitled.${ext}`,
    filters: await filter(),
  });
  if (!path) return false;
  await api.saveScriptFile(path, script);
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

  const path = await open({
    title: "Open script",
    filters: await filter(),
    multiple: false,
    directory: false,
  });
  if (!path || Array.isArray(path)) return null;
  return api.openScriptFile(path);
}
