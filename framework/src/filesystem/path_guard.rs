//! Path-confinement layer for local-filesystem disks.
//!
//! OpenDAL's `Operator` runs `normalize_path` before the accessor - it strips a
//! leading `/` and collapses `//`, but it does NOT resolve `..`. A `..`
//! component therefore reaches the FS backend, which joins it onto the disk
//! root, so `disk.write("../escaped.txt", ..)` escapes the configured root and
//! grants arbitrary read/write/delete outside the disk. This is a custom
//! [`Layer`] that rejects any path which would leave the root.
//!
//! The guard is applied only to local-filesystem disks (`register_fs` /
//! `register_fs_with`). Object-store backends (S3, Azure Blob, GCS) and the
//! in-memory backend confine to a bucket/prefix or have no filesystem at all,
//! where `..` is just an ordinary key character - guarding them would wrongly
//! reject legitimate keys.
//!
//! # Symlink confinement
//!
//! The lexical check ([`validate_storage_path`]) is the first, cheap gate, but
//! it only confines `..`/absolute path *components*. A symlink planted inside
//! the root that points outside it survives the lexical check yet escapes the
//! root once the kernel follows it - a real second-stage traversal vector (an
//! uploaded/extracted symlink, then a read/write through it). After the lexical
//! gate, [`validate_resolved_path`] canonicalizes the on-disk target (resolving
//! every symlink) and re-checks that the canonical path is still inside the
//! canonicalized disk root. For paths that do not exist yet (new writes), the
//! parent directory is canonicalized instead, so the destination directory
//! cannot itself be a symlink leading out of the root. Anything that resolves
//! outside the root is rejected.
//!
//! # The reserved staging directory
//!
//! A local-filesystem disk stages every non-append write under
//! [`ATOMIC_STAGING_DIR`] inside its own root and renames the result onto the
//! target. That directory therefore sits in the caller's namespace, where it
//! would otherwise be an ordinary object: readable, writable, deletable, and
//! visible in a listing. None of that is acceptable - reading another writer's
//! temp file exposes a half-written object, writing into the directory can
//! collide with a name opendal is about to rename away, and deleting it breaks
//! every subsequent write. So the guard reserves the name: any path whose first
//! component is [`ATOMIC_STAGING_DIR`] is refused, and the entry is filtered out
//! of listings ([`PathGuardLister`]) so it never appears as an object.
//!
//! The staging writes themselves are unaffected: opendal opens the temp file
//! inside the FS backend, below this layer, so they never pass through the
//! reservation check.

use super::ATOMIC_STAGING_DIR;
use opendal::raw::{
    Layer, OpCopier, OpCopy, OpCreateDir, OpDelete, OpList, OpPresign, OpRead, OpRename, OpStat,
    OpWrite, RpCreateDir, RpPresign, RpRead, RpRename, RpStat, Service, ServiceInfo, Servicer, oio,
};
use opendal::{
    Buffer, BytesRange, Capability, Error, ErrorKind, Metadata, OperationContext, Result,
};
use std::path::{Component, Path};
use std::sync::Arc;

/// Reject any path that could escape the local-filesystem disk root.
///
/// Rejects a path whose components include a parent-directory (`..`) hop or an
/// absolute/root prefix. A `..` appearing only as a *substring* of a single
/// path segment (e.g. `my..file.txt`) is allowed - the check is component-wise.
/// The separator-agnostic split is belt-and-suspenders: `\` is an ordinary
/// character on Unix (where [`Path::components`] would not split on it) but a
/// separator on Windows, so splitting on both keeps the guard correct wherever
/// it runs.
fn validate_storage_path(path: &str) -> Result<()> {
    // opendal's `normalize_path` collapses an empty path and the disk root to
    // the single indicator "/", which is what reaches this layer for a
    // root-level list/stat. That is the disk root itself, not an escape, so it
    // is allowed. Every other path arrives with its leading `/` already
    // stripped (so a `RootDir` component below can only come from a caller that
    // bypassed normalization - kept rejected as defense-in-depth).
    if path == "/" {
        return Ok(());
    }

    let has_parent_segment = path.split(['/', '\\']).any(|segment| segment == "..");
    let has_traversal_component = Path::new(path).components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    });

    if has_parent_segment || has_traversal_component {
        tracing::warn!(
            path = %path,
            "rejected storage path traversal attempt on local-filesystem disk"
        );
        return Err(Error::new(
            ErrorKind::PermissionDenied,
            format!(
                "path '{path}' is not allowed on a local-filesystem disk: \
                 paths must stay within the disk root (no '..' or absolute components)"
            ),
        ));
    }

    if reaches_staging_directory(path) {
        tracing::warn!(
            path = %path,
            "rejected storage path targeting the reserved atomic-write staging \
             directory on local-filesystem disk"
        );
        return Err(Error::new(
            ErrorKind::PermissionDenied,
            format!(
                "path '{path}' is not allowed on a local-filesystem disk: \
                 '{ATOMIC_STAGING_DIR}' at the disk root is reserved for staging \
                 atomic writes"
            ),
        ));
    }

    Ok(())
}

