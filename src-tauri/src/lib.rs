//! Textream for Windows — the app shell.
//!
//! The engine lives in [`prompt_core`] and knows nothing about windows or audio.
//! This crate owns the Windows-specific parts: microphone capture, streaming
//! speech recognition, overlay placement, the extended style bits that make an
//! overlay behave like an overlay, the tray icon, and the command surface the
//! webview calls.

mod audio;
mod backdrop;
mod document;
mod model;
mod overlay;
mod session;
mod settings;
mod shortcuts;
mod speech;
mod window_effects;

use std::path::PathBuf;
use std::sync::Mutex;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

use audio::{AudioEngine, DiagnosticsView};
use backdrop::Backdrop;
use model::{DownloadProgress, ModelStatus};
use overlay::Geometry;
use session::{Mode, ProgressView, ScriptView, SessionState};
use settings::Settings;
use speech::Recognizer;

/// Label of the overlay window, as declared in `tauri.conf.json`.
const OVERLAY: &str = "overlay";

const EVENT_SCRIPT: &str = "textream://script";
const EVENT_PROGRESS: &str = "textream://progress";
const EVENT_DOWNLOAD: &str = "textream://model-download";
const EVENT_APPEARANCE: &str = "textream://appearance";

/// The running capture session, if any.
#[derive(Default)]
struct AudioState(Mutex<Option<AudioEngine>>);

/// The backdrop the compositor accepted for the main window.
struct BackdropState(Mutex<Backdrop>);

fn overlay_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    app.get_webview_window(OVERLAY)
        .ok_or_else(|| format!("overlay window '{OVERLAY}' is missing"))
}

/// Root directory for downloaded models.
fn data_root(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|error| format!("no app data directory: {error}"))
}

/// Pushes the latest position to the overlay.
///
/// The overlay renders from events rather than polling: at eight words a second
/// a poll loop either lags the highlight or burns CPU next to OBS.
pub(crate) fn broadcast(app: &AppHandle, progress: ProgressView) -> ProgressView {
    let _ = app.emit_to(OVERLAY, EVENT_PROGRESS, progress);
    progress
}

#[tauri::command]
fn load_script(app: AppHandle, state: tauri::State<'_, SessionState>, text: String) -> ScriptView {
    let view = state.0.lock().unwrap().load(&text);
    let _ = app.emit_to(OVERLAY, EVENT_SCRIPT, view.clone());
    view
}

#[tauri::command]
fn set_mode(state: tauri::State<'_, SessionState>, mode: Mode) {
    state.0.lock().unwrap().set_mode(mode);
}

#[tauri::command]
fn set_speed(state: tauri::State<'_, SessionState>, words_per_second: f64) {
    state
        .0
        .lock()
        .unwrap()
        .set_words_per_second(words_per_second);
}

/// Arms the session and, for the modes that need it, opens the microphone.
///
/// The recogniser is built here rather than at launch so a user who only ever
/// uses Classic never pays for loading a 40 MB model — or for having downloaded
/// one at all.
#[tauri::command]
fn start_session(
    app: AppHandle,
    session: tauri::State<'_, SessionState>,
    audio: tauri::State<'_, AudioState>,
    model_id: Option<String>,
) -> Result<ProgressView, String> {
    let mode = {
        let mut live = session.0.lock().unwrap();
        live.start();
        live.mode()
    };

    if mode.needs_microphone() {
        let recognizer = if mode.needs_speech_recognition() {
            let chosen = model_id
                .as_deref()
                .and_then(model::find)
                .unwrap_or_else(model::default_model);
            let paths = model::paths(&data_root(&app)?, chosen);
            if !paths.all_present() {
                return Err(format!(
                    "the {} speech model has not been downloaded yet",
                    chosen.label
                ));
            }
            Some(Recognizer::new(&paths)?)
        } else {
            None
        };

        let engine = AudioEngine::start(app.clone(), recognizer)?;
        *audio.0.lock().unwrap() = Some(engine);
    }

    let progress = session.0.lock().unwrap().progress();
    Ok(broadcast(&app, progress))
}

