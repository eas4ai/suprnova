//! What the `make:*` generators that take a nested name share:
//! `make:middleware Admin/EnsureRole`, `make:view admin.dashboard` and
//! `make:inertia Admin/Users`.
//!
//! A nested name puts the new file in a subdirectory, so a generator has to
//! declare every module on the way to it, and with `--test` it writes a test
//! that imports the new item from the application's library crate. The
//! files of one run are planned in memory and written as one unit: every
//! path is checked first, each file is written atomically, and a failed
//! write puts back every file the run created or changed.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::commands::cargo_meta;
use crate::secure_fs;

/// Split a generator name on `/` and `\`, and on `.` when `dots` is set
/// (`make:view` reads `admin.dashboard` as Laravel's `make:view` does).
/// Refuses a name with an empty segment, so `Admin//X`, a leading `/` and a
/// trailing `.` never reach a path.
pub fn segments(raw: &str, dots: bool) -> Result<Vec<String>, String> {
    let name = raw.trim();
    let parts: Vec<String> = name
        .split(|ch| ch == '/' || ch == '\\' || (dots && ch == '.'))
        .map(str::to_string)
        .collect();
    if name.is_empty() || parts.iter().any(String::is_empty) {
        return Err(format!("'{raw}' is not a valid name: a segment is empty"));
    }
    Ok(parts)
}

/// Whether `source` declares module `name`, with or without a visibility
/// (`mod name;`, `pub mod name;`, `pub(crate) mod name { .. }`).
pub fn declares_module(source: &str, name: &str) -> bool {
    source
        .lines()
        .any(|line| module_declared_by(line).is_some_and(|declared| declared == name))
}

/// The module a line declares, if it is a `mod` item.
fn module_declared_by(line: &str) -> Option<&str> {
    let mut rest = line.trim_start();
    if let Some(after) = rest.strip_prefix("pub") {
        rest = if let Some(scoped) = after.strip_prefix('(') {
            scoped.split_once(')')?.1
        } else if after.starts_with(char::is_whitespace) {
            after
        } else {
            return None;
        }
        .trim_start();
    }
    let rest = rest.strip_prefix("mod ")?.trim_start();
    let end = rest
        .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .unwrap_or(rest.len());
    let (name, tail) = rest.split_at(end);
    let tail = tail.trim_start();
    (!name.is_empty() && (tail.starts_with(';') || tail.starts_with('{'))).then_some(name)
}

/// Insert `line` after the last `mod` declaration of `source`, or, when it
/// declares none, after its leading `//!` comment and a blank line.
pub fn insert_declaration(source: &str, line: &str) -> String {
    let mut lines: Vec<&str> = source.lines().collect();
    match lines
        .iter()
        .rposition(|existing| module_declared_by(existing).is_some())
    {
        Some(index) => lines.insert(index + 1, line),
        None => {
            let doc_end = lines
                .iter()
                .position(|existing| !existing.starts_with("//!"))
                .unwrap_or(lines.len());
            if doc_end == 0 {
                lines.insert(0, line);
                if lines.len() > 1 && !lines[1].is_empty() {
                    lines.insert(1, "");
                }
            } else {
                let at = if lines.get(doc_end).is_some_and(|next| next.is_empty()) {
                    doc_end + 1
                } else {
                    lines.insert(doc_end, "");
                    doc_end + 1
                };
                lines.insert(at, line);
                if lines.get(at + 1).is_some_and(|next| !next.is_empty()) {
                    lines.insert(at + 1, "");
                }
            }
        }
    }
    lines.join("\n") + "\n"
}

/// The file that holds module `name`'s items when it is a directory under
/// `dir`: `dir/name.rs` when the project already uses that layout, else
/// `dir/name/mod.rs`. Writing `mod.rs` beside an existing `name.rs` would
/// make the module ambiguous, which rustc refuses.
pub fn module_file(dir: &Path, name: &str) -> PathBuf {
    let flat = dir.join(format!("{name}.rs"));
    if flat.is_file() {
        flat
    } else {
        dir.join(name).join("mod.rs")
    }
}

