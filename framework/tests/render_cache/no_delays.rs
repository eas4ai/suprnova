//! The RenderCache test tree synchronizes on observed state, never on time.
//!
//! The engine crate carries a `clippy.toml` that disallows `sleep`,
//! `yield_now`, `spin_loop`, and `park_timeout`, and the live gate enforces
//! it with `-D clippy::disallowed_methods`. Neither half of that reaches
//! here: the list is configured under `crates/suprnova-live/`, so it does
//! not apply to the framework crate at all, and an integration test crate
//! under `framework/tests/` is compiled separately from the library
//! besides. This test is that rule, for the one tree where breaking it
//! would be worst - a suite whose whole subject is what happens in windows
//! a few microseconds wide, where a timing wait does not fix a race, it
//! hides one and then reopens it on a loaded machine.
//!
//! It is a text scan, deliberately: it has to catch a wait in any of the
//! several forms Rust spells one in - a thread parked for a `Duration`, a
//! timer awaited on the runtime, a spin loop - including forms no clippy
//! lint in this repository knows about, and it has to do so without
//! compiling anything.
//!
//! The prose here therefore never spells a forbidden call out in full: the
//! scan reads this file too (see [`FORBIDDEN`]), and a doc comment naming
//! one would be an offender like any other line. That is the rule working,
//! not a limitation of it.
use std::path::Path;

/// The needles, each split across a `concat!` so this file's own table
/// does not read as an offender when the scan reaches this file (it does:
/// `no_delays.rs` lives in one of the directories below, and exempting it
/// by name would leave the checker the one file in the tree that may
/// contain a timing wait).
const FORBIDDEN: [&str; 5] = [
    concat!("sleep", "("),
    concat!("yield_now", "("),
    concat!("spin_loop", "("),
    concat!("park_timeout", "("),
    concat!("thread::", "sleep"),
];

/// A database-side lock hold inside a SQL string, not a test wait. It runs
/// on the server, inside a transaction whose lock another connection is
/// then proven to wait on; nothing in the test process sleeps.
const ALLOWED_LINES: [&str; 1] = ["PERFORM pg_sleep(0.2);"];

#[test]
fn the_render_cache_tests_and_their_support_contain_no_timing_waits() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut offenders = Vec::new();
    walk(&root.join("render_cache"), Scope::Everything, &mut offenders);
    walk(&root.join("support"), Scope::RenderCacheOnly, &mut offenders);
    assert!(
        offenders.is_empty(),
        "timing waits in RenderCache tests:\n{}",
        offenders.join("\n")
    );
}

/// Whether the entries directly inside the directory being scanned are
/// filtered by name.
///
/// `tests/support` is shared by every suite in this crate, so only its
/// `render_cache*` entries belong to this rule - the rest belong to suites
/// it was not written for, and sweeping them would make a claim about trees
/// this test does not own. The filter applies to that one level: files
/// sitting directly in `tests/support` (`common.rs`, `env_lock.rs`,
/// `magnetar_auth.rs`) are outside it exactly as the sibling directories
/// are, which is what this doc has always said and what Task 5b's review
/// found the walker was not yet doing. Everything *below* a selected module
/// is that module's own, so it is scanned whole.
#[derive(Clone, Copy)]
enum Scope {
    /// Scan every entry.
    Everything,
    /// Scan only entries whose own name starts with `render_cache`.
    RenderCacheOnly,
}

impl Scope {
    /// Whether an entry named `name` directly inside the scanned directory
    /// is this rule's to scan.
    fn admits(self, name: &str) -> bool {
        match self {
            Self::Everything => true,
            Self::RenderCacheOnly => name.starts_with("render_cache"),
        }
    }
}

/// Scans every `.rs` file under `dir` that `scope` admits, recursing into
/// admitted subdirectories with [`Scope::Everything`]: the filter names
/// which *modules* this rule owns, and it owns each of them entirely.
fn walk(dir: &Path, scope: Scope, offenders: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).expect("readable test dir") {
        let path = entry.expect("entry").path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !scope.admits(name) {
            continue;
        }
        if path.is_dir() {
            walk(&path, Scope::Everything, offenders);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).expect("utf-8 source");
            for (index, line) in text.lines().enumerate() {
                if ALLOWED_LINES.iter().any(|allowed| line.contains(allowed)) {
                    continue;
                }
                if FORBIDDEN.iter().any(|f| line.contains(f)) {
                    offenders.push(format!("{}:{}: {}", path.display(), index + 1, line.trim()));
                }
            }
        }
    }
}