/// True when `path`'s first meaningful component is the reserved staging
/// directory - the directory itself or anything under it.
///
/// Only the *first* component is reserved: the staging directory exists at the
/// disk root and nowhere else, so `reports/.suprnova-atomic/june.csv` is an
/// ordinary object and must stay reachable. Empty segments (from a leading or
/// doubled separator) and `.` segments are skipped so `./.suprnova-atomic/x`
/// and `//.suprnova-atomic` cannot slip past by punctuation alone. The split is
/// separator-agnostic for the same reason [`validate_storage_path`]'s is.
fn reaches_staging_directory(path: &str) -> bool {
    path.split(['/', '\\'])
        .find(|segment| !segment.is_empty() && *segment != ".")
        .is_some_and(|first| first == ATOMIC_STAGING_DIR)
}

/// Build the `PermissionDenied` error returned when a path resolves outside the
/// disk root via a symlink (or any other on-disk indirection).
fn symlink_escape_error(path: &str) -> Error {
    tracing::warn!(
        path = %path,
        "rejected storage path that resolves outside the disk root (symlink escape) \
         on local-filesystem disk"
    );
    Error::new(
        ErrorKind::PermissionDenied,
        format!(
            "path '{path}' is not allowed on a local-filesystem disk: \
             it resolves (via a symlink) outside the disk root"
        ),
    )
}

/// Second-stage guard: after the lexical [`validate_storage_path`] check passes,
/// resolve the on-disk target and confirm it is still inside the disk root.
///
/// `root` is the local-filesystem disk root as reported by the inner accessor
/// ([`opendal::raw::AccessorInfo::root`]); the FS backend already canonicalized
/// it at build time, so it is an absolute, symlink-free directory. `path` is the
/// normalized, leading-`/`-stripped storage path that reached the accessor.
///
/// The full on-disk path is `root + path`. If it exists, it is canonicalized
/// (which resolves every symlink component) and must lie under the canonical
/// root. If it does not exist yet - the common case for a new write - we walk
/// the target's ancestors up to the *nearest ancestor that actually exists* and
/// canonicalize that one, so a symlinked ancestor directory is still rejected
/// even before the leaf (and any intermediate dirs) are created. Components that
/// exist nowhere on disk are the only ones safe to create under the root; an
/// existing ancestor that resolves (or traverses a symlink) outside the root is
/// an escape and is rejected.
///
/// Canonicalization uses `tokio::fs` so it never blocks the async executor,
/// matching the FS backend's own `tokio::fs`-based IO.
async fn validate_resolved_path(root: &str, path: &str) -> Result<()> {
    validate_storage_path(path)?;

    // The post-normalize root indicator is the disk root itself - already inside.
    if path == "/" || path.is_empty() {
        return Ok(());
    }

    let canonical_root = tokio::fs::canonicalize(root).await.map_err(|e| {
        Error::new(
            ErrorKind::Unexpected,
            "canonicalize of local-filesystem disk root failed",
        )
        .set_source(e)
    })?;

    // A trailing `/` (directory marker) doesn't change which on-disk node the
    // path refers to; strip it so `Path` joins cleanly.
    let relative = path.trim_end_matches('/');
    let target = Path::new(root).join(relative);

    // Walk from the leaf upward to the nearest ancestor that exists on disk and
    // canonicalize *that*. `target.ancestors()` yields the target first, then
    // each successive parent, so the first one that resolves is either the leaf
    // itself (existing target) or the deepest existing directory above it (new
    // write). Confining only the *immediate* parent - as an earlier version did -
    // let an intermediate symlink escape: if `root/evil -> /outside`, then
    // writing `evil/newdir/payload` has a missing leaf AND a missing immediate
    // parent (`evil/newdir`), so the old early-return treated it as safe while
    // the FS backend would follow `evil` and write to `/outside/newdir/payload`.
    // Resolving the nearest *existing* ancestor (`root/evil`, the symlink) and
    // requiring it to be within the root catches that escape. Components that
    // exist nowhere on disk - `newdir`, `payload` - are the only ones genuinely
    // safe to create under the root, since the kernel can only follow links that
    // already exist.
    let mut resolved: Option<std::path::PathBuf> = None;
    for ancestor in target.ancestors() {
        match tokio::fs::canonicalize(ancestor).await {
            Ok(resolved_ancestor) => {
                resolved = Some(resolved_ancestor);
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                return Err(Error::new(
                    ErrorKind::Unexpected,
                    "canonicalize of storage path failed",
                )
                .set_source(e));
            }
        }
    }

    // No ancestor resolved at all. This only happens if even the disk root is
    // gone, but `canonical_root` above already required it to exist, so fall back
    // to the canonical root for the prefix check.
    let resolved = resolved.unwrap_or_else(|| canonical_root.clone());

    if is_within_root(&canonical_root, &resolved) {
        Ok(())
    } else {
        Err(symlink_escape_error(path))
    }
}

