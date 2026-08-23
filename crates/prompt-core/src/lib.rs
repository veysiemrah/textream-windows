//! Platform-neutral teleprompter engine.
//!
//! Everything here is pure computation: no windowing, no audio device, no
//! speech backend, no OS calls. The Windows shell, a future Linux build and a
//! WASM preview can all drive the same engine, and it can be tested without a
//! microphone.
//!
//! # The three guidance modes
//!
//! | Mode | Driven by |
//! |---|---|
//! | Word Tracking | [`PromptMatcher`] fed with transcript windows |
//! | Classic | [`PaceScroller`] with the gate held open |
//! | Voice-Activated | [`PaceScroller`] gated by [`VoiceActivityDetector`] |
//!
//! # Offsets
//!
//! Positions are **grapheme cluster** indices into [`PromptScript::text`],
//! never bytes and never `char`s. [`PromptScript::byte_range`] converts to byte
//! ranges when the UI needs to slice the string.
//!
//! # Example
//!
//! ```
//! use prompt_core::{PromptMatcher, PromptScript};
//!
//! let script = PromptScript::new("Welcome back. [smile] Today we ship.");
//! let mut matcher = PromptMatcher::new(script, 0);
//!
//! matcher.match_transcript("welcome back");
//! assert!(matcher.recognized_character_count() > 0);
//!
//! // The cue is never spoken, yet tracking runs straight through it.
//! matcher.match_transcript("welcome back today we ship");
//! assert_eq!(
//!     matcher.recognized_character_count(),
//!     matcher.source().character_count()
//! );
//! ```

pub mod alignment;
pub mod matcher;
pub mod script;
pub mod scroll;
pub mod text;
pub mod vad;

pub use matcher::{edit_distance, is_fuzzy_match, PromptMatcher};
pub use script::{PromptScript, PromptWord};
pub use scroll::{PaceScroller, MAX_WORDS_PER_SECOND, MIN_WORDS_PER_SECOND};
pub use text::{split_into_words, TextDirection};
pub use vad::{normalized_rms, normalized_rms_i16, VoiceActivityDetector};

/// How the prompter follows the presenter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FollowMode {
    /// Speech recognition highlights each word as it is spoken.
    #[default]
    WordTracking,
    /// Constant-speed scroll. No microphone.
    Classic,
    /// Constant-speed scroll that pauses in silence.
    VoiceActivated,
}

impl FollowMode {
    /// Whether the mode needs microphone access at all.
    pub fn requires_microphone(self) -> bool {
        !matches!(self, FollowMode::Classic)
    }

    /// Whether the mode needs a speech recogniser, as opposed to only level
    /// metering. Voice-Activated listens but never transcribes.
    pub fn requires_speech_recognition(self) -> bool {
        matches!(self, FollowMode::WordTracking)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_classic_runs_without_a_microphone() {
        assert!(!FollowMode::Classic.requires_microphone());
        assert!(FollowMode::WordTracking.requires_microphone());
        assert!(FollowMode::VoiceActivated.requires_microphone());
    }

    #[test]
    fn only_word_tracking_needs_transcription() {
        assert!(FollowMode::WordTracking.requires_speech_recognition());
        assert!(!FollowMode::VoiceActivated.requires_speech_recognition());
        assert!(!FollowMode::Classic.requires_speech_recognition());
    }

    #[test]
    fn voice_activated_pipeline_holds_and_resumes() {
        let script = PromptScript::new("one two three four five six seven eight");
        let mut scroller = PaceScroller::new(2.0, script.words().len());
        let mut vad = VoiceActivityDetector::default();

        vad.process(normalized_rms(&[0.6; 64]), 0.0);
        let speaking = scroller.advance(1.0, vad.is_active(0.0));
        assert_eq!(speaking, 2.0);

        // Silence, well past the hangover.
        vad.process(normalized_rms(&[0.0; 64]), 5.0);
        assert_eq!(scroller.advance(1.0, vad.is_active(5.0)), speaking);
    }
}
