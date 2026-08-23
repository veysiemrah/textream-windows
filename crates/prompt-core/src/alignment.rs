//! Cue handling and the commit policy that decides when tracking may advance.
//!
//! A *cue* is a bracketed stage direction — `[pause]`, `[look at camera]` —
//! that appears in the script but is never spoken. Everything here exists so
//! the tracker can slide over cues without waiting for words that will never
//! arrive, and so a bad recognition cannot rocket the highlight down the page.

use std::ops::Range;

use crate::text::is_readable;

/// Marks, per word, whether it belongs to a cue.
///
/// A `[` only opens a cue if some later word actually closes it, so an unmatched
/// bracket in prose degrades to ordinary spoken text instead of swallowing the
/// rest of the script.
pub fn annotation_flags(words: &[String]) -> Vec<bool> {
    let mut closing_at_or_after = vec![false; words.len()];
    let mut has_closing = false;
    for index in (0..words.len()).rev() {
        if words[index].contains(']') {
            has_closing = true;
        }
        closing_at_or_after[index] = has_closing;
    }

    let mut flags = Vec::with_capacity(words.len());
    let mut inside = false;
    for (index, word) in words.iter().enumerate() {
        let begins = word.starts_with('[') && closing_at_or_after[index];
        flags.push(inside || begins);
        if begins {
            inside = true;
        }
        if inside && word.contains(']') {
            inside = false;
        }
    }
    flags
}

/// Grapheme ranges of every balanced `[...]` cue, brackets included.
pub fn annotation_ranges(graphemes: &[String]) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut opening: Option<usize> = None;
    for (index, grapheme) in graphemes.iter().enumerate() {
        if grapheme == "[" && opening.is_none() {
            opening = Some(index);
        } else if grapheme == "]" {
            if let Some(start) = opening.take() {
                ranges.push(start..index + 1);
            }
        }
    }
    ranges
}

/// Moves `offset` forward over any cue it sits inside, plus any cue that begins
/// after the following run of whitespace.
///
/// The whitespace hop is what lets a run of back-to-back cues
/// (`[beat] [smile] next line`) clear in one step. Whitespace is only consumed
/// when a cue was actually skipped, otherwise the caller's position would creep
/// forward on every idle frame.
pub fn advance_past_annotations(
    graphemes: &[String],
    ranges: &[Range<usize>],
    offset: usize,
) -> usize {
    let mut current = offset.min(graphemes.len());
    let mut skipped = false;

    while current < graphemes.len() {
        if let Some(range) = ranges.iter().find(|r| r.contains(&current)) {
            current = range.end;
            skipped = true;
            continue;
        }

        let mut next = current;
        while next < graphemes.len() && graphemes[next].chars().all(char::is_whitespace) {
            next += 1;
        }

        if let Some(range) = ranges.iter().find(|r| r.start == next) {
            current = range.end;
            skipped = true;
            continue;
        }

        return if skipped { next } else { current };
    }
    current
}

/// Reconciles the two independent match estimates.
///
/// Character-level and word-level matching fail in different ways: the former
/// drifts on repeated letters, the latter stalls on a mis-recognised word. When
/// they agree, averaging cancels noise; when they diverge, the larger estimate
/// wins because a stalled highlight is more visible to a presenter than one
/// that is slightly ahead.
pub fn best_offset(
    character_result: usize,
    word_result: usize,
    agreement_tolerance: usize,
) -> usize {
    if character_result.abs_diff(word_result) <= agreement_tolerance {
        (character_result + word_result) / 2
    } else {
        character_result.max(word_result)
    }
}

/// Default tolerance, in graphemes, within which the two estimates are
/// considered to agree.
pub const DEFAULT_AGREEMENT_TOLERANCE: usize = 20;