/// True when `resolved` is the canonical root itself or a descendant of it. Both
/// arguments must already be canonical (absolute, symlink-free) so the
/// component-wise prefix check is sound: [`Path::starts_with`] matches whole
/// path components and returns true for equality, so it is not fooled by
/// `/rootevil` vs `/root` the way a lexical string `starts_with` would be.
fn is_within_root(canonical_root: &Path, resolved: &Path) -> bool {
    resolved.starts_with(canonical_root)
}

/// Fetch the inner FS accessor's canonical root once per guarded operation.
/// The FS backend reports an absolute, already-canonicalized root via
/// [`opendal::raw::AccessorInfo::root`].
fn inner_root_string(inner: &Servicer) -> String {
    inner.info().root().as_ref().to_string()
}

/// [`Layer`] that wraps a local-filesystem accessor so every path-bearing
/// operation is confined to the disk root. Applied by `Storage::register_fs*`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PathGuardLayer;

impl Layer for PathGuardLayer {
    fn apply_service(&self, inner: Servicer) -> Servicer {
        Arc::new(PathGuardService { inner })
    }
}

/// The accessor produced by [`PathGuardLayer`]. Validates every path before
/// forwarding to the inner FS accessor.
#[derive(Debug)]
pub(crate) struct PathGuardService {
    inner: Servicer,
}

impl Service for PathGuardService {
    type Reader = PathGuardReader<oio::Reader>;
    type Writer = PathGuardWriter<oio::Writer>;
    type Lister = PathGuardLister<oio::Lister>;
    type Deleter = PathGuardDeleter<oio::Deleter>;
    type Copier = PathGuardCopier<oio::Copier>;

    fn info(&self) -> ServiceInfo {
        self.inner.info()
    }

    fn capability(&self) -> Capability {
        self.inner.capability()
    }

    async fn create_dir(
        &self,
        ctx: &OperationContext,
        path: &str,
        args: OpCreateDir,
    ) -> Result<RpCreateDir> {
        let root = inner_root_string(&self.inner);
        validate_resolved_path(&root, path).await?;
        self.inner.create_dir(ctx, path, args).await
    }

    async fn stat(&self, ctx: &OperationContext, path: &str, args: OpStat) -> Result<RpStat> {
        let root = inner_root_string(&self.inner);
        validate_resolved_path(&root, path).await?;
        self.inner.stat(ctx, path, args).await
    }

    fn read(&self, ctx: &OperationContext, path: &str, args: OpRead) -> Result<Self::Reader> {
        validate_storage_path(path)?;
        let root = inner_root_string(&self.inner);
        let inner = self.inner.read(ctx, path, args)?;
        Ok(PathGuardReader {
            inner,
            root,
            path: path.to_owned(),
        })
    }

    fn write(&self, ctx: &OperationContext, path: &str, args: OpWrite) -> Result<Self::Writer> {
        validate_storage_path(path)?;
        let root = inner_root_string(&self.inner);
        let inner = self.inner.write(ctx, path, args)?;
        Ok(PathGuardWriter {
            inner,
            root,
            path: path.to_owned(),
            validated: false,
        })
    }

    fn delete(&self, ctx: &OperationContext) -> Result<Self::Deleter> {
        let root = inner_root_string(&self.inner);
        let inner = self.inner.delete(ctx)?;
        Ok(PathGuardDeleter { inner, root })
    }

