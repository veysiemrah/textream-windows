//! Persisted preferences.
//!
//! Settings live in Rust rather than in the webview because two windows need
//! them — the editor renders the controls, the overlay renders the result — and
//! because the overlay has to be placed correctly before either window has
//! finished loading.
//!
//! Everything is optional on read. A settings file written by an older build
//! must never stop the app from starting, so unknown fields are ignored and
//! missing ones fall back to the default.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::overlay::{Geometry, Placement, Target};
use crate::session::Mode;

/// Typeface for the prompter text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum FontFamily {
    #[default]
    Sans,
    Serif,
    Mono,
    /// OpenDyslexic, bundled with the app.
    Dyslexic,
}

/// Text size presets, mirroring the macOS app's four steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum FontSize {
    Xs,
    Sm,
    #[default]
    Lg,
    Xl,
}

impl FontSize {
    /// Point size for the overlay, matching the macOS presets.
    pub fn points(self) -> f64 {
        match self {
            FontSize::Xs => 14.0,
            FontSize::Sm => 16.0,
            FontSize::Lg => 20.0,
            FontSize::Xl => 24.0,
        }
    }
}

/// Highlight and cue colours offered in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ColorPreset {
    White,
    #[default]
    Yellow,
    Green,
    Blue,
    Pink,
    Orange,
}

impl ColorPreset {
    /// CSS colour, matching the macOS palette.
    pub fn css(self) -> &'static str {
        match self {
            ColorPreset::White => "#ffffff",
            ColorPreset::Yellow => "#ffd60a",
            ColorPreset::Green => "#33d64a",
            ColorPreset::Blue => "#4f8cff",
            ColorPreset::Pink => "#ff6191",
            ColorPreset::Orange => "#ff9e0a",
        }
    }
}

/// How strongly `[cue]` text shows, mirroring the macOS app's four steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum CueBrightness {
    Dim,
    Low,
    /// What cues always looked like before this was a setting.
    #[default]
    Medium,
    Bright,
}

impl CueBrightness {
    /// Opacity of a cue, and of the cue at the reading position.
    pub fn opacities(self) -> (f64, f64) {
        match self {
            CueBrightness::Dim => (0.3, 0.5),
            CueBrightness::Low => (0.42, 0.62),
            CueBrightness::Medium => (0.55, 0.75),
            CueBrightness::Bright => (0.85, 1.0),
        }
    }
}

/// Where the line being read sits in the prompter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ReadingPosition {
    /// A third of the way down: more of what is coming than of what is gone.
    #[default]
    UpperThird,
    /// Halfway down, with as much context above as below.
    Centered,
}

impl ReadingPosition {
    /// Fraction of the prompter's height the active line is kept at.
    pub fn anchor(self) -> f64 {
        match self {
            ReadingPosition::UpperThird => 1.0 / 3.0,
            ReadingPosition::Centered => 0.5,
        }
    }
}

/// Lowest and highest background opacity the slider offers.
pub const MIN_OPACITY: f64 = 0.0;
pub const MAX_OPACITY: f64 = 1.0;

/// How the overlay looks.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Appearance {
    pub font_family: FontFamily,
    pub font_size: FontSize,
    /// Colour of the word being spoken.
    pub highlight: ColorPreset,
    /// Colour of `[cue]` text.
    pub cue: ColorPreset,
    pub cue_brightness: CueBrightness,
    /// Opacity of the overlay's own background, 0 (fully clear) to 1 (solid).
    pub opacity: f64,
    pub reading_position: ReadingPosition,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            font_family: FontFamily::default(),
            font_size: FontSize::default(),
            highlight: ColorPreset::Yellow,
            cue: ColorPreset::Orange,
            cue_brightness: CueBrightness::default(),
            opacity: 0.92,
            reading_position: ReadingPosition::default(),
        }
    }
}

/// Appearance resolved to values the overlay can render directly.
///
/// The mapping from preset to CSS lives here rather than in the stylesheet so
/// the editor's preview and the overlay itself cannot drift apart.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppearanceView {
    pub font_stack: &'static str,
    pub font_size_px: f64,
    pub highlight: &'static str,
    pub cue: &'static str,
    pub cue_opacity: f64,
    pub cue_active_opacity: f64,
    pub opacity: f64,
    /// Fraction of the prompter's height the active line is kept at.
    pub reading_anchor: f64,
}

