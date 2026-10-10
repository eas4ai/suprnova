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
//! full hash, and `SUPRNOVA_CLI_BUILD_TAGGED`, whether HEAD is the release
//! `v<version>`. A CLI built from main between two releases has templates
//! that may already use API the last tag lacks, so its scaffold must pin
//! the commit, not the tag. When git cannot answer, the script records
//! nothing and the scaffold pins the tag.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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

/// The commit this CLI is built from, as git reports it.
struct BuildCommit {
    /// HEAD's full commit hash.
    sha: String,
    /// Whether HEAD is the release `v<version>`.
    tagged: bool,
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

/// HEAD's commit and whether it is the release `v<version>`.
///
/// A local tag `v<version>` answers directly. Cargo's own git checkouts,
/// which `cargo install --git` builds from, often carry no tags, so with
/// no such tag the answer comes from the history instead: a release
/// commit is the version bump, the commit that sets the workspace version,
/// so HEAD is the release when it is the newest commit that adds
/// `version = "<version>"` to the root `Cargo.toml`. In a clone one commit
/// deep, HEAD itself seems to add the line, so the scaffold pins the tag.
fn build_commit(workspace: &Path, version: &str) -> Option<BuildCommit> {
    let top = git(workspace, &["rev-parse", "--show-toplevel"])?;
    if fs::canonicalize(&top).ok()? != fs::canonicalize(workspace).ok()? {
        return None;
    }
    let sha = git(workspace, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    if !commit_hash(&sha) {
        return None;
    }
    let tag = format!("refs/tags/v{version}^{{commit}}");
    let release = match git(workspace, &["rev-parse", "-q", "--verify", &tag]) {
        Some(tagged) => tagged,
        None => {
            let line = format!("-Sversion = \"{version}\"");
            git(
                workspace,
                &["log", "-n", "1", "--format=%H", &line, "--", "Cargo.toml"],
            )?
        }
    };
    Some(BuildCommit {
        tagged: release == sha,
        sha,
    })
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

/// Runs git in `directory` and returns its trimmed output, or `None` when
/// git is missing, fails or prints nothing. `GIT_DIR` and `GIT_WORK_TREE`
/// are cleared so a build started from a git hook still reads the
/// repository the sources are in.
fn git(directory: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// Whether `text` is a full commit hash: 40 hex digits for SHA-1, 64 for
/// SHA-256, in lowercase as git prints them.
fn commit_hash(text: &str) -> bool {
    matches!(text.len(), 40 | 64)
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
