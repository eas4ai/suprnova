//! The build script's answer to "which commit is this CLI built from, and
//! is it the release?" (PAR-080), checked on real git repositories. HEAD is
//! the release only when the local tag `v<version>` names it. Every other
//! build is untagged, a one-commit-deep clone included, so `suprnova new`
//! pins the commit the templates were written for rather than a tag that
//! may lack their API.

#[path = "../build_commit.rs"]
mod build_commit;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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
