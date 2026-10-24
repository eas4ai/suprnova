//! The build script's answer to "which commit is this CLI built from, and
//! is it the release?" (PAR-080), checked on real git repositories. HEAD is
//! the release when the local tag `v<version>` names it, or, with no local
//! tag, when an `origin` on this machine lists that tag on HEAD, as Cargo's
//! git database does for the checkout `cargo install --git --tag` builds.
//! Every other build is untagged, a one-commit-deep clone included, so
//! `suprnova new` pins the commit the templates were written for rather
//! than a tag that may lack their API.

#[path = "../build_commit.rs"]
mod build_commit;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use build_commit::build_commit;

/// The version the fixture repositories release.
const VERSION: &str = "9.9.9";

/// A temporary directory for fixture repositories, with an empty git
/// configuration file that the fixture's own git commands read instead of
/// the machine's, so a global hook, signing rule or default branch cannot
/// change what a fixture holds.
struct Fixture {
    temp: tempfile::TempDir,
}

/// A fixture repository and its two commits.
struct Repository {
    /// The repository's top directory.
    path: PathBuf,
    /// The first commit, which adds `version = "9.9.9"` to `Cargo.toml` as
    /// a release's version bump does.
    release: String,
    /// The second commit, HEAD, which changes another file as work after a
    /// release does.
    head: String,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("create a temporary directory");
        fs::write(temp.path().join("gitconfig"), "").expect("write an empty git config");
        Self { temp }
    }

    /// Runs git in `directory` and returns its trimmed output, failing the
    /// test when git fails.
    fn git(&self, directory: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(directory)
            .args(args)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env("GIT_CONFIG_GLOBAL", self.temp.path().join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Suprnova Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Suprnova Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} in {} failed:\n{}",
            directory.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("git prints UTF-8")
            .trim()
            .to_owned()
    }

    /// A new repository named `name` holding the release commit and one
    /// commit after it, with no tags.
    fn repository(&self, name: &str) -> Repository {
        let path = self.temp.path().join(name);
        fs::create_dir(&path).expect("create the repository directory");
        self.git(&path, &["init", "-q", "-b", "main"]);
        fs::write(
            path.join("Cargo.toml"),
            format!("[workspace.package]\nversion = \"{VERSION}\"\n"),
        )
        .expect("write Cargo.toml");
        self.git(&path, &["add", "Cargo.toml"]);
        self.git(&path, &["commit", "-q", "-m", "Release 9.9.9"]);
        let release = self.git(&path, &["rev-parse", "HEAD"]);
        fs::write(path.join("README.md"), "Work after the release.\n").expect("write README.md");
        self.git(&path, &["add", "README.md"]);
        self.git(&path, &["commit", "-q", "-m", "Work after the release"]);
        let head = self.git(&path, &["rev-parse", "HEAD"]);
        assert_ne!(release, head, "the fixture needs two distinct commits");
        Repository {
            path,
            release,
            head,
        }
    }

    /// A checkout laid out as Cargo lays out a git dependency, built from
    /// `source`: a bare database repository that fetched `main` and the
    /// source's tags, storing the tags under `tag_namespace`
    /// (`refs/remotes/origin/tags/` as current Cargo does, `refs/tags/` as
    /// older Cargo did), and a full checkout cloned from the database by
    /// its `file://` URL with no tags of its own. HEAD is the later commit.
    fn cargo_checkout(&self, source: &Repository, tag_namespace: &str) -> PathBuf {
        let database = self.temp.path().join("database");
        let database_path = database.to_str().expect("the temporary path is UTF-8");
        self.git(
            self.temp.path(),
            &["init", "-q", "--bare", "-b", "main", database_path],
        );
        let source_url = format!("file://{}", source.path.display());
        let tags = format!("+refs/tags/*:{tag_namespace}*");
        self.git(
            &database,
            &[
                "fetch",
                "-q",
                "--no-tags",
                &source_url,
                "+refs/heads/main:refs/heads/main",
                &tags,
            ],
        );
        let checkout = self.temp.path().join("checkout");
        let database_url = format!("file://{database_path}");
        let checkout_path = checkout.to_str().expect("the temporary path is UTF-8");
        self.git(
            self.temp.path(),
            &["clone", "-q", "--no-tags", &database_url, checkout_path],
        );
        assert_eq!(
            self.git(&checkout, &["rev-parse", "--is-shallow-repository"]),
            "false",
            "Cargo's checkouts are not shallow"
        );
        assert_eq!(
            self.git(&checkout, &["for-each-ref", "refs/tags"]),
            "",
            "the checkout must carry no tags of its own"
        );
        let listing = self.git(&checkout, &["ls-remote", "origin"]);
        assert!(
            listing.contains(&format!("{tag_namespace}v{VERSION}")),
            "the database must hold the release tag under {tag_namespace}; it lists:\n{listing}"
        );
        checkout
    }
}

