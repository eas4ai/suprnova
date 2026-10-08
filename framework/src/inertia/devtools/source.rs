//! Where in the application's code a page was rendered and a prop was
//! shared: Laravel's `SourceLocator`.
//!
//! Laravel walks a backtrace and scans the PHP file for a prop's key. Here
//! the render and share calls, and `InertiaConfig::hooks`, take
//! `#[track_caller]`, so the compiler hands over the file and line of the
//! call, and the file is scanned from that line for the line that names a
//! prop's key, as Laravel's `findPropKeyLine` does: `"key":` in the props
//! of `inertia_response!`, `("key",` in a builder call.

use std::collections::HashMap;
use std::panic::Location;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{Value, json};

/// How many lines past the call a prop's key is looked for, Laravel's
/// default scan window.
const SCAN_LINES: u32 = 100;

/// A file and line in the application's code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SourceLocation {
    /// The file as the compiler named it.
    file: &'static str,
    /// The line, 1-based.
    line: u32,
}

impl SourceLocation {
    /// The location of a `#[track_caller]` call.
    pub(crate) fn of(location: &'static Location<'static>) -> Self {
        Self {
            file: location.file(),
            line: location.line(),
        }
    }

    /// The same file at `line`.
    fn at(self, line: u32) -> Self {
        Self { line, ..self }
    }

    /// The text of this source's file, when it can be read.
    pub(crate) fn text(&self) -> Option<String> {
        std::fs::read_to_string(resolve(self.file)?).ok()
    }

    /// The line from this one on, within the scan window, that names the
    /// prop `key`; `None` when no line does or the file cannot be read.
    pub(crate) fn find_key_line(&self, key: &str) -> Option<u32> {
        self.find_key_line_in(&self.text()?, key)
    }

    /// [`find_key_line`](Self::find_key_line) in the file's text `text`,
    /// read once for many keys.
    pub(crate) fn find_key_line_in(&self, text: &str, key: &str) -> Option<u32> {
        let quoted = format!("\"{key}\"");
        text.lines()
            .enumerate()
            .skip(self.line.saturating_sub(1) as usize)
            .take(SCAN_LINES as usize)
            .find(|(_, line)| names_key(line, &quoted))
            .and_then(|(index, _)| u32::try_from(index + 1).ok())
    }

    /// This source moved to the line naming `key`, or left where it is.
    pub(crate) fn refined_for(self, key: &str) -> Self {
        match self.find_key_line(key) {
            Some(line) => self.at(line),
            None => self,
        }
    }

    /// The `{file, line}` object an entry carries: the file as a path on
    /// this machine when it can be found from the working directory, so
    /// the extension can open it, else as the compiler named it.
    pub(crate) fn to_json(self) -> Value {
        let file = resolve(self.file)
            .map_or_else(|| self.file.to_string(), |path| path.display().to_string());
        json!({"file": file, "line": self.line})
    }
}

/// Whether `line` names the quoted key: followed by `:` as in the props of
/// `inertia_response!`, or by `,` as in a builder call's first argument.
fn names_key(line: &str, quoted: &str) -> bool {
    line.match_indices(quoted).any(|(at, _)| {
        line[at + quoted.len()..]
            .trim_start()
            .starts_with([':', ','])
    })
}

/// The files already looked for, by the name the compiler gave them.
static RESOLVED: Mutex<Option<HashMap<&'static str, Option<PathBuf>>>> = Mutex::new(None);

/// Where the file the compiler named `file` is on disk.
///
/// The compiler names a file relative to the directory it ran in, the
/// workspace or the crate, and the process may run in either or below
/// them, so the name is tried against the working directory and each
/// directory above it. A file that is found once is remembered.
fn resolve(file: &'static str) -> Option<PathBuf> {
    let mut cache = crate::lock::recover(&RESOLVED);
    let cache = cache.get_or_insert_with(HashMap::new);
    if let Some(found) = cache.get(file) {
        return found.clone();
    }
    let found = locate(Path::new(file));
    cache.insert(file, found.clone());
    found
}

/// The first existing file `file` names, as given when it is absolute,
/// else under the working directory or one of its ancestors.
fn locate(file: &Path) -> Option<PathBuf> {
    if file.is_absolute() {
        return file.is_file().then(|| file.to_path_buf());
    }
    let cwd = std::env::current_dir().ok()?;
    cwd.ancestors()
        .map(|dir| dir.join(file))
        .find(|candidate| candidate.is_file())
        .map(|candidate| std::fs::canonicalize(&candidate).unwrap_or(candidate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indt_a_key_is_found_after_its_call_in_either_syntax() {
        assert!(names_key(r#"    "users": users,"#, "\"users\""));
        assert!(names_key(r#"    .with("users", users)"#, "\"users\""));
        assert!(names_key(r#"    .with( "users" , users)"#, "\"users\""));
        assert!(!names_key(r#"    let label = "users";"#, "\"users\""));
        assert!(!names_key(r#"    "users_count": 3,"#, "\"users\""));
    }

    #[test]
    fn indt_this_file_resolves_and_its_key_lines_are_found() {
        #[track_caller]
        fn here() -> SourceLocation {
            SourceLocation::of(Location::caller())
        }
        let source = here();
        let marker = source.find_key_line("indt_marker_key");
        // The marker is below the call, inside the scan window.
        let _ = ("indt_marker_key", 1);
        assert!(marker.is_some_and(|line| line > source.line), "{marker:?}");
        let rendered = source.to_json();
        assert!(
            rendered["file"].as_str().unwrap().ends_with("source.rs"),
            "{rendered}"
        );
    }
}
