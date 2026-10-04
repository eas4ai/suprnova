//! The local-filesystem service behind `Storage::register_fs` on Unix.
//!
//! It stands in for opendal's own `fs` service and does the same work, with
//! one difference: every path is resolved through [`ConfinedRoot`], one
//! component at a time, and every syscall names its target relative to a
//! directory the resolution holds open. opendal's service joins the caller's
//! key onto the root and acts by pathname, so a directory swapped for a
//! symlink between [`super::path_guard`]'s check and the syscall sent a read,
//! a write, or a publish wherever the symlink pointed. Here the swap is either
//! met by the walk, which refuses a link out of the root, or arrives after the
//! walk, when it no longer matters.
//!
//! It is the base of the operator, below opendal's own completion,
//! simulation, and correctness layers, so recursive listing and argument
//! checks work exactly as they do over opendal's service.

use super::ATOMIC_STAGING_DIR;
use super::confined::{ConfinedRoot, Node, NodeKind};
use opendal::raw::{
    OpCopier, OpCopy, OpCreateDir, OpDelete, OpList, OpPresign, OpRead, OpRename, OpStat, OpWrite,
    RpCreateDir, RpPresign, RpRename, RpStat, Service, ServiceInfo, Timestamp, new_std_io_error,
    oio,
};
use opendal::{
    Buffer, Builder, Capability, EntryMode, Error, ErrorKind, Metadata, OperationContext, Result,
};
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::Write as _;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::sync::{Arc, Mutex};
use uuid::Uuid;
use xattr::FileExt as _;

/// The prefix of the extended attributes that carry user metadata, the one
/// opendal's `fs` service reads and writes.
const XATTR_USER_PREFIX: &str = "user.";

/// How many directory entries one blocking call reads for a listing.
const LIST_BATCH: usize = 256;

/// Builds the confined local-filesystem service for `root`.
#[derive(Debug, Default)]
pub(crate) struct ConfinedFs {
    root: String,
}

impl ConfinedFs {
    /// A builder for a disk rooted at `root`.
    pub(crate) fn new(root: &str) -> Self {
        Self {
            root: root.to_owned(),
        }
    }
}

impl Builder for ConfinedFs {
    type Config = ();

    fn build(self) -> Result<impl Service> {
        if self.root.is_empty() {
            return Err(Error::new(
                ErrorKind::ConfigInvalid,
                "root is not specified",
            ));
        }
        std::fs::create_dir_all(&self.root).map_err(|e| {
            Error::new(ErrorKind::Unexpected, "create root dir failed")
                .with_context("root", &self.root)
                .set_source(e)
        })?;
        let root = ConfinedRoot::new(&self.root).map_err(|e| {
            Error::new(
                ErrorKind::Unexpected,
                "canonicalize of root directory failed",
            )
            .set_source(e)
        })?;
        // The staging directory is created now, so a refusal to create it
        // surfaces at registration rather than at the first write.
        root.root_dir(ATOMIC_STAGING_DIR).map_err(|e| {
            Error::new(ErrorKind::Unexpected, "create atomic write dir failed")
                .with_context("atomic_write_dir", ATOMIC_STAGING_DIR)
                .set_source(e)
        })?;
        let info = ServiceInfo::new("fs", root.path().to_string_lossy(), "");
        Ok(ConfinedFsService {
            core: Arc::new(Core { root, info }),
        })
    }
}

/// What every handle of one disk shares.
#[derive(Debug)]
struct Core {
    root: ConfinedRoot,
    info: ServiceInfo,
}

/// Run blocking filesystem work on the blocking pool.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> std::io::Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| {
            Error::new(ErrorKind::Unexpected, "local filesystem task failed").set_source(e)
        })?
        .map_err(new_std_io_error)
}

