//! The immutable, analysed form of a script.
//!
//! A [`PromptScript`] is built once when the user hits play and then only read.
//! It owns the whitespace-normalised text, the word table, the cue ranges, and
//! the conversions between the two coordinate systems the app juggles: word
//! progress (what the scroller animates) and grapheme offsets (what the matcher
//! and the highlight renderer speak).

use std::ops::Range;

use crate::alignment::{annotation_flags, annotation_ranges};
use crate::text::{graphemes, infer_direction, is_readable, split_into_words, TextDirection};

/// One trackable word, positioned in the normalised text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptWord {
    pub id: usize,
    pub text: String,
    /// Grapheme range within [`PromptScript::text`].
    pub range: Range<usize>,
    /// Part of a cue, or otherwise unspeakable — the tracker slides over it.
    pub is_annotation: bool,
}

/// A parsed script ready for tracking and rendering.
#[derive(Debug, Clone)]
pub struct PromptScript {
    text: String,
    graphemes: Vec<String>,
    /// Byte offset of each grapheme, plus a terminator, for slicing `text`.
    byte_offsets: Vec<usize>,
    words: Vec<PromptWord>,
    annotation_ranges: Vec<Range<usize>>,
    annotation_style_ranges: Vec<Range<usize>>,
    direction: TextDirection,
}