    fn list(&self, ctx: &OperationContext, path: &str, args: OpList) -> Result<Self::Lister> {
        validate_storage_path(path)?;
        let root = inner_root_string(&self.inner);
        let inner = self.inner.list(ctx, path, args)?;
        Ok(PathGuardLister {
            inner,
            root,
            path: path.to_owned(),
            validated: false,
        })
    }

    fn copy(
        &self,
        ctx: &OperationContext,
        from: &str,
        to: &str,
        args: OpCopy,
        opts: OpCopier,
    ) -> Result<Self::Copier> {
        validate_storage_path(from)?;
        validate_storage_path(to)?;
        let root = inner_root_string(&self.inner);
        let inner = self.inner.copy(ctx, from, to, args, opts)?;
        Ok(PathGuardCopier {
            inner,
            root,
            from: from.to_owned(),
            to: to.to_owned(),
            validated: false,
        })
    }

    async fn rename(
        &self,
        ctx: &OperationContext,
        from: &str,
        to: &str,
        args: OpRename,
    ) -> Result<RpRename> {
        let root = inner_root_string(&self.inner);
        validate_resolved_path(&root, from).await?;
        validate_resolved_path(&root, to).await?;
        self.inner.rename(ctx, from, to, args).await
    }

    async fn presign(
        &self,
        ctx: &OperationContext,
        path: &str,
        args: OpPresign,
    ) -> Result<RpPresign> {
        let root = inner_root_string(&self.inner);
        validate_resolved_path(&root, path).await?;
        self.inner.presign(ctx, path, args).await
    }
}

pub(crate) struct PathGuardReader<R> {
    inner: R,
    root: String,
    path: String,
}

impl<R: oio::Read> oio::Read for PathGuardReader<R> {
    async fn open(&self, range: BytesRange) -> Result<(RpRead, Box<dyn oio::ReadStreamDyn>)> {
        validate_resolved_path(&self.root, &self.path).await?;
        self.inner.open(range).await
    }

    async fn read(&self, range: BytesRange) -> Result<(RpRead, Buffer)> {
        validate_resolved_path(&self.root, &self.path).await?;
        self.inner.read(range).await
    }
}

pub(crate) struct PathGuardWriter<W> {
    inner: W,
    root: String,
    path: String,
    validated: bool,
}

impl<W> PathGuardWriter<W> {
    async fn validate_once(&mut self) -> Result<()> {
        if !self.validated {
            validate_resolved_path(&self.root, &self.path).await?;
            self.validated = true;
        }
        Ok(())
    }
}

impl<W: oio::Write> oio::Write for PathGuardWriter<W> {
    async fn write(&mut self, buffer: Buffer) -> Result<()> {
        self.validate_once().await?;
        self.inner.write(buffer).await
    }

    async fn close(&mut self) -> Result<Metadata> {
        self.validate_once().await?;
        self.inner.close().await
    }

    async fn abort(&mut self) -> Result<()> {
        // Aborting cannot create or expose data. Always forward it so an inner
        // writer can release resources even when the path disappears after
        // activation or validation can no longer complete.
        self.inner.abort().await
    }
}

pub(crate) struct PathGuardLister<L> {
    inner: L,
    root: String,
    path: String,
    validated: bool,
}

impl<L> PathGuardLister<L> {
    async fn validate_once(&mut self) -> Result<()> {
        if !self.validated {
            validate_resolved_path(&self.root, &self.path).await?;
            self.validated = true;
        }
        Ok(())
    }
}

impl<L: oio::List> oio::List for PathGuardLister<L> {
    async fn next(&mut self) -> Result<Option<oio::Entry>> {
        // The FS lister returns normalized paths relative to the configured
        // root and does not follow each entry's symlink target. The caller's
        // requested list root is the confinement boundary to validate once.
        self.validate_once().await?;

        // Drop the reserved staging directory before it reaches the caller.
        // Entry paths are relative to the disk root, not to the requested list
        // prefix, so the same first-component test works at every depth. This
        // is also what stops a recursive listing from descending into staging:
        // opendal's FS lister is not recursive, so the recursion happens above
        // this layer and only follows directory entries it is handed.
        loop {
            match self.inner.next().await? {
                Some(entry) if reaches_staging_directory(entry.path()) => continue,
                other => return Ok(other),
            }
        }
    }
}

pub(crate) struct PathGuardCopier<C> {
    inner: C,
    root: String,
    from: String,
    to: String,
    validated: bool,
}

impl<C> PathGuardCopier<C> {
    async fn validate_once(&mut self) -> Result<()> {
        if !self.validated {
            validate_resolved_path(&self.root, &self.from).await?;
            validate_resolved_path(&self.root, &self.to).await?;
            self.validated = true;
        }
        Ok(())
    }
}