impl Appearance {
    pub fn view(&self) -> AppearanceView {
        AppearanceView {
            font_stack: match self.font_family {
                FontFamily::Sans => {
                    "\"Segoe UI Variable Display\", \"Segoe UI\", system-ui, sans-serif"
                }
                FontFamily::Serif => "Georgia, \"Times New Roman\", serif",
                FontFamily::Mono => "\"Cascadia Mono\", Consolas, ui-monospace, monospace",
                FontFamily::Dyslexic => "\"OpenDyslexic Three\", \"Segoe UI\", sans-serif",
            },
            font_size_px: self.font_size.points(),
            highlight: self.highlight.css(),
            cue: self.cue.css(),
            cue_opacity: self.cue_brightness.opacities().0,
            cue_active_opacity: self.cue_brightness.opacities().1,
            opacity: self.opacity,
            reading_anchor: self.reading_position.anchor(),
        }
    }

    fn sanitised(mut self) -> Self {
        self.opacity = if self.opacity.is_finite() {
            self.opacity.clamp(MIN_OPACITY, MAX_OPACITY)
        } else {
            Self::default().opacity
        };
        self
    }
}

/// Everything the app remembers between launches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub mode: Mode,
    pub words_per_second: f64,
    pub placement: Placement,
    pub target: Target,
    pub width: f64,
    pub height: f64,
    /// Where the floating prompter was last left, in physical pixels.
    pub floating_x: Option<i32>,
    pub floating_y: Option<i32>,
    pub hide_from_capture: bool,
    pub click_through: bool,
    /// Stops the display sleeping while a take runs. Off by default: a
    /// presenter who wants it knows to turn it on, and nobody else should
    /// find their screen never sleeping.
    pub keep_screen_awake: bool,
    pub appearance: Appearance,
    /// Speech model to use, `None` meaning the registry default.
    pub model_id: Option<String>,
    /// The input device to capture from, keyed by name (`None` meaning the
    /// system default). A name that no longer matches any device falls back
    /// to the default at capture time rather than failing to start.
    pub microphone_id: Option<String>,
    /// The script last edited, restored on launch.
    pub script: String,
}

impl Default for Settings {
    fn default() -> Self {
        let geometry = Geometry::default();
        Self {
            mode: Mode::Classic,
            words_per_second: 2.0,
            placement: geometry.placement,
            target: geometry.target,
            width: geometry.width,
            height: geometry.height,
            floating_x: geometry.x,
            floating_y: geometry.y,
            hide_from_capture: true,
            click_through: true,
            keep_screen_awake: false,
            appearance: Appearance::default(),
            model_id: None,
            microphone_id: None,
            script: String::new(),
        }
    }
}

impl Settings {
    /// Clamps every numeric field into range.
    ///
    /// A hand-edited or older settings file can carry a pace of zero or a
    /// negative width, and those would otherwise reach the scroller and the
    /// window manager as-is.
    pub fn sanitised(mut self) -> Self {
        let fallback = Self::default();
        self.words_per_second = if self.words_per_second.is_finite() {
            self.words_per_second.clamp(
                prompt_core::MIN_WORDS_PER_SECOND,
                prompt_core::MAX_WORDS_PER_SECOND,
            )
        } else {
            fallback.words_per_second
        };
        self.width = if self.width.is_finite() {
            self.width
                .clamp(crate::overlay::MIN_WIDTH, crate::overlay::MAX_WIDTH)
        } else {
            fallback.width
        };
        self.height = if self.height.is_finite() {
            self.height
                .clamp(crate::overlay::MIN_HEIGHT, crate::overlay::MAX_HEIGHT)
        } else {
            fallback.height
        };
        self.appearance = self.appearance.sanitised();
        // An empty string is not a device name anything will ever match; treat
        // it the same as never having chosen one, rather than carrying a value
        // that only ever falls back.
        if self.microphone_id.as_deref() == Some("") {
            self.microphone_id = None;
        }
        self
    }
}

fn file_path(root: &Path) -> PathBuf {
    root.join("settings.json")
}

/// Whether a settings file has ever been written.
///
/// A dedicated check rather than inferring "first run" from an empty script or
/// default values: those are also what a returning user sees if they clear
/// their script, and a heuristic that occasionally re-shows a welcome banner to
/// an existing user is worse than one extra file access.
pub fn exists(root: &Path) -> bool {
    file_path(root).is_file()
}

/// Reads settings, falling back to defaults for anything unreadable.
///
/// A corrupt or partial file is not an error worth surfacing: the app has a
/// working default for every field, and refusing to start over a bad
/// preferences file would be a far worse outcome than losing preferences.
pub fn load(root: &Path) -> Settings {
    fs::read_to_string(file_path(root))
        .ok()
        .and_then(|raw| serde_json::from_str::<Settings>(&raw).ok())
        .unwrap_or_default()
        .sanitised()
}

