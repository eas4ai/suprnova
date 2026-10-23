//! Every `--sn-` token a shipped stylesheet, view, or script reads resolves.
//!
//! A `var(--sn-...)` that names no defined custom property computes to the
//! property's inherited or initial value, so the component silently ignores
//! every theme. The token stylesheet defines the shared tokens, a component may
//! define its own, and the Tailwind preset maps every color token.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn live_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// The `--sn-` custom properties `text` declares (`--sn-x:`) and the ones it
/// reads (`var(--sn-x`), by name.
fn tokens(text: &str) -> (BTreeSet<String>, BTreeSet<String>) {
    let (mut declared, mut read) = (BTreeSet::new(), BTreeSet::new());
    let mut rest = text;
    while let Some(at) = rest.find("--sn-") {
        let before = rest[..at].trim_end();
        let tail = &rest[at..];
        let length = tail
            .char_indices()
            .skip(2)
            .find(|(_, c)| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-'))
            .map_or(tail.len(), |(index, _)| index);
        let name = &tail[..length];
        if name.len() > "--sn-".len() && !name.ends_with('-') {
            if before.ends_with("var(") {
                read.insert(name.to_owned());
            } else if tail[length..].trim_start().starts_with(':') {
                declared.insert(name.to_owned());
            }
        }
        rest = &tail[length..];
    }
    (declared, read)
}

fn token_stylesheet() -> String {
    read(&live_root().join("browser/src/styles/suprnova-ui.css"))
}

fn component_directories() -> Vec<PathBuf> {
    let root = live_root().join("components");
    let mut directories: Vec<PathBuf> = fs::read_dir(&root)
        .unwrap_or_else(|error| panic!("{}: {error}", root.display()))
        .map(|entry| entry.expect("component directory entry").path())
        .filter(|path| path.is_dir())
        .collect();
    directories.sort();
    directories
}

#[test]
fn every_token_a_shipped_file_reads_is_defined() {
    let (shared, shared_reads) = tokens(&token_stylesheet());
    let mut unresolved = Vec::new();
    for name in shared_reads.difference(&shared) {
        unresolved.push(format!("suprnova-ui.css reads {name}"));
    }
    let preset = read(&live_root().join("browser/src/styles/suprnova-ui.tailwind.css"));
    for name in tokens(&preset).1.difference(&shared) {
        unresolved.push(format!("suprnova-ui.tailwind.css reads {name}"));
    }
    let directories = component_directories();
    assert!(
        directories.len() > 50,
        "the component library is missing: {} directories",
        directories.len()
    );
    for directory in directories {
        let mut files: Vec<PathBuf> = fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("{}: {error}", directory.display()))
            .map(|entry| entry.expect("component file entry").path())
            .filter(|path| {
                path.extension().is_some_and(|extension| {
                    ["css", "html", "js"].contains(&extension.to_str().unwrap_or(""))
                })
            })
            .collect();
        files.sort();
        let texts: Vec<(PathBuf, String)> = files
            .into_iter()
            .map(|path| {
                let text = read(&path);
                (path, text)
            })
            .collect();
        let local: BTreeSet<String> = texts.iter().flat_map(|(_, text)| tokens(text).0).collect();
        for (path, text) in &texts {
            for name in tokens(text).1 {
                if !shared.contains(&name) && !local.contains(&name) {
                    let file = path.strip_prefix(live_root()).unwrap_or(path);
                    unresolved.push(format!("{} reads {name}", file.display()));
                }
            }
        }
    }
    assert!(
        unresolved.is_empty(),
        "tokens read but never defined:\n{}",
        unresolved.join("\n")
    );
}

#[test]
fn the_tailwind_preset_maps_every_color_token() {
    let (shared, _) = tokens(&token_stylesheet());
    let preset = read(&live_root().join("browser/src/styles/suprnova-ui.tailwind.css"));
    let mapped = tokens(&preset).1;
    let missing: Vec<&String> = shared
        .iter()
        .filter(|name| name.starts_with("--sn-color-") && !mapped.contains(*name))
        .collect();
    assert!(
        missing.is_empty(),
        "color tokens the preset does not map: {missing:?}"
    );
}

#[test]
fn the_scanner_tells_declarations_from_reads() {
    let (declared, read) = tokens(
        ":root { --sn-color-a: red; --sn-color-b : var(--sn-color-a); }\n\
         .x { color: var( --sn-color-c, blue); }\n\
         /* every --sn-color-* role, and the `--sn-` prefix */",
    );
    assert_eq!(
        declared,
        BTreeSet::from(["--sn-color-a".to_owned(), "--sn-color-b".to_owned()])
    );
    assert_eq!(
        read,
        BTreeSet::from(["--sn-color-a".to_owned(), "--sn-color-c".to_owned()])
    );
}
