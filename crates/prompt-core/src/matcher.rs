//! Aligns a live speech transcript against a known script.
//!
//! This is deliberately *not* a transcription-quality problem. The script is
//! already known, so the job is to find how far into it the speaker has got —
//! which a mediocre recogniser plus a forgiving matcher solves well. That is
//! what makes a small on-device model viable and keeps the "nothing leaves your
//! machine" promise intact.
//!
//! Two independent estimates run on every update:
//!
//! * **Character level** walks graphemes and tolerates insertions and deletions
//!   on either side. It is resilient to word-boundary errors ("a lot" vs
//!   "alot") but drifts on repeated letters.
//! * **Word level** compares whole words with a fuzzy predicate. It is stable
//!   across long stretches but stalls whenever a word is badly recognised.
//!
//! Their failure modes barely overlap, so reconciling them beats either alone.

use crate::alignment::{
    advance_past_annotations, best_offset, is_annotation_word, should_commit,
    DEFAULT_AGREEMENT_TOLERANCE,
};
use crate::script::PromptScript;
use crate::text::{graphemes, is_readable, split_into_words};

/// How far the matcher may look ahead on either side when it hits a mismatch.
const MAX_SKIP: usize = 5;

/// How many recent candidates are kept to confirm a position.
const RECENT_CAPACITY: usize = 3;

/// Two candidates this close together count as the same position.
const CONFIRMATION_RADIUS: usize = 10;

/// Tracks reading position within a [`PromptScript`] from speech transcripts.
#[derive(Debug, Clone)]
pub struct PromptMatcher {
    source: PromptScript,
    /// Confirmed highlight position, in graphemes. Monotonically increasing
    /// until an explicit jump or reset.
    recognized: usize,
    /// Where the current transcript window is measured from.
    match_start_offset: usize,
    recent: Vec<usize>,
    /// Lowercased graphemes of the source, cached for the character pass.
    lowered: Vec<String>,
}

impl PromptMatcher {
    /// Builds a matcher positioned at `offset`, nudged past any cue there.
    pub fn new(source: PromptScript, offset: usize) -> Self {
        let start = advance_past_annotations(
            source.graphemes(),
            source.annotation_ranges(),
            offset.min(source.character_count()),
        );
        // Lowercasing per grapheme rather than over the whole string keeps a
        // 1:1 index mapping. A whole-string lowercase can change the grapheme
        // count (Turkish `İ` expands to two scalars) and silently shift every
        // offset the matcher returns.
        let lowered = source
            .graphemes()
            .iter()
            .map(|g| g.to_lowercase())
            .collect();

        Self {
            source,
            recognized: start,
            match_start_offset: start,
            recent: Vec::with_capacity(RECENT_CAPACITY),
            lowered,
        }
    }

    pub fn source(&self) -> &PromptScript {
        &self.source
    }

    /// Current highlight position, in graphemes.
    pub fn recognized_character_count(&self) -> usize {
        self.recognized
    }

    pub fn match_start_offset(&self) -> usize {
        self.match_start_offset
    }

    /// Word progress for the scroller to animate towards.
    pub fn word_progress(&self) -> f64 {
        self.source
            .word_progress_for_character_offset(self.recognized)
    }

    /// Swaps in a new script, restarting at `offset`.
    pub fn reset(&mut self, source: PromptScript, offset: usize) {
        *self = Self::new(source, offset);
    }

    /// Re-anchors the transcript window to the current position.
    ///
    /// The recogniser restarts periodically (session limits, silence timeouts)
    /// and hands back a transcript that begins from nothing. Without this the
    /// next window would be measured from a stale origin and the highlight
    /// would snap backwards.
    pub fn restart_from_current_progress(&mut self) {
        self.match_start_offset = self.recognized;
        self.recent.clear();
    }

    /// Moves the highlight to `offset` — user tapped a word or scrolled.
    pub fn jump(&mut self, offset: usize) -> usize {
        let clamped = offset.min(self.source.character_count());
        let target = advance_past_annotations(
            self.source.graphemes(),
            self.source.annotation_ranges(),
            clamped,
        );
        self.recognized = target;
        self.match_start_offset = target;
        self.recent.clear();
        target
    }