/// The name the application's library crate is imported by in a test under
/// `tests/`: `[lib] name` when the manifest sets one, else the package name
/// with `-` read as `_`. A `--test` run needs `src/lib.rs`, since a test
/// links the library, not a binary.
pub fn test_crate_name() -> Result<String, String> {
    let manifest = fs::read_to_string("Cargo.toml").map_err(|_| {
        "--test needs a Cargo.toml in the current directory (run it from the project root)"
            .to_string()
    })?;
    if !Path::new("src/lib.rs").is_file() {
        return Err(
            "--test needs src/lib.rs: a test under tests/ imports from the library crate"
                .to_string(),
        );
    }
    let lib_name = cargo_meta::parse_cargo_toml(&manifest)
        .ok()
        .and_then(|table| table.get("lib")?.get("name")?.as_str().map(str::to_string));
    lib_name
        .or_else(|| cargo_meta::package_name_from_content(&manifest))
        .map(|name| name.replace('-', "_"))
        .ok_or_else(|| "Cargo.toml has no [package] name".to_string())
}

/// One planned file: its new content and, for a file that exists, the
/// content to put back if a later write of the run fails.
struct Planned {
    content: String,
    previous: Option<String>,
}

/// The files one generator run writes, kept in memory until [`Self::apply`].
#[derive(Default)]
pub struct WriteSet {
    order: Vec<PathBuf>,
    files: BTreeMap<PathBuf, Planned>,
}

/// What [`WriteSet::apply`] did to one file, for the generator's report.
pub enum Written {
    /// The file did not exist.
    Created(PathBuf),
    /// The file existed and now holds new content.
    Updated(PathBuf),
}

impl WriteSet {
    /// Plan `path` to hold `content`, replacing what the file holds now.
    pub fn put(&mut self, path: PathBuf, content: String) -> Result<(), String> {
        let previous = self.current(&path)?;
        self.record(path, content, previous);
        Ok(())
    }

    /// Plan an edit of `path`: `edit` receives the content the run would
    /// leave there so far (an earlier planned write, the file on disk, or
    /// `None` when there is neither) and returns the new content, or `None`
    /// to leave the file as it is.
    pub fn edit(
        &mut self,
        path: PathBuf,
        edit: impl FnOnce(Option<&str>) -> Option<String>,
    ) -> Result<(), String> {
        let now = match self.files.get(&path) {
            Some(planned) => Some(planned.content.clone()),
            None => self.current(&path)?,
        };
        if let Some(content) = edit(now.as_deref()) {
            // An earlier plan for the same file keeps its own `previous`
            // (see `record`), so `now` only matters for a first plan.
            self.record(path, content, now);
        }
        Ok(())
    }

    /// Plan `content` for `path`. The first plan of a file sets the content
    /// a rollback restores; a later plan of the same file replaces only the
    /// content.
    fn record(&mut self, path: PathBuf, content: String, previous: Option<String>) {
        if !self.files.contains_key(&path) {
            self.order.push(path.clone());
        }
        let previous = match self.files.remove(&path) {
            Some(existing) => existing.previous,
            None => previous,
        };
        self.files.insert(path, Planned { content, previous });
    }

    /// What `path` holds on disk, `None` when it does not exist.
    fn current(&self, path: &Path) -> Result<Option<String>, String> {
        secure_fs::ensure_contained(Path::new("."), path)?;
        if !path.exists() {
            return Ok(None);
        }
        fs::read_to_string(path)
            .map(Some)
            .map_err(|e| format!("Failed to read {}: {e}", path.display()))
    }

