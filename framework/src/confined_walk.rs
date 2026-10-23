//! Path resolution beneath a root that a symlink cannot redirect out of.
//!
//! A check on a path followed by a syscall on that path by name is a race.
//! An actor who can rename entries inside the root swaps a checked
//! directory for a symlink after the check, and the syscall follows the
//! symlink wherever it points. The local disks (`filesystem::confined`),
//! `StaticFiles` and the Live UI asset server resolve their paths here
//! instead.
//!
//! A resolution walks its path one component at a time. It holds each
//! directory open and names the next component relative to that directory,
//! so the directory the final syscall acts in is the one the walk reached,
//! whatever happens to the pathnames afterwards. Symlinks are still
//! followed, but by this walk and never by the kernel: a relative target is
//! resolved from the directory holding the link, an absolute target is
//! translated into the root, and a target that leaves the root - through
//! `..` or an absolute path outside it - is refused. Every directory and
//! every final entry is opened with `O_NOFOLLOW`, so a symlink swapped in
//! after the walk looked at a component fails the operation instead of
//! redirecting it.
//!
//! All functions here block; callers run them on the blocking pool.

use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::fs::{File, Metadata};
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use rustix::fs::{Mode, OFlags};
use rustix::io::Errno;

/// The most symlinks one resolution follows, the same limit Linux applies.
const MAX_SYMLINKS: usize = 40;

/// The mode new directories are created with, before the umask. It is what
/// `std::fs` uses, so a confined disk creates exactly what an unconfined
/// one did.
pub(crate) const DIR_MODE: u32 = 0o777;

/// A root, by its canonical path.
#[derive(Debug, Clone)]
pub(crate) struct ConfinedRoot {
    root: PathBuf,
}

/// A directory held open, and the name of one entry in it. The name is `.`
/// when the path named the directory itself.
pub(crate) struct Entry {
    pub(crate) dir: OwnedFd,
    pub(crate) name: OsString,
}

/// Whether resolution follows a symlink in the final component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Last {
    /// Follow it, for operations that act on what the path leads to: read,
    /// stat, append, list.
    Follow,
    /// Keep it, for operations that act on the entry itself: publish by
    /// rename or link, delete, rename, create a directory.
    Keep,
}

/// The error for a path whose resolution leaves the root.
fn escape(path: &OsStr) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!(
            "'{}' resolves outside the local disk root",
            path.to_string_lossy()
        ),
    )
}

/// Map the error of an `O_NOFOLLOW` open. `ELOOP` (and `EMLINK` on the BSDs)
/// means the entry became a symlink after the walk looked at it - the swap
/// this module exists to defeat - so it is reported as a refusal.
pub(crate) fn nofollow_error(e: Errno, path: &str) -> io::Error {
    if e == Errno::LOOP || e == Errno::MLINK {
        return io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("'{path}' changed into a symlink while it was in use"),
        );
    }
    e.into()
}

/// Split a storage path into the components a walk visits. Empty and `.`
/// components are dropped; `..` is kept for the walk to apply.
fn components(path: &OsStr) -> VecDeque<OsString> {
    path.as_bytes()
        .split(|byte| *byte == b'/')
        .filter(|part| !part.is_empty() && *part != b".")
        .map(|part| OsString::from_vec(part.to_vec()))
        .collect()
}

/// Open the directory `name` in the open directory `dir`, never following
/// a symlink there. `path` names the whole path in the error.
pub(crate) fn open_dir_in(dir: &OwnedFd, name: &OsStr, path: &str) -> io::Result<OwnedFd> {
    rustix::fs::openat(
        dir,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| nofollow_error(e, path))
}

/// Open the regular file `name` in the open directory `dir` for reading,
/// never following a symlink there, and return it with the metadata of
/// that same handle. `None` when the entry is not a regular file.
///
/// A file server takes the type and the size it checks from this metadata
/// and reads the bytes from this handle, so the checks hold for exactly
/// the bytes it sends. The open does not wait: an ordinary open of a FIFO
/// blocks until a writer appears, so the entry is opened non-blocking and
/// only a regular file is switched back to blocking reads.
pub(crate) fn open_file_in(
    dir: &OwnedFd,
    name: &OsStr,
    path: &str,
) -> io::Result<Option<(File, Metadata)>> {
    let fd = rustix::fs::openat(
        dir,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| nofollow_error(e, path))?;
    let file = File::from(fd);
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Ok(None);
    }
    rustix::fs::fcntl_setfl(&file, rustix::fs::fcntl_getfl(&file)? - OFlags::NONBLOCK)?;
    Ok(Some((file, metadata)))
}

impl ConfinedRoot {
    /// The root at `root`, which must exist. It is canonicalized here, so an
    /// absolute symlink inside the root can be recognized as pointing back
    /// into it. The root itself may be a symlink; only what lies below it is
    /// confined.
    pub(crate) fn new(root: impl AsRef<Path>) -> io::Result<Self> {
        Ok(Self {
            root: std::fs::canonicalize(root)?,
        })
    }

    /// The canonical root path.
    #[cfg(feature = "filesystem")]
    pub(crate) fn path(&self) -> &Path {
        &self.root
    }

    /// Open the root directory itself.
    pub(crate) fn open_root(&self) -> io::Result<OwnedFd> {
        Ok(rustix::fs::open(
            &self.root,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )?)
    }

    /// The part of the absolute `target` below the root.
    ///
    /// A link can name the root by a path that is not its canonical one, so a
    /// target outside the canonical prefix is canonicalized once to translate
    /// it. That probe follows links by pathname, but only to find a name: the
    /// walk that uses the result is the confined one, and a symlink swapped in
    /// meanwhile is met there.
    fn inside(&self, target: &Path) -> io::Result<PathBuf> {
        if let Ok(rest) = target.strip_prefix(&self.root) {
            return Ok(rest.to_path_buf());
        }
        let canonical = std::fs::canonicalize(target)?;
        canonical
            .strip_prefix(&self.root)
            .map(Path::to_path_buf)
            .map_err(|_| escape(target.as_os_str()))
    }

