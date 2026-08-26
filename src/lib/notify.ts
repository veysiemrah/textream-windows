/**
 * Native toast notifications, for the one blind spot the inline status
 * message cannot cover.
 *
 * A global shortcut can start or stop a take while the editor window sits
 * hidden in the tray — the entire point of the shortcuts is to do exactly
 * that without touching the app. If starting fails in that moment (no
 * microphone, a script cleared earlier), the inline status text in `App.svelte`
 * updates in a window nobody is looking at, which is not a message at all.
 * A toast reaches the presenter without stealing focus from whatever they are
 * actually doing.
 */

import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";

let granted = false;

/**
 * Resolves notification permission once, at startup.
 *
 * Not requested lazily on the first warning: a Windows permission prompt
 * appearing mid-presentation, triggered by the very problem it exists to
 * report, would be a worse interruption than the silence it replaces.
 */
export async function init(): Promise<void> {
  try {
    granted = await isPermissionGranted();
    if (!granted) {
      granted = (await requestPermission()) === "granted";
    }
  } catch {
    granted = false;
  }
}

/** Shows `message` as a toast, if permission was granted at startup. */
export function notifyWarning(message: string): void {
  if (!granted) return;
  try {
    sendNotification({ title: "Textream", body: message });
  } catch {
    // A failed notification must never be the reason anything else breaks.
  }
}