impl<C: oio::Copy> oio::Copy for PathGuardCopier<C> {
    async fn next(&mut self) -> Result<Option<usize>> {
        self.validate_once().await?;
        self.inner.next().await
    }

    async fn close(&mut self) -> Result<Metadata> {
        self.validate_once().await?;
        self.inner.close().await
    }

    async fn abort(&mut self) -> Result<()> {
        // Abort is cleanup-only and must never be suppressed by path validation.
        self.inner.abort().await
    }
}

/// The deleter produced by [`PathGuardService`]. `delete(path)` is where the
/// deletion path arrives, so it is validated here before forwarding. It carries
/// the disk root captured at `delete()` time so it can run the same
/// resolved-path (symlink) check as the other operations.
pub(crate) struct PathGuardDeleter<D> {
    inner: D,
    root: String,
}

impl<D: oio::Delete> oio::Delete for PathGuardDeleter<D> {
    async fn delete(&mut self, path: &str, args: OpDelete) -> Result<()> {
        validate_resolved_path(&self.root, path).await?;
        self.inner.delete(path, args).await
    }

    async fn close(&mut self) -> Result<()> {
        self.inner.close().await
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ATOMIC_STAGING_DIR, PathGuardCopier, PathGuardLister, PathGuardWriter,
        validate_resolved_path, validate_storage_path,
    };
    use opendal::raw::oio::{self, Copy as _, List as _, Write as _};
    use opendal::{Buffer, EntryMode, ErrorKind, Metadata, Result};

    #[derive(Default)]
    struct RecordingWriter {
        writes: usize,
        closes: usize,
        aborts: usize,
    }

    impl oio::Write for RecordingWriter {
        async fn write(&mut self, _buffer: Buffer) -> Result<()> {
            self.writes += 1;
            Ok(())
        }

        async fn close(&mut self) -> Result<Metadata> {
            self.closes += 1;
            Ok(Metadata::new(EntryMode::FILE))
        }

