//! Markdown and plain-text scripts.
//!
//! Many presenters already keep their talk in a notes app as Markdown. Opening
//! that file directly saves copying it over by hand and then deleting every
//! `#`, `**` and `- ` the prompter would otherwise show as part of the script.
//!
//! This keeps what is read aloud and drops what is formatting: heading text
//! stays, the `#` goes; a link keeps its text, the URL goes; code blocks,
//! images, comments and frontmatter go entirely. A horizontal rule — a page
//! break in the macOS app — becomes a paragraph break, since this port has one
//! continuous script. Modelled on upstream Textream's `MarkdownScriptParser`
//! (f/textream#130), without its Obsidian-specific rules and page splitting.

/// Turns Markdown into a script.
pub fn to_script(source: &str) -> String {
    let text = drop_comments(&normalise(source));
    let lines: Vec<&str> = text.lines().collect();
    let lines = drop_frontmatter(&lines);

    let mut out: Vec<String> = Vec::new();
    let mut fence: Option<&str> = None;
    for line in lines {
        if let Some(open) = fence {
            if fence_marker(line) == Some(open) {
                fence = None;
            }
            continue;
        }
        if let Some(marker) = fence_marker(line) {
            fence = Some(marker);
            continue;
        }
        out.extend(convert_line(line));
    }
    tidy(&out)
}

/// Reads a plain-text script: only line endings and a byte-order mark change.
pub fn plain_to_script(source: &str) -> String {
    normalise(source).trim().to_string()
}

fn normalise(source: &str) -> String {
    source
        .strip_prefix('\u{FEFF}')
        .unwrap_or(source)
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

/// Removes `<!-- … -->`, including across lines.
fn drop_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        match rest[start..].find("-->") {
            Some(end) => rest = &rest[start + end + 3..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Drops a leading `---` … `---` block, if what is inside looks like YAML —
/// otherwise the first `---` is a horizontal rule and stays.
fn drop_frontmatter<'a>(lines: &[&'a str]) -> Vec<&'a str> {
    if lines.first().map(|line| line.trim()) != Some("---") {
        return lines.to_vec();
    }
    let Some(close) = lines
        .iter()
        .skip(1)
        .position(|line| matches!(line.trim(), "---" | "..."))
        .map(|index| index + 1)
    else {
        return lines.to_vec();
    };
    let yaml = lines[1..close].iter().all(|line| {
        let trimmed = line.trim();
        trimmed.is_empty()
            || trimmed.starts_with('#')
            || trimmed.starts_with("- ")
            || line.starts_with(' ')
            || trimmed.contains(':')
    });
    if yaml {
        lines[close + 1..].to_vec()
    } else {
        lines.to_vec()
    }
}

fn fence_marker(line: &str) -> Option<&'static str> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let rest = line.trim_start_matches(' ');
    if indent > 3 {
        return None;
    }
    if rest.starts_with("```") {
        Some("```")
    } else if rest.starts_with("~~~") {
        Some("~~~")
    } else {
        None
    }
}

/// A `---`, `***` or `___` line.
fn is_rule(line: &str) -> bool {
    let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    compact.len() >= 3
        && ['-', '*', '_']
            .iter()
            .any(|&mark| compact.chars().all(|c| c == mark))
}

/// The line as it should read, `Some("")` for a paragraph break, or `None`
/// for a line that is not part of the script at all.
fn convert_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() || is_rule(trimmed) {
        return Some(String::new());
    }
    // Link and footnote definitions are not read aloud.
    if trimmed.starts_with('[') && trimmed.contains("]:") && !trimmed.starts_with("[[") {
        return None;
    }

    if let Some(text) = heading(trimmed) {
        return Some(strip_inline(text));
    }

    if trimmed.starts_with('|') || (trimmed.contains('|') && is_table_separator(trimmed)) {
        if is_table_separator(trimmed) {
            return None;
        }
        let cells: Vec<String> = trimmed
            .split('|')
            .map(|cell| strip_inline(cell.trim()))
            .filter(|cell| !cell.is_empty())
            .collect();
        return Some(cells.join(" "));
    }

    let mut text = trimmed;
    while let Some(rest) = text.strip_prefix('>') {
        text = rest.trim_start();
    }
    text = strip_list_marker(text);
    Some(strip_inline(text))
}