/// Writes settings, replacing the file atomically.
///
/// Writing in place risks leaving a half-written file if the process dies
/// mid-save — and that file is read at every launch.
pub fn save(root: &Path, settings: &Settings) -> Result<(), String> {
    fs::create_dir_all(root).map_err(|error| error.to_string())?;
    let serialised = serde_json::to_string_pretty(settings).map_err(|error| error.to_string())?;

    let destination = file_path(root);
    let temporary = destination.with_extension("json.tmp");
    fs::write(&temporary, serialised).map_err(|error| error.to_string())?;
    fs::rename(&temporary, &destination).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("textream-settings-{name}"));
        let _ = fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn missing_file_yields_defaults() {
        let settings = load(Path::new("C:/definitely/not/here"));
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn exists_reports_true_only_after_a_save() {
        let root = temp_root("exists");
        assert!(!exists(&root));
        save(&root, &Settings::default()).unwrap();
        assert!(exists(&root));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_round_trip_preserves_everything() {
        let root = temp_root("round-trip");
        let settings = Settings {
            mode: Mode::WordTracking,
            words_per_second: 5.5,
            script: "hello [beat] world".into(),
            appearance: Appearance {
                font_family: FontFamily::Dyslexic,
                highlight: ColorPreset::Green,
                ..Appearance::default()
            },
            model_id: Some("en-20m".into()),
            microphone_id: Some("Microphone (USB Audio Device)".into()),
            ..Settings::default()
        };

        save(&root, &settings).unwrap();
        assert_eq!(load(&root), settings);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_file_from_before_the_reading_settings_keeps_todays_behaviour() {
        let settings: Settings =
            serde_json::from_str(r#"{ "appearance": { "fontSize": "xl" } }"#).unwrap();
        assert!(!settings.keep_screen_awake);
        assert_eq!(settings.appearance.reading_position, ReadingPosition::UpperThird);
        assert!((settings.appearance.view().reading_anchor - 1.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn cues_keep_their_old_look_until_the_brightness_is_changed() {
        let view = Appearance::default().view();
        assert_eq!((view.cue_opacity, view.cue_active_opacity), (0.55, 0.75));
        let bright = Appearance {
            cue_brightness: CueBrightness::Bright,
            ..Appearance::default()
        };
        assert_eq!(bright.view().cue_active_opacity, 1.0);
    }

    #[test]
    fn centred_reading_parks_the_active_line_halfway_down() {
        let appearance = Appearance {
            reading_position: ReadingPosition::Centered,
            ..Appearance::default()
        };
        assert_eq!(appearance.view().reading_anchor, 0.5);
    }

    #[test]
    fn an_empty_microphone_id_sanitises_to_the_system_default() {
        let settings = Settings {
            microphone_id: Some(String::new()),
            ..Settings::default()
        }
        .sanitised();

        assert_eq!(settings.microphone_id, None);
    }

    #[test]
    fn a_corrupt_file_falls_back_instead_of_failing() {
        let root = temp_root("corrupt");
        fs::create_dir_all(&root).unwrap();
        fs::write(file_path(&root), "{ this is not json").unwrap();
        assert_eq!(load(&root), Settings::default());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn fields_an_older_build_never_wrote_take_defaults() {
        let root = temp_root("partial");
        fs::create_dir_all(&root).unwrap();
        fs::write(file_path(&root), r#"{"wordsPerSecond": 3.5}"#).unwrap();

        let settings = load(&root);
        assert_eq!(settings.words_per_second, 3.5);
        assert_eq!(settings.width, Settings::default().width);
        assert_eq!(settings.appearance, Appearance::default());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let settings = Settings {
            words_per_second: 9_000.0,
            width: -50.0,
            height: f64::NAN,
            appearance: Appearance {
                opacity: 4.0,
                ..Appearance::default()
            },
            ..Settings::default()
        }
        .sanitised();

        assert_eq!(settings.words_per_second, prompt_core::MAX_WORDS_PER_SECOND);
        assert_eq!(settings.width, crate::overlay::MIN_WIDTH);
        assert_eq!(settings.height, Settings::default().height);
        assert_eq!(settings.appearance.opacity, MAX_OPACITY);
    }

    #[test]
    fn saving_replaces_rather_than_appends() {
        let root = temp_root("replace");
        save(&root, &Settings::default()).unwrap();
        let settings = Settings {
            words_per_second: 7.0,
            ..Settings::default()
        };
        save(&root, &settings).unwrap();

        assert_eq!(load(&root).words_per_second, 7.0);
        assert!(!file_path(&root).with_extension("json.tmp").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn font_sizes_and_colours_map_to_render_values() {
        assert_eq!(FontSize::Xs.points(), 14.0);
        assert_eq!(FontSize::Xl.points(), 24.0);
        assert_eq!(ColorPreset::Yellow.css(), "#ffd60a");
        assert_eq!(ColorPreset::White.css(), "#ffffff");
    }
}