        async fn abort(&mut self) -> Result<()> {
            self.aborts += 1;
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingLister {
        calls: usize,
    }

    impl oio::List for RecordingLister {
        async fn next(&mut self) -> Result<Option<oio::Entry>> {
            self.calls += 1;
            if self.calls == 1 {
                Ok(Some(oio::Entry::new(
                    "dir/item.txt",
                    Metadata::new(EntryMode::FILE),
                )))
            } else {
                Ok(None)
            }
        }
    }

    #[derive(Default)]
    struct RecordingCopier {
        nexts: usize,
        closes: usize,
        aborts: usize,
    }

    impl oio::Copy for RecordingCopier {
        async fn next(&mut self) -> Result<Option<usize>> {
            self.nexts += 1;
            Ok((self.nexts == 1).then_some(4))
        }

        async fn close(&mut self) -> Result<Metadata> {
            self.closes += 1;
            Ok(Metadata::new(EntryMode::FILE))
        }

        async fn abort(&mut self) -> Result<()> {
            self.aborts += 1;
            Ok(())
        }
    }

    #[test]
    fn rejects_traversal_and_absolute_paths() {
        for bad in [
            "..",
            "../x",
            "../../x",
            "a/../../b",
            "a/b/../../../c",
            "./../x",
            "foo/..",
            "/abs/x",
            "/etc/passwd",
            "..\\windows",
            "a\\..\\b",
        ] {
            assert!(
                validate_storage_path(bad).is_err(),
                "must reject traversal path {bad:?}"
            );
        }
    }

    #[test]
    fn allows_legitimate_paths() {
        for ok in [
            "a.txt",
            "a/b/c.txt",
            "my..file.txt",
            "deeply/nested/ok.bin",
            "./relative.txt",
            "name..with..dots.txt",
            "",
            "dir/",
            // The post-normalize root indicator (root list/stat) - allowed.
            "/",
        ] {
            assert!(
                validate_storage_path(ok).is_ok(),
                "must allow legitimate path {ok:?}"
            );
        }
    }

    // ------------------------------------------------------------------
    // The reserved atomic-write staging directory.
    // ------------------------------------------------------------------

    #[test]
    fn rejects_every_spelling_of_the_reserved_staging_directory() {
        for bad in [
            ATOMIC_STAGING_DIR,
            ".suprnova-atomic/",
            ".suprnova-atomic/leaked.tmp",
            ".suprnova-atomic/nested/leaked.tmp",
            // Punctuation that opendal's own normalization may or may not have
            // collapsed by the time the path reaches this layer.
            "./.suprnova-atomic/leaked.tmp",
            ".suprnova-atomic\\leaked.tmp",
        ] {
            let err = validate_storage_path(bad)
                .expect_err("the reserved staging directory must be refused: {bad:?}");
            assert_eq!(
                err.kind(),
                ErrorKind::PermissionDenied,
                "the refusal for {bad:?} must be PermissionDenied, got: {err}"
            );
            let message = err.to_string();
            assert!(
                message.contains(ATOMIC_STAGING_DIR) && message.contains("reserved"),
                "the refusal for {bad:?} must name the reservation, got: {message}"
            );
        }

        // A leading separator is refused a step earlier, by the absolute-path
        // gate, so it never reaches the reservation branch. Still refused.
        assert!(
            validate_storage_path("//.suprnova-atomic").is_err(),
            "a leading-separator spelling must still be refused"
        );
    }

    #[test]
    fn allows_the_reserved_name_anywhere_but_the_first_component() {
        // The staging directory exists at the disk root and nowhere else, so
        // reserving the name deeper down would refuse ordinary objects.
        for ok in [
            "reports/.suprnova-atomic",
            "reports/.suprnova-atomic/june.csv",
            ".suprnova-atomic-backup.txt",
            ".suprnova-atomicx/file.txt",
            "a/b/.suprnova-atomic/c",
        ] {
            assert!(
                validate_storage_path(ok).is_ok(),
                "must allow the reserved name below the root: {ok:?}"
            );
        }
    }

    /// A lister that replays a fixed sequence of entry paths, so the filter can
    /// be observed without a real filesystem.
    struct SequenceLister {
        entries: std::vec::IntoIter<&'static str>,
    }

    impl SequenceLister {
        fn new(entries: Vec<&'static str>) -> Self {
            Self {
                entries: entries.into_iter(),
            }
        }
    }

    impl oio::List for SequenceLister {
        async fn next(&mut self) -> Result<Option<oio::Entry>> {
            Ok(self.entries.next().map(|path| {
                let mode = if path.ends_with('/') {
                    EntryMode::DIR
                } else {
                    EntryMode::FILE
                };
                oio::Entry::new(path, Metadata::new(mode))
            }))
        }
    }

    #[tokio::test]
    async fn lister_drops_the_reserved_staging_directory_at_every_depth() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("root");
        std::fs::create_dir_all(&root).expect("create root");

        let mut lister = PathGuardLister {
            inner: SequenceLister::new(vec![
                ".suprnova-atomic/",
                "visible.txt",
                ".suprnova-atomic/big.bin.a1b2c3d4",
                "reports/.suprnova-atomic/june.csv",
                "real-dir/",
            ]),
            root: canonical_root(&root),
            path: "/".to_owned(),
            validated: false,
        };

        let mut seen = Vec::new();
        while let Some(entry) = lister.next().await.expect("the listing advances") {
            seen.push(entry.path().to_owned());
        }

        assert_eq!(
            seen,
            vec![
                "visible.txt".to_string(),
                // Only the *first* component is reserved, so this one is an
                // ordinary object and must survive the filter.
                "reports/.suprnova-atomic/june.csv".to_string(),
                "real-dir/".to_string(),
            ],
            "the staging directory and its contents must never reach the caller"
        );
    }

    // ------------------------------------------------------------------
    // Symlink confinement (`validate_resolved_path`). The lexical gate is
    // clean for every path below - these escapes only manifest once the
    // on-disk symlink is followed, which is exactly what the resolved check
    // catches.
    // ------------------------------------------------------------------

    /// Canonical disk root for the test, mirroring the FS backend (which
    /// canonicalizes its root at build time). On macOS `/tmp` is itself a
    /// symlink, so the root must be canonicalized for the prefix check to hold.
    fn canonical_root(dir: &std::path::Path) -> String {
        std::fs::canonicalize(dir)
            .expect("canonicalize test root")
            .to_string_lossy()
            .into_owned()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlink_pointing_outside_root_is_rejected_for_existing_target() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("root");
        std::fs::create_dir_all(&root).expect("create root");
        // A real directory OUTSIDE the root, with a secret file in it.
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside).expect("create outside");
        std::fs::write(outside.join("secret.txt"), b"TOP SECRET").expect("plant secret");
        // A symlink INSIDE the root that points at the outside directory.
        std::os::unix::fs::symlink(&outside, root.join("escape")).expect("create escaping symlink");

