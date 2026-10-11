//! Path resolution for local disks that cannot be redirected out of the root.
//!
//! [`super::path_guard`] checks a path before an operation runs, but a check
//! followed by a syscall by pathname is a race. An actor who can rename
//! entries inside the root swaps a checked directory for a symlink after the
//! check, and the syscall follows the symlink wherever it points. On the write
//! path the window was the whole upload: the path was checked at the first
//! chunk and published by pathname at close.
//!
//! Every operation here resolves its path one component at a time. It holds
//! each directory open and names the next component relative to that
//! directory, so the directory the final syscall acts in is the one the walk
//! reached, whatever happens to the pathnames afterwards. Symlinks are still
//! followed, but by this walk and never by the kernel: a relative target is
//! resolved from the directory holding the link, an absolute target is
//! translated into the root, and a target that leaves the root - through `..`
//! or an absolute path outside it - is refused. Every directory and every
//! final entry is opened with `O_NOFOLLOW`, so a symlink swapped in after the
//! walk looked at a component fails the operation instead of redirecting it.
//!
//! All functions here block; callers run them on the blocking pool.

use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use rustix::fs::{AtFlags, FileType, Mode, OFlags};
use rustix::io::Errno;

/// The most symlinks one resolution follows, the same limit Linux applies.
const MAX_SYMLINKS: usize = 40;

/// The mode new files and directories are created with, before the umask.
/// It is what `std::fs` uses, so a confined disk creates exactly what an
/// unconfined one did.
const FILE_MODE: u32 = 0o666;
const DIR_MODE: u32 = 0o777;

/// A local disk's root, by its canonical path.
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

/// What [`ConfinedRoot::stat`] found.
pub(crate) struct Node {
    pub(crate) kind: NodeKind,
    pub(crate) len: u64,
    pub(crate) modified: SystemTime,
}

