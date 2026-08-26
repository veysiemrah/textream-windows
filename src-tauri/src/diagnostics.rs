//! Crash and background-error logging for release builds.
//!
//! `windows_subsystem = "windows"` means a release build attaches no console,
//! so a panic or a background thread's error would otherwise vanish without a
//! trace. "It just closed" is not a bug report anyone can act on; a plain text
//! file next to the settings is.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Appends one line to `path`, timestamped in Unix seconds.
///
/// Unix time rather than a formatted date: this crate pulls in no date/time
/// dependency, and a raw timestamp still answers the two questions a log line
/// needs to — how recently, and in what order — which is all a handful of
/// crash reports ever need.
///
/// Failures to write are swallowed. A logging failure must never be the
/// reason something else goes wrong, and by the time one happens there is
/// nowhere left to report it to anyway.
pub fn append(path: &Path, message: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let _ = writeln!(file, "[{timestamp}] {message}");
}

/// Installs a panic hook that logs to `path` in addition to Rust's default
/// hook — which still reaches stderr whenever a console happens to be
/// attached, as it is for every debug build. This does not stop the panic
/// from unwinding; it only makes sure it leaves a trace on the way out.
pub fn install_panic_hook(path: PathBuf) {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        append(&path, &format!("panic: {info}"));
        default_hook(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("textream-diagnostics-{name}"));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn appending_creates_the_file_and_any_missing_directories() {
        let dir = temp_dir("create");
        let path = dir.join("nested").join("textream.log");

        append(&path, "hello");
        assert!(fs::read_to_string(&path).unwrap().contains("hello"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn appending_twice_keeps_both_lines_in_order() {
        let dir = temp_dir("append");
        let path = dir.join("textream.log");

        append(&path, "first");
        append(&path, "second");

        let contents = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].ends_with("first"));
        assert!(lines[1].ends_with("second"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_line_opens_with_a_timestamp() {
        let dir = temp_dir("timestamp");
        let path = dir.join("textream.log");

        append(&path, "hello");
        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.starts_with('['));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_message_containing_newlines_does_not_break_the_log_format() {
        let dir = temp_dir("multiline");
        let path = dir.join("textream.log");

        append(&path, "line one\nline two");
        append(&path, "next entry");

        // Not a claim that embedded newlines are escaped — only that a later,
        // well-formed entry is still readable after one that was not.
        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.trim_end().ends_with("next entry"));

        let _ = fs::remove_dir_all(&dir);
    }
}
