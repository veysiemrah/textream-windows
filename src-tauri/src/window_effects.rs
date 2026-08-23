//! Win32 window traits the overlay needs and no cross-platform API exposes.
//!
//! Tauri can make a window borderless, transparent and always-on-top. It cannot
//! make one that never takes focus, never appears in a screen share, and lets
//! clicks fall through to OBS behind it. Those are extended window style bits
//! and one display-affinity call.
//!
//! HWNDs cross this module boundary as `isize` on purpose. Tauri links its own
//! version of the `windows` crate, and passing its `HWND` type through would
//! pin us to whatever version Tauri happens to depend on.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowDisplayAffinity, SetWindowLongPtrW, GWL_EXSTYLE,
    WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WINDOW_DISPLAY_AFFINITY, WINDOW_EX_STYLE, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
};

fn handle(hwnd: isize) -> HWND {
    HWND(hwnd as *mut core::ffi::c_void)
}

/// Reads, edits and writes back the extended style bits.
fn update_ex_style(hwnd: isize, add: WINDOW_EX_STYLE, remove: WINDOW_EX_STYLE) {
    let window = handle(hwnd);
    // SAFETY: the HWND comes from Tauri's live window; both calls are plain
    // reads/writes of the window's own style word.
    unsafe {
        let current = WINDOW_EX_STYLE(GetWindowLongPtrW(window, GWL_EXSTYLE) as u32);
        let updated = (current & !remove) | add;
        SetWindowLongPtrW(window, GWL_EXSTYLE, updated.0 as isize);
    }
}

/// Stops the overlay from ever stealing keyboard focus.
///
/// Without `WS_EX_NOACTIVATE`, showing the prompter pulls focus off PowerPoint
/// or the browser the presenter is actually driving, and their next keystroke
/// goes nowhere. `WS_EX_TOOLWINDOW` additionally keeps it out of Alt-Tab, where
/// a borderless strip is only ever noise.
pub fn make_non_activating(hwnd: isize) {
    update_ex_style(
        hwnd,
        WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
        WINDOW_EX_STYLE(0),
    );
}

/// Lets mouse input pass through to whatever is behind the overlay.
///
/// Turned off while the reader needs to tap a word to jump, on the rest of the
/// time so the overlay never intercepts a click meant for the app underneath.
pub fn set_click_through(hwnd: isize, enabled: bool) {
    if enabled {
        update_ex_style(hwnd, WS_EX_TRANSPARENT, WINDOW_EX_STYLE(0));
    } else {
        update_ex_style(hwnd, WINDOW_EX_STYLE(0), WS_EX_TRANSPARENT);
    }
}

/// Hides the window from screen capture, screen sharing and recording.
///
/// `WDA_EXCLUDEFROMCAPTURE` keeps the window visible on the physical display
/// while the compositor omits it from every capture surface. It needs Windows
/// 10 2004 (build 19041) or newer; on anything older the call fails and the
/// overlay simply stays visible in shares, which is why the result is reported
/// rather than swallowed.
pub fn set_excluded_from_capture(hwnd: isize, enabled: bool) -> bool {
    let affinity: WINDOW_DISPLAY_AFFINITY = if enabled {
        WDA_EXCLUDEFROMCAPTURE
    } else {
        WDA_NONE
    };
    // SAFETY: valid live HWND; the call only sets this window's affinity.
    unsafe { SetWindowDisplayAffinity(handle(hwnd), affinity).is_ok() }
}
