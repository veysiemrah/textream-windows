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
use tauri::{LogicalSize, Monitor, PhysicalPosition, PhysicalSize, Size, WebviewWindow};

/// Gap between the top edge of the screen and the pill, in logical pixels.
const TOP_MARGIN: f64 = 8.0;

/// Height of the transport strip, in logical pixels.
const STRIP_HEIGHT: f64 = 44.0;

/// Gap between the transport strip and the taskbar, in logical pixels.
const STRIP_BOTTOM_MARGIN: f64 = 12.0;

/// Overlay width bounds. The macOS slider stops at 500, which suits a notch
/// pill; a floating or top-centre prompter on a wide monitor reads better wider.
pub const MIN_WIDTH: f64 = 280.0;
pub const MAX_WIDTH: f64 = 1200.0;

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
    /// Where the presenter last left the floating window, in physical pixels.
    /// Only Floating reads it; `None` until the window has been placed once.
    #[serde(default)]
    pub x: Option<i32>,
    #[serde(default)]
    pub y: Option<i32>,
}

impl Default for Geometry {
    fn default() -> Self {
        Self {
            placement: Placement::default(),
            target: Target::default(),
            width: 420.0,
            height: 160.0,
            x: None,
            y: None,
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

/// Whether a window whose top-left corner is at (`x`, `y`) would show enough
/// of itself on one of `screens` (origin and size, physical pixels) to be
/// grabbed and dragged back.
///
/// A saved position can point at a monitor that has since been unplugged or
/// rearranged, and restoring it blindly would put the prompter where nobody
/// can see it, let alone move it.
fn is_reachable(screens: &[(PhysicalPosition<i32>, PhysicalSize<u32>)], x: i32, y: i32) -> bool {
    // A point a little inside the top edge — the grab bar lives there.
    let (grab_x, grab_y) = (x.saturating_add(40), y.saturating_add(8));
    screens.iter().any(|(origin, size)| {
        grab_x >= origin.x
            && grab_x < origin.x + size.width as i32
            && grab_y >= origin.y
            && grab_y < origin.y + size.height as i32
    })
}

/// Sizes and positions the overlay window for `geometry`.
///
/// Floating restores the position the presenter left it at rather than
/// re-centring it: the whole point is that they own where it goes. Only
/// Floating can be resized by hand; the other placements are sized from
/// settings, and fullscreen must not be capped by the floating bounds.
pub fn apply(window: &WebviewWindow, geometry: Geometry) -> tauri::Result<()> {
    let geometry = geometry.clamped();
    let Some(monitor) = resolve_monitor(window, geometry.target)? else {
        return Ok(());
    };

    let scale = monitor.scale_factor();
    let origin = *monitor.position();
    let screen = *monitor.size();

    let floating = geometry.placement == Placement::Floating;
    window.set_resizable(floating)?;
    window.set_min_size(floating.then(|| Size::from(LogicalSize::new(MIN_WIDTH, MIN_HEIGHT))))?;
    window.set_max_size(floating.then(|| Size::from(LogicalSize::new(MAX_WIDTH, MAX_HEIGHT))))?;

    match geometry.placement {
        Placement::Fullscreen => {
            window.set_size(PhysicalSize::new(screen.width, screen.height))?;
            window.set_position(PhysicalPosition::new(origin.x, origin.y))?;
        }
        Placement::Floating => {
            window.set_size(LogicalSize::new(geometry.width, geometry.height))?;
            let screens: Vec<_> = window
                .available_monitors()?
                .iter()
                .map(|monitor| (*monitor.position(), *monitor.size()))
                .collect();
            let position = match (geometry.x, geometry.y) {
                (Some(x), Some(y)) if is_reachable(&screens, x, y) => PhysicalPosition::new(x, y),
                // First use, or its display is gone: start where Top centre
                // would be, a little lower so it reads as a separate window.
                _ => {
                    let width = (geometry.width * scale).round() as i32;
                    PhysicalPosition::new(
                        origin.x + (screen.width as i32 - width) / 2,
                        origin.y + (TOP_MARGIN * 6.0 * scale).round() as i32,
                    )
                }
            };
            window.set_position(position)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(x: i32, y: i32, width: u32, height: u32) -> (PhysicalPosition<i32>, PhysicalSize<u32>) {
        (PhysicalPosition::new(x, y), PhysicalSize::new(width, height))
    }

    #[test]
    fn a_position_on_a_connected_display_is_restored() {
        let screens = [screen(0, 0, 1920, 1080), screen(1920, 0, 2560, 1440)];
        assert!(is_reachable(&screens, 100, 100));
        assert!(is_reachable(&screens, 3000, 200));
    }

    #[test]
    fn a_position_on_an_unplugged_display_is_not() {
        let screens = [screen(0, 0, 1920, 1080)];
        assert!(!is_reachable(&screens, 2500, 200));
        assert!(!is_reachable(&screens, 100, -400));
    }

    #[test]
    fn a_window_hanging_off_the_left_edge_is_unreachable_once_its_grab_bar_is() {
        let screens = [screen(0, 0, 1920, 1080)];
        assert!(is_reachable(&screens, -30, 0));
        assert!(!is_reachable(&screens, -100, 0));
    }
}