    /// Folds a transcript window into the reading position.
    ///
    /// `transcript` is everything the recogniser has produced since the last
    /// restart, not just the newest word.
    pub fn match_transcript(&mut self, transcript: &str) -> usize {
        if transcript.is_empty() || self.match_start_offset >= self.source.character_count() {
            return self.recognized;
        }

        let character_result = self.character_level_match(transcript);
        let word_result = self.word_level_match(transcript);
        let best = best_offset(character_result, word_result, DEFAULT_AGREEMENT_TOLERANCE);

        let raw_candidate = (self.match_start_offset + best).min(self.source.character_count());
        let candidate = advance_past_annotations(
            self.source.graphemes(),
            self.source.annotation_ranges(),
            raw_candidate,
        );

        if candidate <= self.recognized {
            return self.recognized;
        }

        self.recent.push(candidate);
        if self.recent.len() > RECENT_CAPACITY {
            self.recent.remove(0);
        }
        let confirmed = self
            .recent
            .iter()
            .filter(|position| position.abs_diff(candidate) <= CONFIRMATION_RADIUS)
            .count()
            >= 2;

        if should_commit(
            character_result,
            word_result,
            self.recognized,
            raw_candidate,
            candidate,
            confirmed,
        ) {
            self.recognized = candidate;
        }
        self.recognized
    }

    /// Grapheme-level walk. Returns how far into the remaining source the
    /// transcript reaches.
    fn character_level_match(&self, spoken: &str) -> usize {
        let source = &self.lowered[self.match_start_offset..];
        let spoken: Vec<String> = graphemes(&spoken.to_lowercase())
            .into_iter()
            .filter(|g| is_readable(g) || g.chars().all(char::is_whitespace))
            .collect();

        let mut source_index = 0usize;
        let mut spoken_index = 0usize;
        let mut last_good = 0usize;

        while source_index < source.len() && spoken_index < spoken.len() {
            // Cues are never spoken; step over the whole bracket at once.
            if source[source_index] == "[" {
                if let Some(relative) = source[source_index..].iter().position(|g| g == "]") {
                    source_index += relative + 1;
                    last_good = source_index;
                    continue;
                }
            }
            if !is_readable(&source[source_index]) {
                source_index += 1;
                continue;
            }
            if !is_readable(&spoken[spoken_index]) {
                spoken_index += 1;
                continue;
            }
            if source[source_index] == spoken[spoken_index] {
                source_index += 1;
                spoken_index += 1;
                last_good = source_index;
                continue;
            }

            // A mismatch is either something the recogniser inserted or
            // something the speaker skipped. Probe both directions before
            // giving up on the character.
            let spoken_budget = MAX_SKIP.min(spoken.len() - spoken_index - 1);
            if let Some(skip) =
                (1..=spoken_budget).find(|&s| spoken[spoken_index + s] == source[source_index])
            {
                spoken_index += skip;
                continue;
            }

            let source_budget = MAX_SKIP.min(source.len() - source_index - 1);
            if let Some(skip) =
                (1..=source_budget).find(|&s| source[source_index + s] == spoken[spoken_index])
            {
                source_index += skip;
                continue;
            }

            spoken_index += 1;
        }

        // The transcript ran out. Any trailing cues and punctuation are
        // unspeakable, so credit them rather than parking the highlight in
        // front of a bracket the reader will never say.
        while source_index < source.len() {
            if source[source_index] == "[" {
                if let Some(relative) = source[source_index..].iter().position(|g| g == "]") {
                    source_index += relative + 1;
                    last_good = source_index;
                    continue;
                }
            }
            if !is_readable(&source[source_index]) {
                source_index += 1;
                last_good = source_index;
                continue;
            }
            break;
        }

        last_good
    }

