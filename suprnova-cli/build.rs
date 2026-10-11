//! Embeds the shipped Live component library so `live:add` can install a
//! component from the binary alone (Cairn UI-017), as the same tree every
//! library has (REG-016, REG-025).
//!
//! Every directory under `crates/suprnova-live/components/` is one component:
//! a `manifest.json` naming the component and its files, beside those files.
//! The build walks the directory and writes a table of `include_str!` paths
//! into `OUT_DIR`, so the binary carries exactly the reviewed bytes and a new
//! component needs no hand-kept list. Beside the table it writes the shipped
//! library's `library.json`: the namespace and source `suprnova`, and the
//! CLI's own version as the library's version and its framework
//! requirement. The version is spelled with `env!` in the generated code,
//! so it is read when the crate compiles and can never disagree with the
//! version the CLI reports, whether or not this script ran again.
//!
//! It also records the commit the CLI is built from, for the framework pin
//! `suprnova new` writes (PAR-080): `SUPRNOVA_CLI_BUILD_COMMIT`, HEAD's
//! full hash, and `SUPRNOVA_CLI_BUILD_TAGGED`, whether the local tag
//! `v<version>` names HEAD. A CLI built from main between two releases has
//! templates that may already use API the last tag lacks, so its scaffold
//! must pin the commit, not the tag; so does any build without the release
//! tag on HEAD. When git cannot answer, the script records nothing and the
//! scaffold pins the tag. `build_commit.rs` holds the classification, so
//! `tests/build_commit.rs` can check it on real repositories.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// The generated code that spells the shipped library's `library.json`,
/// with the version read by `env!` when the crate compiles.
const LIBRARY_JSON: &str = r##"/// The shipped library's `library.json`: the keys a third-party library's
/// holds but the signing key, which the shipped library does not carry.
pub static LIBRARY_JSON: &str = concat!(
    "{\n  \"namespace\": \"suprnova\",\n  \"source\": \"suprnova\",\n  \"version\": \"",
    env!("CARGO_PKG_VERSION"),
    "\",\n  \"framework\": \"",
    env!("CARGO_PKG_VERSION"),
    "\"\n}\n",
);

"##;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    record_build_commit(&manifest_dir);
    let components = manifest_dir.join("../crates/suprnova-live/components");
    println!("cargo:rerun-if-changed={}", components.display());
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("live_components.rs");
    let mut source = String::from(LIBRARY_JSON);
    source.push_str(
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

/// Records the commit facts `templates::recorded_build_commit` reads, and
/// asks cargo to run this script again when HEAD moves or a tag changes.
/// Records nothing when git is missing or the CLI's sources are not the
/// top of a repository of their own, as in a tree vendored inside another
/// repository, so that repository's commit is never taken for the CLI's.
fn record_build_commit(manifest_dir: &Path) {
    let Some(workspace) = manifest_dir.parent() else {
        return;
    };
    let Ok(version) = env::var("CARGO_PKG_VERSION") else {
        return;
    };
    let Some(commit) = build_commit(workspace, &version) else {
        return;
    };
    for watched in watched_git_paths(workspace) {
        println!("cargo:rerun-if-changed={}", watched.display());
    }
    println!("cargo:rustc-env=SUPRNOVA_CLI_BUILD_COMMIT={}", commit.sha);
    println!(
        "cargo:rustc-env=SUPRNOVA_CLI_BUILD_TAGGED={}",
        commit.tagged
    );
}

/// The git files whose change can change [`build_commit`]'s answer: HEAD,
/// the branch it names, the packed refs and the tags. Only paths that
/// exist are returned, because cargo runs a script again on every build
/// while a path it watches is missing.
fn watched_git_paths(workspace: &Path) -> Vec<PathBuf> {
    let mut names = vec![
        "HEAD".to_owned(),
        "packed-refs".to_owned(),
        "refs/tags".to_owned(),
    ];
    if let Some(branch) = git(workspace, &["symbolic-ref", "-q", "HEAD"]) {
        names.push(branch);
    }
    names
        .iter()
        .filter_map(|name| git(workspace, &["rev-parse", "--git-path", name]))
        .map(|path| workspace.join(path))
        .filter(|path| path.exists())
        .collect()
}

include!("build_commit.rs");
