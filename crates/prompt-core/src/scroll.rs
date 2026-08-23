//! Constant-pace scrolling for the two modes that do not track words.
//!
//! Classic and Voice-Activated are the same clock with a different gate:
//! Classic runs it permanently open, Voice-Activated hands it the
//! [`VoiceActivityDetector`](crate::vad::VoiceActivityDetector) verdict so the
//! script advances while the presenter speaks and holds while they pause.

/// Slowest pace offered in the UI, in words per second.
pub const MIN_WORDS_PER_SECOND: f64 = 0.5;
/// Fastest pace offered in the UI, in words per second.
pub const MAX_WORDS_PER_SECOND: f64 = 8.0;

/// Advances word progress at a fixed rate while its gate is open.
#[derive(Debug, Clone)]
pub struct PaceScroller {
    words_per_second: f64,
    progress: f64,
    total_words: f64,
}

impl PaceScroller {
    pub fn new(words_per_second: f64, total_words: usize) -> Self {
        Self {
            words_per_second: words_per_second.clamp(MIN_WORDS_PER_SECOND, MAX_WORDS_PER_SECOND),
            progress: 0.0,
            total_words: total_words as f64,
        }
    }

    /// Advances by `delta_seconds` when `gate_open`, and returns word progress.
    ///
    /// A closed gate holds position rather than decaying, so the presenter
    /// resumes exactly where they stopped. Negative or non-finite deltas are
    /// ignored — a clock that steps backwards over a suspend/resume would drag
    /// the highlight up the page.
    pub fn advance(&mut self, delta_seconds: f64, gate_open: bool) -> f64 {
        if gate_open && delta_seconds.is_finite() && delta_seconds > 0.0 {
            self.progress =
                (self.progress + delta_seconds * self.words_per_second).min(self.total_words);
        }
        self.progress
    }

    pub fn progress(&self) -> f64 {
        self.progress
    }

    /// Jumps to a position — user scrolled or tapped a word.
    pub fn seek(&mut self, progress: f64) {
        self.progress = progress.clamp(0.0, self.total_words);
    }

    pub fn set_words_per_second(&mut self, words_per_second: f64) {
        self.words_per_second = words_per_second.clamp(MIN_WORDS_PER_SECOND, MAX_WORDS_PER_SECOND);
    }

    pub fn words_per_second(&self) -> f64 {
        self.words_per_second
    }

    /// True once the end of the script is reached.
    pub fn is_finished(&self) -> bool {
        self.progress >= self.total_words
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advances_at_the_configured_pace() {
        let mut scroller = PaceScroller::new(2.0, 100);
        assert_eq!(scroller.advance(1.0, true), 2.0);
        assert_eq!(scroller.advance(0.5, true), 3.0);
    }

    #[test]
    fn a_closed_gate_holds_position() {
        let mut scroller = PaceScroller::new(2.0, 100);
        scroller.advance(1.0, true);
        assert_eq!(scroller.advance(5.0, false), 2.0);
        assert_eq!(scroller.advance(1.0, true), 4.0);
    }

    #[test]
    fn speed_is_clamped_to_the_supported_range() {
        let mut scroller = PaceScroller::new(999.0, 100);
        assert_eq!(scroller.words_per_second(), MAX_WORDS_PER_SECOND);
        scroller.set_words_per_second(0.0);
        assert_eq!(scroller.words_per_second(), MIN_WORDS_PER_SECOND);
    }

    #[test]
    fn progress_stops_at_the_end_of_the_script() {
        let mut scroller = PaceScroller::new(8.0, 10);
        scroller.advance(100.0, true);
        assert_eq!(scroller.progress(), 10.0);
        assert!(scroller.is_finished());
    }

    #[test]
    fn a_backwards_or_broken_clock_is_ignored() {
        let mut scroller = PaceScroller::new(2.0, 100);
        scroller.advance(1.0, true);
        assert_eq!(scroller.advance(-5.0, true), 2.0);
        assert_eq!(scroller.advance(f64::NAN, true), 2.0);
    }

    #[test]
    fn seek_clamps_within_the_script() {
        let mut scroller = PaceScroller::new(2.0, 10);
        scroller.seek(500.0);
        assert_eq!(scroller.progress(), 10.0);
        scroller.seek(-5.0);
        assert_eq!(scroller.progress(), 0.0);
    }
}
