//! Embeds the shipped Live component library so `live:add` can install a
//! component from the binary alone (Cairn UI-017).
//!
//! Every directory under `crates/suprnova-live/components/` is one component:
//! a `manifest.json` naming the component and its files, beside those files.
//! The build walks the directory and writes a table of `include_str!` paths
//! into `OUT_DIR`, so the binary carries exactly the reviewed bytes and a new
//! component needs no hand-kept list.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let components = manifest_dir.join("../crates/suprnova-live/components");
    println!("cargo:rerun-if-changed={}", components.display());
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("live_components.rs");
    let mut source = String::from(
        "/// The shipped Live component library, one entry per component directory.\n\
         pub static COMPONENTS: &[EmbeddedComponent] = &[\n",
    );
    let mut directories: Vec<PathBuf> = match fs::read_dir(&components) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect(),
        Err(_) => Vec::new(),
    };
    directories.sort();
    for directory in directories {
        let manifest = directory.join("manifest.json");
        if !manifest.is_file() {
            continue;
        }
        println!("cargo:rerun-if-changed={}", manifest.display());
        let name = directory
            .file_name()
            .and_then(|name| name.to_str())
            .expect("component directory name")
            .to_owned();
        let mut files: Vec<PathBuf> = fs::read_dir(&directory)
            .expect("component directory")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_file())
            .collect();
        files.sort();
        source.push_str(&format!(
            "    EmbeddedComponent {{\n        directory: {name:?},\n        manifest: include_str!({:?}),\n        files: &[\n",
            absolute(&manifest)
        ));
        for file in files {
            let file_name = file
                .file_name()
                .and_then(|name| name.to_str())
                .expect("component file name")
                .to_owned();
            println!("cargo:rerun-if-changed={}", file.display());
            source.push_str(&format!(
                "            ({file_name:?}, include_str!({:?})),\n",
                absolute(&file)
            ));
        }
        source.push_str("        ],\n    },\n");
    }
    source.push_str("];\n");
    fs::write(out, source).expect("write the embedded component table");
}

fn absolute(path: &Path) -> String {
    fs::canonicalize(path)
        .expect("component path canonicalizes")
        .to_str()
        .expect("component path is UTF-8")
        .to_owned()
}
