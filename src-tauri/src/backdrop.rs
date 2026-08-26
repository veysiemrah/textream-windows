//! Desktop-composited translucency for the frameless main window.
//!
//! The window is transparent and draws no chrome of its own, so something has
//! to sit behind the UI. Doing it in CSS would mean `backdrop-filter` over a
//! see-through window, which blurs nothing — there is no page content behind
//! it, only the desktop, and the webview cannot reach that. DWM can.

use serde::Serialize;
use tauri::WebviewWindow;

/// Which effect the compositor accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Backdrop {
    /// Windows 11's material, tinted by the desktop wallpaper.
    Mica,
    /// The Windows 10 blur behind.
    Blur,
    /// Nothing applied — the UI must paint an opaque background itself.
    None,
}

/// Applies the best backdrop this version of Windows can render well.
///
/// Acrylic is deliberately not in the chain. It is available from Windows 10
/// v1809, but from v1903 onwards DWM stops compositing it smoothly and dragging
/// the window visibly lags the cursor. A teleprompter that stutters when you
/// move it reads as broken software, so the older and cheaper blur is the
/// better trade on Windows 10.
pub fn apply(window: &WebviewWindow) -> Backdrop {
    // Windows 11: mica is the native material and costs almost nothing.
    if window_vibrancy::apply_mica(window, Some(true)).is_ok() {
        return Backdrop::Mica;
    }
    // Windows 10 v1809+.
    if window_vibrancy::apply_blur(window, Some((16, 18, 24, 160))).is_ok() {
        return Backdrop::Blur;
    }
    Backdrop::None
}