/// Read the `user.` extended attributes of an open file, as opendal's `fs`
/// service does: a filesystem without them reports none.
fn user_metadata(file: &File) -> std::io::Result<HashMap<String, String>> {
    let mut found = HashMap::new();
    let names = match file.list_xattr() {
        Ok(names) => names,
        Err(e) if e.kind() == std::io::ErrorKind::Unsupported => return Ok(found),
        Err(e) if e.raw_os_error() == Some(rustix::io::Errno::NOTSUP.raw_os_error()) => {
            return Ok(found);
        }
        Err(e) => return Err(e),
    };
    for name in names {
        let key = name.to_string_lossy();
        if let Some(key) = key.strip_prefix(XATTR_USER_PREFIX)
            && let Ok(Some(value)) = file.get_xattr(&name)
            && let Ok(value) = String::from_utf8(value)
        {
            found.insert(key.to_string(), value);
        }
    }
    Ok(found)
}

/// Write user metadata onto an open file as `user.` extended attributes.
fn set_user_metadata(file: &File, metadata: &HashMap<String, String>) -> std::io::Result<()> {
    for (key, value) in metadata {
        file.set_xattr(format!("{XATTR_USER_PREFIX}{key}"), value.as_bytes())?;
    }
    Ok(())
}

/// The opendal metadata for what a stat or a listing found.
fn metadata_of(node: &Node) -> Result<Metadata> {
    let mode = match node.kind {
        NodeKind::File => EntryMode::FILE,
        NodeKind::Dir => EntryMode::DIR,
        NodeKind::Other => EntryMode::Unknown,
    };
    Ok(Metadata::new(mode)
        .with_content_length(node.len)
        .with_last_modified(Timestamp::try_from(node.modified)?))
}

/// The metadata of a file a writer just closed.
fn written_metadata(file: &File) -> std::io::Result<(u64, std::time::SystemTime)> {
    let metadata = file.metadata()?;
    Ok((metadata.len(), metadata.modified()?))
}

/// A unique name for a staged write or copy of `path`, recognizable by the
/// object it is for.
fn staged_name(path: &str) -> OsString {
    let base = path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    OsString::from(format!("{base}.{}.write", Uuid::new_v4().simple()))
}

/// The confined local-filesystem service.
#[derive(Debug, Clone)]
pub(crate) struct ConfinedFsService {
    core: Arc<Core>,
}

impl Service for ConfinedFsService {
    type Reader = oio::PositionReader<ConfinedReader>;
    type Writer = ConfinedWriter;
    type Lister = ConfinedLister;
    type Deleter = oio::OneShotDeleter<ConfinedDeleter>;
    type Copier = oio::OneShotCopier;

    fn info(&self) -> ServiceInfo {
        self.core.info.clone()
    }

    fn capability(&self) -> Capability {
        // Exactly what opendal's own `fs` service advertises, so nothing above
        // the service can tell the two apart except by what they refuse.
        Capability {
            stat: true,
            read: true,
            write: true,
            write_can_empty: true,
            write_can_append: true,
            write_can_multi: true,
            write_with_if_not_exists: true,
            write_with_user_metadata: true,
            create_dir: true,
            delete: true,
            delete_with_recursive: true,
            list: true,
            copy: true,
            rename: true,
            shared: true,
            ..Default::default()
        }
    }

    async fn create_dir(
        &self,
        _ctx: &OperationContext,
        path: &str,
        _args: OpCreateDir,
    ) -> Result<RpCreateDir> {
        let core = Arc::clone(&self.core);
        let path = path.to_owned();
        blocking(move || core.root.create_dir(&path)).await?;
        Ok(RpCreateDir::default())
    }

    async fn stat(&self, _ctx: &OperationContext, path: &str, _args: OpStat) -> Result<RpStat> {
        let core = Arc::clone(&self.core);
        let path = path.to_owned();
        let (node, user) = blocking(move || {
            let (node, handle) = core.root.stat(&path)?;
            let user = match handle {
                Some(file) => user_metadata(&file)?,
                None => HashMap::new(),
            };
            Ok((node, user))
        })
        .await?;
        let mut metadata = metadata_of(&node)?;
        if !user.is_empty() {
            metadata = metadata.with_user_metadata(user);
        }
        Ok(RpStat::new(metadata))
    }

    fn read(&self, _ctx: &OperationContext, path: &str, _args: OpRead) -> Result<Self::Reader> {
        Ok(oio::PositionReader::new(ConfinedReader {
            core: Arc::clone(&self.core),
            path: path.to_owned(),
        }))
    }

