/**
 * Typed wrappers over the Rust command surface.
 *
 * Every shape here mirrors a `#[derive(Serialize)]` struct in `src-tauri`.
 * Keeping them in one file means a rename on the Rust side breaks the build in
 * exactly one place instead of scattering `invoke("...")` string literals.
 *
 * Audio never crosses this boundary. The microphone is opened in Rust so the
 * recogniser and the level meter share one input device, and the UI learns
 * about both through `ProgressView`.
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
  /** Microphone level, 0..1, metered in Rust. */
  level: number;
  /** Armed but holding position. */
  paused: boolean;
  finished: boolean;
}

export type FontFamily = "sans" | "serif" | "mono" | "dyslexic";
export type FontSize = "xs" | "sm" | "lg" | "xl";
export type ColorPreset =
  | "white"
  | "yellow"
  | "green"
  | "blue"
  | "pink"
  | "orange";

export interface Appearance {
  fontFamily: FontFamily;
  fontSize: FontSize;
  highlight: ColorPreset;
  cue: ColorPreset;
  /** 0 (fully clear) to 1 (solid). */
  opacity: number;
}

/** Appearance resolved to values the overlay renders directly. */
export interface AppearanceView {
  fontStack: string;
  fontSizePx: number;
  highlight: string;
  cue: string;
  opacity: number;
}

export interface Settings {
  mode: Mode;
  wordsPerSecond: number;
  placement: Placement;
  target: Target;
  width: number;
  height: number;
  hideFromCapture: boolean;
  clickThrough: boolean;
  appearance: Appearance;
  modelId: string | null;
  script: string;
}

export interface ModelStatus {
  id: string;
  label: string;
  language: string;
  installed: boolean;
  downloadBytes: number;
}

export interface DownloadProgress {
  received: number;
  total: number;
}

export const EVENT_SCRIPT = "textream://script";
export const EVENT_PROGRESS = "textream://progress";
export const EVENT_DOWNLOAD = "textream://model-download";
export const EVENT_APPEARANCE = "textream://appearance";

/**
 * Whether no settings file has ever been written.
 *
 * Backed by a file check in Rust, not a guess from empty-looking values —
 * those are also what a returning user sees after clearing their script.
 */
export const isFirstRun = () => invoke<boolean>("is_first_run");

export const loadSettings = () => invoke<Settings>("load_settings");

/**
 * Persists settings and pushes appearance to the overlay.
 *
 * Returns the stored value, which is the sanitised one — out-of-range numbers
 * are clamped in Rust, so the caller should adopt what comes back rather than
 * assume what it sent was kept.
 */
export const saveSettings = (settings: Settings) =>
  invoke<Settings>("save_settings", { settings });

export const loadScript = (text: string) =>
  invoke<ScriptView>("load_script", { text });

export const setMode = (mode: Mode) => invoke<void>("set_mode", { mode });

export const setSpeed = (wordsPerSecond: number) =>
  invoke<void>("set_speed", { wordsPerSecond });

/**
 * Arms the session and opens the microphone if the mode needs one.
 *
 * Rejects when Word Tracking is selected and its model has not been
 * downloaded, so the caller can offer the download rather than start a session
 * that would silently never move.
 */
export const startSession = (modelId: string | null) =>
  invoke<ProgressView>("start_session", { modelId });

export const stopSession = () => invoke<ProgressView>("stop_session");
export const isRunning = () => invoke<boolean>("is_running");

/** Holds or resumes without releasing the microphone. */
export const setPaused = (paused: boolean) =>
  invoke<ProgressView>("set_paused", { paused });

export const setMicrophoneMuted = (muted: boolean) =>
  invoke<void>("set_microphone_muted", { muted });

/**
 * Advances the clock for Classic and Voice-Activated.
 *
 * Word Tracking is driven by the audio worker instead, which pushes progress
 * events as speech arrives; calling this in that mode is a harmless no-op that
 * keeps the UI down to one animation loop.
 */
export const tick = (deltaSeconds: number) =>
  invoke<ProgressView>("tick", { deltaSeconds });

export const jumpToWord = (index: number) =>
  invoke<ProgressView>("jump_to_word", { index });

export const speechModels = () => invoke<ModelStatus[]>("speech_models");

export const downloadSpeechModel = (id: string) =>
  invoke<ModelStatus>("download_speech_model", { id });

export const removeSpeechModel = (id: string) =>
  invoke<ModelStatus>("remove_speech_model", { id });

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

export type BackdropKind = "mica" | "blur" | "none";

/**
 * Which desktop backdrop the compositor gave the main window.
 *
 * `"none"` means the frameless window is transparent with nothing behind it,
 * so the UI has to paint an opaque background rather than show the desktop.
 */
export const windowBackdrop = () => invoke<BackdropKind>("window_backdrop");

export type ShortcutAction = "toggle" | "hold" | "mute";

export const EVENT_SHORTCUT = "textream://shortcut";

/** Global shortcut names paired with the keys they are bound to. */
export const shortcutBindings = () =>
  invoke<[string, string][]>("shortcut_bindings");

export interface SpeechDiagnostics {
  /** Audio the decoder could not keep up with. Should stay at zero. */
  droppedChunks: number;
  decodes: number;
  /** What the recogniser last transcribed, before any script matching. */
  heard: string;
  inputFormat: string;
}

export const speechDiagnostics = () =>
  invoke<SpeechDiagnostics>("speech_diagnostics");