#[tauri::command]
fn stop_session(
    app: AppHandle,
    session: tauri::State<'_, SessionState>,
    audio: tauri::State<'_, AudioState>,
) -> ProgressView {
    // Dropping the engine joins the capture and worker threads, so the
    // microphone is released before the session reports stopped.
    *audio.0.lock().unwrap() = None;

    let progress = {
        let mut live = session.0.lock().unwrap();
        live.stop();
        live.progress()
    };
    broadcast(&app, progress)
}

#[tauri::command]
fn is_running(state: tauri::State<'_, SessionState>) -> bool {
    state.0.lock().unwrap().is_running()
}

/// Holds or resumes the prompter without releasing the microphone.
#[tauri::command]
fn set_paused(app: AppHandle, state: tauri::State<'_, SessionState>, paused: bool) -> ProgressView {
    let progress = {
        let mut session = state.0.lock().unwrap();
        session.set_paused(paused);
        session.progress()
    };
    broadcast(&app, progress)
}

/// Mutes the microphone for the running session.
#[tauri::command]
fn set_microphone_muted(audio: tauri::State<'_, AudioState>, muted: bool) {
    if let Some(engine) = audio.0.lock().unwrap().as_ref() {
        engine.set_muted(muted);
    }
}

/// Advances the clock for the paced modes.
///
/// Word Tracking needs no ticking — the audio worker pushes progress events as
/// speech arrives — but the UI calls this regardless so one animation loop
/// covers every mode.
#[tauri::command]
fn tick(app: AppHandle, state: tauri::State<'_, SessionState>, delta_seconds: f64) -> ProgressView {
    let progress = state.0.lock().unwrap().tick(delta_seconds);
    broadcast(&app, progress)
}

#[tauri::command]
fn jump_to_word(
    app: AppHandle,
    state: tauri::State<'_, SessionState>,
    index: usize,
) -> ProgressView {
    let progress = state.0.lock().unwrap().jump_to_word(index);
    broadcast(&app, progress)
}

#[tauri::command]
fn jump_to_offset(
    app: AppHandle,
    state: tauri::State<'_, SessionState>,
    offset: usize,
) -> ProgressView {
    let progress = state.0.lock().unwrap().jump_to_offset(offset);
    broadcast(&app, progress)
}

/// What the capture path is doing right now.
///
/// Words going missing because audio was dropped and words going missing
/// because the model is weak look the same from the outside. This is what
/// separates them.
#[tauri::command]
fn speech_diagnostics(audio: tauri::State<'_, AudioState>) -> DiagnosticsView {
    audio
        .0
        .lock()
        .unwrap()
        .as_ref()
        .map(|engine| engine.diagnostics())
        .unwrap_or_default()
}

/// The global shortcuts and the keys they are bound to.
#[tauri::command]
fn shortcut_bindings() -> Vec<(String, String)> {
    shortcuts::described()
        .into_iter()
        .map(|(name, keys)| (name.to_string(), keys.to_string()))
        .collect()
}

/// The file extension `.textream` files use, without the leading dot.
///
/// Read by the frontend when it builds the save/open dialog's filter, so the
/// extension is written in exactly one place rather than duplicated as a
/// string literal on both sides of the IPC boundary.
#[tauri::command]
fn script_file_extension() -> &'static str {
    document::EXTENSION
}

/// Writes the script to a `.textream` file at `path`.
///
/// The dialog itself is chosen in the frontend, through `@tauri-apps/plugin-dialog`
/// — this command only owns the file format, matching the split the macOS app
/// makes between its `NSSavePanel` and `saveToURL`.
#[tauri::command]
fn save_script_file(path: String, text: String) -> Result<(), String> {
    document::save(std::path::Path::new(&path), &text)
}