    fn write(&self, _ctx: &OperationContext, path: &str, args: OpWrite) -> Result<Self::Writer> {
        Ok(ConfinedWriter {
            core: Arc::clone(&self.core),
            path: path.to_owned(),
            args,
            open: None,
        })
    }

    fn delete(&self, _ctx: &OperationContext) -> Result<Self::Deleter> {
        Ok(oio::OneShotDeleter::new(ConfinedDeleter {
            core: Arc::clone(&self.core),
        }))
    }

    fn list(&self, _ctx: &OperationContext, path: &str, _args: OpList) -> Result<Self::Lister> {
        Ok(ConfinedLister {
            core: Arc::clone(&self.core),
            path: path.to_owned(),
            state: ListState::Unopened,
        })
    }

    fn copy(
        &self,
        _ctx: &OperationContext,
        from: &str,
        to: &str,
        _args: OpCopy,
        _opts: OpCopier,
    ) -> Result<Self::Copier> {
        let core = Arc::clone(&self.core);
        let from = from.to_owned();
        let to = to.to_owned();
        Ok(oio::OneShotCopier::new(async move {
            blocking(move || copy_confined(&core.root, &from, &to)).await?;
            Ok(Metadata::default())
        }))
    }

    async fn rename(
        &self,
        _ctx: &OperationContext,
        from: &str,
        to: &str,
        _args: OpRename,
    ) -> Result<RpRename> {
        let core = Arc::clone(&self.core);
        let from = from.to_owned();
        let to = to.to_owned();
        blocking(move || core.root.rename(&from, to.trim_end_matches('/'))).await?;
        Ok(RpRename::default())
    }

    async fn presign(
        &self,
        _ctx: &OperationContext,
        _path: &str,
        _args: OpPresign,
    ) -> Result<RpPresign> {
        Err(Error::new(
            ErrorKind::Unsupported,
            "operation is not supported",
        ))
    }
}

/// Copy `from` to `to` through a staged file, so the destination appears
/// whole or not at all, carrying the source's user metadata.
fn copy_confined(root: &ConfinedRoot, from: &str, to: &str) -> std::io::Result<()> {
    let mut source = root.open_read(from)?;
    let staging = root.root_dir(ATOMIC_STAGING_DIR)?;
    let staged = staged_name(to);
    let result = stage_copy(&mut source, &staging, &staged)
        .and_then(|()| root.publish(&staging, &staged, to.trim_end_matches('/'), false));
    if result.is_err() {
        let _ = ConfinedRoot::remove_in(&staging, &staged);
    }
    result
}

/// Fill the staged file `staged` with `source`'s bytes and user metadata.
fn stage_copy(source: &mut File, staging: &OwnedFd, staged: &OsStr) -> std::io::Result<()> {
    let mut target = ConfinedRoot::create_in(staging, staged)?;
    std::io::copy(source, &mut target)?;
    if let Ok(metadata) = user_metadata(source)
        && !metadata.is_empty()
    {
        set_user_metadata(&target, &metadata)?;
    }
    target.flush()
}

/// Positioned reads from one file, opened through the confined walk the
/// first time a range is asked for.
pub(crate) struct ConfinedReader {
    core: Arc<Core>,
    path: String,
}

impl oio::PositionRead for ConfinedReader {
    type Handle = Arc<File>;

    async fn open(&self) -> Result<Self::Handle> {
        let core = Arc::clone(&self.core);
        let path = self.path.clone();
        let file = blocking(move || core.root.open_read(&path)).await?;
        Ok(Arc::new(file))
    }

    async fn read_at(handle: &Self::Handle, offset: u64, size: usize) -> Result<Buffer> {
        if size == 0 {
            return Ok(Buffer::new());
        }
        let file = Arc::clone(handle);
        let bytes = blocking(move || {
            let mut buf = vec![0u8; size];
            let n = std::os::unix::fs::FileExt::read_at(&*file, &mut buf, offset)?;
            buf.truncate(n);
            Ok(buf)
        })
        .await?;
        Ok(Buffer::from(bytes))
    }
}

/// A write in flight: the file being filled, and - unless it is an append in
/// place - the staged name it gets published from.
struct OpenWrite {
    file: Arc<File>,
    staged: Option<(Arc<OwnedFd>, OsString)>,
}