    /// Resolve `path` beneath the root.
    ///
    /// Intermediate components must be directories, reached without leaving
    /// the root. With `create`, missing intermediate directories are created.
    /// The final component is returned unresolved with [`Last::Keep`], and
    /// with [`Last::Follow`] is resolved until it is not a symlink; it need
    /// not exist either way.
    pub(crate) fn resolve(&self, path: &str, last: Last, create: bool) -> io::Result<Entry> {
        let root = self.open_root()?;
        let mut stack: Vec<OwnedFd> = Vec::new();
        let mut pending = components(OsStr::new(path));
        let mut followed = 0usize;

        loop {
            let Some(name) = pending.pop_front() else {
                // The path named a directory: the root, or one reached by `..`.
                let dir = match stack.pop() {
                    Some(dir) => dir,
                    None => root,
                };
                return Ok(Entry {
                    dir,
                    name: OsString::from("."),
                });
            };
            if name == ".." {
                if stack.pop().is_none() {
                    return Err(escape(OsStr::new(path)));
                }
                continue;
            }

            let is_last = pending.is_empty();
            let current = stack.last().unwrap_or(&root);
            if is_last && last == Last::Keep {
                return Ok(Entry {
                    dir: current.try_clone()?,
                    name,
                });
            }

            match rustix::fs::readlinkat(current, name.as_os_str(), Vec::new()) {
                Ok(target) => {
                    followed += 1;
                    if followed > MAX_SYMLINKS {
                        return Err(Errno::LOOP.into());
                    }
                    let target = PathBuf::from(OsString::from_vec(target.into_bytes()));
                    let mut next = if target.is_absolute() {
                        let inside = self.inside(&target)?;
                        stack.clear();
                        components(inside.as_os_str())
                    } else {
                        components(target.as_os_str())
                    };
                    next.extend(pending.drain(..));
                    pending = next;
                    continue;
                }
                // Not a symlink: an ordinary entry.
                Err(Errno::INVAL) => {}
                Err(Errno::NOENT) if is_last => {
                    return Ok(Entry {
                        dir: current.try_clone()?,
                        name,
                    });
                }
                Err(Errno::NOENT) if create => {
                    match rustix::fs::mkdirat(
                        current,
                        name.as_os_str(),
                        Mode::from_raw_mode(DIR_MODE as _),
                    ) {
                        Ok(()) | Err(Errno::EXIST) => {}
                        Err(e) => return Err(e.into()),
                    }
                }
                Err(e) => return Err(e.into()),
            }

            if is_last {
                return Ok(Entry {
                    dir: current.try_clone()?,
                    name,
                });
            }
            let next = open_dir_in(current, name.as_os_str(), path)?;
            stack.push(next);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read as _;

    fn open(path: &Path) -> OwnedFd {
        ConfinedRoot::new(path)
            .expect("the root exists")
            .open_root()
            .expect("the root opens")
    }

    #[test]
    fn a_regular_file_opens_with_the_metadata_of_its_own_handle() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("app.css"), "body{}").expect("file");

        let (mut file, metadata) =
            open_file_in(&open(tmp.path()), OsStr::new("app.css"), "app.css")
                .expect("the file opens")
                .expect("a regular file");
        assert_eq!(metadata.len(), 6);
        assert!(
            !rustix::fs::fcntl_getfl(&file)
                .expect("file flags")
                .contains(OFlags::NONBLOCK),
            "reads block again once the entry is known to be a regular file"
        );
        let mut text = String::new();
        file.read_to_string(&mut text).expect("read the file");
        assert_eq!(text, "body{}");
    }

    #[test]
    fn a_symlink_is_refused_and_what_is_not_a_regular_file_is_skipped() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("real.txt"), "inside").expect("file");
        std::fs::create_dir(tmp.path().join("sub")).expect("directory");
        std::os::unix::fs::symlink("real.txt", tmp.path().join("link.txt")).expect("file link");
        std::os::unix::fs::symlink("sub", tmp.path().join("linked")).expect("directory link");
        let root = open(tmp.path());

        let refused = open_file_in(&root, OsStr::new("link.txt"), "link.txt")
            .expect_err("a file reached through a symlink is refused");
        assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied, "{refused}");
        let refused = open_dir_in(&root, OsStr::new("linked"), "linked")
            .expect_err("a directory reached through a symlink is refused");
        // Linux answers `O_DIRECTORY | O_NOFOLLOW` on a symlink with ENOTDIR,
        // the BSDs with ELOOP; either way the link is not followed.
        assert!(
            matches!(
                refused.kind(),
                io::ErrorKind::NotADirectory | io::ErrorKind::PermissionDenied
            ),
            "{refused}"
        );
        assert!(
            open_file_in(&root, OsStr::new("sub"), "sub")
                .expect("a directory opens")
                .is_none(),
            "a directory is not a file to serve"
        );
        let missing = open_file_in(&root, OsStr::new("missing"), "missing")
            .expect_err("a missing entry is an error");
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);
    }

    /// An ordinary open of a FIFO waits for a writer, and this one never
    /// gets one: the open must return at once and skip it.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_fifo_is_skipped_without_waiting_for_a_writer() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = open(tmp.path());
        rustix::fs::mkfifoat(&root, "pipe", Mode::from_raw_mode(0o600)).expect("make the FIFO");

        assert!(
            open_file_in(&root, OsStr::new("pipe"), "pipe")
                .expect("the FIFO opens without waiting")
                .is_none()
        );
    }
}
