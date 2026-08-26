//! Live prompting state, and the shape of the data the UI sees.
//!
//! [`prompt_core`] stays free of serialisation and of any notion of a UI. This
//! module owns the running session and translates between the engine and the
//! webview.

use std::sync::Mutex;

use prompt_core::{
    FollowMode, PaceScroller, PromptMatcher, PromptScript, TextDirection, VoiceActivityDetector,
};
use serde::{Deserialize, Serialize};

/// Guidance mode as the UI names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    #[default]
    WordTracking,
    Classic,
    VoiceActivated,
}

impl From<Mode> for FollowMode {
    fn from(mode: Mode) -> Self {
        match mode {
            Mode::WordTracking => FollowMode::WordTracking,
            Mode::Classic => FollowMode::Classic,
            Mode::VoiceActivated => FollowMode::VoiceActivated,
        }
    }
}

impl Mode {
    /// Whether starting this mode has to open the microphone.
    pub fn needs_microphone(self) -> bool {
        FollowMode::from(self).requires_microphone()
    }

    /// Whether this mode has to load a speech model.
    ///
    /// Voice-Activated listens but never transcribes, so it costs no model
    /// download and no ONNX runtime.
    pub fn needs_speech_recognition(self) -> bool {
        FollowMode::from(self).requires_speech_recognition()
    }
}

/// One word, as the overlay renders it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WordView {
    pub id: usize,
    pub text: String,
    /// Grapheme offset of the word's first character.
    pub start: usize,
    /// Grapheme offset one past the word's last character.
    pub end: usize,
    /// Cue or unspeakable — rendered dimmed and never waited for.
    pub is_annotation: bool,
}

/// A freshly loaded script.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptView {
    pub text: String,
    pub words: Vec<WordView>,
    pub character_count: usize,
    /// `"rtl"` or `"ltr"`, ready for the `dir` attribute.
    pub direction: &'static str,
}

/// Where the presenter is, after the latest update.
#[derive(Debug, Clone, Copy, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProgressView {
    /// Highlight position, in graphemes.
    pub character_offset: usize,
    /// Fractional word position, for smooth scrolling.
    pub word_progress: f64,
    /// Word the highlight currently sits on, if any.
    pub active_word: Option<usize>,
    /// Whether speech is currently detected — drives the waveform indicator.
    pub voice_active: bool,
    /// Latest microphone level, 0..1, for the waveform.
    pub level: f32,
    /// Armed but holding position.
    pub paused: bool,
    pub finished: bool,
}

/// The running prompter.
#[derive(Default)]
pub struct Session {
    script: Option<PromptScript>,
    matcher: Option<PromptMatcher>,
    scroller: Option<PaceScroller>,
    vad: VoiceActivityDetector,
    mode: Mode,
    words_per_second: f64,
    running: bool,
    paused: bool,
    voice_active: bool,
    level: f32,
}

impl Session {
    pub fn new() -> Self {
        Self {
            words_per_second: 2.0,
            ..Default::default()
        }
    }