/// Writes one object. An ordinary write fills a staged file and publishes it
/// at close with `rename(2)`, or with `link(2)` for `if_not_exists`, which
/// fails rather than replace what is there. An append writes in place.
pub(crate) struct ConfinedWriter {
    core: Arc<Core>,
    path: String,
    args: OpWrite,
    open: Option<OpenWrite>,
}

impl ConfinedWriter {
    async fn opened(&mut self) -> Result<&OpenWrite> {
        if self.open.is_none() {
            let core = Arc::clone(&self.core);
            let path = self.path.clone();
            let append = self.args.append();
            let exclusive = self.args.if_not_exists();
            let open = blocking(move || {
                if append {
                    let file = core.root.open_append(&path)?;
                    return Ok(OpenWrite {
                        file: Arc::new(file),
                        staged: None,
                    });
                }
                if exclusive && core.root.stat(&path).is_ok() {
                    // An early answer for the common case; the `link(2)` at
                    // close is what makes it exclusive.
                    return Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists));
                }
                let staging = core.root.root_dir(ATOMIC_STAGING_DIR)?;
                let staged = staged_name(&path);
                let file = ConfinedRoot::create_in(&staging, &staged)?;
                Ok(OpenWrite {
                    file: Arc::new(file),
                    staged: Some((Arc::new(staging), staged)),
                })
            })
            .await
            .map_err(|e| self.condition_error(e))?;
            self.open = Some(open);
        }
        self.open
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Unexpected, "the writer did not open"))
    }

    /// Report a refused `if_not_exists` as the condition failure it is.
    fn condition_error(&self, e: Error) -> Error {
        if self.args.if_not_exists() && e.kind() == ErrorKind::AlreadyExists {
            return Error::new(
                ErrorKind::ConditionNotMatch,
                format!(
                    "'{}' already exists, doesn't match the condition if_not_exists",
                    self.path
                ),
            );
        }
        e
    }
}

impl oio::Write for ConfinedWriter {
    async fn write(&mut self, bs: Buffer) -> Result<()> {
        let file = Arc::clone(&self.opened().await?.file);
        blocking(move || {
            let mut file = &*file;
            for chunk in bs {
                file.write_all(&chunk)?;
            }
            Ok(())
        })
        .await
    }

    async fn close(&mut self) -> Result<Metadata> {
        self.opened().await?;
        let Some(open) = self.open.take() else {
            return Err(Error::new(ErrorKind::Unexpected, "the writer did not open"));
        };
        let core = Arc::clone(&self.core);
        let path = self.path.trim_end_matches('/').to_owned();
        let exclusive = self.args.if_not_exists();
        let user = self.args.user_metadata().cloned();
        let staged = open.staged.clone();
        let file = Arc::clone(&open.file);
        let closed = blocking(move || {
            file.sync_all()?;
            if let Some(user) = &user {
                set_user_metadata(&file, user)?;
            }
            let written = written_metadata(&file)?;
            if let Some((staging, name)) = &staged {
                core.root.publish(staging, name, &path, exclusive)?;
            }
            Ok(written)
        })
        .await;
        match closed {
            Ok((len, modified)) => Ok(Metadata::new(EntryMode::FILE)
                .with_content_length(len)
                .with_last_modified(Timestamp::try_from(modified)?)),
            Err(e) => {
                // Put the handle back so an abort can still remove the stage.
                self.open = Some(open);
                Err(self.condition_error(e))
            }
        }
    }

    async fn abort(&mut self) -> Result<()> {
        let Some(open) = self.open.take() else {
            return Ok(());
        };
        match open.staged {
            Some((staging, name)) => {
                blocking(move || ConfinedRoot::remove_in(&staging, &name)).await
            }
            // An append writes in place, so there is nothing of its own to
            // remove; opendal's service reports the same.
            None => Err(Error::new(
                ErrorKind::Unsupported,
                "an append in place cannot be aborted",
            )),
        }
    }
}

/// Removes one path per call, never following a symlink out of the root.
pub(crate) struct ConfinedDeleter {
    core: Arc<Core>,
}

