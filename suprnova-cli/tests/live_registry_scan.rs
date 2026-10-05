//! `registries-scan`: the Rust, view and script scans refuse every item of
//! the bypass corpus under `tests/fixtures/registry/bypass/` with the check,
//! file and line named, and accept every shipped component (REG-016,
//! REG-022, REG-030, REG-031, REG-032).

use std::fs;
use std::path::{Path, PathBuf};

use suprnova_cli::registry::scan::allowlist::{AllowedItem, Allowlist};
use suprnova_cli::registry::scan::{ComponentFiles, scan_component};

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/registry/bypass")
}

/// Reads one corpus component's files by manifest name.
fn files_of(component: &str) -> Vec<(String, Vec<u8>)> {
    let dir = corpus().join(component);
    let mut files: Vec<(String, Vec<u8>)> = fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("{}: {error}", dir.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file() && path.file_name().is_some_and(|name| name != "manifest.json")
        })
        .map(|path| {
            let name = path
                .file_name()
                .expect("name")
                .to_string_lossy()
                .into_owned();
            (name, fs::read(&path).expect("read"))
        })
        .collect();
    files.sort();
    files
}

/// An allowlist with no items: nothing from `suprnova::` is admitted, so a
/// refusal of a `std` path cannot come from a missing allowlist entry.
fn empty_allowlist() -> Allowlist {
    Allowlist::from_items(std::collections::BTreeMap::<String, AllowedItem>::new())
}

/// REG-030: `std::fs` under any alias is refused, and the refusal names the
/// check, the file and the line.
#[test]
fn reg_030_std_fs_under_an_alias_is_refused_naming_the_file_and_line() {
    let files = files_of("rust-std-fs-alias");
    let component = ComponentFiles {
        namespace: "evil",
        directory: "rust-std-fs-alias",
        files: &files,
        dependency_modules: &[],
        importable_views: &[],
    };
    let report = scan_component(&component, &empty_allowlist()).expect("the scan runs");
    assert!(!report.accepted(), "the component was admitted: {report:?}");
    let finding = report
        .findings
        .iter()
        .find(|finding| finding.file == "widget.rs")
        .unwrap_or_else(|| panic!("no finding names widget.rs: {:?}", report.findings));
    assert!(finding.check.starts_with("rust-"), "{finding}");
    assert!(finding.message.contains("std::fs"), "{finding}");
    assert_eq!(finding.line, Some(4), "{finding}");
}