/// The classification of `repository`, failing the test when git could not
/// answer.
fn classify(repository: &Path) -> (String, bool) {
    let commit = build_commit(repository, VERSION).unwrap_or_else(|| {
        panic!(
            "git must classify the repository at {}",
            repository.display()
        )
    });
    (commit.sha, commit.tagged)
}

#[test]
fn build_commit_is_tagged_when_the_release_tag_names_head() {
    let fixture = Fixture::new();
    let repository = fixture.repository("tagged");
    fixture.git(&repository.path, &["tag", "v9.9.9"]);
    assert_eq!(classify(&repository.path), (repository.head, true));
}

#[test]
fn build_commit_is_tagged_when_an_annotated_release_tag_names_head() {
    let fixture = Fixture::new();
    let repository = fixture.repository("annotated");
    fixture.git(
        &repository.path,
        &["tag", "-a", "v9.9.9", "-m", "Release 9.9.9"],
    );
    assert_eq!(classify(&repository.path), (repository.head, true));
}

#[test]
fn build_commit_is_untagged_when_the_release_tag_names_an_older_commit() {
    let fixture = Fixture::new();
    let repository = fixture.repository("older");
    fixture.git(&repository.path, &["tag", "v9.9.9", &repository.release]);
    assert_eq!(classify(&repository.path), (repository.head, false));
}

#[test]
fn build_commit_is_untagged_without_a_release_tag() {
    let fixture = Fixture::new();
    let repository = fixture.repository("untagged");
    assert_eq!(classify(&repository.path), (repository.head, false));
}

#[test]
fn build_commit_is_untagged_when_only_another_version_is_tagged() {
    let fixture = Fixture::new();
    let repository = fixture.repository("other");
    fixture.git(&repository.path, &["tag", "v9.9.8"]);
    assert_eq!(classify(&repository.path), (repository.head, false));
}

/// The case the adversary named: in a clone one commit deep, the shallow
/// root seems to add every line it holds, the version line included, so
/// the history cannot tell a release from any other commit.
#[test]
fn build_commit_is_untagged_in_a_shallow_clone_without_the_tag() {
    let fixture = Fixture::new();
    let repository = fixture.repository("source");
    let shallow = fixture.temp.path().join("shallow");
    let url = format!("file://{}", repository.path.display());
    let destination = shallow.to_str().expect("the temporary path is UTF-8");
    fixture.git(
        fixture.temp.path(),
        &["clone", "-q", "--depth", "1", &url, destination],
    );
    assert_eq!(
        fixture.git(&shallow, &["rev-parse", "--is-shallow-repository"]),
        "true",
        "the clone must be shallow"
    );
    assert_eq!(classify(&shallow), (repository.head, false));
}

#[test]
fn build_commit_is_none_outside_a_repository() {
    let fixture = Fixture::new();
    let directory = fixture.temp.path().join("plain");
    fs::create_dir(&directory).expect("create a plain directory");
    assert!(build_commit(&directory, VERSION).is_none());
}