fn heading(line: &str) -> Option<&str> {
    let hashes = line.chars().take_while(|&c| c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = &line[hashes..];
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None; // "#hashtag", not a heading
    }
    Some(rest.trim().trim_end_matches('#').trim_end())
}

fn is_table_separator(line: &str) -> bool {
    line.contains('-') && line.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t'))
}

fn strip_list_marker(text: &str) -> &str {
    let after_bullet = ["- ", "* ", "+ "]
        .iter()
        .find_map(|bullet| text.strip_prefix(bullet))
        .or_else(|| {
            let digits = text.chars().take_while(char::is_ascii_digit).count();
            (1..=3).contains(&digits).then(|| {
                let rest = &text[digits..];
                rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") "))
            })?
        });
    match after_bullet {
        Some(rest) => ["[ ] ", "[x] ", "[X] "]
            .iter()
            .find_map(|task| rest.strip_prefix(task))
            .unwrap_or(rest),
        None => text,
    }
}

/// Strips inline formatting, keeping the words.
fn strip_inline(text: &str) -> String {
    let text = strip_links(text);
    let text = strip_tags(&text);
    let text = text.replace("**", "").replace("~~", "").replace('`', "");
    let text = strip_emphasis(&text, '*');
    let text = strip_emphasis(&text, '_');
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `![alt](url)` and `![[embed]]` go; `[text](url)`, `[text][ref]` and
/// `[[page|alias]]` keep their text; `[^1]` footnote marks go.
fn strip_links(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let image = chars[i] == '!' && chars.get(i + 1) == Some(&'[');
        let start = if image { i + 1 } else { i };
        if chars.get(start) != Some(&'[') {
            out.push(chars[i]);
            i += 1;
            continue;
        }

        // Wiki-link or embed: [[target#section|alias]].
        if chars.get(start + 1) == Some(&'[') {
            if let Some(close) = find(&chars, start + 2, &[']', ']']) {
                if !image {
                    let inner: String = chars[start + 2..close].iter().collect();
                    let shown = match inner.split_once('|') {
                        Some((_, alias)) => alias,
                        None => inner.split('#').next().unwrap_or(""),
                    };
                    out.push_str(shown);
                }
                i = close + 2;
                continue;
            }
        }

        let Some(close) = find(&chars, start + 1, &[']']) else {
            out.push(chars[i]);
            i += 1;
            continue;
        };
        let label: String = chars[start + 1..close].iter().collect();
        if label.starts_with('^') {
            i = close + 1; // footnote reference
            continue;
        }
        let target_end = match chars.get(close + 1) {
            Some('(') => find(&chars, close + 2, &[')']),
            Some('[') => find(&chars, close + 2, &[']']),
            _ => None,
        };
        match target_end {
            Some(end) => {
                if !image {
                    out.push_str(&label);
                }
                i = end + 1;
            }
            None => {
                out.push(chars[i]);
                i += 1;
            }
        }
    }
    out
}

fn find(chars: &[char], from: usize, needle: &[char]) -> Option<usize> {
    (from..chars.len().saturating_sub(needle.len() - 1))
        .find(|&at| chars[at..at + needle.len()] == *needle)
}