        let root_str = canonical_root(&root);
        // Reading/stat-ing through the symlink resolves to the outside file and
        // must be rejected even though "escape/secret.txt" is lexically clean.
        assert!(
            validate_resolved_path(&root_str, "escape/secret.txt")
                .await
                .is_err(),
            "read through a symlink that escapes the root must be rejected"
        );
        // Writing a NEW file through the escaping symlink: the leaf doesn't
        // exist, so the parent ("escape" -> outside) is canonicalized and must
        // be rejected.
        assert!(
            validate_resolved_path(&root_str, "escape/newfile.txt")
                .await
                .is_err(),
            "write through a symlinked directory that escapes the root must be rejected"
        );
        // The symlink target itself, resolved, is outside the root.
        assert!(
            validate_resolved_path(&root_str, "escape").await.is_err(),
            "operating on the escaping symlink node (resolved) must be rejected"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlink_ancestor_escape_with_missing_immediate_parent_is_rejected() {
        // The escape the nearest-existing-ancestor walk closes: the symlinked
        // ancestor is NOT the immediate parent of the write target - both the
        // leaf and its immediate parent don't exist, so an
        // immediate-parent-only check would canonicalize NotFound and wave the
        // write through, letting the FS backend follow the symlink out of root.
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("root");
        std::fs::create_dir_all(&root).expect("create root");
        // A real directory OUTSIDE the root.
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside).expect("create outside");
        // A symlink INSIDE the root that points at the outside directory.
        std::os::unix::fs::symlink(&outside, root.join("evil")).expect("create escaping symlink");

        let root_str = canonical_root(&root);
        // Writing `evil/newdir/payload`: leaf missing AND immediate parent
        // (`evil/newdir`) missing, but `evil` -> outside exists. The walk must
        // resolve `evil` (the nearest existing ancestor), see it escapes, and
        // reject. `outside/newdir/payload` would otherwise be created off-root.
        assert!(
            validate_resolved_path(&root_str, "evil/newdir/payload")
                .await
                .is_err(),
            "write whose nearest existing ancestor is an escaping symlink must be rejected"
        );
        // A deeper missing chain through the same symlink is rejected too.
        assert!(
            validate_resolved_path(&root_str, "evil/a/b/c/payload")
                .await
                .is_err(),
            "an even deeper missing chain through the escaping symlink must be rejected"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlink_pointing_inside_root_is_allowed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("root");
        let real_dir = root.join("real");
        std::fs::create_dir_all(&real_dir).expect("create nested dir");
        std::fs::write(real_dir.join("data.txt"), b"inside").expect("write data");
        // A symlink inside the root that points at another location inside the
        // root - legitimate, must be allowed.
        std::os::unix::fs::symlink(&real_dir, root.join("link")).expect("create inside symlink");

        let root_str = canonical_root(&root);
        assert!(
            validate_resolved_path(&root_str, "link/data.txt")
                .await
                .is_ok(),
            "a symlink that stays inside the root must be allowed"
        );
    }

    #[tokio::test]
    async fn legitimate_nested_path_passes_resolved_check() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("root");
        std::fs::create_dir_all(root.join("a/b")).expect("create nested dirs");
        std::fs::write(root.join("a/b/c.txt"), b"deep").expect("write file");

        let root_str = canonical_root(&root);
        // Existing nested file: canonicalizes to a descendant of the root.
        assert!(validate_resolved_path(&root_str, "a/b/c.txt").await.is_ok());
        // New file under an existing nested dir: parent canonicalizes inside.
        assert!(
            validate_resolved_path(&root_str, "a/b/new.txt")
                .await
                .is_ok()
        );
        // New file under a NOT-yet-existing nested dir: parent missing, which
        // the backend will create under the root - allowed.
        assert!(validate_resolved_path(&root_str, "x/y/z.txt").await.is_ok());
        // The root indicator itself.
        assert!(validate_resolved_path(&root_str, "/").await.is_ok());
        assert!(validate_resolved_path(&root_str, "").await.is_ok());
    }

    #[tokio::test]
    async fn lexical_escape_is_still_rejected_before_resolution() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("root");
        std::fs::create_dir_all(&root).expect("create root");