/// Reads a `.textream` file at `path`, flattened to one script.
#[tauri::command]
fn open_script_file(path: String) -> Result<String, String> {
    document::load(std::path::Path::new(&path))
}

/// Which backdrop the compositor gave the main window.
///
/// The UI needs this because a frameless transparent window with no effect
/// applied shows the desktop straight through — it has to paint an opaque
/// background instead of looking broken.
#[tauri::command]
fn window_backdrop(state: tauri::State<'_, BackdropState>) -> Backdrop {
    *state.0.lock().unwrap()
}

/// Whether no settings file exists yet.
///
/// Read before `load_settings`, which never writes one — so this stays
/// accurate for as long as the caller waits to check it. The editor uses it to
/// decide whether to show the welcome banner, once, without an ever-growing
/// pile of heuristics for guessing at "new user".
#[tauri::command]
fn is_first_run(app: AppHandle) -> Result<bool, String> {
    Ok(!settings::exists(&data_root(&app)?))
}

#[tauri::command]
fn load_settings(app: AppHandle) -> Result<Settings, String> {
    let settings = settings::load(&data_root(&app)?);
    // Push appearance straight away: the overlay may already be listening, and
    // it must never render one frame in last session's colours.
    let _ = app.emit_to(OVERLAY, EVENT_APPEARANCE, settings.appearance.view());
    Ok(settings)
}

/// Persists settings and pushes the visual half to the overlay.
#[tauri::command]
fn save_settings(app: AppHandle, settings: Settings) -> Result<Settings, String> {
    let settings = settings.sanitised();
    settings::save(&data_root(&app)?, &settings)?;
    let _ = app.emit_to(OVERLAY, EVENT_APPEARANCE, settings.appearance.view());
    Ok(settings)
}

#[tauri::command]
fn speech_models(app: AppHandle) -> Result<Vec<ModelStatus>, String> {
    Ok(model::statuses(&data_root(&app)?))
}