    /// Word-level walk with fuzzy comparison.
    fn word_level_match(&self, spoken: &str) -> usize {
        let remaining = self
            .source
            .slice(self.match_start_offset..self.source.character_count());
        let source_words: Vec<&str> = remaining.split(' ').filter(|w| !w.is_empty()).collect();
        let spoken_words = split_into_words(&spoken.to_lowercase());

        let mut source_index = 0usize;
        let mut spoken_index = 0usize;
        let mut matched = 0usize;
        let mut inside_annotation = false;

        let alnum = |word: &str| -> String {
            word.chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect()
        };
        let width = |word: &str| graphemes(word).len();
        let opens_cue = |words: &[&str], index: usize| {
            words[index].starts_with('[') && words[index..].iter().any(|w| w.contains(']'))
        };

        while source_index < source_words.len() && spoken_index < spoken_words.len() {
            let begins = opens_cue(&source_words, source_index);
            if inside_annotation || begins || is_annotation_word(source_words[source_index]) {
                if begins {
                    inside_annotation = true;
                }
                if source_words[source_index].contains(']') {
                    inside_annotation = false;
                }
                matched += width(source_words[source_index]);
                if source_index < source_words.len() - 1 {
                    matched += 1;
                }
                source_index += 1;
                continue;
            }

            let source_word = alnum(source_words[source_index]);
            let spoken_word = alnum(&spoken_words[spoken_index]);

            if source_word == spoken_word || is_fuzzy_match(&source_word, &spoken_word) {
                matched += width(source_words[source_index]);
                source_index += 1;
                spoken_index += 1;
                if source_index < source_words.len() {
                    matched += 1;
                }
                continue;
            }

            // Filler word or misfire in the transcript: look ahead in speech.
            let spoken_budget = MAX_SKIP.min(spoken_words.len() - spoken_index - 1);
            if let Some(skip) = (1..=spoken_budget).find(|&s| {
                let next = alnum(&spoken_words[spoken_index + s]);
                source_word == next || is_fuzzy_match(&source_word, &next)
            }) {
                spoken_index += skip;
                continue;
            }

            // Speaker skipped ahead: look forward in the script and credit
            // everything jumped over.
            let source_budget = MAX_SKIP.min(source_words.len() - source_index - 1);
            if let Some(skip) = (1..=source_budget).find(|&s| {
                let next = alnum(source_words[source_index + s]);
                next == spoken_word || is_fuzzy_match(&next, &spoken_word)
            }) {
                for skipped in 0..skip {
                    matched += width(source_words[source_index + skipped]) + 1;
                }
                source_index += skip;
                continue;
            }

            if source_word.is_empty() {
                matched += width(source_words[source_index]);
                if source_index < source_words.len() - 1 {
                    matched += 1;
                }
                source_index += 1;
            } else {
                spoken_index += 1;
            }
        }

        while source_index < source_words.len() {
            let begins = opens_cue(&source_words, source_index);
            if !(inside_annotation || begins || is_annotation_word(source_words[source_index])) {
                break;
            }
            if begins {
                inside_annotation = true;
            }
            if source_words[source_index].contains(']') {
                inside_annotation = false;
            }
            matched += width(source_words[source_index]);
            if source_index < source_words.len() - 1 {
                matched += 1;
            }
            source_index += 1;
        }

        matched
    }
}

/// Whether two alphanumeric-only words are close enough to be the same word.
///
/// The tolerance widens with length because a one-character error in a long
/// word is almost certainly a recognition artefact, while in a short word it is
/// probably a different word entirely ("in" vs "it").
pub fn is_fuzzy_match(first: &str, second: &str) -> bool {
    if first.is_empty() || second.is_empty() {
        return false;
    }
    if first == second {
        return true;
    }

    let first_chars: Vec<char> = first.chars().collect();
    let second_chars: Vec<char> = second.chars().collect();
    let shorter = first_chars.len().min(second_chars.len());

    // A truncated word — the recogniser emitting "compl" mid-utterance.
    if shorter >= 3 && (first.starts_with(second) || second.starts_with(first)) {
        return true;
    }

    let shared_prefix = first_chars
        .iter()
        .zip(&second_chars)
        .take_while(|(a, b)| a == b)
        .count();
    if shorter >= 3 && shared_prefix >= (3).max(shorter * 3 / 5) {
        return true;
    }

    let distance = edit_distance(&first_chars, &second_chars);
    match shorter {
        0..=2 => false,
        3..=4 => distance <= 1,
        5..=8 => distance <= 2,
        _ => distance <= first_chars.len().max(second_chars.len()) / 3,
    }
}