impl PromptScript {
    /// Parses raw user input into a trackable script.
    ///
    /// The text is rebuilt from its own word table, so every run of whitespace
    /// becomes exactly one space. That single-space invariant is what lets word
    /// ranges be computed by simple accumulation rather than by searching the
    /// original string.
    pub fn new(raw: &str) -> Self {
        let raw_words = split_into_words(raw);
        let collapsed = raw_words.join(" ");
        let flags = annotation_flags(&raw_words);

        let mut offset = 0usize;
        let mut words = Vec::with_capacity(raw_words.len());
        for (index, word) in raw_words.iter().enumerate() {
            let length = crate::text::graphemes(word).len();
            let range = offset..offset + length;
            words.push(PromptWord {
                id: index,
                text: word.clone(),
                range: range.clone(),
                is_annotation: flags[index] || !is_readable(word),
            });
            offset = range.end + 1; // the joining space
        }

        let graphemes = graphemes(&collapsed);
        let mut byte_offsets = Vec::with_capacity(graphemes.len() + 1);
        let mut running = 0usize;
        for grapheme in &graphemes {
            byte_offsets.push(running);
            running += grapheme.len();
        }
        byte_offsets.push(running);

        let annotation_ranges = annotation_ranges(&graphemes);
        let annotation_style_ranges =
            Self::compute_annotation_style_ranges(&graphemes, &words, &annotation_ranges);
        let direction = infer_direction(&collapsed);

        Self {
            text: collapsed,
            graphemes,
            byte_offsets,
            words,
            annotation_ranges,
            annotation_style_ranges,
            direction,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn graphemes(&self) -> &[String] {
        &self.graphemes
    }

    pub fn words(&self) -> &[PromptWord] {
        &self.words
    }

    pub fn annotation_ranges(&self) -> &[Range<usize>] {
        &self.annotation_ranges
    }

    /// Spans the renderer should paint in the cue colour.
    pub fn annotation_style_ranges(&self) -> &[Range<usize>] {
        &self.annotation_style_ranges
    }

    pub fn direction(&self) -> TextDirection {
        self.direction
    }

    /// Length in graphemes — the unit of every offset in this crate.
    pub fn character_count(&self) -> usize {
        self.graphemes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// Converts a grapheme range into a byte range, for slicing [`text`].
    ///
    /// [`text`]: PromptScript::text
    pub fn byte_range(&self, range: Range<usize>) -> Range<usize> {
        let last = self.byte_offsets.len() - 1;
        let start = self.byte_offsets[range.start.min(last)];
        let end = self.byte_offsets[range.end.min(last)];
        start..end.max(start)
    }

    /// Slices the normalised text by grapheme range.
    pub fn slice(&self, range: Range<usize>) -> &str {
        &self.text[self.byte_range(range)]
    }

    /// Maps fractional word progress onto a grapheme offset.
    ///
    /// The fraction is spent inside the current word so a constant-speed scroll
    /// glides through long words instead of stepping between them.
    pub fn character_offset_for_word_progress(&self, progress: f64) -> usize {
        if self.words.is_empty() {
            return 0;
        }
        let clamped = progress.clamp(0.0, self.words.len() as f64);
        let whole = (clamped as usize).min(self.words.len());
        let Some(word) = self.words.get(whole) else {
            return self.character_count();
        };
        let fraction = clamped - whole as f64;
        let length = crate::text::graphemes(&word.text).len() as f64;
        let within = (length * fraction) as usize;
        (word.range.start + within).min(self.character_count())
    }

    /// Inverse of [`character_offset_for_word_progress`].
    ///
    /// [`character_offset_for_word_progress`]: PromptScript::character_offset_for_word_progress
    pub fn word_progress_for_character_offset(&self, offset: usize) -> f64 {
        let clamped = offset.min(self.character_count());
        for word in &self.words {
            if clamped <= word.range.end {
                let position = clamped.saturating_sub(word.range.start);
                let length = crate::text::graphemes(&word.text).len().max(1);
                return word.id as f64 + position as f64 / length as f64;
            }
        }
        self.words.len() as f64
    }

    /// The word the highlight currently sits on.
    pub fn active_word_at(&self, offset: usize) -> Option<&PromptWord> {
        self.words
            .iter()
            .find(|word| offset <= word.range.end)
            .or_else(|| self.words.last())
    }

    /// Cue-coloured spans: whole balanced cues, plus any standalone emoji or
    /// punctuation word.
    ///
    /// Balanced cues are kept intact so their brackets and inner spaces share
    /// one colour. Each word is then clipped against those cues before its
    /// leftovers are considered, which stops `[pause]continue` from tinting the
    /// prose welded onto the cue.
    fn compute_annotation_style_ranges(
        graphemes: &[String],
        words: &[PromptWord],
        balanced: &[Range<usize>],
    ) -> Vec<Range<usize>> {
        let mut result = balanced.to_vec();

        for word in words {
            let mut segments = vec![word.range.clone()];
            for cue in balanced {
                segments = segments
                    .into_iter()
                    .flat_map(|segment| {
                        if segment.start >= cue.end || cue.start >= segment.end {
                            return vec![segment];
                        }
                        let mut pieces = Vec::new();
                        if segment.start < cue.start {
                            pieces.push(segment.start..cue.start);
                        }
                        if cue.end < segment.end {
                            pieces.push(cue.end..segment.end);
                        }
                        pieces
                    })
                    .collect();
            }

            for segment in segments {
                if segment.is_empty() {
                    continue;
                }
                let readable = graphemes[segment.clone()].iter().any(|g| is_readable(g));
                if !readable {
                    result.push(segment);
                }
            }
        }

        result.sort_by_key(|range| range.start);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapses_whitespace_and_indexes_words() {
        let script = PromptScript::new("hello   brave\nnew  world");
        assert_eq!(script.text(), "hello brave new world");
        assert_eq!(script.words().len(), 4);
        assert_eq!(script.words()[0].range, 0..5);
        assert_eq!(script.words()[1].range, 6..11);
        assert_eq!(script.words()[3].range, 16..21);
    }

    #[test]
    fn word_ranges_slice_back_to_their_own_text() {
        let script = PromptScript::new("alpha beta gamma");
        for word in script.words() {
            assert_eq!(script.slice(word.range.clone()), word.text);
        }
    }

    #[test]
    fn cue_words_are_flagged() {
        let script = PromptScript::new("read [look up] now");
        let flags: Vec<bool> = script.words().iter().map(|w| w.is_annotation).collect();
        assert_eq!(flags, [false, true, true, false]);
    }

    #[test]
    fn standalone_emoji_is_an_annotation() {
        let script = PromptScript::new("hello 👍 world");
        assert!(script.words()[1].is_annotation);
        assert!(!script.words()[0].is_annotation);
    }

    #[test]
    fn progress_round_trips_at_word_boundaries() {
        let script = PromptScript::new("alpha beta gamma");
        for (index, word) in script.words().iter().enumerate() {
            let offset = script.character_offset_for_word_progress(index as f64);
            assert_eq!(offset, word.range.start);
            assert_eq!(
                script.word_progress_for_character_offset(offset),
                index as f64
            );
        }
    }

    #[test]
    fn fractional_progress_lands_inside_the_word() {
        let script = PromptScript::new("alpha beta");
        // Half way through "alpha" (5 graphemes) is offset 2.
        assert_eq!(script.character_offset_for_word_progress(0.5), 2);
    }

    #[test]
    fn progress_past_the_end_clamps_to_the_tail() {
        let script = PromptScript::new("alpha beta");
        assert_eq!(
            script.character_offset_for_word_progress(99.0),
            script.character_count()
        );
    }

    #[test]
    fn active_word_tracks_the_offset() {
        let script = PromptScript::new("alpha beta gamma");
        assert_eq!(script.active_word_at(0).unwrap().text, "alpha");
        assert_eq!(script.active_word_at(7).unwrap().text, "beta");
        assert_eq!(script.active_word_at(999).unwrap().text, "gamma");
    }

    #[test]
    fn cue_styling_covers_the_whole_bracket() {
        let script = PromptScript::new("hi [take a beat] there");
        assert!(script.annotation_style_ranges().contains(&(3..16)));
        assert_eq!(script.slice(3..16), "[take a beat]");
    }

    #[test]
    fn prose_welded_to_a_cue_is_not_tinted() {
        let script = PromptScript::new("[pause]continue");
        let styled = script.annotation_style_ranges();
        assert!(styled.contains(&(0..7)));
        assert!(styled.iter().all(|r| r.start >= 7 || r.end <= 7));
    }

    #[test]
    fn byte_ranges_survive_multibyte_text() {
        let script = PromptScript::new("añé 👍🏽 son");
        for word in script.words() {
            assert_eq!(script.slice(word.range.clone()), word.text);
        }
    }

    #[test]
    fn empty_input_is_handled() {
        let script = PromptScript::new("   \n ");
        assert!(script.is_empty());
        assert_eq!(script.character_count(), 0);
        assert_eq!(script.character_offset_for_word_progress(3.0), 0);
        assert!(script.active_word_at(0).is_none());
    }

    #[test]
    fn direction_is_carried_through() {
        assert_eq!(
            PromptScript::new("שלום עולם").direction(),
            TextDirection::RightToLeft
        );
    }
}