    /// Write every planned file, creating its directory first. A failed
    /// write puts back every file the run already wrote and removes the
    /// directories it created, and the error names any file it could not
    /// put back.
    pub fn apply(self) -> Result<Vec<Written>, String> {
        let mut created_dirs: Vec<PathBuf> = Vec::new();
        let mut done: Vec<&PathBuf> = Vec::new();
        let mut outcome = Ok(());
        for path in &self.order {
            let planned = &self.files[path];
            let step = secure_fs::ensure_contained(Path::new("."), path)
                .and_then(|()| create_parents(path, &mut created_dirs))
                .and_then(|()| secure_fs::write_atomic(path, planned.content.as_bytes()));
            if let Err(e) = step {
                outcome = Err(e);
                break;
            }
            done.push(path);
        }
        if let Err(error) = outcome {
            let mut unrestored = Vec::new();
            for path in done.iter().rev() {
                let restored = match &self.files[*path].previous {
                    Some(previous) => secure_fs::write_atomic(path, previous.as_bytes()),
                    None => fs::remove_file(path).map_err(|e| e.to_string()),
                };
                if let Err(e) = restored {
                    unrestored.push(format!("{} ({e})", path.display()));
                }
            }
            for dir in created_dirs.iter().rev() {
                let _ = fs::remove_dir(dir);
            }
            return Err(if unrestored.is_empty() {
                format!("{error}; nothing was written")
            } else {
                format!(
                    "{error}; these files could not be put back: {}",
                    unrestored.join(", ")
                )
            });
        }
        Ok(self
            .order
            .iter()
            .map(|path| match self.files[path].previous {
                Some(_) => Written::Updated(path.clone()),
                None => Written::Created(path.clone()),
            })
            .collect())
    }
}

/// Create the missing directories above `path`, outermost first, recording
/// each so a rollback can remove it.
fn create_parents(path: &Path, created: &mut Vec<PathBuf>) -> Result<(), String> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    let mut missing: Vec<&Path> = parent
        .ancestors()
        .take_while(|dir| !dir.as_os_str().is_empty() && !dir.exists())
        .collect();
    missing.reverse();
    for dir in missing {
        fs::create_dir(dir).map_err(|e| format!("Failed to create {}: {e}", dir.display()))?;
        created.push(dir.to_path_buf());
    }
    Ok(())
}

/// Print one line per file a run wrote.
pub fn report(written: &[Written]) {
    for file in written {
        match file {
            Written::Created(path) => crate::ui::success(&format!("Created {}", path.display())),
            Written::Updated(path) => crate::ui::success(&format!("Updated {}", path.display())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_split_on_slashes_and_optionally_dots() {
        assert_eq!(
            segments("Admin/EnsureRole", false).unwrap(),
            ["Admin", "EnsureRole"]
        );
        assert_eq!(segments("Admin\\Users", false).unwrap(), ["Admin", "Users"]);
        assert_eq!(
            segments("admin.dashboard", false).unwrap(),
            ["admin.dashboard"]
        );
        assert_eq!(
            segments("admin.dashboard", true).unwrap(),
            ["admin", "dashboard"]
        );
        for bad in ["", "/x", "x/", "a//b", ".x"] {
            assert!(segments(bad, true).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn declarations_are_found_with_any_visibility() {
        let source = "//! x\n\npub mod admin;\nmod logging;\npub(crate) mod tools { }\n";
        assert!(declares_module(source, "admin"));
        assert!(declares_module(source, "logging"));
        assert!(declares_module(source, "tools"));
        assert!(!declares_module(source, "log"));
        assert!(!declares_module("// mod admin;\n", "admin"));
        assert!(!declares_module("pubmod admin;\n", "admin"));
    }

    #[test]
    fn a_declaration_goes_after_the_last_one_or_after_the_doc_comment() {
        assert_eq!(
            insert_declaration("//! m\n\nmod a;\n\npub use a::A;\n", "pub mod b;"),
            "//! m\n\nmod a;\npub mod b;\n\npub use a::A;\n"
        );
        assert_eq!(
            insert_declaration("//! m\n", "pub mod b;"),
            "//! m\n\npub mod b;\n"
        );
        assert_eq!(
            insert_declaration("use x::y;\n", "pub mod b;"),
            "pub mod b;\n\nuse x::y;\n"
        );
    }
}
