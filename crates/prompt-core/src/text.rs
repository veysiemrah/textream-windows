//! Script tokenisation and writing-direction inference.
//!
//! Offsets throughout `prompt-core` are **grapheme cluster** indices, matching
//! the way the macOS app counts `Character`s. Rust's `char` is a Unicode scalar,
//! so a `👍🏽` or a combining sequence would otherwise count as two positions and
//! drift the highlight away from what the reader sees.

use unicode_segmentation::UnicodeSegmentation;

/// Han, Hiragana, Katakana and Hangul blocks.
///
/// CJK scripts are written without spaces, so a whole sentence arrives as one
/// whitespace-delimited token. Tracking needs per-character granularity to
/// highlight anything, hence the explicit block test.
pub fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x4E00..=0x9FFF     // CJK Unified Ideographs
        | 0x3400..=0x4DBF   // Extension A
        | 0x20000..=0x2A6DF // Extension B
        | 0xF900..=0xFAFF   // Compatibility Ideographs
        | 0x3040..=0x309F   // Hiragana
        | 0x30A0..=0x30FF   // Katakana
        | 0xAC00..=0xD7AF   // Hangul Syllables
    )
}

/// Scalars with a strong right-to-left bidi class (Hebrew, Arabic, Syriac,
/// Thaana, N'Ko, and the Arabic presentation forms).
pub fn is_strong_rtl(c: char) -> bool {
    matches!(c as u32,
        0x0590..=0x08FF
        | 0xFB1D..=0xFDFF
        | 0xFE70..=0xFEFF
        | 0x10800..=0x10FFF
        | 0x1E800..=0x1EEFF
    )
}

/// True when the grapheme carries readable content rather than being pure
/// punctuation, whitespace or symbol.
pub fn is_readable(grapheme: &str) -> bool {
    grapheme
        .chars()
        .any(|c| c.is_alphabetic() || c.is_numeric())
}

/// Splits a script into trackable words.
///
/// Newlines collapse to spaces and runs of whitespace are dropped. Tokens that
/// contain CJK are further exploded so each ideograph or kana becomes its own
/// word, while any Latin run embedded in the same token stays glued together.
pub fn split_into_words(text: &str) -> Vec<String> {
    let normalised = text.replace('\n', " ");
    let mut result = Vec::new();

    for token in normalised.split_whitespace() {
        if !token.chars().any(is_cjk) {
            result.push(token.to_string());
            continue;
        }

        let mut buffer = String::new();
        for grapheme in token.graphemes(true) {
            if grapheme.chars().next().is_some_and(is_cjk) {
                if !buffer.is_empty() {
                    result.push(std::mem::take(&mut buffer));
                }
                result.push(grapheme.to_string());
            } else {
                buffer.push_str(grapheme);
            }
        }
        if !buffer.is_empty() {
            result.push(buffer);
        }
    }

    result
}

/// Writing direction of a script.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextDirection {
    #[default]
    LeftToRight,
    RightToLeft,
}

/// Infers direction from the first strongly-directional letter outside of a
/// `[cue]`.
///
/// Cues are stage directions the reader never speaks and are frequently written
/// in English inside an otherwise Hebrew or Arabic script, so letting them vote
/// would flip the whole prompter the wrong way round.
pub fn infer_direction(text: &str) -> TextDirection {
    let mut inside_cue = false;
    for c in text.chars() {
        match c {
            '[' => {
                inside_cue = true;
                continue;
            }
            ']' => {
                inside_cue = false;
                continue;
            }
            _ => {}
        }
        if inside_cue || !c.is_alphabetic() {
            continue;
        }
        return if is_strong_rtl(c) {
            TextDirection::RightToLeft
        } else {
            TextDirection::LeftToRight
        };
    }
    TextDirection::LeftToRight
}

/// Splits `text` into owned grapheme clusters — the unit every offset in this
/// crate is measured in.
pub fn graphemes(text: &str) -> Vec<String> {
    text.graphemes(true).map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_whitespace_and_collapses_newlines() {
        assert_eq!(
            split_into_words("hello\nbrave   new\tworld"),
            ["hello", "brave", "new", "world"]
        );
    }

    #[test]
    fn explodes_cjk_into_single_characters() {
        assert_eq!(split_into_words("你好世界"), ["你", "好", "世", "界"]);
    }

    #[test]
    fn keeps_latin_runs_glued_inside_mixed_tokens() {
        assert_eq!(
            split_into_words("你好OK世界"),
            ["你", "好", "OK", "世", "界"]
        );
    }

    #[test]
    fn treats_kana_and_hangul_as_cjk() {
        assert_eq!(
            split_into_words("こんにちは"),
            ["こ", "ん", "に", "ち", "は"]
        );
        assert_eq!(split_into_words("안녕"), ["안", "녕"]);
    }

    #[test]
    fn empty_script_yields_no_words() {
        assert!(split_into_words("   \n  ").is_empty());
    }

    #[test]
    fn direction_defaults_to_ltr() {
        assert_eq!(infer_direction("hello"), TextDirection::LeftToRight);
        assert_eq!(infer_direction("123 !!!"), TextDirection::LeftToRight);
    }

    #[test]
    fn direction_detects_hebrew_and_arabic() {
        assert_eq!(infer_direction("שלום עולם"), TextDirection::RightToLeft);
        assert_eq!(infer_direction("مرحبا"), TextDirection::RightToLeft);
    }

    #[test]
    fn cue_text_does_not_vote_on_direction() {
        assert_eq!(
            infer_direction("[pause and smile] שלום"),
            TextDirection::RightToLeft
        );
    }

    #[test]
    fn emoji_is_a_single_grapheme() {
        assert_eq!(graphemes("a👍🏽b").len(), 3);
    }

    #[test]
    fn readability_ignores_punctuation_and_emoji() {
        assert!(is_readable("word"));
        assert!(is_readable("42"));
        assert!(!is_readable("—"));
        assert!(!is_readable("👍🏽"));
    }
}
