use std::path::{Path, PathBuf};
use std::process::Command;
use std::{fs, io};

use super::path::repository_path;

pub const MANIFEST: &str = "tests/fixtures/manifest.json";

pub fn sha256(relative: &str) -> String {
    let output = Command::new("sha256sum")
        .arg(repository_path(relative))
        .output()
        .expect("sha256sum must be available to verify fixture checksums");
    assert!(output.status.success(), "sha256sum failed for {relative}");
    String::from_utf8(output.stdout)
        .expect("sha256sum output is UTF-8")
        .split_whitespace()
        .next()
        .expect("sha256sum output has a digest")
        .to_owned()
}

pub fn manifest() -> String {
    fs::read_to_string(repository_path(MANIFEST)).expect("fixture manifest must be readable")
}

pub fn json_string(line: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\":\"");
    let start = line.find(&needle)? + needle.len();
    let end = line[start..].find('"')? + start;
    Some(line[start..end].to_owned())
}

pub fn manifest_generated_fixture_paths(manifest: &str) -> Vec<String> {
    manifest
        .lines()
        .filter(|line| line.contains("\"status\":\"generated\""))
        .filter_map(|line| json_string(line, "fixture_path"))
        .collect()
}

pub fn files_under(relative: &str) -> Vec<String> {
    let root = repository_path(relative);
    let repository_root = repository_path("");
    let mut files = Vec::new();
    collect_files(&root, &mut files).expect("fixture directory must be readable");
    files
        .into_iter()
        .map(|path| {
            path.strip_prefix(&repository_root)
                .expect("fixture path must be inside repository")
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect()
}

fn collect_files(path: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let entry_path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_files(&entry_path, files)?;
        } else if entry.file_type()?.is_file() {
            files.push(entry_path);
        }
    }
    Ok(())
}
