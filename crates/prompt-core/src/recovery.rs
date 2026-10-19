//! Picks the reading position back up after the matcher has lost it.
//!
//! The matcher's walks only look [`MAX_SKIP`](crate::matcher) words ahead, so a
//! presenter who skips a sentence, or a burst of misrecognised words, leaves
//! the highlight stuck until they tap. This looks for the last few spoken words
//! as an exact phrase a little further on and, when it is unambiguous, jumps
//! there. It never moves just because sound arrived: a phrase has to be long,
//! unique, and ahead of where the highlight already is.
//!
//! It also follows the presenter *back*: going back to re-read a line leaves
//! the walks with nothing ahead to match, so the last few spoken words are
//! looked for behind the highlight too ([`PhraseIndex::reread`]).
//!
//! Ported from upstream Textream's `SpeechRecoveryMatcher` (f/textream#119)
//! and its re-read regression (f/textream#122), with one change: words are
//! compared through [`fold_case`] rather than with accents stripped, since
//! stripping would make Turkish `ı`/`i` or `ş`/`s` — different letters —
//! compare equal.

use crate::matcher::is_fuzzy_match;
use crate::script::PromptScript;
use crate::text::{fold_case, split_into_words};

/// How far behind the highlight a re-read is looked for, in graphemes —
/// a few lines.
const REREAD_WINDOW: usize = 700;

/// How many of the newest spoken words must match a run behind the highlight
/// before it counts as a re-read.
const REREAD_MIN_WORDS: usize = 4;

/// A re-read must land at least this far back, in graphemes; anything closer
/// is ordinary jitter around the current word.
const REREAD_MIN_DISTANCE: usize = 20;

/// How many of the newest spoken words make up a candidate phrase.
const RECENT_WORDS: usize = 6;

/// How many already-read words are searched too, so a phrase that was just
/// spoken there counts as ambiguous rather than jumping to a later repeat.
const LOOK_BEHIND: usize = 6;

/// Search windows, nearest first: (words ahead, shortest phrase accepted).
/// A short phrase may repair a nearby misread; skipping further needs more
/// evidence.
const WINDOWS: [(usize, usize); 2] = [(24, 3), (80, 5)];

/// A phrase needs at least this many letters, so a run of short function
/// words ("and in the") cannot qualify.
const MIN_PHRASE_LETTERS: usize = 12;

/// A recovery must move at least this far, in graphemes, to be worth taking.
const MIN_ADVANCE: usize = 12;

#[derive(Debug, Clone)]
struct IndexedWord {
    normalized: String,
    /// Grapheme offset of the word's first character.
    start: usize,
    /// Grapheme offset just past the word.
    end: usize,
}

/// Where a re-read starts, and how much of the transcript it accounts for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Reread {
    /// Grapheme offset of the first re-read word in the script.
    pub start: usize,
    /// Characters at the end of the transcript that belong to the re-read
    /// (the matched words and the spaces before them), to be matched again
    /// from `start`.
    pub spoken_chars: usize,
}

/// The script's speakable words, normalised once for phrase lookup.
#[derive(Debug, Clone)]
pub(crate) struct PhraseIndex {
    words: Vec<IndexedWord>,
}

impl PhraseIndex {
    pub(crate) fn new(script: &PromptScript) -> Self {
        let words = script
            .words()
            .iter()
            .filter(|word| !word.is_annotation)
            .map(|word| IndexedWord {
                normalized: normalize(&word.text),
                start: word.range.start,
                end: word.range.end,
            })
            .filter(|word| !word.normalized.is_empty())
            .collect();
        Self { words }
    }

