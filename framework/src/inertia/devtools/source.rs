//! Where in the application's code a page was rendered: Laravel's
//! `SourceLocator`.
//!
//! Laravel walks a backtrace. Here the render calls take
//! `#[track_caller]`, so the compiler hands over the file and line of the
//! call.

use std::collections::HashMap;
use std::panic::Location;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{Value, json};

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

    /// The `{file, line}` object an entry carries: the file as a path on
    /// this machine when it can be found from the working directory, so
    /// the extension can open it, else as the compiler named it.
    pub(crate) fn to_json(self) -> Value {
        let file = resolve(self.file).map_or_else(
            || self.file.to_string(),
            |path| path.display().to_string(),
        );
        json!({"file": file, "line": self.line})
    }
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
    fn indt_a_call_site_resolves_to_this_file() {
        #[track_caller]
        fn here() -> SourceLocation {
            SourceLocation::of(Location::caller())
        }
        let source = here();
        let rendered = source.to_json();
        assert!(
            rendered["file"].as_str().unwrap().ends_with("source.rs"),
            "{rendered}"
        );
        assert_eq!(rendered["line"], line!() - 6);
    }
}
