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
  /** Logical pixels, clamped to 280..1200 in Rust. */
  width: number;
  /** Logical pixels, clamped to 100..400 in Rust. */
  height: number;
  /** Floating only: where it was last left, in physical pixels. */
  x: number | null;
  y: number | null;
}

/** The floating overlay's bounds after the presenter moved or resized it. */
export interface OverlayBounds {
  /** Physical pixels. */
  x: number;
  y: number;
  /** Logical pixels. */
  width: number;
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
  /** Line breaks the author put before this word: 0, 1, or 2 for a blank line. */
  breaksBefore: number;
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
  cueBrightness: CueBrightness;
  /** 0 (fully clear) to 1 (solid). */
  opacity: number;
  readingPosition: ReadingPosition;
  /** Fullscreen only: 0.5–1.5 times the size worked out from the width. */
  fullscreenTextScale: number;
  /** Fullscreen only: 0–0.4 of the screen width on each side. */
  fullscreenSideMargin: number;
}

export type CueBrightness = "dim" | "low" | "medium" | "bright";

/** Where the line being read sits in the prompter. */
export type ReadingPosition = "upperThird" | "centered";

/** Appearance resolved to values the overlay renders directly. */
export interface AppearanceView {
  fontStack: string;
  fontSizePx: number;
  highlight: string;
  cue: string;
  cueOpacity: number;
  cueActiveOpacity: number;
  opacity: number;
  /** Fraction of the prompter's height the active line is kept at. */
  readingAnchor: number;
  fullscreenTextScale: number;
  fullscreenSideMargin: number;
}

export interface Settings {
  mode: Mode;
  wordsPerSecond: number;
  placement: Placement;
  target: Target;
  width: number;
  height: number;
  /** Where the floating prompter was last left, in physical pixels. */
  floatingX: number | null;
  floatingY: number | null;
  hideFromCapture: boolean;
  clickThrough: boolean;
  /** Stop the display sleeping while a take runs. */
  keepScreenAwake: boolean;
  appearance: Appearance;
  modelId: string | null;
  /** Input device to capture from, by name. `null` means system default. */
  microphoneId: string | null;
  script: string;
}

/**
 * An input device as cpal sees it.
 *
 * `id` is the device's name — cpal exposes no other stable identifier on
 * Windows, so the name doubles as the id on both sides of the IPC boundary.
 */
export interface InputDevice {
  id: string;
  name: string;
  isDefault: boolean;
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
export const EVENT_PLACEMENT = "textream://placement";
export const EVENT_MICROPHONE = "textream://microphone";

/** What happened to the microphone during a take. */
export type MicrophoneEvent =
  | { state: "reconnecting" }
  | { state: "reconnected"; device: string }
  | { state: "lost"; message: string };
export const EVENT_OVERLAY_BOUNDS = "textream://overlay-bounds";

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
export const startSession = (modelId: string | null, deviceId: string | null) =>
  invoke<ProgressView>("start_session", { modelId, deviceId });

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

/**
 * Every currently usable microphone.
 *
 * Backed by a fresh cpal enumeration on every call rather than a cache, so
 * calling this again when the dropdown opens picks up a device plugged in
 * mid-session.
 */
export const listInputDevices = () =>
  invoke<InputDevice[]>("list_input_devices");

/**
 * Switches the running session to a different microphone.
 *
 * A no-op when nothing is currently listening — the new device only takes
 * effect the next time a session starts. When something is listening, this
 * restarts capture (and, for Follow mode, the recogniser) against the new
 * device.
 */
export const switchMicrophoneDevice = (
  deviceId: string | null,
  modelId: string | null,
) => invoke<void>("switch_microphone_device", { deviceId, modelId });

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

export const setKeepAwake = (enabled: boolean) =>
  invoke<void>("set_keep_awake", { enabled });

export type BackdropKind = "mica" | "blur" | "none";

/**
 * Which desktop backdrop the compositor gave the main window.
 *
 * `"none"` means the frameless window is transparent with nothing behind it,
 * so the UI has to paint an opaque background rather than show the desktop.
 */
export const windowBackdrop = () => invoke<BackdropKind>("window_backdrop");

export type ShortcutAction = "toggle" | "hold" | "mute";

/**
 * What the taskbar strip's buttons need to show that the overlay cannot see
 * itself. Its buttons send a {@link ShortcutAction} back, so they take the
 * same path as the global shortcuts.
 */
export interface TransportView {
  muted: boolean;
  /** The mode uses the microphone, so muting means something. */
  microphone: boolean;
}

export const EVENT_TRANSPORT = "textream://transport";

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

/**
 * The `.textream` extension, without the leading dot.
 *
 * Read from Rust rather than hardcoded here, so the string exists in exactly
 * one place instead of duplicated across the IPC boundary.
 */
export const scriptFileExtension = () =>
  invoke<string>("script_file_extension");

/**
 * Writes `text` as a `.textream` file at `path`.
 *
 * `path` comes from the native save dialog (see `lib/files.ts`), not from
 * this module — the dialog is a webview concern, the file format is a Rust
 * one, and keeping that split means the format can be unit-tested without a
 * window.
 */
export const saveScriptFile = (path: string, text: string) =>
  invoke<void>("save_script_file", { path, text });

/** Reads a `.textream` file at `path`, flattened to one script. */
export const openScriptFile = (path: string) =>
  invoke<string>("open_script_file", { path });
