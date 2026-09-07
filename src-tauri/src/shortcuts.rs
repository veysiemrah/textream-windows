//! Global shortcuts for hands-free control.
//!
//! A presenter reading to camera cannot reach for the app window, so start,
//! hold and mute have to work while something else has focus. That is the whole
//! reason these are system-wide rather than window accelerators.
//!
//! The handler emits to the main window rather than driving the session
//! directly. Starting a take needs the script, the mode and the overlay
//! geometry — all of which the UI already assembles — and duplicating that here
//! would be a second code path that could disagree with the first.

use serde::Serialize;
use tauri::plugin::TauriPlugin;
use tauri::{AppHandle, Emitter, Wry};
use tauri_plugin_global_shortcut::{Builder, Code, Modifiers, Shortcut, ShortcutState};

/// Event carrying which shortcut fired.
pub const EVENT: &str = "textream://shortcut";

/// What the presenter asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    /// Start a take, or stop the running one.
    Toggle,
    /// Hold position, or carry on.
    Hold,
    /// Mute or unmute the microphone.
    Mute,
}

/// Chosen to sit under three modifiers because a teleprompter runs alongside
/// whatever the presenter is actually doing, and a system-wide binding that
/// shadows an ordinary editing key would be worse than no binding at all.
const BINDINGS: &[(Code, Action, &str)] = &[
    (Code::F9, Action::Toggle, "Ctrl+Alt+Shift+F9"),
    (Code::F10, Action::Hold, "Ctrl+Alt+Shift+F10"),
    (Code::F11, Action::Mute, "Ctrl+Alt+Shift+F11"),
];

fn modifiers() -> Modifiers {
    Modifiers::CONTROL | Modifiers::ALT | Modifiers::SHIFT
}

/// Human-readable bindings, so the UI can show what the keys are.
pub fn described() -> Vec<(&'static str, &'static str)> {
    BINDINGS
        .iter()
        .map(|(_, action, label)| {
            let name = match action {
                Action::Toggle => "Start or stop",
                Action::Hold => "Hold or resume",
                Action::Mute => "Mute the microphone",
            };
            (name, *label)
        })
        .collect()
}

/// Builds the OS-level shortcut plugin with the handler already attached.
///
/// `tauri-plugin-global-shortcut` registers these with Windows rather than with
/// either webview, so delivery does not depend on Textream being the foreground
/// application. OBS, a browser, PowerPoint, or any other app can own focus and
/// these bindings still arrive here.
pub fn plugin() -> TauriPlugin<Wry> {
    Builder::new()
        .with_handler(|app, shortcut, event| {
            // Key-down only. Without this every shortcut fires twice, and a
            // toggle that fires twice does nothing at all.
            if event.state() != ShortcutState::Pressed {
                return;
            }
            if let Some(action) = action_for(shortcut) {
                let _ = app.emit_to("main", EVENT, action);
            }
        })
        .build()
}

fn action_for(shortcut: &Shortcut) -> Option<Action> {
    BINDINGS
        .iter()
        .find(|(code, _, _)| shortcut.matches(modifiers(), *code))
        .map(|(_, action, _)| *action)
}

/// Registers every binding, reporting the ones the system refused.
///
/// A shortcut another application already owns fails to register, and that is
/// worth surfacing rather than leaving the presenter pressing a dead key.
pub fn register(app: &AppHandle) -> Vec<&'static str> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;

    let manager = app.global_shortcut();
    let mut refused = Vec::new();
    for (code, _, label) in BINDINGS {
        if manager
            .register(Shortcut::new(Some(modifiers()), *code))
            .is_err()
        {
            refused.push(*label);
        }
    }
    refused
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_binding_is_described() {
        assert_eq!(described().len(), BINDINGS.len());
        for (name, keys) in described() {
            assert!(!name.is_empty());
            assert!(keys.starts_with("Ctrl+Alt+Shift+"));
        }
    }

    #[test]
    fn bindings_use_distinct_keys() {
        let mut codes: Vec<String> = BINDINGS.iter().map(|(c, _, _)| format!("{c:?}")).collect();
        codes.sort();
        let count = codes.len();
        codes.dedup();
        assert_eq!(codes.len(), count, "two shortcuts share a key");
    }

    #[test]
    fn matching_resolves_every_binding_to_the_expected_action() {
        let expected = [Action::Toggle, Action::Hold, Action::Mute];
        for ((code, _, _), expected_action) in BINDINGS.iter().zip(expected) {
            let shortcut = Shortcut::new(Some(modifiers()), *code);
            assert_eq!(action_for(&shortcut), Some(expected_action));
        }
    }

    #[test]
    fn missing_or_wrong_modifiers_do_not_trigger_an_action() {
        assert_eq!(action_for(&Shortcut::new(None, Code::F9)), None);
        assert_eq!(
            action_for(&Shortcut::new(Some(Modifiers::CONTROL), Code::F9)),
            None
        );
    }
}
