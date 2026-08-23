//! Textream for Windows — the app shell.
//!
//! The engine lives in [`prompt_core`] and knows nothing about windows or audio.
//! This crate owns the Windows-specific parts: overlay placement, the extended
//! style bits that make an overlay behave like an overlay, the tray icon, and
//! the command surface the webview calls.

mod overlay;
mod session;
mod window_effects;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

use overlay::Geometry;
use session::{Mode, ProgressView, ScriptView, SessionState};

/// Label of the overlay window, as declared in `tauri.conf.json`.
const OVERLAY: &str = "overlay";

fn overlay_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    app.get_webview_window(OVERLAY)
        .ok_or_else(|| format!("overlay window '{OVERLAY}' is missing"))
}

/// Pushes the latest position to the overlay.
///
/// The overlay renders from events rather than polling: at eight words a second
/// a poll loop either lags the highlight or burns CPU next to OBS.
fn broadcast(app: &AppHandle, progress: ProgressView) -> ProgressView {
    let _ = app.emit_to(OVERLAY, "textream://progress", progress);
    progress
}

#[tauri::command]
fn load_script(app: AppHandle, state: tauri::State<'_, SessionState>, text: String) -> ScriptView {
    let view = state.0.lock().unwrap().load(&text);
    let _ = app.emit_to(OVERLAY, "textream://script", view.clone());
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

#[tauri::command]
fn start_session(app: AppHandle, state: tauri::State<'_, SessionState>) -> ProgressView {
    let progress = {
        let mut session = state.0.lock().unwrap();
        session.start();
        session.progress()
    };
    broadcast(&app, progress)
}

#[tauri::command]
fn stop_session(app: AppHandle, state: tauri::State<'_, SessionState>) -> ProgressView {
    let progress = {
        let mut session = state.0.lock().unwrap();
        session.stop();
        session.progress()
    };
    broadcast(&app, progress)
}

#[tauri::command]
fn is_running(state: tauri::State<'_, SessionState>) -> bool {
    state.0.lock().unwrap().is_running()
}

#[tauri::command]
fn feed_transcript(
    app: AppHandle,
    state: tauri::State<'_, SessionState>,
    transcript: String,
) -> ProgressView {
    let progress = state.0.lock().unwrap().feed_transcript(&transcript);
    broadcast(&app, progress)
}

/// Drives one animation frame: meters audio, advances the clock, reports back.
///
/// One call per frame rather than one per concern — the webview runs this at
/// display rate, and each extra round trip is paid 60 times a second next to
/// whatever else the presenter is streaming with.
#[tauri::command]
fn tick(
    app: AppHandle,
    state: tauri::State<'_, SessionState>,
    delta_seconds: f64,
    level: Option<f32>,
    timestamp: f64,
) -> ProgressView {
    let progress = state
        .0
        .lock()
        .unwrap()
        .tick(delta_seconds, level, timestamp);
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

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show Textream", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "hide-overlay", "Hide overlay", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &hide, &quit])?;

    TrayIconBuilder::with_id("textream")
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("Textream")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }
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
        .manage(SessionState::new())
        .setup(|app| {
            let handle = app.handle().clone();
            build_tray(&handle)?;

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
            feed_transcript,
            tick,
            jump_to_word,
            jump_to_offset,
            show_overlay,
            hide_overlay,
            set_overlay_geometry,
            set_click_through,
            set_hide_from_capture,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Textream");
}