    /// Parses `text` and arms both drivers.
    ///
    /// Word Tracking and the paced modes are swapped between at runtime, so
    /// both a matcher and a scroller are kept ready rather than rebuilt on every
    /// mode change — rebuilding would reset the reading position mid-take.
    pub fn load(&mut self, text: &str) -> ScriptView {
        let script = PromptScript::new(text);
        let view = ScriptView {
            text: script.text().to_string(),
            words: script
                .words()
                .iter()
                .map(|word| WordView {
                    id: word.id,
                    text: word.text.clone(),
                    start: word.range.start,
                    end: word.range.end,
                    is_annotation: word.is_annotation,
                })
                .collect(),
            character_count: script.character_count(),
            direction: match script.direction() {
                TextDirection::RightToLeft => "rtl",
                TextDirection::LeftToRight => "ltr",
            },
        };

        self.scroller = Some(PaceScroller::new(
            self.words_per_second,
            script.words().len(),
        ));
        self.matcher = Some(PromptMatcher::new(script.clone(), 0));
        self.script = Some(script);
        self.running = false;
        self.voice_active = false;
        self.vad.reset();
        view
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
        // A mode switch mid-session must not teleport the highlight, so the
        // incoming driver is seeded from the outgoing one's position.
        let offset = self.character_offset();
        self.sync_drivers_to(offset);
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn set_words_per_second(&mut self, words_per_second: f64) {
        self.words_per_second = words_per_second;
        if let Some(scroller) = self.scroller.as_mut() {
            scroller.set_words_per_second(words_per_second);
        }
    }

    pub fn start(&mut self) {
        self.running = true;
        self.paused = false;
        // The recogniser hands back a transcript starting from nothing, so
        // the window has to be rebased or the first update snaps backwards.
        self.rebase_transcript_window();
    }

    pub fn stop(&mut self) {
        self.running = false;
        self.paused = false;
        self.voice_active = false;
        self.vad.reset();
    }

    /// Holds position without releasing the microphone or the overlay.
    ///
    /// Resuming rebases the transcript window: the presenter almost certainly
    /// kept talking — that is usually *why* they paused — and a window still
    /// measured from before the break would match that speech against the
    /// script and jump the highlight.
    pub fn set_paused(&mut self, paused: bool) {
        let resuming = self.paused && !paused;
        self.paused = paused;
        if resuming {
            self.rebase_transcript_window();
        }
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Armed and actually advancing.
    fn advancing(&self) -> bool {
        self.running && !self.paused
    }

    /// Folds a transcript window in. Ignored outside Word Tracking.
    pub fn feed_transcript(&mut self, transcript: &str) -> ProgressView {
        if self.advancing() && self.mode == Mode::WordTracking {
            if let Some(matcher) = self.matcher.as_mut() {
                matcher.match_transcript(transcript);
                let offset = matcher.recognized_character_count();
                self.sync_scroller_to(offset);
            }
        }
        self.progress()
    }

    /// Feeds one metered audio frame to the speech gate.
    ///
    /// Called from the audio worker, not from the UI — capture lives in Rust so
    /// the recogniser and the meter can share one input device.
    pub fn feed_audio_level(&mut self, level: f32, timestamp: f64) -> ProgressView {
        self.level = level;
        self.vad.process(level, timestamp);
        self.voice_active = self.vad.is_active(timestamp);
        self.progress()
    }

    /// Re-anchors the transcript window without losing the reading position.
    ///
    /// Called when the recogniser reports an endpoint and its stream is reset:
    /// the next transcript starts from nothing, so a window still measured from
    /// the old origin would drag the highlight backwards.
    pub fn rebase_transcript_window(&mut self) {
        if let Some(matcher) = self.matcher.as_mut() {
            matcher.restart_from_current_progress();
        }
    }

    /// Advances the paced modes by `delta_seconds`.
    ///
    /// Word Tracking ignores the clock entirely — its position comes from
    /// speech, and letting a timer nudge it too would race the matcher.
    pub fn tick(&mut self, delta_seconds: f64) -> ProgressView {
        if self.advancing() && self.mode != Mode::WordTracking {
            let gate_open = self.mode == Mode::Classic || self.voice_active;
            if let Some(scroller) = self.scroller.as_mut() {
                scroller.advance(delta_seconds, gate_open);
            }
            let offset = self.character_offset();
            self.sync_matcher_to(offset);
        }
        self.progress()
    }

    /// Moves the highlight to a grapheme offset — tapped word or manual scroll.
    pub fn jump_to_offset(&mut self, offset: usize) -> ProgressView {
        self.sync_drivers_to(offset);
        self.progress()
    }

    /// Moves the highlight to the start of a word.
    pub fn jump_to_word(&mut self, index: usize) -> ProgressView {
        let offset = self
            .script
            .as_ref()
            .and_then(|script| script.words().get(index))
            .map(|word| word.range.start)
            .unwrap_or(0);
        self.jump_to_offset(offset)
    }

    fn character_offset(&self) -> usize {
        match self.mode {
            Mode::WordTracking => self
                .matcher
                .as_ref()
                .map(PromptMatcher::recognized_character_count)
                .unwrap_or(0),
            _ => match (self.script.as_ref(), self.scroller.as_ref()) {
                (Some(script), Some(scroller)) => {
                    script.character_offset_for_word_progress(scroller.progress())
                }
                _ => 0,
            },
        }
    }

    fn sync_drivers_to(&mut self, offset: usize) {
        self.sync_matcher_to(offset);
        self.sync_scroller_to(offset);
    }

    fn sync_matcher_to(&mut self, offset: usize) {
        if let Some(matcher) = self.matcher.as_mut() {
            matcher.jump(offset);
        }
    }

    fn sync_scroller_to(&mut self, offset: usize) {
        if let (Some(script), Some(scroller)) = (self.script.as_ref(), self.scroller.as_mut()) {
            scroller.seek(script.word_progress_for_character_offset(offset));
        }
    }

    pub fn progress(&self) -> ProgressView {
        let Some(script) = self.script.as_ref() else {
            return ProgressView::default();
        };
        let offset = self.character_offset();
        ProgressView {
            character_offset: offset,
            word_progress: script.word_progress_for_character_offset(offset),
            active_word: script.active_word_at(offset).map(|word| word.id),
            voice_active: self.voice_active,
            level: self.level,
            paused: self.paused,
            finished: offset >= script.character_count() && !script.is_empty(),
        }
    }
}

/// Tauri-managed handle to the one live session.
#[derive(Default)]
pub struct SessionState(pub Mutex<Session>);

impl SessionState {
    pub fn new() -> Self {
        Self(Mutex::new(Session::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loaded(text: &str) -> Session {
        let mut session = Session::new();
        session.load(text);
        session
    }

    #[test]
    fn loading_exposes_words_and_direction() {
        let mut session = Session::new();
        let view = session.load("hello [beat] world");
        assert_eq!(view.words.len(), 3);
        assert!(view.words[1].is_annotation);
        assert_eq!(view.direction, "ltr");
        assert_eq!(view.text, "hello [beat] world");
    }

    #[test]
    fn rtl_scripts_are_flagged_for_the_dir_attribute() {
        let mut session = Session::new();
        assert_eq!(session.load("שלום עולם").direction, "rtl");
    }

    #[test]
    fn a_stopped_session_ignores_speech_and_the_clock() {
        let mut session = loaded("alpha beta gamma");
        assert_eq!(session.feed_transcript("alpha beta").character_offset, 0);
        session.set_mode(Mode::Classic);
        assert_eq!(session.tick(10.0).character_offset, 0);
    }

    #[test]
    fn word_tracking_follows_the_transcript() {
        let mut session = loaded("alpha beta gamma");
        session.start();
        let progress = session.feed_transcript("alpha beta gamma");
        assert!(progress.finished);
        assert_eq!(progress.active_word, Some(2));
    }

    #[test]
    fn classic_advances_on_the_clock_alone() {
        let mut session = loaded("one two three four five six");
        session.set_mode(Mode::Classic);
        session.set_words_per_second(2.0);
        session.start();
        assert!(session.tick(1.0).word_progress >= 2.0);
    }

    #[test]
    fn voice_activated_holds_while_silent() {
        let mut session = loaded("one two three four five six");
        session.set_mode(Mode::VoiceActivated);
        session.set_words_per_second(2.0);
        session.start();

        session.feed_audio_level(0.0, 0.0);
        assert_eq!(session.tick(1.0).word_progress, 0.0);

        session.feed_audio_level(0.9, 1.0);
        assert!(session.tick(1.0).word_progress > 0.0);
    }

    #[test]
    fn metering_then_ticking_opens_the_gate() {
        let mut session = loaded("one two three four five six");
        session.set_mode(Mode::VoiceActivated);
        session.set_words_per_second(2.0);
        session.start();

        // The audio worker meters continuously; the frame tick reads the gate.
        session.feed_audio_level(0.0, 0.0);
        assert_eq!(session.tick(1.0).word_progress, 0.0);
        session.feed_audio_level(0.9, 1.0);
        let progress = session.tick(1.0);
        assert!(progress.voice_active);
        assert!(progress.word_progress > 0.0);
        assert!(
            progress.level > 0.0,
            "level should surface for the waveform"
        );
    }

    #[test]
    fn word_tracking_ignores_the_clock() {
        let mut session = loaded("one two three four five six");
        session.start();
        assert_eq!(session.tick(10.0).character_offset, 0);
    }

    #[test]
    fn switching_mode_preserves_the_reading_position() {
        let mut session = loaded("one two three four five six seven eight");
        session.start();
        session.feed_transcript("one two three");
        let before = session.progress().character_offset;
        assert!(before > 0);

        session.set_mode(Mode::Classic);
        assert_eq!(session.progress().character_offset, before);
    }

    #[test]
    fn jumping_moves_both_drivers() {
        let mut session = loaded("alpha beta gamma delta");
        session.set_mode(Mode::Classic);
        session.start();
        session.tick(5.0);

        let progress = session.jump_to_word(1);
        assert_eq!(progress.active_word, Some(1));
        assert_eq!(progress.character_offset, 6);

        // The matcher agrees, so switching back does not jump.
        session.set_mode(Mode::WordTracking);
        assert_eq!(session.progress().character_offset, 6);
    }

    #[test]
    fn pausing_holds_position_and_resuming_continues() {
        let mut session = loaded("one two three four five six seven eight");
        session.set_mode(Mode::Classic);
        session.set_words_per_second(2.0);
        session.start();
        session.tick(1.0);
        let held = session.progress().word_progress;

        session.set_paused(true);
        let paused = session.tick(5.0);
        assert!(paused.paused);
        assert_eq!(paused.word_progress, held);

        session.set_paused(false);
        assert!(session.tick(1.0).word_progress > held);
    }

    #[test]
    fn a_pause_stops_word_tracking_too() {
        let mut session = loaded("alpha beta gamma delta");
        session.start();
        session.set_paused(true);
        assert_eq!(session.feed_transcript("alpha beta").character_offset, 0);
    }

    #[test]
    fn resuming_rebases_so_speech_during_the_pause_is_not_replayed() {
        let mut session = loaded("alpha beta gamma delta epsilon");
        session.start();
        session.feed_transcript("alpha beta");
        let before = session.progress().character_offset;

        session.set_paused(true);
        session.set_paused(false);
        // A fresh window from the recogniser starts at the current position
        // rather than being measured from the origin.
        assert_eq!(session.progress().character_offset, before);
        assert!(session.feed_transcript("gamma").character_offset >= before);
    }

    #[test]
    fn stopping_clears_a_pause() {
        let mut session = loaded("alpha beta");
        session.start();
        session.set_paused(true);
        session.stop();
        assert!(!session.progress().paused);
    }

    #[test]
    fn jumping_past_the_end_is_clamped() {
        let mut session = loaded("alpha beta");
        assert_eq!(session.jump_to_word(99).character_offset, 0);
    }

    #[test]
    fn an_unloaded_session_reports_nothing() {
        let session = Session::new();
        let progress = session.progress();
        assert_eq!(progress.character_offset, 0);
        assert!(!progress.finished);
        assert_eq!(progress.active_word, None);
    }

    #[test]
    fn an_empty_script_never_reports_finished() {
        let mut session = loaded("   ");
        session.set_mode(Mode::Classic);
        session.start();
        assert!(!session.tick(100.0).finished);
    }

    #[test]
    fn speed_changes_apply_to_a_live_scroller() {
        let mut session = loaded("one two three four five six seven eight");
        session.set_mode(Mode::Classic);
        session.start();
        session.set_words_per_second(4.0);
        assert!(session.tick(1.0).word_progress >= 4.0);
    }

    #[test]
    fn voice_activity_surfaces_for_the_waveform() {
        let mut session = loaded("alpha beta");
        session.set_mode(Mode::VoiceActivated);
        session.start();
        assert!(session.feed_audio_level(0.9, 0.0).voice_active);
        assert!(!session.feed_audio_level(0.0, 10.0).voice_active);
    }
}