/// Levenshtein distance over two character slices, in O(min(n, m)) space.
pub fn edit_distance(first: &[char], second: &[char]) -> usize {
    if first.is_empty() {
        return second.len();
    }
    if second.is_empty() {
        return first.len();
    }

    let mut row: Vec<usize> = (0..=second.len()).collect();
    for (i, a) in first.iter().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, b) in second.iter().enumerate() {
            let temporary = row[j + 1];
            row[j + 1] = if a == b {
                previous
            } else {
                previous.min(row[j + 1]).min(row[j]) + 1
            };
            previous = temporary;
        }
    }
    row[second.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matcher(script: &str) -> PromptMatcher {
        PromptMatcher::new(PromptScript::new(script), 0)
    }

    fn distance(a: &str, b: &str) -> usize {
        let a: Vec<char> = a.chars().collect();
        let b: Vec<char> = b.chars().collect();
        edit_distance(&a, &b)
    }

    #[test]
    fn edit_distance_basics() {
        assert_eq!(distance("", ""), 0);
        assert_eq!(distance("abc", ""), 3);
        assert_eq!(distance("kitten", "sitting"), 3);
        assert_eq!(distance("same", "same"), 0);
    }

    #[test]
    fn fuzzy_match_accepts_recognition_noise() {
        assert!(is_fuzzy_match("teleprompter", "teleprompter"));
        assert!(is_fuzzy_match("presentation", "presentaion"));
        assert!(is_fuzzy_match("compl", "complete")); // truncated mid-utterance
    }

    #[test]
    fn fuzzy_match_rejects_short_lookalikes() {
        assert!(!is_fuzzy_match("in", "it"));
        assert!(!is_fuzzy_match("a", "i"));
        assert!(!is_fuzzy_match("", "word"));
    }

    #[test]
    fn fuzzy_match_rejects_genuinely_different_words() {
        assert!(!is_fuzzy_match("microphone", "television"));
    }

    #[test]
    fn perfect_reading_reaches_the_end() {
        let mut m = matcher("hello brave new world");
        m.match_transcript("hello brave new world");
        assert_eq!(m.recognized_character_count(), m.source().character_count());
    }

    #[test]
    fn partial_reading_stops_partway() {
        let mut m = matcher("hello brave new world");
        m.match_transcript("hello brave");
        let offset = m.recognized_character_count();
        assert!(offset > 0);
        assert!(offset < m.source().character_count());
    }

    #[test]
    fn progress_is_monotonic_across_updates() {
        let mut m = matcher("the quick brown fox jumps over the lazy dog");
        let mut previous = 0;
        for transcript in [
            "the quick",
            "the quick brown",
            "the quick brown fox",
            "the quick brown fox jumps over",
        ] {
            let current = m.match_transcript(transcript);
            assert!(
                current >= previous,
                "went backwards: {previous} -> {current}"
            );
            previous = current;
        }
        assert!(previous > 0);
    }

    #[test]
    fn cues_are_skipped_without_being_spoken() {
        let mut m = matcher("welcome [smile at camera] to the show");
        m.match_transcript("welcome to the show");
        assert_eq!(m.recognized_character_count(), m.source().character_count());
    }

    #[test]
    fn a_leading_cue_is_cleared_before_any_speech() {
        let m = matcher("[wait for cue] begin now");
        assert_eq!(m.recognized_character_count(), 15);
        assert_eq!(m.source().slice(15..20), "begin");
    }

    #[test]
    fn filler_words_do_not_derail_tracking() {
        let mut m = matcher("today we ship the release");
        m.match_transcript("today um we uh ship the release");
        assert_eq!(m.recognized_character_count(), m.source().character_count());
    }

    #[test]
    fn misrecognised_words_are_tolerated() {
        let mut m = matcher("the teleprompter tracks your speech");
        m.match_transcript("the teleprompta tracks your speach");
        assert!(m.recognized_character_count() > 20);
    }

    #[test]
    fn jump_moves_the_position_and_clears_history() {
        let mut m = matcher("alpha beta gamma delta");
        m.match_transcript("alpha beta");
        m.jump(0);
        assert_eq!(m.recognized_character_count(), 0);
        assert_eq!(m.match_start_offset(), 0);
    }

    #[test]
    fn jump_lands_past_a_cue() {
        let mut m = matcher("alpha [beat] beta");
        assert_eq!(m.jump(6), 13);
    }

    #[test]
    fn jump_clamps_beyond_the_end() {
        let mut m = matcher("alpha beta");
        assert_eq!(m.jump(9_999), 10);
    }

    #[test]
    fn restart_rebases_the_window_without_losing_position() {
        let mut m = matcher("alpha beta gamma delta epsilon");
        m.match_transcript("alpha beta gamma");
        let before = m.recognized_character_count();
        m.restart_from_current_progress();
        assert_eq!(m.recognized_character_count(), before);
        assert_eq!(m.match_start_offset(), before);
        // A fresh window continues from there rather than restarting at zero.
        m.match_transcript("delta");
        assert!(m.recognized_character_count() >= before);
    }

    #[test]
    fn an_empty_transcript_changes_nothing() {
        let mut m = matcher("alpha beta");
        assert_eq!(m.match_transcript(""), 0);
    }

    #[test]
    fn unrelated_speech_does_not_rocket_to_the_end() {
        let mut m =
            matcher("the quarterly results exceeded every projection we published last spring");
        m.match_transcript("banana helicopter tuesday");
        assert!(m.recognized_character_count() < m.source().character_count() / 2);
    }

    #[test]
    fn an_empty_script_is_inert() {
        let mut m = matcher("");
        assert_eq!(m.match_transcript("anything at all"), 0);
    }

    #[test]
    fn cjk_scripts_track_per_character() {
        let mut m = matcher("你好世界");
        m.match_transcript("你好");
        assert!(m.recognized_character_count() > 0);
    }

    #[test]
    fn word_progress_follows_the_highlight() {
        let mut m = matcher("alpha beta gamma");
        assert_eq!(m.word_progress(), 0.0);
        m.match_transcript("alpha beta gamma");
        assert_eq!(m.word_progress(), 3.0);
    }
}