/// Downloads a speech model, streaming progress to the UI.
///
/// Blocking IO on a worker thread rather than in the command itself, so the
/// webview stays responsive for the minute or two this takes.
#[tauri::command]
async fn download_speech_model(app: AppHandle, id: String) -> Result<ModelStatus, String> {
    let chosen = model::find(&id).ok_or_else(|| format!("unknown speech model: {id}"))?;
    let root = data_root(&app)?;
    let emitter = app.clone();

    tauri::async_runtime::spawn_blocking(move || {
        model::download(&root, chosen, |progress: DownloadProgress| {
            let _ = emitter.emit(EVENT_DOWNLOAD, progress);
        })
        .map(|()| model::status(&root, chosen))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn remove_speech_model(app: AppHandle, id: String) -> Result<ModelStatus, String> {
    let chosen = model::find(&id).ok_or_else(|| format!("unknown speech model: {id}"))?;
    let root = data_root(&app)?;
    model::remove(&root, chosen)?;
    Ok(model::status(&root, chosen))
}

#[tauri::command]
fn show_overlay(app: AppHandle, geometry: Geometry) -> Result<(), String> {
    let window = overlay_window(&app)?;
    overlay::apply(&window, geometry).map_err(|error| error.to_string())?;
    // Plain `show` is safe here only because `WS_EX_NOACTIVATE` is already on
    // the window: the style is what stops this call from yanking focus off
    // whatever the presenter is actually driving.
    window.show().map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
fn hide_overlay(app: AppHandle) -> Result<(), String> {
    overlay_window(&app)?
        .hide()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn set_overlay_geometry(app: AppHandle, geometry: Geometry) -> Result<(), String> {
    let window = overlay_window(&app)?;
    overlay::apply(&window, geometry).map_err(|error| error.to_string())
}

/// Lets clicks fall through the overlay to the app behind it.
#[tauri::command]
fn set_click_through(app: AppHandle, enabled: bool) -> Result<(), String> {
    let window = overlay_window(&app)?;
    window
        .set_ignore_cursor_events(enabled)
        .map_err(|error| error.to_string())?;
    let hwnd = window.hwnd().map_err(|error| error.to_string())?;
    window_effects::set_click_through(hwnd.0 as isize, enabled);
    Ok(())
}

/// Hides the overlay from screen shares and recordings.
///
/// Returns whether the platform accepted it — `WDA_EXCLUDEFROMCAPTURE` needs
/// Windows 10 2004 or newer, and the UI has to be able to say so rather than
/// promise privacy it cannot deliver.
#[tauri::command]
fn set_hide_from_capture(app: AppHandle, enabled: bool) -> Result<bool, String> {
    let window = overlay_window(&app)?;
    let hwnd = window.hwnd().map_err(|error| error.to_string())?;
    Ok(window_effects::set_excluded_from_capture(
        hwnd.0 as isize,
        enabled,
    ))
}

/// Brings the main window to the front, restoring it if minimised.
///
/// Shared by the tray's left-click and its "Show Textream" menu item so the
/// two gestures a user reaches for — click the icon, or right-click and pick
/// the item — do exactly the same thing.
fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show Textream", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "hide-overlay", "Hide overlay", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &hide, &quit])?;

    TrayIconBuilder::with_id("textream")
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("Textream")
        .menu(&menu)
        // The menu still opens on right-click regardless of this setting; it
        // only frees up left-click, which every other tray app on Windows
        // treats as "show me the window" rather than "show me the menu".
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            // Windows itself treats WM_LBUTTONUP as the tray icon's activation
            // signal, not the button-down that precedes it — matching that is
            // what makes this feel like every other tray icon.
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main_window(app),
            "hide-overlay" => {
                if let Some(window) = app.get_webview_window(OVERLAY) {
                    let _ = window.hide();
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(shortcuts::plugin())
        .manage(SessionState::new())
        .manage(AudioState::default())
        .manage(BackdropState(Mutex::new(Backdrop::None)))
        .setup(|app| {
            let handle = app.handle().clone();
            build_tray(&handle)?;

            if let Some(window) = app.get_webview_window("main") {
                let applied = backdrop::apply(&window);
                *app.state::<BackdropState>().0.lock().unwrap() = applied;

                // `close()` — what the custom title bar's ✕ button calls —
                // fires this event and then destroys the window unless something
                // intervenes. A destroyed window is gone for the rest of the
                // process: `get_webview_window("main")` returns `None` from then
                // on, so neither the tray's "Show Textream" item nor a left-click
                // could ever bring it back. Hiding it here instead is what makes
                // the tray icon's "show" gesture mean anything at all — the ✕
                // button tucks the editor away rather than ending the session
                // the overlay may still be running.
                let hideable = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = hideable.hide();
                    }
                });
            }

            let refused = shortcuts::register(&handle);
            if !refused.is_empty() {
                eprintln!("shortcuts already taken: {}", refused.join(", "));
            }

            if let Some(window) = app.get_webview_window(OVERLAY) {
                if let Ok(hwnd) = window.hwnd() {
                    // Applied once at startup: these style bits describe what
                    // the window *is*, not a state that toggles.
                    window_effects::make_non_activating(hwnd.0 as isize);
                }
                let _ = window.hide();
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_script,
            set_mode,
            set_speed,
            start_session,
            stop_session,
            is_running,
            set_paused,
            set_microphone_muted,
            tick,
            jump_to_word,
            jump_to_offset,
            window_backdrop,
            shortcut_bindings,
            script_file_extension,
            save_script_file,
            open_script_file,
            speech_diagnostics,
            is_first_run,
            load_settings,
            save_settings,
            speech_models,
            download_speech_model,
            remove_speech_model,
            show_overlay,
            hide_overlay,
            set_overlay_geometry,
            set_click_through,
            set_hide_from_capture,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Textream");
}
