//! Where the prompter sits on screen.
//!
//! The macOS app anchors its overlay under the notch. Windows has no notch, but
//! it has the thing the notch was a proxy for: the webcam, which sits above the
//! monitor. So the default placement is a pill centred on the top edge, which
//! keeps the presenter's eyes within a few degrees of the lens.
//!
//! The taskbar is deliberately *not* a text placement. It is pinned to the
//! bottom of the screen on Windows 11, and reading from down there points the
//! presenter's gaze away from the camera — the exact failure a teleprompter
//! exists to prevent. It gets transport controls only.

use serde::{Deserialize, Serialize};
use tauri::{LogicalSize, Monitor, PhysicalPosition, PhysicalSize, WebviewWindow};

/// Gap between the top edge of the screen and the pill, in logical pixels.
const TOP_MARGIN: f64 = 8.0;

/// Height of the transport strip, in logical pixels.
const STRIP_HEIGHT: f64 = 44.0;

/// Gap between the transport strip and the taskbar, in logical pixels.
const STRIP_BOTTOM_MARGIN: f64 = 12.0;

/// Overlay width bounds, matching the macOS app's slider.
pub const MIN_WIDTH: f64 = 280.0;
pub const MAX_WIDTH: f64 = 500.0;

/// Overlay text-area height bounds.
pub const MIN_HEIGHT: f64 = 100.0;
pub const MAX_HEIGHT: f64 = 400.0;

/// How the overlay is placed on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Placement {
    /// Pill centred on the top edge, near the webcam.
    #[default]
    TopCenter,
    /// Free-floating window the presenter drags where they like.
    Floating,
    /// Fills the target display — second monitor or prompter rig.
    Fullscreen,
    /// Controls-only strip above the taskbar. Never carries script text.
    TransportStrip,
}

/// Which display the overlay belongs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Target {
    /// Follow the display the mouse cursor is on.
    #[default]
    FollowCursor,
    /// Stay on one display, by index into the monitor list.
    Fixed(usize),
}

/// Everything that decides overlay geometry.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Geometry {
    pub placement: Placement,
    pub target: Target,
    /// Logical width, clamped to [`MIN_WIDTH`]..=[`MAX_WIDTH`].
    pub width: f64,
    /// Logical height, clamped to [`MIN_HEIGHT`]..=[`MAX_HEIGHT`].
    pub height: f64,
}

impl Default for Geometry {
    fn default() -> Self {
        Self {
            placement: Placement::default(),
            target: Target::default(),
            width: 420.0,
            height: 160.0,
        }
    }
}

impl Geometry {
    fn clamped(self) -> Self {
        Self {
            width: self.width.clamp(MIN_WIDTH, MAX_WIDTH),
            height: self.height.clamp(MIN_HEIGHT, MAX_HEIGHT),
            ..self
        }
    }
}

/// Picks the display the overlay should appear on.
///
/// Falls back through cursor monitor, primary, then first available, because a
/// disconnected display or a cursor on no monitor at all must not leave the
/// prompter unplaced.
fn resolve_monitor(window: &WebviewWindow, target: Target) -> tauri::Result<Option<Monitor>> {
    let monitors = window.available_monitors()?;
    if monitors.is_empty() {
        return Ok(None);
    }

    let chosen = match target {
        Target::Fixed(index) => monitors.get(index).cloned(),
        Target::FollowCursor => window.cursor_position().ok().and_then(|cursor| {
            monitors
                .iter()
                .find(|monitor| {
                    let position = monitor.position();
                    let size = monitor.size();
                    let x = cursor.x as i32;
                    let y = cursor.y as i32;
                    x >= position.x
                        && x < position.x + size.width as i32
                        && y >= position.y
                        && y < position.y + size.height as i32
                })
                .cloned()
        }),
    };

    Ok(chosen
        .or_else(|| window.primary_monitor().ok().flatten())
        .or_else(|| monitors.first().cloned()))
}

/// Sizes and positions the overlay window for `geometry`.
///
/// Floating placement only resizes — the whole point is that the presenter owns
/// the position, so re-centring it on every settings change would fight them.
pub fn apply(window: &WebviewWindow, geometry: Geometry) -> tauri::Result<()> {
    let geometry = geometry.clamped();
    let Some(monitor) = resolve_monitor(window, geometry.target)? else {
        return Ok(());
    };

    let scale = monitor.scale_factor();
    let origin = *monitor.position();
    let screen = *monitor.size();

    match geometry.placement {
        Placement::Fullscreen => {
            window.set_size(PhysicalSize::new(screen.width, screen.height))?;
            window.set_position(PhysicalPosition::new(origin.x, origin.y))?;
        }
        Placement::Floating => {
            window.set_size(LogicalSize::new(geometry.width, geometry.height))?;
        }
        Placement::TopCenter => {
            window.set_size(LogicalSize::new(geometry.width, geometry.height))?;
            let width = (geometry.width * scale).round() as i32;
            let x = origin.x + (screen.width as i32 - width) / 2;
            let y = origin.y + (TOP_MARGIN * scale).round() as i32;
            window.set_position(PhysicalPosition::new(x, y))?;
        }
        Placement::TransportStrip => {
            window.set_size(LogicalSize::new(geometry.width, STRIP_HEIGHT))?;
            let width = (geometry.width * scale).round() as i32;
            let height = (STRIP_HEIGHT * scale).round() as i32;
            let margin = (STRIP_BOTTOM_MARGIN * scale).round() as i32;
            let x = origin.x + (screen.width as i32 - width) / 2;
            // Sits above the taskbar rather than reserving space with an
            // AppBar: a real AppBar shrinks every maximised window and leaves
            // the desktop work area wrong if the app dies.
            let y = origin.y + screen.height as i32 - height - margin;
            window.set_position(PhysicalPosition::new(x, y))?;
        }
    }

    Ok(())
}
