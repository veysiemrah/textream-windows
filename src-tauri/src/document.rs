//! `.textream` files.
//!
//! The macOS app stores a script as a plain JSON array of page strings —
//! `["page one", "page two"]`, nothing else. That format is deliberately kept
//! here rather than inventing a Windows-specific one: a script saved on one
//! platform opens cleanly on the other, and the format is simple enough that
//! there is no reason to diverge from it.
//!
//! Multi-page scripts are out of scope for this port (see the README), so
//! multiple pages are flattened into the one continuous script this app
//! edits, separated by a blank line. Nothing is dropped — a script written on
//! a Mac with several pages still opens here in full, just without the page
//! boundaries.

use std::fs;
use std::path::Path;

/// Extension `.textream` files use, without the leading dot.
pub const EXTENSION: &str = "textream";

/// Serialises a script into the `.textream` file format.
///
/// The whole script becomes the file's one page. Pretty-printed rather than
/// compact: these are small text files a person may reasonably open outside
/// the app, and pretty-printing costs nothing a script-sized file would
/// notice.
pub fn encode(script: &str) -> Result<String, String> {
    serde_json::to_string_pretty(&[script]).map_err(|error| error.to_string())
}

/// Parses a `.textream` file's pages back into one script.
///
/// Rejects a file with no pages at all — that is not a script the editor can
/// do anything useful with — but a file with only blank pages is valid; it
/// simply produces an empty script.
pub fn decode(contents: &str) -> Result<String, String> {
    let pages: Vec<String> = serde_json::from_str(contents)
        .map_err(|_| "This file is not a valid .textream script.".to_string())?;
    if pages.is_empty() {
        return Err("This .textream file has no pages to read.".to_string());
    }
    Ok(pages.join("\n\n"))
}

/// Writes `script` to `path` as a `.textream` file.
pub fn save(path: &Path, script: &str) -> Result<(), String> {
    let encoded = encode(script)?;
    fs::write(path, encoded).map_err(|error| error.to_string())
}

/// Reads a `.textream` file at `path`, flattened to one script.
pub fn load(path: &Path) -> Result<String, String> {
    let contents = fs::read_to_string(path).map_err(|error| error.to_string())?;
    decode(&contents)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_round_trips_through_encode_and_decode() {
        let script = "Welcome back. [smile]\n\nToday we ship.";
        let encoded = encode(script).unwrap();
        assert_eq!(decode(&encoded).unwrap(), script);
    }

    #[test]
    fn encoding_produces_a_single_element_array() {
        let encoded = encode("hello").unwrap();
        let pages: Vec<String> = serde_json::from_str(&encoded).unwrap();
        assert_eq!(pages, vec!["hello".to_string()]);
    }

    #[test]
    fn multiple_pages_from_the_macos_app_are_flattened_in_order() {
        let file = r#"["Page one.", "Page two.", "Page three."]"#;
        assert_eq!(
            decode(file).unwrap(),
            "Page one.\n\nPage two.\n\nPage three."
        );
    }

    #[test]
    fn an_empty_page_array_is_rejected() {
        assert!(decode("[]").is_err());
    }

    #[test]
    fn a_single_blank_page_decodes_to_an_empty_script() {
        assert_eq!(decode(r#"[""]"#).unwrap(), "");
    }

    #[test]
    fn malformed_json_is_rejected_with_a_plain_message() {
        let error = decode("not json at all").unwrap_err();
        assert!(!error.contains("serde"), "leaked a parser's own vocabulary");
    }

    #[test]
    fn a_plain_string_instead_of_an_array_is_rejected() {
        assert!(decode(r#""just a string"#).is_err());
    }

    #[test]
    fn save_then_load_preserves_the_script_on_disk() {
        let dir = std::env::temp_dir().join("textream-document-test");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("sample.textream");

        save(&path, "alpha\n\nbeta").unwrap();
        assert_eq!(load(&path).unwrap(), "alpha\n\nbeta");

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn loading_a_missing_file_fails_without_panicking() {
        let missing = Path::new("C:/definitely/not/here.textream");
        assert!(load(missing).is_err());
    }
}