    /// Where reading has got to, if the end of `spoken` is a clear phrase
    /// ahead of `current`; `None` otherwise.
    pub(crate) fn recovered_offset(&self, spoken: &str, current: usize) -> Option<usize> {
        let mut recent: Vec<String> = split_into_words(spoken)
            .iter()
            .map(|word| normalize(word))
            .filter(|word| !word.is_empty())
            .collect();
        if recent.len() > RECENT_WORDS {
            recent.drain(..recent.len() - RECENT_WORDS);
        }
        if recent.len() < 3 {
            return None;
        }
        let current_word = self.words.iter().position(|word| word.end > current)?;
        let lower = current_word.saturating_sub(LOOK_BEHIND);

        for (look_ahead, min_length) in WINDOWS {
            if recent.len() < min_length {
                continue;
            }
            let upper = (current_word + look_ahead).min(self.words.len());
            for length in (min_length..=recent.len()).rev() {
                let phrase = &recent[recent.len() - length..];
                let letters: usize = phrase.iter().map(|word| word.chars().count()).sum();
                if letters < MIN_PHRASE_LETTERS || upper < lower + length {
                    continue;
                }
                let mut ends = (lower..=upper - length)
                    .filter(|&start| {
                        phrase
                            .iter()
                            .zip(&self.words[start..start + length])
                            .all(|(spoken, word)| *spoken == word.normalized)
                    })
                    .map(|start| self.words[start + length - 1].end);
                // A longer unique phrase settles what a shorter repeated one
                // cannot, so only a single match is taken.
                if let (Some(end), None) = (ends.next(), ends.next()) {
                    return (end >= current + MIN_ADVANCE).then_some(end);
                }
            }
        }
        None
    }
}

impl PhraseIndex {
    /// Where the presenter went back to, if the newest spoken words match a
    /// run of words a few lines behind `current`; `None` otherwise.
    ///
    /// One result is never enough to act on — the caller confirms it against
    /// the next one, so a single misheard phrase cannot drag the highlight
    /// back.
    pub(crate) fn reread(&self, spoken: &str, current: usize) -> Option<Reread> {
        let spoken: Vec<(String, usize)> = split_into_words(spoken)
            .iter()
            .map(|word| (normalize(word), word.chars().count()))
            .filter(|(word, _)| !word.is_empty())
            .collect();
        if spoken.len() < REREAD_MIN_WORDS {
            return None;
        }
        let tail = &spoken[spoken.len().saturating_sub(REREAD_MIN_WORDS + 2)..];

        let window_start = current.saturating_sub(REREAD_WINDOW);
        let behind: Vec<&IndexedWord> = self
            .words
            .iter()
            .filter(|word| word.start >= window_start && word.start < current)
            .collect();

        // The longest run of script words, from each starting point, that the
        // tail spells out in order. Spoken words that match nothing (a
        // stumble, a filler) are stepped over.
        let mut best: Option<(usize, usize)> = None;
        for from in 0..behind.len() {
            let mut length = 0;
            for (spoken, _) in tail {
                let Some(word) = behind.get(from + length) else { break };
                if is_fuzzy_match(&word.normalized, spoken) {
                    length += 1;
                }
            }
            if length > best.map_or(0, |(_, longest)| longest) {
                best = Some((from, length));
            }
        }

        let (from, length) = best?;
        let (start, end) = (behind[from].start, behind[from + length - 1].end);
        // A run that reaches up to the highlight is just the words read last —
        // a pause, or the recogniser revising them — not a step back. A real
        // re-read ends well behind it.
        (length >= REREAD_MIN_WORDS && end + REREAD_MIN_DISTANCE < current).then(|| Reread {
            start,
            spoken_chars: tail.iter().map(|(_, chars)| chars + 1).sum(),
        })
    }
}

