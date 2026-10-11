// The commit the CLI is built from, as git reports it (PAR-080). The build
// script pulls this file in with `include!`, and `tests/build_commit.rs`
// mounts it as a module, so the test checks the same classification the
// build records. It holds items only, spelled with full paths, so it adds
// no `use` that could clash with the build script's own.

/// The commit this CLI is built from, as git reports it.
pub(crate) struct BuildCommit {
    /// HEAD's full commit hash.
    pub(crate) sha: String,
    /// Whether the local tag `v<version>` names HEAD.
    pub(crate) tagged: bool,
}

/// HEAD's commit and whether it is the release `v<version>`, or `None`
/// when git cannot answer or `workspace` is not the top of its own
/// repository.
///
/// HEAD is the release only when the local tag `v<version>` names it. With
/// no such tag, or with the tag on another commit, the build is untagged,
/// so the scaffold pins HEAD. The history cannot stand in for a missing
/// tag: in a clone one commit deep, HEAD is the shallow root and seems to
/// add the version line, so it would pass for the release whatever it is.
/// A release built from a checkout without its tag, as Cargo's own git
/// checkouts usually are, pins the release commit's hash, which names the
/// same source as the tag.
pub(crate) fn build_commit(workspace: &std::path::Path, version: &str) -> Option<BuildCommit> {
    let top = git(workspace, &["rev-parse", "--show-toplevel"])?;
    if std::fs::canonicalize(&top).ok()? != std::fs::canonicalize(workspace).ok()? {
        return None;
    }
    let sha = git(workspace, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    if !commit_hash(&sha) {
        return None;
    }
    let tag = format!("refs/tags/v{version}^{{commit}}");
    let tagged = git(workspace, &["rev-parse", "-q", "--verify", &tag])
        .is_some_and(|release| release == sha);
    Some(BuildCommit { sha, tagged })
}

/// Runs git in `directory` and returns its trimmed output, or `None` when
/// git is missing, fails or prints nothing. `GIT_DIR` and `GIT_WORK_TREE`
/// are cleared so a build started from a git hook still reads the
/// repository the sources are in.
fn git(directory: &std::path::Path, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git")
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