/// Whether a candidate position is trustworthy enough to become the new
/// highlight.
///
/// Any single condition is enough. The point is not to be strict — it is to
/// reject the specific failure where one estimate is zero, nothing was skipped,
/// no repeat confirmed it, and the jump is large. That combination is what a
/// spurious recognition looks like, and committing it throws the reader off the
/// page.
pub fn should_commit(
    character_result: usize,
    word_result: usize,
    current: usize,
    raw_candidate: usize,
    candidate: usize,
    confirmed: bool,
) -> bool {
    let both_progressed = character_result.min(word_result) > 0;
    let skipped_annotation = candidate > raw_candidate;
    let small_step = candidate.saturating_sub(current) <= 15;
    both_progressed || skipped_annotation || confirmed || small_step
}

/// True when a word is a self-contained cue or carries nothing readable.
pub fn is_annotation_word(word: &str) -> bool {
    (word.starts_with('[') && word.ends_with(']')) || !is_readable(word)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{graphemes, split_into_words};

    fn words(text: &str) -> Vec<String> {
        split_into_words(text)
    }

    #[test]
    fn flags_span_a_multi_word_cue() {
        assert_eq!(
            annotation_flags(&words("read [look at camera] now")),
            [false, true, true, true, false]
        );
    }

    #[test]
    fn unmatched_bracket_is_not_a_cue() {
        assert_eq!(
            annotation_flags(&words("read [unclosed and on we go")),
            [false, false, false, false, false, false]
        );
    }

    #[test]
    fn ranges_cover_brackets_inclusively() {
        let g = graphemes("ab [cue] cd");
        assert_eq!(annotation_ranges(&g), vec![3..8]);
    }

    #[test]
    fn ranges_find_every_cue() {
        let g = graphemes("[a] mid [b]");
        assert_eq!(annotation_ranges(&g), [0..3, 8..11]);
    }

    #[test]
    fn advance_steps_over_a_cue_at_the_cursor() {
        let g = graphemes("[cue] word");
        let r = annotation_ranges(&g);
        assert_eq!(advance_past_annotations(&g, &r, 0), 6);
    }

    #[test]
    fn advance_clears_consecutive_cues_in_one_call() {
        let g = graphemes("[a] [b] word");
        let r = annotation_ranges(&g);
        assert_eq!(advance_past_annotations(&g, &r, 0), 8);
    }

    #[test]
    fn advance_is_a_no_op_in_plain_prose() {
        let g = graphemes("just words here");
        let r = annotation_ranges(&g);
        assert_eq!(advance_past_annotations(&g, &r, 4), 4);
    }

    #[test]
    fn advance_saturates_at_the_end() {
        let g = graphemes("word");
        let r = annotation_ranges(&g);
        assert_eq!(advance_past_annotations(&g, &r, 99), 4);
    }

    #[test]
    fn close_estimates_average() {
        assert_eq!(best_offset(100, 110, DEFAULT_AGREEMENT_TOLERANCE), 105);
    }

    #[test]
    fn diverging_estimates_take_the_optimistic_one() {
        assert_eq!(best_offset(10, 200, DEFAULT_AGREEMENT_TOLERANCE), 200);
    }

    #[test]
    fn a_large_unconfirmed_jump_with_a_dead_estimate_is_rejected() {
        assert!(!should_commit(0, 400, 0, 400, 400, false));
    }

    #[test]
    fn any_single_signal_permits_a_commit() {
        assert!(should_commit(5, 5, 0, 400, 400, false)); // both progressed
        assert!(should_commit(0, 400, 0, 390, 400, false)); // cue skipped
        assert!(should_commit(0, 400, 0, 400, 400, true)); // repeated
        assert!(should_commit(0, 400, 395, 400, 400, false)); // small step
    }

    #[test]
    fn annotation_words_are_recognised() {
        assert!(is_annotation_word("[beat]"));
        assert!(is_annotation_word("—"));
        assert!(is_annotation_word("👍"));
        assert!(!is_annotation_word("hello"));
    }
}
