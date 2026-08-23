/**
 * Typed wrappers over the Rust command surface.
 *
 * Every shape here mirrors a `#[derive(Serialize)]` struct in `src-tauri`.
 * Keeping them in one file means a rename on the Rust side breaks the build in
 * exactly one place instead of scattering `invoke("...")` string literals.
 */

import { invoke } from "@tauri-apps/api/core";

export type Mode = "wordTracking" | "classic" | "voiceActivated";
export type Placement =
  | "topCenter"
  | "floating"
  | "fullscreen"
  | "transportStrip";

export type Target = "followCursor" | { fixed: number };

export interface Geometry {
  placement: Placement;
  target: Target;
  /** Logical pixels, clamped to 280..500 in Rust. */
  width: number;
  /** Logical pixels, clamped to 100..400 in Rust. */
  height: number;
}

export interface WordView {
  id: number;
  text: string;
  /** Grapheme offset of the word's first character. */
  start: number;
  /** Grapheme offset one past the word's last character. */
  end: number;
  isAnnotation: boolean;
}

export interface ScriptView {
  text: string;
  words: WordView[];
  characterCount: number;
  direction: "ltr" | "rtl";
}

export interface ProgressView {
  characterOffset: number;
  wordProgress: number;
  activeWord: number | null;
  voiceActive: boolean;
  finished: boolean;
}

export const EVENT_SCRIPT = "textream://script";
export const EVENT_PROGRESS = "textream://progress";

export const loadScript = (text: string) =>
  invoke<ScriptView>("load_script", { text });

export const setMode = (mode: Mode) => invoke<void>("set_mode", { mode });

export const setSpeed = (wordsPerSecond: number) =>
  invoke<void>("set_speed", { wordsPerSecond });

export const startSession = () => invoke<ProgressView>("start_session");
export const stopSession = () => invoke<ProgressView>("stop_session");
export const isRunning = () => invoke<boolean>("is_running");

export const feedTranscript = (transcript: string) =>
  invoke<ProgressView>("feed_transcript", { transcript });

/**
 * One call per animation frame. Pass `level` when a microphone is running so
 * the speech gate and the clock advance together — splitting them would double
 * the IPC traffic and let the gate be read a frame stale.
 *
 * @param timestamp seconds from the same monotonic clock across frames.
 */
export const tick = (
  deltaSeconds: number,
  level: number | null,
  timestamp: number,
) => invoke<ProgressView>("tick", { deltaSeconds, level, timestamp });

export const jumpToWord = (index: number) =>
  invoke<ProgressView>("jump_to_word", { index });

export const showOverlay = (geometry: Geometry) =>
  invoke<void>("show_overlay", { geometry });

export const hideOverlay = () => invoke<void>("hide_overlay");

export const setOverlayGeometry = (geometry: Geometry) =>
  invoke<void>("set_overlay_geometry", { geometry });

export const setClickThrough = (enabled: boolean) =>
  invoke<void>("set_click_through", { enabled });

/** Resolves to whether Windows accepted the request — needs 10 2004+. */
export const setHideFromCapture = (enabled: boolean) =>
  invoke<boolean>("set_hide_from_capture", { enabled });
