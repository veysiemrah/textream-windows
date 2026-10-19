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
    tokenize(text).into_iter().map(|(word, _)| word).collect()
}

/// [`split_into_words`], plus how the author broke the text before each word:
/// `0` within a line, `1` after a line break, `2` after a blank line or more.
///
/// The words are exactly the same, so offsets computed from them do not move;
/// the breaks are only for laying the text out.
pub fn tokenize(text: &str) -> Vec<(String, u8)> {
    let mut result = Vec::new();
    let mut newlines = 0usize;
    // Start of the current token, and the breaks it opens with.
    let mut token: Option<(usize, u8)> = None;

    for (index, c) in text.char_indices() {
        if c.is_whitespace() {
            if let Some((start, breaks)) = token.take() {
                push_token(&text[start..index], breaks, &mut result);
                newlines = 0;
            }
            if c == '\n' {
                newlines += 1;
            }
        } else if token.is_none() {
            // Newlines before the very first word are not a break.
            let breaks = if result.is_empty() { 0 } else { newlines.min(2) as u8 };
            token = Some((index, breaks));
        }
    }
    if let Some((start, breaks)) = token {
        push_token(&text[start..], breaks, &mut result);
    }

    result
}

/// Pushes `token`, exploding any CJK in it into one word per character. Only
/// the first of those words carries `breaks`.
fn push_token(token: &str, breaks: u8, result: &mut Vec<(String, u8)>) {
    let mut breaks = breaks;
    let mut push = |word: String| result.push((word, std::mem::take(&mut breaks)));

    if !token.chars().any(is_cjk) {
        push(token.to_string());
        return;
    }

    let mut buffer = String::new();
    for grapheme in token.graphemes(true) {
        if grapheme.chars().next().is_some_and(is_cjk) {
            if !buffer.is_empty() {
                push(std::mem::take(&mut buffer));
            }
            push(grapheme.to_string());
        } else {
            buffer.push_str(grapheme);
        }
    }
    if !buffer.is_empty() {
        push(buffer);
    }
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

/// Lowercases `text` for comparison, folding the four Turkish i's together.
///
/// Unicode's default lowercase is wrong for Turkish in both directions: `I`
/// becomes `i` where a Turkish recogniser writes `ı`, and `İ` becomes `i` plus
/// a combining dot that no transcript ever contains. So a script line such as
/// "IŞIK" or "İSTANBUL" would never match what was said. Folding `I`, `İ`, `i`
/// and `ı` to one `i` fixes Turkish without needing to know the language, and
/// costs the matcher nothing elsewhere — it is fuzzy about single letters
/// anyway, and no other language contrasts dotted with dotless i.
pub fn fold_case(text: &str) -> String {
    let mut folded = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        match c {
            'ı' => folded.push('i'),
            '\u{0307}' if folded.ends_with('i') => {}
            other => folded.push(other),
        }
    }
    folded
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
    fn counts_the_line_breaks_before_each_word() {
        let words = tokenize("one two\nthree\r\n\n\n four\n五六");
        let texts: Vec<_> = words.iter().map(|(text, _)| text.as_str()).collect();
        let breaks: Vec<_> = words.iter().map(|&(_, breaks)| breaks).collect();
        assert_eq!(texts, ["one", "two", "three", "four", "五", "六"]);
        // Two or more newlines are one paragraph break, however many.
        assert_eq!(breaks, [0, 0, 1, 2, 1, 0]);
    }

    #[test]
    fn leading_newlines_are_not_a_break() {
        assert_eq!(tokenize("\n\nhello"), [("hello".to_string(), 0)]);
    }

    #[test]
    fn tokenize_splits_exactly_like_split_into_words() {
        let text = "Hello [smile]\n\nworld 你好 abc世界def\n end";
        let words: Vec<_> = tokenize(text).into_iter().map(|(word, _)| word).collect();
        assert_eq!(words, split_into_words(text));
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
    fn fold_case_unifies_the_turkish_i() {
        assert_eq!(fold_case("IŞIK"), fold_case("ışık"));
        assert_eq!(fold_case("İSTANBUL"), "istanbul");
        assert_eq!(fold_case("I\u{0307}stanbul"), "istanbul");
        assert_eq!(fold_case("Iğdır"), "iğdir");
    }

    #[test]
    fn fold_case_keeps_one_grapheme_per_grapheme() {
        for g in ["İ", "I", "ı", "Ş", "Ğ", "Ö", "Ü", "Ç"] {
            assert_eq!(graphemes(&fold_case(g)).len(), 1, "{g}");
        }
    }

    #[test]
    fn fold_case_leaves_other_combining_dots_alone() {
        // U+0307 after a letter other than i is real content and must survive.
        assert_eq!(fold_case("a\u{0307}"), "a\u{0307}");
    }

    #[test]
    fn readability_ignores_punctuation_and_emoji() {
        assert!(is_readable("word"));
        assert!(is_readable("42"));
        assert!(!is_readable("—"));
        assert!(!is_readable("👍🏽"));
    }
}