/// The kind of a node, in the three classes storage cares about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NodeKind {
    File,
    Dir,
    Other,
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
fn nofollow_error(e: Errno, path: &str) -> io::Error {
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

/// Convert whatever integer width a platform's `stat` uses.
fn widen<T: TryInto<i64>>(value: T) -> i64 {
    value.try_into().unwrap_or(0)
}

impl ConfinedRoot {
    /// The root at `root`, which must exist. It is canonicalized here, so an
    /// absolute symlink inside the disk can be recognized as pointing back
    /// into it.
    pub(crate) fn new(root: impl AsRef<Path>) -> io::Result<Self> {
        Ok(Self {
            root: std::fs::canonicalize(root)?,
        })
    }

    /// The canonical root path.
    pub(crate) fn path(&self) -> &Path {
        &self.root
    }

    fn open_root(&self) -> io::Result<OwnedFd> {
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
            let next = rustix::fs::openat(
                current,
                name.as_os_str(),
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|e| nofollow_error(e, path))?;
            stack.push(next);
        }
    }

    /// Open (creating it if missing) the directory `name` directly inside the
    /// root, without following a symlink there.
    pub(crate) fn root_dir(&self, name: &str) -> io::Result<OwnedFd> {
        let root = self.open_root()?;
        match rustix::fs::mkdirat(&root, name, Mode::from_raw_mode(DIR_MODE as _)) {
            Ok(()) | Err(Errno::EXIST) => {}
            Err(e) => return Err(e.into()),
        }
        rustix::fs::openat(
            &root,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|e| nofollow_error(e, name))
    }

    /// Open `path` for reading.
    pub(crate) fn open_read(&self, path: &str) -> io::Result<File> {
        let entry = self.resolve(path, Last::Follow, false)?;
        let fd = rustix::fs::openat(
            &entry.dir,
            entry.name.as_os_str(),
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|e| nofollow_error(e, path))?;
        Ok(File::from(fd))
    }

    /// Open `path` for an append in place, creating it - and its missing
    /// parent directories - if it does not exist.
    pub(crate) fn open_append(&self, path: &str) -> io::Result<File> {
        let entry = self.resolve(path, Last::Follow, true)?;
        let flags = OFlags::WRONLY | OFlags::APPEND | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let fd = match rustix::fs::openat(
            &entry.dir,
            entry.name.as_os_str(),
            flags | OFlags::CREATE,
            Mode::from_raw_mode(FILE_MODE as _),
        ) {
            Ok(fd) => fd,
            Err(e) => return Err(nofollow_error(e, path)),
        };
        Ok(File::from(fd))
    }

    /// Create `name` exclusively in the open directory `dir`.
    pub(crate) fn create_in(dir: &OwnedFd, name: &OsStr) -> io::Result<File> {
        let fd = rustix::fs::openat(
            dir,
            name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(FILE_MODE as _),
        )?;
        Ok(File::from(fd))
    }

    /// Materialize `path` as an empty object if nothing is there, without
    /// truncating one that is. Anything already at the path - an object, or
    /// a symlink `O_EXCL` refuses to follow - is left as it is.
    pub(crate) fn ensure_exists(&self, path: &str) -> io::Result<()> {
        let entry = self.resolve(path, Last::Keep, true)?;
        match Self::create_in(&entry.dir, entry.name.as_os_str()) {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Publish `staged` in `staging` at `path`, by `rename(2)` or - when
    /// `exclusive` - by `link(2)`, which fails rather than replace an existing
    /// entry. Missing parent directories of `path` are created.
    pub(crate) fn publish(
        &self,
        staging: &OwnedFd,
        staged: &OsStr,
        path: &str,
        exclusive: bool,
    ) -> io::Result<()> {
        let target = self.resolve(path, Last::Keep, true)?;
        if exclusive {
            rustix::fs::linkat(
                staging,
                staged,
                &target.dir,
                target.name.as_os_str(),
                AtFlags::empty(),
            )?;
            // The link is the publish; the staged name only has to go.
            let _ = rustix::fs::unlinkat(staging, staged, AtFlags::empty());
            Ok(())
        } else {
            Ok(rustix::fs::renameat(
                staging,
                staged,
                &target.dir,
                target.name.as_os_str(),
            )?)
        }
    }

    /// Remove the file `name` from the open directory `dir`; a missing one is
    /// not an error.
    pub(crate) fn remove_in(dir: &OwnedFd, name: &OsStr) -> io::Result<()> {
        match rustix::fs::unlinkat(dir, name, AtFlags::empty()) {
            Ok(()) | Err(Errno::NOENT) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// What is at `path`, following a final symlink inside the root.
    pub(crate) fn stat(&self, path: &str) -> io::Result<(Node, Option<File>)> {
        let entry = self.resolve(path, Last::Follow, false)?;
        let stat = rustix::fs::statat(
            &entry.dir,
            entry.name.as_os_str(),
            AtFlags::SYMLINK_NOFOLLOW,
        )?;
        let file_type = FileType::from_raw_mode(stat.st_mode);
        if file_type == FileType::Symlink {
            return Err(nofollow_error(Errno::LOOP, path));
        }
        let kind = match file_type {
            FileType::RegularFile => NodeKind::File,
            FileType::Directory => NodeKind::Dir,
            _ => NodeKind::Other,
        };
        let node = Node {
            kind,
            len: u64::try_from(widen(stat.st_size)).unwrap_or(0),
            modified: modified(widen(stat.st_mtime), widen(stat.st_mtime_nsec)),
        };
        // Extended attributes are read through a handle on the entry itself.
        // Only files and directories are opened for that: opening a device or
        // a FIFO can block or have side effects.
        let handle = match kind {
            NodeKind::File | NodeKind::Dir => rustix::fs::openat(
                &entry.dir,
                entry.name.as_os_str(),
                OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .ok()
            .map(File::from),
            NodeKind::Other => None,
        };
        Ok((node, handle))
    }

    /// Create the directory `path` and every missing directory above it.
    pub(crate) fn create_dir(&self, path: &str) -> io::Result<()> {
        let entry = self.resolve(path, Last::Keep, true)?;
        if entry.name == "." {
            return Ok(());
        }
        match rustix::fs::mkdirat(
            &entry.dir,
            entry.name.as_os_str(),
            Mode::from_raw_mode(DIR_MODE as _),
        ) {
            Ok(()) => Ok(()),
            Err(Errno::EXIST) => {
                // `create_dir_all` accepts a directory, or a link to one,
                // already there; anything else is the error it would raise.
                let stat = rustix::fs::statat(
                    &entry.dir,
                    entry.name.as_os_str(),
                    AtFlags::SYMLINK_NOFOLLOW,
                )?;
                match FileType::from_raw_mode(stat.st_mode) {
                    FileType::Directory | FileType::Symlink => Ok(()),
                    _ => Err(Errno::EXIST.into()),
                }
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Delete `path`. A missing path is not an error. A symlink is removed
    /// itself, never followed. A directory needs `recursive` unless empty.
    pub(crate) fn remove(&self, path: &str, recursive: bool) -> io::Result<()> {
        let entry = match self.resolve(path, Last::Keep, false) {
            Ok(entry) => entry,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        let stat = match rustix::fs::statat(
            &entry.dir,
            entry.name.as_os_str(),
            AtFlags::SYMLINK_NOFOLLOW,
        ) {
            Ok(stat) => stat,
            Err(Errno::NOENT) => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        if FileType::from_raw_mode(stat.st_mode) != FileType::Directory {
            return Self::remove_in(&entry.dir, entry.name.as_os_str());
        }
        if entry.name == "." {
            // The root itself: empty it, keep the directory.
            return if recursive {
                Self::empty_dir(&entry.dir)
            } else {
                Err(Errno::BUSY.into())
            };
        }
        if recursive {
            let dir = rustix::fs::openat(
                &entry.dir,
                entry.name.as_os_str(),
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|e| nofollow_error(e, path))?;
            Self::empty_dir(&dir)?;
        }
        match rustix::fs::unlinkat(&entry.dir, entry.name.as_os_str(), AtFlags::REMOVEDIR) {
            Ok(()) | Err(Errno::NOENT) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Remove everything inside the open directory `dir`, never following a
    /// symlink out of it.
    fn empty_dir(dir: &OwnedFd) -> io::Result<()> {
        let mut entries = rustix::fs::Dir::read_from(dir)?;
        while let Some(entry) = entries.read() {
            let entry = entry?;
            let name = OsStr::from_bytes(entry.file_name().to_bytes());
            if name == "." || name == ".." {
                continue;
            }
            let stat = match rustix::fs::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(stat) => stat,
                Err(Errno::NOENT) => continue,
                Err(e) => return Err(e.into()),
            };
            if FileType::from_raw_mode(stat.st_mode) == FileType::Directory {
                let child = rustix::fs::openat(
                    dir,
                    name,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )?;
                Self::empty_dir(&child)?;
                match rustix::fs::unlinkat(dir, name, AtFlags::REMOVEDIR) {
                    Ok(()) | Err(Errno::NOENT) => {}
                    Err(e) => return Err(e.into()),
                }
            } else {
                Self::remove_in(dir, name)?;
            }
        }
        Ok(())
    }

    /// Move `from` to `to`, creating `to`'s missing parent directories. A
    /// symlink at `from` is moved itself.
    pub(crate) fn rename(&self, from: &str, to: &str) -> io::Result<()> {
        let source = self.resolve(from, Last::Keep, false)?;
        // `rename` reports a missing source as `NotFound`, as the unconfined
        // backend's own probe did.
        rustix::fs::statat(
            &source.dir,
            source.name.as_os_str(),
            AtFlags::SYMLINK_NOFOLLOW,
        )?;
        let target = self.resolve(to, Last::Keep, true)?;
        Ok(rustix::fs::renameat(
            &source.dir,
            source.name.as_os_str(),
            &target.dir,
            target.name.as_os_str(),
        )?)
    }

    /// Open the directory `path` for listing, following a final symlink
    /// inside the root. `None` when nothing is there or it is not a
    /// directory, which a listing reports as empty.
    pub(crate) fn open_dir(&self, path: &str) -> io::Result<Option<OwnedFd>> {
        let entry = match self.resolve(path, Last::Follow, false) {
            Ok(entry) => entry,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) if e.raw_os_error() == Some(Errno::NOTDIR.raw_os_error()) => return Ok(None),
            Err(e) => return Err(e),
        };
        match rustix::fs::openat(
            &entry.dir,
            entry.name.as_os_str(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => Ok(Some(fd)),
            Err(Errno::NOENT) | Err(Errno::NOTDIR) => Ok(None),
            Err(e) => Err(nofollow_error(e, path)),
        }
    }

    /// The listing facts for the entry `name` of the open directory `dir`,
    /// without following it. `None` when it vanished meanwhile.
    pub(crate) fn entry_node(dir: &OwnedFd, name: &OsStr) -> io::Result<Option<Node>> {
        let stat = match rustix::fs::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => stat,
            Err(Errno::NOENT) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let kind = match FileType::from_raw_mode(stat.st_mode) {
            FileType::RegularFile => NodeKind::File,
            FileType::Directory => NodeKind::Dir,
            _ => NodeKind::Other,
        };
        Ok(Some(Node {
            kind,
            len: u64::try_from(widen(stat.st_size)).unwrap_or(0),
            modified: modified(widen(stat.st_mtime), widen(stat.st_mtime_nsec)),
        }))
    }
}

/// A `stat` modification time as a `SystemTime`.
fn modified(seconds: i64, nanos: i64) -> SystemTime {
    let nanos = Duration::from_nanos(u64::try_from(nanos).unwrap_or(0));
    match u64::try_from(seconds) {
        Ok(after) => SystemTime::UNIX_EPOCH + Duration::from_secs(after) + nanos,
        Err(_) => SystemTime::UNIX_EPOCH - Duration::from_secs(seconds.unsigned_abs()) + nanos,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A root with `inside/data.txt` and a sibling `outside/data.txt`.
    fn fixture() -> (tempfile::TempDir, ConfinedRoot) {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join("root/dir")).expect("root");
        std::fs::create_dir_all(tmp.path().join("outside")).expect("outside");
        std::fs::write(tmp.path().join("root/dir/data.txt"), "inside").expect("inside file");
        std::fs::write(tmp.path().join("outside/data.txt"), "SECRET").expect("outside file");
        let root = ConfinedRoot::new(tmp.path().join("root")).expect("root opens");
        (tmp, root)
    }

    fn read(file: File) -> String {
        std::io::read_to_string(file).expect("read the file")
    }

    #[test]
    fn a_plain_path_and_an_inside_symlink_resolve() {
        let (tmp, root) = fixture();
        assert_eq!(read(root.open_read("dir/data.txt").unwrap()), "inside");

        std::os::unix::fs::symlink(tmp.path().join("root/dir"), tmp.path().join("root/abs"))
            .unwrap();
        std::os::unix::fs::symlink("dir", tmp.path().join("root/rel")).unwrap();
        assert_eq!(read(root.open_read("abs/data.txt").unwrap()), "inside");
        assert_eq!(read(root.open_read("rel/data.txt").unwrap()), "inside");
    }

    #[test]
    fn a_symlink_out_of_the_root_is_refused() {
        let (tmp, root) = fixture();
        std::os::unix::fs::symlink(tmp.path().join("outside"), tmp.path().join("root/out"))
            .unwrap();
        std::os::unix::fs::symlink("../outside", tmp.path().join("root/up")).unwrap();
        for path in ["out/data.txt", "up/data.txt", "dir/../../outside/data.txt"] {
            let err = root.open_read(path).expect_err(path);
            assert_eq!(err.kind(), io::ErrorKind::PermissionDenied, "{path}: {err}");
        }
    }

    /// The property the module exists for: once resolution holds a
    /// directory, renaming that directory away and planting a symlink in its
    /// place does not move the operation.
    #[test]
    fn a_held_directory_is_not_redirected_by_a_swap() {
        let (tmp, root) = fixture();
        let entry = root.resolve("dir/new.txt", Last::Keep, false).unwrap();
        std::fs::rename(tmp.path().join("root/dir"), tmp.path().join("root/dir.old")).unwrap();
        std::os::unix::fs::symlink(tmp.path().join("outside"), tmp.path().join("root/dir"))
            .unwrap();

        ConfinedRoot::create_in(&entry.dir, entry.name.as_os_str()).unwrap();
        assert!(tmp.path().join("root/dir.old/new.txt").exists());
        assert!(!tmp.path().join("outside/new.txt").exists());

        // A fresh resolution through the swapped-in symlink is refused.
        let err = root
            .open_read("dir/data.txt")
            .expect_err("the swap is refused");
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied, "{err}");
    }

    #[test]
    fn delete_removes_a_symlink_itself_and_never_its_target() {
        let (tmp, root) = fixture();
        std::os::unix::fs::symlink(tmp.path().join("outside"), tmp.path().join("root/out"))
            .unwrap();
        root.remove("out", true).unwrap();
        assert!(std::fs::symlink_metadata(tmp.path().join("root/out")).is_err());
        assert!(tmp.path().join("outside/data.txt").exists());

        root.remove("dir", true).unwrap();
        assert!(!tmp.path().join("root/dir").exists());
        root.remove("missing/thing", false).unwrap();
    }
}