fn normalize(word: &str) -> String {
    fold_case(word)
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENGLISH: &str = "New extraordinary interconnected computational research methods \
        and technologies are changing our everyday lives around the world.";

    fn indexed(text: &str) -> (PhraseIndex, PromptScript) {
        let script = PromptScript::new(text);
        (PhraseIndex::new(&script), script)
    }

    /// Grapheme offset just past `phrase` in the normalised script.
    fn end_of(script: &PromptScript, phrase: &str) -> usize {
        let byte = script.text().find(phrase).expect("phrase in script") + phrase.len();
        script.text()[..byte].chars().count()
    }

    #[test]
    fn recovers_after_several_misread_words() {
        let (index, script) = indexed(ENGLISH);
        let anchor = end_of(&script, "New");
        let target = end_of(&script, "changing our everyday lives");
        assert_eq!(
            index.recovered_offset("New something wrong changing our everyday lives", anchor),
            Some(target)
        );
        assert_eq!(index.recovered_offset("our everyday lives", anchor), Some(target));
    }

    #[test]
    fn waits_for_phrase_evidence() {
        let (index, script) = indexed(ENGLISH);
        let anchor = end_of(&script, "New");
        assert_eq!(index.recovered_offset("changing our", anchor), None);
        assert_eq!(index.recovered_offset("", anchor), None);
        assert_eq!(
            index.recovered_offset("please pause I need a glass of water", anchor),
            None
        );
    }

    #[test]
    fn never_moves_without_progress() {
        let (index, script) = indexed(ENGLISH);
        let target = end_of(&script, "changing our everyday lives");
        assert_eq!(index.recovered_offset("our everyday lives", target), None);
        assert_eq!(
            index.recovered_offset("around the world", target),
            Some(script.character_count())
        );
    }

    #[test]
    fn turkish_resumes_and_keeps_its_letters_apart() {
        let text = "Bugün yapay zekânın olağanüstü gelişmelerini ve bilimsel araştırmalarını \
            konuşacağız. Teknoloji günlük hayatımızı hızla değiştiriyor.";
        let (index, script) = indexed(text);
        assert_eq!(
            index.recovered_offset("bir kelimeyi yanlış söyledim teknoloji günlük hayatımızı", 5),
            Some(end_of(&script, "Teknoloji günlük hayatımızı"))
        );

        let (index, script) = indexed("Yeni İLERİ ARAŞTIRMALAR İNSANLIĞI DEĞİŞTİRİYOR.");
        assert_eq!(
            index.recovered_offset("ileri araştırmalar insanlığı değiştiriyor", 0),
            Some(script.character_count())
        );

        // ş and s are different letters: "kasa" (safe) is not "kaşa".
        let (index, _) = indexed("Bir iki üç dört beş altı yedi sekiz kasa masa dolap kapı.");
        assert_eq!(index.recovered_offset("kaşa maşa dolap kapı", 0), None);
    }

    #[test]
    fn ambiguous_repeats_do_not_skip_ahead() {
        let (index, _) = indexed(
            "Please read the important research findings and then read the important \
             research findings again.",
        );
        assert_eq!(index.recovered_offset("important research findings", 0), None);
    }

    #[test]
    fn skipping_far_needs_a_longer_phrase_and_stays_bounded() {
        let filler = |count: usize| vec!["unread"; count].join(" ");

        let (index, _) = indexed(&format!("Start {} important research findings", filler(30)));
        assert_eq!(index.recovered_offset("important research findings", 0), None);

        let text = format!(
            "Start {} researchers discover entirely new scientific methods today.",
            filler(45)
        );
        let (index, script) = indexed(&text);
        let spoken = "researchers discover entirely new scientific methods";
        assert_eq!(index.recovered_offset(spoken, 0), Some(end_of(&script, spoken)));

        let text = format!(
            "Start {} researchers discover entirely new scientific methods today.",
            filler(90)
        );
        let (index, _) = indexed(&text);
        assert_eq!(index.recovered_offset(spoken, 0), None);
    }

    #[test]
    fn cues_neither_block_recovery_nor_anchor_it() {
        let (index, script) =
            indexed("Today [PAUSE AND LOOK AT CAMERA] new research changes everyday life.");
        assert_eq!(
            index.recovered_offset("new research changes everyday life", 0),
            Some(script.character_count())
        );
        assert_eq!(index.recovered_offset("pause and look at camera", 0), None);
    }
}
