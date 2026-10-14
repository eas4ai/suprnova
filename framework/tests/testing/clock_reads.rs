//! No new direct reads of the wall clock in `framework/src`.
//!
//! Every read of "the time now" goes through `crate::clock::now()`, so a
//! test that moves `TestClock` moves the whole framework. A direct
//! `Utc::now()` is a place a test cannot reach: an expiry it cannot cross
//! without sleeping. This scan fails with the `path:line` of each one that is
//! not in [`EXCEPTIONS`].
//!
//! The scan is textual: it reads every `.rs` file under `framework/src`,
//! skips comment lines (a doc comment may name the call), and flags a line
//! that spells it. The needle is split with `concat!` so this file does not
//! spell it either.

use std::path::{Path, PathBuf};

/// Spellings of a read of the wall clock, matched with the whitespace of the
/// line removed, so `Utc :: now()` is caught too.
const NEEDLES: [&str; 2] = [concat!("Utc::", "now("), concat!("Local::", "now(")];

/// The one file whose job is to read the system clock.
const CLOCK_MODULE: &str = "clock.rs";

/// Reads that stay on the system clock: (file relative to `framework/src`,
/// the trimmed text of the line, why it is not routed through the clock).
const EXCEPTIONS: [(&str, &str, &str); 2] = [
    (
        "server.rs",
        concat!("let timestamp = Utc::", "now().to_rfc3339();"),
        "The health endpoint reports the time of this machine to an outside \
         probe; the value is not compared with anything the framework stores.",
    ),
    (
        "workflow/mod.rs",
        concat!("let wall_now = chrono::Utc::", "now().naive_utc();"),
        "The retry time of a workflow run is compared with NOW() of the \
         database when a worker claims the run, so both sides stay on the \
         wall clock.",
    ),
];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|entry| entry.expect("dir entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn framework_source_reads_the_clock_through_the_clock_module() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    assert!(!files.is_empty(), "the scan found no source files");

    let mut offenders = Vec::new();
    for path in files {
        let relative = path
            .strip_prefix(&src)
            .expect("under src")
            .to_string_lossy()
            .replace('\\', "/");
        if relative == CLOCK_MODULE {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        for (index, line) in text.lines().enumerate() {
            let trimmed = line.trim();
            let compact: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
            if trimmed.starts_with("//") || !NEEDLES.iter().any(|needle| compact.contains(needle)) {
                continue;
            }
            let excepted = EXCEPTIONS
                .iter()
                .any(|(file, text, _reason)| *file == relative && *text == trimmed);
            if !excepted {
                offenders.push(format!("framework/src/{relative}:{}", index + 1));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "read the clock with `crate::clock::now()`, not the system clock, so a test can move it:\n{}",
        offenders.join("\n")
    );
}
