// The commit the CLI is built from, as git reports it (PAR-080). The build
// script pulls this file in with `include!`, and `tests/build_commit.rs`
// mounts it as a module, so the test checks the same classification the
// build records. It holds items only, spelled with full paths, so it adds
// no `use` that could clash with the build script's own.

/// The commit this CLI is built from, as git reports it.
pub(crate) struct BuildCommit {
    /// HEAD's full commit hash.
    pub(crate) sha: String,
    /// Whether the tag `v<version>` names HEAD: the local tag, or with no
    /// local tag, the tag an `origin` on this machine lists.
    pub(crate) tagged: bool,
}

/// HEAD's commit and whether it is the release `v<version>`, or `None`
/// when git cannot answer or `workspace` is not the top of its own
/// repository.
///
/// HEAD is the release when the local tag `v<version>` names it. With no
/// local tag, HEAD is still the release when the `origin` remote is a
/// repository on this machine that lists the tag on HEAD
/// ([`origin_tags_head`]). That is the case `cargo install --git --tag`
/// builds: Cargo's checkouts carry no tags, but their `origin` is Cargo's
/// git database on disk, which holds the tags Cargo fetched. With the tag
/// on another commit, or with no tag anywhere git can read offline, the
/// build is untagged, so the scaffold pins HEAD. The history cannot stand
/// in for a missing tag: in a clone one commit deep, HEAD is the shallow
/// root and seems to add the version line, so it would pass for the
/// release whatever it is.
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
    let tagged = match git(workspace, &["rev-parse", "-q", "--verify", &tag]) {
        Some(release) => release == sha,
        None => origin_tags_head(workspace, version, &sha),
    };
    Some(BuildCommit { sha, tagged })
}

/// Whether the `origin` remote lists the tag `v<version>` on `sha`, asked
/// only when `origin` is a repository on this machine, so no build ever
/// waits on the network.
///
/// Cargo's git database stores the tags it fetched as
/// `refs/remotes/origin/tags/v<version>`, and older Cargo versions stored
/// them as `refs/tags/v<version>`, so both names count. `git ls-remote`
/// runs without `--refs` so that an annotated tag, whose ref names the tag
/// object, also lists its peeled `^{}` entry, which names the commit.
fn origin_tags_head(workspace: &std::path::Path, version: &str, sha: &str) -> bool {
    let Some(url) = git(workspace, &["remote", "get-url", "origin"]) else {
        return false;
    };
    if !local_url(&url) {
        return false;
    }
    let Some(listing) = git(workspace, &["ls-remote", "origin"]) else {
        return false;
    };
    let names = [
        format!("refs/tags/v{version}"),
        format!("refs/remotes/origin/tags/v{version}"),
    ];
    listing.lines().any(|line| {
        let Some((hash, name)) = line.split_once('\t') else {
            return false;
        };
        let name = name.strip_suffix("^{}").unwrap_or(name);
        hash == sha && names.iter().any(|release| release == name)
    })
}

/// Whether a remote `url` names a repository on this machine: a `file://`
/// URL, as Cargo gives its checkouts, or an absolute path. Every other
/// form (`https://`, `ssh://`, `git://`, `host:path`) may reach the
/// network, so it is never asked.
fn local_url(url: &str) -> bool {
    url.starts_with("file://") || std::path::Path::new(url).is_absolute()
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