impl oio::OneShotDelete for ConfinedDeleter {
    async fn delete_once(&self, path: String, args: OpDelete) -> Result<()> {
        let core = Arc::clone(&self.core);
        let recursive = args.recursive();
        blocking(move || core.root.remove(&path, recursive)).await
    }
}

/// Where a listing is.
enum ListState {
    /// Nothing read yet.
    Unopened,
    /// The listed directory, and the entries read but not yet returned.
    Reading {
        dir: Arc<OwnedFd>,
        entries: Arc<Mutex<rustix::fs::Dir>>,
        batch: std::vec::IntoIter<(OsString, Node)>,
        exhausted: bool,
    },
    /// Nothing more to return.
    Done,
}

/// Lists one directory level, in the shape opendal's `fs` lister uses: the
/// directory itself first, then each entry by its path from the root, with a
/// trailing `/` on directories.
pub(crate) struct ConfinedLister {
    core: Arc<Core>,
    path: String,
    state: ListState,
}

/// The storage path of the entry `name` in the directory listed as `listed`.
fn entry_path(listed: &str, name: &OsStr, kind: NodeKind) -> String {
    let name = String::from_utf8_lossy(name.as_bytes());
    let parent = listed.trim_start_matches('/');
    let mut path = if parent.is_empty() {
        name.into_owned()
    } else if parent.ends_with('/') {
        format!("{parent}{name}")
    } else {
        format!("{parent}/{name}")
    };
    if kind == NodeKind::Dir {
        path.push('/');
    }
    path
}

impl oio::List for ConfinedLister {
    async fn next(&mut self) -> Result<Option<oio::Entry>> {
        loop {
            match &mut self.state {
                ListState::Done => return Ok(None),
                ListState::Unopened => {
                    let core = Arc::clone(&self.core);
                    let path = self.path.clone();
                    let opened = blocking(move || {
                        let Some(dir) = core.root.open_dir(&path)? else {
                            return Ok(None);
                        };
                        let entries = rustix::fs::Dir::read_from(&dir)?;
                        Ok(Some((dir, entries)))
                    })
                    .await?;
                    let Some((dir, entries)) = opened else {
                        self.state = ListState::Done;
                        return Ok(None);
                    };
                    self.state = ListState::Reading {
                        dir: Arc::new(dir),
                        entries: Arc::new(Mutex::new(entries)),
                        batch: Vec::new().into_iter(),
                        exhausted: false,
                    };
                    // A listing returns the directory itself first.
                    return Ok(Some(oio::Entry::new(
                        &self.path,
                        Metadata::new(EntryMode::DIR),
                    )));
                }
                ListState::Reading {
                    dir,
                    entries,
                    batch,
                    exhausted,
                } => {
                    if let Some((name, node)) = batch.next() {
                        let path = entry_path(&self.path, &name, node.kind);
                        return Ok(Some(oio::Entry::new(&path, metadata_of(&node)?)));
                    }
                    if *exhausted {
                        self.state = ListState::Done;
                        return Ok(None);
                    }
                    let dir = Arc::clone(dir);
                    let entries = Arc::clone(entries);
                    let (next, done) = blocking(move || read_batch(&dir, &entries)).await?;
                    *batch = next.into_iter();
                    *exhausted = done;
                }
            }
        }
    }
}

/// Read up to [`LIST_BATCH`] entries of a listing, with the facts each one
/// needs, and whether the directory has no more.
fn read_batch(
    dir: &OwnedFd,
    entries: &Mutex<rustix::fs::Dir>,
) -> std::io::Result<(Vec<(OsString, Node)>, bool)> {
    let mut entries = entries
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut batch = Vec::new();
    while batch.len() < LIST_BATCH {
        let Some(entry) = entries.read() else {
            return Ok((batch, true));
        };
        let entry = entry?;
        let name = OsStr::from_bytes(entry.file_name().to_bytes());
        if name == "." || name == ".." {
            continue;
        }
        // An entry deleted between the directory read and its stat is
        // skipped, as a listing of a directory being written to must allow.
        if let Some(node) = ConfinedRoot::entry_node(dir, name)? {
            batch.push((name.to_os_string(), node));
        }
    }
    Ok((batch, false))
}