        let root_str = canonical_root(&root);
        // The cheap lexical gate fires first, before any filesystem touch.
        assert!(
            validate_resolved_path(&root_str, "../escaped.txt")
                .await
                .is_err()
        );
        assert!(
            validate_resolved_path(&root_str, "a/../../b")
                .await
                .is_err()
        );
        assert!(
            validate_resolved_path(&root_str, "/etc/passwd")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn writer_validates_once_and_forwards_close_after_activation() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("root");
        std::fs::create_dir_all(&root).expect("create root");

        let mut writer = PathGuardWriter {
            inner: RecordingWriter::default(),
            root: canonical_root(&root),
            path: "file.txt".to_owned(),
            validated: false,
        };

        writer
            .write(Buffer::from("first"))
            .await
            .expect("first write activates the validated writer");
        std::fs::remove_dir_all(&root).expect("remove root after activation");
        writer
            .write(Buffer::from("second"))
            .await
            .expect("active writer must not revalidate");
        writer
            .close()
            .await
            .expect("active writer close must always forward");

        assert_eq!(writer.inner.writes, 2);
        assert_eq!(writer.inner.closes, 1);
    }

    #[tokio::test]
    async fn writer_forwards_abort_after_activation_without_revalidation() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("root");
        std::fs::create_dir_all(&root).expect("create root");

        let mut writer = PathGuardWriter {
            inner: RecordingWriter::default(),
            root: canonical_root(&root),
            path: "file.txt".to_owned(),
            validated: false,
        };

        writer
            .write(Buffer::from("partial"))
            .await
            .expect("first write activates the validated writer");
        std::fs::remove_dir_all(&root).expect("remove root after activation");
        writer
            .abort()
            .await
            .expect("active writer abort must always forward");

        assert_eq!(writer.inner.writes, 1);
        assert_eq!(writer.inner.aborts, 1);
    }

    #[tokio::test]
    async fn lister_validates_once_per_session() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("root");
        std::fs::create_dir_all(root.join("dir")).expect("create list root");

        let mut lister = PathGuardLister {
            inner: RecordingLister::default(),
            root: canonical_root(&root),
            path: "dir/".to_owned(),
            validated: false,
        };

        assert!(lister.next().await.expect("first list item").is_some());
        std::fs::remove_dir_all(&root).expect("remove root after activation");
        assert!(
            lister
                .next()
                .await
                .expect("active lister must not revalidate")
                .is_none()
        );
        assert_eq!(lister.inner.calls, 2);
    }

    #[tokio::test]
    async fn copier_validates_once_and_forwards_close_after_activation() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("root");
        std::fs::create_dir_all(&root).expect("create root");
        std::fs::write(root.join("source.txt"), b"copy").expect("create source");

        let mut copier = PathGuardCopier {
            inner: RecordingCopier::default(),
            root: canonical_root(&root),
            from: "source.txt".to_owned(),
            to: "destination.txt".to_owned(),
            validated: false,
        };

        assert_eq!(copier.next().await.expect("activate copier"), Some(4));
        std::fs::remove_dir_all(&root).expect("remove root after activation");
        assert_eq!(
            copier
                .next()
                .await
                .expect("active copier must not revalidate"),
            None
        );
        copier
            .close()
            .await
            .expect("active copier close must always forward");

        assert_eq!(copier.inner.nexts, 2);
        assert_eq!(copier.inner.closes, 1);
    }

    #[tokio::test]
    async fn copier_forwards_abort_after_activation_without_revalidation() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("root");
        std::fs::create_dir_all(&root).expect("create root");
        std::fs::write(root.join("source.txt"), b"copy").expect("create source");

        let mut copier = PathGuardCopier {
            inner: RecordingCopier::default(),
            root: canonical_root(&root),
            from: "source.txt".to_owned(),
            to: "destination.txt".to_owned(),
            validated: false,
        };

        assert_eq!(copier.next().await.expect("activate copier"), Some(4));
        std::fs::remove_dir_all(&root).expect("remove root after activation");
        copier
            .abort()
            .await
            .expect("active copier abort must always forward");

        assert_eq!(copier.inner.nexts, 1);
        assert_eq!(copier.inner.aborts, 1);
    }

    #[tokio::test]
    async fn copier_forwards_abort_without_validation_when_not_activated() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("root");
        std::fs::create_dir_all(&root).expect("create root");
        std::fs::write(root.join("source.txt"), b"copy").expect("create source");

        let mut copier = PathGuardCopier {
            inner: RecordingCopier::default(),
            root: canonical_root(&root),
            from: "source.txt".to_owned(),
            to: "destination.txt".to_owned(),
            validated: false,
        };

        std::fs::remove_dir_all(&root).expect("remove root before activation");
        copier
            .abort()
            .await
            .expect("unactivated copier abort must always forward");
        assert_eq!(copier.inner.aborts, 1);
    }
}
