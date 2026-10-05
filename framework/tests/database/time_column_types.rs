//! No framework-owned migration declares a time column that MySQL creates as
//! `TIMESTAMP`.
//!
//! sea-query renders `timestamp_with_time_zone()` and `timestamp()` as
//! `TIMESTAMP` on MySQL and MariaDB, which refuses any time after
//! 2038-01-19 03:14:07 UTC. A framework migration declares its time columns
//! through `migration_guard::utc_timestamp_column`, which is `DATETIME`
//! there. This scan reads every `.rs` file under `framework/src` and
//! Magnetar's `src`, skips comment lines, and fails with the `path:line` of
//! each spelling that is not in [`EXCEPTIONS`]. The needles are split with
//! `concat!` so this file does not spell them either.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The column builder that renders `TIMESTAMP` on MySQL, matched with the
/// whitespace of the line removed.
const WITH_ZONE: &str = concat!(".timestamp_with", "_time_zone()");
/// `timestamp()` is also chrono's, so a builder line is one that starts with
/// it in a file that builds columns.
const BARE: &str = concat!(".timest", "amp()");

/// Files that still spell one, how many lines, and why.
const EXCEPTIONS: [(&str, usize, &str); 7] = [
    (
        "framework/src/schema/column.rs",
        1,
        "The schema builder an application's own migrations use: its timestamp \
         column follows Laravel's, and its tables are the application's.",
    ),
    (
        "framework/src/payments/migrations/m_2026_05_22_000001_create_payments_tables.rs",
        16,
        "The payments tables are still TIMESTAMP on MySQL until their upgrade lands.",
    ),
    (
        "framework/src/rbac/migrations/m_create_rbac_tables.rs",
        4,
        "The RBAC tables are still TIMESTAMP on MySQL until their upgrade lands.",
    ),
    (
        "framework/src/auth_flows/two_factor/migration.rs",
        3,
        "The two-factor tables are still TIMESTAMP on MySQL until their upgrade lands.",
    ),
    (
        "framework/src/auth_flows/two_factor/migration_attempts.rs",
        1,
        "The two-factor tables are still TIMESTAMP on MySQL until their upgrade lands.",
    ),
    (
        "framework/src/auth_flows/two_factor/migration_rotation.rs",
        1,
        "The two-factor tables are still TIMESTAMP on MySQL until their upgrade lands.",
    ),
    (
        "crates/suprnova-magnetar/src/default_schema.rs",
        1,
        "Magnetar's default tables are still TIMESTAMP on MySQL until their upgrade \
         lands.",
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
fn framework_migrations_declare_no_mysql_timestamp_column() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the workspace root")
        .to_path_buf();
    let mut files = Vec::new();
    rust_files(&root.join("framework/src"), &mut files);
    rust_files(&root.join("crates/suprnova-magnetar/src"), &mut files);
    assert!(!files.is_empty(), "the scan found no source files");

    let mut found: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for path in files {
        let relative = path
            .strip_prefix(&root)
            .expect("under the workspace root")
            .to_string_lossy()
            .replace('\\', "/");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let builds_columns = text.contains("ColumnDef");
        for (index, line) in text.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with("//") {
                continue;
            }
            let compact: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
            if compact.contains(WITH_ZONE) || (builds_columns && compact.starts_with(BARE)) {
                found.entry(relative.clone()).or_default().push(index + 1);
            }
        }
    }

    let mut offenders = Vec::new();
    for (file, lines) in &found {
        match EXCEPTIONS.iter().find(|(path, _, _)| path == file) {
            Some((_, count, _)) if *count == lines.len() => {}
            Some((_, count, _)) => offenders.push(format!(
                "{file}: {} lines, the exception names {count}; update it: {lines:?}",
                lines.len()
            )),
            None => {
                for line in lines {
                    offenders.push(format!("{file}:{line}"));
                }
            }
        }
    }
    for (path, _, _) in EXCEPTIONS {
        if !found.contains_key(path) {
            offenders.push(format!("{path}: no longer spells one; drop its exception"));
        }
    }
    assert!(
        offenders.is_empty(),
        "a framework time column is TIMESTAMP on MySQL, which stops at 2038-01-19; declare \
         it through migration_guard::utc_timestamp_column:\n{}",
        offenders.join("\n")
    );
}
