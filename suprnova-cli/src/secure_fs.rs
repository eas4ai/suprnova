//! Filesystem writes that do not depend on the ambient umask, and path
//! checks that do not depend on the target being what it claims.
//!
//! The scaffolder writes secrets (`APP_KEY`, generated service passwords)
//! and writes into paths derived from user-supplied names. Plain
//! `fs::write` gets both wrong: it takes its mode from the umask, which
//! on a default Linux install leaves a secret 0644, and it follows
//! symlinks, so a planted link can redirect a generated file anywhere the
//! running user can write.

use std::io;
use std::path::{Component, Path};

/// Write `contents` to `path` with mode 0600 on Unix.
///
/// `fs::write` uses `O_CREAT` with mode 0666 masked by the umask — 0644
/// under the common default — so a file holding a key is world-readable
/// on any shared machine or CI runner. Creating with an explicit mode
/// closes the window entirely rather than writing first and chmod-ing
/// after, which would leave the secret readable in between.
///
/// On non-Unix targets this is `fs::write`: Windows has no mode bits to
/// set here, and its default ACLs already scope a user-profile file to
/// the creating user.
pub fn write_private(path: &Path, contents: &str) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt as _;

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(contents.as_bytes())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, contents)
    }
}

/// Drop-in replacement for `fs::write` in the `make:*` generators.
///
/// Every generator already validates its name is a Rust identifier, so
/// lexical traversal (`../../etc/cron.d/x`) cannot get through the front
/// door. What is left is symlinks: the generators write into fixed
/// relative directories, and if `src/controllers` is a link to `~/.ssh`
/// then `make:controller authorized_keys` writes there instead. That
/// needs prior write access to the project to set up, so it is an
/// escalation rather than a break-in — but it escalates *out* of the
/// project, which is exactly the boundary worth holding.
///
/// Generic over the same bounds as `fs::write`, so call sites change by
/// one path segment and nothing else.
pub fn write_generated<P: AsRef<Path>, C: AsRef<[u8]>>(path: P, contents: C) -> io::Result<()> {
    let path = path.as_ref();
    if let Err(reason) = ensure_contained(Path::new("."), path) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, reason));
    }
    std::fs::write(path, contents)
}

/// Reject a path that escapes `root`, or that traverses a symlink on the
/// way there.
///
/// Two distinct attacks, one check:
///
/// - **Traversal.** A generated name like `../../etc/cron.d/x` resolves
///   outside the project. Checked lexically, on the joined path, so it
///   catches the case where the target does not exist yet — which is the
///   normal case for a generator.
/// - **Symlink redirection.** An existing component that is a symlink
///   sends the write somewhere else entirely, even though every lexical
///   component looks innocent. Checked against the real filesystem, one
///   component at a time, because only existing prefixes can be links.
///
/// Returns the reason on rejection so the caller can print something the
/// user can act on.
pub fn ensure_contained(root: &Path, candidate: &Path) -> Result<(), String> {
    if candidate.is_absolute() {
        return Err(format!(
            "{} is an absolute path; generated files must stay inside the project",
            candidate.display()
        ));
    }

    // Lexical containment. `..` is rejected outright rather than resolved:
    // a generator has no legitimate reason to emit one, so allowing "as
    // long as it lands back inside" only widens what has to be reasoned
    // about.
    for component in candidate.components() {
        match component {
            Component::ParentDir => {
                return Err(format!(
                    "{} contains `..`; generated paths may not traverse upward",
                    candidate.display()
                ));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "{} is not a relative path inside the project",
                    candidate.display()
                ));
            }
            _ => {}
        }
    }

    // Symlink check over the existing prefix. Walk down from the root and
    // stop at the first component that does not exist yet — everything
    // past that point is being created by us, so it cannot be a link
    // someone planted.
    let mut walked = root.to_path_buf();
    for component in candidate.components() {
        walked.push(component);
        match std::fs::symlink_metadata(&walked) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(format!(
                    "{} is a symlink; refusing to write through it",
                    walked.display()
                ));
            }
            Ok(_) => {}
            // Does not exist yet: nothing below this can be a symlink
            // either, so the walk is done.
            Err(_) => break,
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_parent_traversal() {
        let root = Path::new(".");
        let err = ensure_contained(root, Path::new("../../etc/passwd"))
            .expect_err("`..` must be rejected");
        assert!(err.contains(".."), "message should name the cause: {err}");
    }

    #[test]
    fn rejects_absolute_paths() {
        let root = Path::new(".");
        let err = ensure_contained(root, Path::new("/etc/passwd"))
            .expect_err("an absolute path must be rejected");
        assert!(
            err.contains("absolute"),
            "message should name the cause: {err}"
        );
    }

    #[test]
    fn accepts_an_ordinary_nested_path() {
        let root = Path::new(".");
        ensure_contained(root, Path::new("src/controllers/posts.rs"))
            .expect("an ordinary relative path must be accepted");
    }

    /// The lexical check alone passes this — every component is a plain
    /// name. Only the filesystem walk catches it.
    #[cfg(unix)]
    #[test]
    fn rejects_a_symlinked_directory_component() {
        let tmp = std::env::temp_dir().join(format!(
            "suprnova-secure-fs-{}-{}",
            std::process::id(),
            line!()
        ));
        let outside = tmp.join("outside");
        let root = tmp.join("project");
        std::fs::create_dir_all(&outside).expect("create outside dir");
        std::fs::create_dir_all(&root).expect("create project dir");
        std::os::unix::fs::symlink(&outside, root.join("src")).expect("create symlink");

        let err = ensure_contained(&root, Path::new("src/controllers/posts.rs"))
            .expect_err("a symlinked component must be rejected");
        assert!(
            err.contains("symlink"),
            "message should name the cause: {err}"
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[cfg(unix)]
    #[test]
    fn write_private_creates_an_owner_only_file() {
        use std::os::unix::fs::PermissionsExt as _;

        let path = std::env::temp_dir().join(format!(
            "suprnova-secure-fs-env-{}-{}",
            std::process::id(),
            line!()
        ));
        write_private(&path, "APP_KEY=secret\n").expect("write");

        let mode = std::fs::metadata(&path)
            .expect("stat the written file")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            mode, 0o600,
            "a file holding APP_KEY must not be group- or world-readable"
        );

        std::fs::remove_file(&path).ok();
    }
}