/// Removes HTML tags such as `<br>` or `<span class="x">`, but not a lone
/// `<` used as "less than".
fn strip_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        let after = &rest[open + 1..];
        let is_tag = after.starts_with(|c: char| c.is_ascii_alphabetic() || c == '/');
        match (is_tag, after.find('>')) {
            (true, Some(close)) => {
                out.push_str(&rest[..open]);
                // A line break tag still separates the words around it.
                out.push(' ');
                rest = &after[close + 1..];
            }
            _ => {
                out.push_str(&rest[..=open]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Removes single `*` or `_` emphasis marks: one that opens a run (nothing
/// word-like before it, text after) or closes one (text before, nothing
/// word-like after). `5 * 3` and `snake_case` are left alone.
fn strip_emphasis(text: &str, mark: char) -> String {
    let chars: Vec<char> = text.chars().collect();
    let wordy = |c: Option<&char>| c.is_some_and(|c| c.is_alphanumeric());
    let blank = |c: Option<&char>| c.map_or(true, |c| c.is_whitespace());
    chars
        .iter()
        .enumerate()
        .filter(|&(i, &c)| {
            if c != mark {
                return true;
            }
            let before = i.checked_sub(1).and_then(|at| chars.get(at));
            let after = chars.get(i + 1);
            let opens = !wordy(before) && !blank(after);
            let closes = !blank(before) && !wordy(after);
            !(opens || closes)
        })
        .map(|(_, &c)| c)
        .collect()
}

/// Joins the lines, keeping at most one blank line between paragraphs.
fn tidy(lines: &[String]) -> String {
    let mut out = String::new();
    let mut blank_run = 0;
    for line in lines {
        if line.is_empty() {
            blank_run += 1;
            continue;
        }
        if !out.is_empty() {
            out.push_str(if blank_run > 0 { "\n\n" } else { "\n" });
        }
        out.push_str(line);
        blank_run = 0;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_and_emphasis_keep_only_their_words() {
        assert_eq!(
            to_script("# Opening ##\n\nWelcome **everyone** to _the_ show."),
            "Opening\n\nWelcome everyone to the show."
        );
    }

    #[test]
    fn links_keep_their_text_and_images_go() {
        assert_eq!(
            to_script("See [our site](https://example.com) ![logo](logo.png) and [[Notes#Part|the notes]]."),
            "See our site and the notes."
        );
        assert_eq!(to_script("A claim[^1] stands.\n\n[^1]: Source."), "A claim stands.");
    }

    #[test]
    fn lists_quotes_and_tasks_lose_their_markers() {
        assert_eq!(
            to_script("- first\n* second\n1. third\n- [x] done\n> quoted"),
            "first\nsecond\nthird\ndone\nquoted"
        );
    }

    #[test]
    fn code_frontmatter_and_comments_are_dropped() {
        let source = "---\ntitle: Talk\ntags: [a, b]\n---\nHello\n```\nlet x = 1;\n```\n<!-- note\nto self -->world";
        assert_eq!(to_script(source), "Hello\nworld");
    }

    #[test]
    fn a_rule_becomes_a_paragraph_break_but_not_frontmatter() {
        assert_eq!(to_script("One\n---\nTwo"), "One\n\nTwo");
        // Not YAML inside, so the leading rule is a rule.
        assert_eq!(to_script("---\nJust a line of text\n---\nNext"), "Just a line of text\n\nNext");
    }

    #[test]
    fn tables_read_row_by_row() {
        assert_eq!(
            to_script("| Name | Role |\n|---|:---:|\n| Ada | **Host** |"),
            "Name Role\nAda Host"
        );
    }

    #[test]
    fn plain_symbols_survive() {
        assert_eq!(to_script("5 * 3 < 20 and snake_case stays"), "5 * 3 < 20 and snake_case stays");
        assert_eq!(to_script("#hashtag is not a heading"), "#hashtag is not a heading");
    }

    #[test]
    fn cues_pass_through_untouched() {
        assert_eq!(to_script("Hi [smile] there"), "Hi [smile] there");
    }

    #[test]
    fn turkish_text_is_unchanged() {
        assert_eq!(to_script("## Giriş\n\n**Işık** ve _İstanbul_"), "Giriş\n\nIşık ve İstanbul");
    }

    #[test]
    fn plain_text_only_normalises_line_endings() {
        assert_eq!(plain_to_script("\u{FEFF}# not a heading\r\n**kept**\r\n"), "# not a heading\n**kept**");
    }
}