/// Sources vendored below the top of another repository take none of that
/// repository's commits for their own.
#[test]
fn build_commit_is_none_below_the_top_of_a_repository() {
    let fixture = Fixture::new();
    let repository = fixture.repository("outer");
    fixture.git(&repository.path, &["tag", "v9.9.9"]);
    let vendored = repository.path.join("vendored");
    fs::create_dir(&vendored).expect("create the vendored directory");
    assert!(build_commit(&vendored, VERSION).is_none());
}

/// The checkout `cargo install --git --tag v<version>` builds from: Cargo's
/// checkouts carry no tags, but their `origin` is Cargo's git database on
/// disk, which stores the fetched tags under `refs/remotes/origin/tags/`
/// and answers `git ls-remote` offline.
#[test]
fn build_commit_is_tagged_in_a_cargo_checkout_whose_database_tags_head() {
    let fixture = Fixture::new();
    let source = fixture.repository("source");
    fixture.git(&source.path, &["tag", "v9.9.9", &source.release]);
    let checkout = fixture.cargo_checkout(&source, "refs/remotes/origin/tags/");
    fixture.git(&checkout, &["checkout", "-q", "--detach", &source.release]);
    assert_eq!(classify(&checkout), (source.release, true));
}

/// Older Cargo versions stored the tags they fetched as `refs/tags/`.
#[test]
fn build_commit_is_tagged_in_a_cargo_checkout_whose_database_keeps_refs_tags() {
    let fixture = Fixture::new();
    let source = fixture.repository("source");
    fixture.git(&source.path, &["tag", "v9.9.9", &source.release]);
    let checkout = fixture.cargo_checkout(&source, "refs/tags/");
    fixture.git(&checkout, &["checkout", "-q", "--detach", &source.release]);
    assert_eq!(classify(&checkout), (source.release, true));
}

/// Suprnova's release tags are annotated, so the database's tag names a
/// tag object and only its peeled `^{}` entry names the commit.
#[test]
fn build_commit_is_tagged_in_a_cargo_checkout_with_an_annotated_release_tag() {
    let fixture = Fixture::new();
    let source = fixture.repository("source");
    fixture.git(
        &source.path,
        &[
            "tag",
            "-a",
            "v9.9.9",
            "-m",
            "Release 9.9.9",
            &source.release,
        ],
    );
    let checkout = fixture.cargo_checkout(&source, "refs/remotes/origin/tags/");
    fixture.git(&checkout, &["checkout", "-q", "--detach", &source.release]);
    assert_eq!(classify(&checkout), (source.release, true));
}

/// The database's release tag names another commit than HEAD, as for a
/// checkout of a commit made after the release.
#[test]
fn build_commit_is_untagged_in_a_cargo_checkout_past_the_release() {
    let fixture = Fixture::new();
    let source = fixture.repository("source");
    fixture.git(
        &source.path,
        &[
            "tag",
            "-a",
            "v9.9.9",
            "-m",
            "Release 9.9.9",
            &source.release,
        ],
    );
    let checkout = fixture.cargo_checkout(&source, "refs/remotes/origin/tags/");
    assert_eq!(classify(&checkout), (source.head, false));
}

/// An `origin` reached over the network is never asked for its tags, so
/// no build waits on the network. The URL is never contacted: the call
/// must answer untagged at once.
#[test]
fn build_commit_is_untagged_without_waiting_when_origin_is_a_network_url() {
    let fixture = Fixture::new();
    let source = fixture.repository("source");
    fixture.git(&source.path, &["tag", "v9.9.9", &source.release]);
    let checkout = fixture.cargo_checkout(&source, "refs/remotes/origin/tags/");
    fixture.git(&checkout, &["checkout", "-q", "--detach", &source.release]);
    fixture.git(
        &checkout,
        &[
            "remote",
            "set-url",
            "origin",
            "https://example.invalid/repo.git",
        ],
    );
    let started = Instant::now();
    let classified = classify(&checkout);
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(5),
        "classifying a checkout with a network origin took {elapsed:?}"
    );
    assert_eq!(classified, (source.release, false));
}
