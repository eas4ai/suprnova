//! Read-through layer for composite disks.
//!
//! A read-through disk pairs a fast *primary* with a slower *fallback* and
//! migrates objects from the second to the first as they are read. It exists
//! for the migration case: point the primary at the new store, the fallback at
//! the old one, and the working set moves across under real traffic instead of
//! during a maintenance window.
//!
//! The composite is an [`opendal::raw::Layer`] rather than a wrapper type on
//! purpose. `Storage::disk(name)` hands back an `Operator`, and every Laravel
//! convenience on [`crate::DiskExt`] is an extension trait over that operator -
//! so a composite that is still an `Operator` inherits the entire surface for
//! free, including anything opendal adds later. `path_guard::PathGuardLayer` is
//! the same shape and the precedent this module follows.
//!
//! # Which disk answers which operation
//!
//! | operation | disk |
//! |---|---|
//! | `read` | primary if it holds the object, else the fallback, promoting what it finds |
//! | `stat` (and everything built on it: `exists`, `size`, `last_modified`, `mime_type`) | primary if it holds the object, else the fallback |
//! | `write`, `create_dir` | primary only |
//! | `list` | primary only - fallback entries are invisible to a listing |
//! | `delete` | both, fallback first |
//! | `presign` read/stat | primary if it holds the object, else the fallback |
//! | `presign` write/delete | primary only - an upload has to land where writes land |
//!
//! # Promotion is published atomically
//!
//! A promotion must never be observable half-written, because the object it
//! writes is exactly the one a concurrent reader is about to route by
//! existence. A local filesystem creates the target file and then fills it in
//! place, so a direct write leaves a zero-length object visible for the
//! duration of the write - long enough for another cold reader to see it,
//! delegate to the primary, and read nothing at all with no error to show for
//! it. So the promotion stages the bytes at a unique sibling path and publishes
//! them with a `rename` whenever the primary advertises one. Backends without a
//! rename (memory, S3, Azure Blob, GCS) publish a write as a single indivisible
//! operation, so for those the direct write is already atomic and is what runs.

use opendal::options::{DeleteOptions, ReadOptions, WriteOptions};
use opendal::raw::{
    Layer, OpCopier, OpCopy, OpCreateDir, OpDelete, OpList, OpPresign, OpRead, OpRename, OpStat,
    OpWrite, PresignOperation, RpCreateDir, RpPresign, RpRead, RpRename, RpStat, Service,
    ServiceInfo, Servicer, oio,
};
use opendal::{
    Buffer, BytesRange, Capability, Error, ErrorKind, Metadata, OperationContext, Operator, Result,
};
use std::sync::Arc;
use uuid::Uuid;

/// Build the sibling path a promotion stages its bytes at before renaming them
/// onto `path`.
///
/// The suffix is random so two promoters never collide on the staging object,
/// and the path is a sibling so the rename stays inside the same directory or
/// key prefix - a rename across filesystems is not atomic, and on an object
/// store a cross-prefix rename can be a different operation entirely.
fn staging_path(path: &str) -> String {
    format!("{path}.suprnova-promote-{}.tmp", Uuid::new_v4().simple())
}

/// [`Layer`] that turns the primary disk it wraps into a read-through disk over
/// `fallback`. Applied by `Storage::register_read_through`.
///
/// `primary` is a clone of the operator this layer is applied to, taken before
/// the layer exists. It is not a second disk: it is the same backend reached
/// through the high-level API, which is what lets the promotion write and the
/// existence probes be ordinary `Operator` calls instead of hand-driven raw
/// writers. Because it is the *un-layered* operator, using it cannot recurse
/// back into this layer.
#[derive(Debug, Clone)]
pub(crate) struct ReadThroughLayer {
    /// The primary disk, reached through the high-level operator API.
    pub(crate) primary: Operator,
    /// The disk consulted when the primary does not hold an object.
    pub(crate) fallback: Operator,
    /// Whether a failed promotion fails the read. See [`ReadThroughReader::promote`].
    pub(crate) throw_on_promotion_failure: bool,
}

impl Layer for ReadThroughLayer {
    fn apply_service(&self, inner: Servicer) -> Servicer {
        // Read the primary's capabilities once here rather than per read: they
        // are properties of the backend, not of any single operation.
        let capability = inner.capability();
        Arc::new(ReadThroughService {
            inner,
            primary: self.primary.clone(),
            fallback: self.fallback.clone(),
            throw_on_promotion_failure: self.throw_on_promotion_failure,
            promote_conditionally: capability.write_with_if_not_exists,
            promote_atomically: capability.rename,
        })
    }
}

/// The accessor produced by [`ReadThroughLayer`].
#[derive(Debug)]
pub(crate) struct ReadThroughService {
    /// The primary's raw accessor stack, which every pass-through operation
    /// forwards to with the caller's `Op*` arguments untouched.
    inner: Servicer,
    /// The primary disk as a high-level operator.
    primary: Operator,
    /// The disk consulted when the primary does not hold an object.
    fallback: Operator,
    /// Whether a failed promotion fails the read.
    throw_on_promotion_failure: bool,
    /// Whether the primary can express a "write only if absent" condition.
    promote_conditionally: bool,
    /// Whether the primary can publish a promotion with an atomic `rename`.
    promote_atomically: bool,
}

impl Service for ReadThroughService {
    type Reader = ReadThroughReader;
    type Writer = oio::Writer;
    type Lister = oio::Lister;
    type Deleter = ReadThroughDeleter;
    type Copier = oio::Copier;

    fn info(&self) -> ServiceInfo {
        // The composite's identity is the primary's: that is where writes land
        // and what a listing describes.
        self.inner.info()
    }

    fn capability(&self) -> Capability {
        let mut capability = self.inner.capability();
        let fallback = self.fallback.service().capability();
        // Advertise the union for exactly the operations that resolve against
        // either disk. Everything else - write, list, copy, rename, create_dir -
        // is the primary's alone, so its capability is the honest answer.
        capability.read |= fallback.read;
        capability.stat |= fallback.stat;
        capability.presign |= fallback.presign;
        capability.presign_read |= fallback.presign_read;
        capability.presign_stat |= fallback.presign_stat;
        capability
    }

    async fn create_dir(
        &self,
        ctx: &OperationContext,
        path: &str,
        args: OpCreateDir,
    ) -> Result<RpCreateDir> {
        self.inner.create_dir(ctx, path, args).await
    }

    async fn stat(&self, ctx: &OperationContext, path: &str, args: OpStat) -> Result<RpStat> {
        match self.inner.stat(ctx, path, args.clone()).await {
            Ok(reply) => Ok(reply),
            // Only a genuine miss routes to the fallback. Any other error is a
            // real backend failure and must reach the caller instead of being
            // masked by a second lookup on a different disk.
            Err(e) if e.kind() == ErrorKind::NotFound => {
                self.fallback
                    .service()
                    .stat(self.fallback.context(), path, args)
                    .await
            }
            Err(e) => Err(e),
        }
    }

    fn read(&self, ctx: &OperationContext, path: &str, args: OpRead) -> Result<Self::Reader> {
        // `read` cannot await, so the primary-or-fallback decision happens in
        // the reader. Building the primary's reader here is free: backends
        // return a lazy handle and open nothing until the first range is asked
        // for. The reader keeps its own clone of `args` because a fallback read
        // has to carry the caller's version and conditional headers too.
        let primary_reader = self.inner.read(ctx, path, args.clone())?;
        Ok(ReadThroughReader {
            primary_reader,
            primary: self.primary.clone(),
            fallback: self.fallback.clone(),
            path: path.to_owned(),
            args,
            throw_on_promotion_failure: self.throw_on_promotion_failure,
            promote_conditionally: self.promote_conditionally,
            promote_atomically: self.promote_atomically,
        })
    }

    fn write(&self, ctx: &OperationContext, path: &str, args: OpWrite) -> Result<Self::Writer> {
        // Writes are primary-only: a read-through disk migrates *towards* the
        // primary, so writing to the fallback would move data backwards.
        self.inner.write(ctx, path, args)
    }

    fn delete(&self, ctx: &OperationContext) -> Result<Self::Deleter> {
        Ok(ReadThroughDeleter {
            inner: self.inner.delete(ctx)?,
            fallback: self.fallback.clone(),
        })
    }

    fn list(&self, ctx: &OperationContext, path: &str, args: OpList) -> Result<Self::Lister> {
        // Listing is primary-only. A union listing would have to reconcile
        // paging, ordering, and duplicates across two backends, and it would
        // report objects that a later `list` no longer returns once they are
        // promoted - so the fallback stays invisible to a listing.
        self.inner.list(ctx, path, args)
    }

    fn copy(
        &self,
        ctx: &OperationContext,
        from: &str,
        to: &str,
        args: OpCopy,
        opts: OpCopier,
    ) -> Result<Self::Copier> {
        self.inner.copy(ctx, from, to, args, opts)
    }

    async fn rename(
        &self,
        ctx: &OperationContext,
        from: &str,
        to: &str,
        args: OpRename,
    ) -> Result<RpRename> {
        self.inner.rename(ctx, from, to, args).await
    }

    async fn presign(
        &self,
        ctx: &OperationContext,
        path: &str,
        args: OpPresign,
    ) -> Result<RpPresign> {
        let routes_to_holder = matches!(
            args.operation(),
            PresignOperation::Read(..) | PresignOperation::Stat(_)
        );

        // A write or delete URL must point at the disk that accepts writes, so
        // it is always the primary. A read or stat URL has to point at whichever
        // disk actually holds the object, or the signed URL 404s.
        if routes_to_holder && !self.primary.exists(path).await? {
            // Forwarding the raw `OpPresign` keeps range and content-type
            // overrides the high-level `presign_read` wrapper cannot rebuild.
            return self
                .fallback
                .service()
                .presign(self.fallback.context(), path, args)
                .await;
        }

        self.inner.presign(ctx, path, args).await
    }
}

/// The reader produced by [`ReadThroughService`]. Resolves each range against
/// the primary first and promotes what it has to fetch from the fallback.
pub(crate) struct ReadThroughReader {
    /// The primary's lazy reader, used whenever the primary owns the object.
    primary_reader: oio::Reader,
    /// The primary disk, for the existence probes and the promotion write.
    primary: Operator,
    /// The disk a miss on the primary falls back to.
    fallback: Operator,
    /// The object this reader was opened for.
    path: String,
    /// The caller's read arguments, replayed onto the fallback read.
    args: OpRead,
    /// Whether a failed promotion fails the read.
    throw_on_promotion_failure: bool,
    /// Whether the promotion write can be made conditional on absence.
    promote_conditionally: bool,
    /// Whether the promotion can be published with an atomic `rename`.
    promote_atomically: bool,
}

impl ReadThroughReader {
    /// Whether this read is plain enough for its result to be promoted.
    ///
    /// A versioned read asks for one specific historical object, and a
    /// conditional read asks for the object only if it still matches what the
    /// caller last saw. Writing either answer to the primary under the
    /// unversioned, unconditional path would publish a value the caller never
    /// asked to make current - an old version presented as the live object, or
    /// a body cached under a validator it does not match. Such a read is served
    /// from the fallback and left there.
    fn is_promotable(&self) -> bool {
        self.args.version().is_none()
            && self.args.if_match().is_none()
            && self.args.if_none_match().is_none()
            && self.args.if_modified_since().is_none()
            && self.args.if_unmodified_since().is_none()
    }

    /// The options the fallback read runs under.
    ///
    /// Everything the caller set on the original read that selects *which*
    /// object comes back is replayed here. Dropping any of it would answer a
    /// versioned read with the fallback's current object, or hand back a body
    /// where the caller expected `ConditionNotMatch`. The range is deliberately
    /// left at its default: promotion needs the whole object, and the requested
    /// range is sliced out of it afterwards.
    fn fallback_read_options(&self) -> ReadOptions {
        ReadOptions {
            version: self.args.version().map(str::to_owned),
            if_match: self.args.if_match().map(str::to_owned),
            if_none_match: self.args.if_none_match().map(str::to_owned),
            if_modified_since: self.args.if_modified_since(),
            if_unmodified_since: self.args.if_unmodified_since(),
            ..Default::default()
        }
    }

    /// Resolve one range.
    ///
    /// `Ok(None)` means the primary owns the object and the caller should
    /// delegate to its reader - that keeps ranged and conditional reads on the
    /// backend instead of buffering them here, and it is also how the race
    /// re-check reports "somebody else got there first".
    async fn resolve_from_fallback(&self, range: BytesRange) -> Result<Option<Buffer>> {
        if self.primary.exists(&self.path).await? {
            return Ok(None);
        }

        // Promotion needs the whole object, so the whole object is what we
        // fetch. A fallback-resolved read therefore holds the object in memory
        // until the promotion write completes.
        let full = self
            .fallback
            .read_options(&self.path, self.fallback_read_options())
            .await?;

        // Re-check after the fetch: a writer that landed on the primary while
        // we were pulling the fallback bytes must win, not be overwritten by a
        // stale copy of the cold tier.
        if self.primary.exists(&self.path).await? {
            return Ok(None);
        }

        if self.is_promotable() {
            self.promote(&full).await?;
        }

        let slice = range.to_content_range(full.len())?;
        Ok(Some(full.slice(slice)))
    }

    /// Write a fallback hit through to the primary.
    ///
    /// Failure is a performance problem rather than a read failure unless
    /// `throw_on_promotion_failure` is set: the caller already holds the bytes
    /// it asked for, so an unwritable primary degrades the disk to "read the
    /// fallback every time" instead of taking the application down. Losing a
    /// conditional write is not a failure at all - the object that won came
    /// from the same fallback and holds the same bytes.
    async fn promote(&self, contents: &Buffer) -> Result<()> {
        match self.publish(contents).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == ErrorKind::ConditionNotMatch => Ok(()),
            Err(e) if self.throw_on_promotion_failure => Err(Error::new(
                ErrorKind::Unexpected,
                format!(
                    "read-through promotion of '{}' to the primary disk failed",
                    self.path
                ),
            )
            .set_source(e)),
            Err(e) => {
                tracing::warn!(
                    path = %self.path,
                    error = %e,
                    "read-through promotion to the primary disk failed; serving the fallback bytes"
                );
                Ok(())
            }
        }
    }

    /// Put the promoted bytes on the primary so that no reader can observe them
    /// half-written.
    ///
    /// Where the primary advertises a `rename`, the bytes are staged at a
    /// unique sibling and renamed onto the target, because that backend - the
    /// local filesystem - creates the target file first and fills it in place.
    /// Where it does not, the write itself is the atomic publish and runs
    /// directly, conditional on the object not already existing so two
    /// concurrent readers do not both promote.
    ///
    /// The staged form cannot use that condition: its path is unique, so the
    /// condition would be vacuous. It re-checks the primary immediately before
    /// the rename instead, so a write that lands on the primary inside that
    /// window is overwritten rather than winning.
    ///
    /// Every step here belongs to the promotion, the fallback `stat` included.
    /// The caller's bytes are already in hand by the time this runs, so a
    /// fallback that has just pruned the object or is briefly unreachable has
    /// to degrade the promotion through [`ReadThroughReader::promote`] rather
    /// than fail a read that already succeeded.
    async fn publish(&self, contents: &Buffer) -> Result<()> {
        // The fallback's own metadata rides along with the bytes. Without it an
        // S3-to-S3 read-through would silently drop `Content-Type` the first
        // time each object crossed over, and nothing would ever restore it.
        let metadata = self.fallback.stat(&self.path).await?;

        if !self.promote_atomically {
            let options = self.promotion_options(&metadata, self.promote_conditionally);
            self.primary
                .write_options(&self.path, contents.clone(), options)
                .await?;
            return Ok(());
        }

        let staged = staging_path(&self.path);
        let options = self.promotion_options(&metadata, false);
        if let Err(e) = self
            .primary
            .write_options(&staged, contents.clone(), options)
            .await
        {
            // A backend that creates the target before filling it - the local
            // filesystem does - leaves a partial staging object behind when the
            // write fails part-way, and nothing else ever sweeps it. Deleting a
            // path that was never created is a no-op, so this is safe either
            // way.
            self.discard(&staged).await;
            return Err(e);
        }

        match self.primary.exists(&self.path).await {
            // Somebody published while we were staging. Their object wins.
            Ok(true) => {
                self.discard(&staged).await;
                return Ok(());
            }
            Ok(false) => {}
            // The staged object is already written, so every path out of here
            // has to clean it up or it is left behind for good.
            Err(e) => {
                self.discard(&staged).await;
                return Err(e);
            }
        }

        if let Err(e) = self.primary.rename(&staged, &self.path).await {
            self.discard(&staged).await;
            return Err(e);
        }

        Ok(())
    }

    /// The write options a promotion runs under, carrying the fallback
    /// object's content metadata so the promoted copy is not a downgrade.
    fn promotion_options(&self, metadata: &Metadata, if_not_exists: bool) -> WriteOptions {
        WriteOptions {
            if_not_exists,
            content_type: metadata.content_type().map(str::to_owned),
            cache_control: metadata.cache_control().map(str::to_owned),
            content_disposition: metadata.content_disposition().map(str::to_owned),
            content_encoding: metadata.content_encoding().map(str::to_owned),
            user_metadata: metadata.user_metadata().cloned(),
            ..Default::default()
        }
    }

    /// Remove a staging object that will never be published. Best-effort: the
    /// caller is already returning the bytes or a more useful error, so a
    /// failure here is logged and dropped rather than replacing that outcome.
    async fn discard(&self, staged: &str) {
        if let Err(e) = self.primary.delete(staged).await {
            tracing::warn!(
                path = %self.path,
                staging_path = %staged,
                error = %e,
                "failed to remove a read-through staging object; it will need cleaning up by hand"
            );
        }
    }
}

impl oio::Read for ReadThroughReader {
    async fn open(&self, range: BytesRange) -> Result<(RpRead, Box<dyn oio::ReadStreamDyn>)> {
        match self.resolve_from_fallback(range).await? {
            None => self.primary_reader.open(range).await,
            // `Buffer` is itself a `ReadStream`, so the promoted bytes can be
            // handed back without another adapter type.
            Some(buffer) => Ok((
                RpRead::default(),
                Box::new(buffer) as Box<dyn oio::ReadStreamDyn>,
            )),
        }
    }

    async fn read(&self, range: BytesRange) -> Result<(RpRead, Buffer)> {
        match self.resolve_from_fallback(range).await? {
            None => self.primary_reader.read(range).await,
            Some(buffer) => Ok((RpRead::default(), buffer)),
        }
    }
}

/// The deleter produced by [`ReadThroughService`]. Removes the object from the
/// fallback as well as the primary, so a delete cannot be undone by the next
/// read promoting the cold copy back.
pub(crate) struct ReadThroughDeleter {
    /// The primary's deleter.
    inner: oio::Deleter,
    /// The disk the same delete is replayed against first.
    fallback: Operator,
}

impl oio::Delete for ReadThroughDeleter {
    async fn delete(&mut self, path: &str, args: OpDelete) -> Result<()> {
        // OpenDAL specifies deleting a missing path as a success, so unlike
        // Laravel there is no `fileExists` probe to pay for here. Fallback
        // first, primary second - Laravel's order, and the one that leaves no
        // window where the object is gone from the primary but still
        // promotable from the fallback.
        let options = DeleteOptions {
            version: args.version().map(str::to_owned),
            recursive: args.recursive(),
        };
        self.fallback.delete_options(path, options).await?;
        self.inner.delete(path, args).await
    }

    async fn close(&mut self) -> Result<()> {
        self.inner.close().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opendal::raw::Timestamp;
    use opendal::{EntryMode, services};
    use std::sync::Mutex;

    /// What a [`StubDisk`] was asked to do.
    #[derive(Debug, Default)]
    struct Journal {
        reads: Mutex<Vec<OpRead>>,
        writes: Mutex<Vec<(String, OpWrite)>>,
        renames: Mutex<Vec<(String, String)>>,
        deletes: Mutex<Vec<String>>,
    }

    /// Take a lock without caring whether a failing test poisoned it first.
    fn locked<T>(cell: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        cell.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    impl Journal {
        fn reads(&self) -> Vec<OpRead> {
            locked(&self.reads).clone()
        }

        fn writes(&self) -> Vec<(String, OpWrite)> {
            locked(&self.writes).clone()
        }

        fn write_paths(&self) -> Vec<String> {
            locked(&self.writes)
                .iter()
                .map(|(path, _)| path.clone())
                .collect()
        }

        fn renames(&self) -> Vec<(String, String)> {
            locked(&self.renames).clone()
        }

        fn deletes(&self) -> Vec<String> {
            locked(&self.deletes).clone()
        }
    }

    /// How a [`StubDisk`] behaves when it is written to.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    enum WriteBehaviour {
        /// Refuse to open a writer at all.
        #[default]
        Refuse,
        /// Open a writer and then fail part-way, the way a local filesystem
        /// does when it runs out of room after creating the file.
        FailAfterOpen,
        /// Accept the write.
        Accept,
    }

    /// What a [`StubDisk`] holds and how it behaves.
    #[derive(Debug, Clone, Copy, Default)]
    struct StubSpec {
        contents: Option<&'static str>,
        content_type: Option<&'static str>,
        /// Whether `stat` fails rather than answering.
        stat_fails: bool,
        /// Whether the disk advertises and implements a rename.
        renames: bool,
        writes: WriteBehaviour,
    }

    /// A disk that answers from a fixed body and records what it is asked for.
    ///
    /// It exists because opendal rejects a versioned or conditional read before
    /// it reaches a backend that does not advertise support for one, and
    /// neither the in-memory nor the local-filesystem service does. So whether
    /// this layer replays the caller's read arguments onto the fallback is only
    /// observable against a disk that accepts them - which in production means
    /// an object store, and here means this. It is also the only way to observe
    /// *how* a promotion is published: the staged-and-renamed shape is a
    /// property of the calls the layer makes, not of what a reader can see
    /// afterwards. Reaching it through [`Operator::from_parts`] keeps opendal's
    /// correctness check out of the stack, so every argument arrives exactly as
    /// the layer sent it.
    #[derive(Debug)]
    struct StubDisk {
        info: ServiceInfo,
        spec: StubSpec,
        contents: Option<Buffer>,
        journal: Arc<Journal>,
    }

    impl StubDisk {
        fn operator(spec: StubSpec) -> (Operator, Arc<Journal>) {
            let journal = Arc::new(Journal::default());
            let stub = StubDisk {
                // Borrowing an in-memory disk's identity avoids inventing a
                // scheme; nothing under test reads it.
                info: memory().service().info(),
                spec,
                contents: spec.contents.map(Buffer::from),
                journal: Arc::clone(&journal),
            };
            (
                Operator::from_parts(OperationContext::default(), Arc::new(stub)),
                journal,
            )
        }

        fn missing(&self) -> Error {
            Error::new(ErrorKind::NotFound, "the stub disk does not hold this path")
        }
    }

    struct StubReader(Buffer);

    impl oio::Read for StubReader {
        async fn open(&self, range: BytesRange) -> Result<(RpRead, Box<dyn oio::ReadStreamDyn>)> {
            let slice = range.to_content_range(self.0.len())?;
            Ok((
                RpRead::default(),
                Box::new(self.0.slice(slice)) as Box<dyn oio::ReadStreamDyn>,
            ))
        }

        async fn read(&self, range: BytesRange) -> Result<(RpRead, Buffer)> {
            let slice = range.to_content_range(self.0.len())?;
            Ok((RpRead::default(), self.0.slice(slice)))
        }
    }

    struct StubWriter {
        fails: bool,
    }

    impl StubWriter {
        fn failure() -> Error {
            Error::new(
                ErrorKind::Unexpected,
                "the stub disk ran out of room part-way through the write",
            )
        }
    }

    impl oio::Write for StubWriter {
        async fn write(&mut self, _buffer: Buffer) -> Result<()> {
            if self.fails {
                return Err(Self::failure());
            }
            Ok(())
        }

        async fn close(&mut self) -> Result<Metadata> {
            if self.fails {
                return Err(Self::failure());
            }
            Ok(Metadata::new(EntryMode::FILE))
        }

        async fn abort(&mut self) -> Result<()> {
            // A local filesystem without an atomic write directory removes
            // nothing on abort, which is the case this stub stands in for.
            Ok(())
        }
    }

    struct StubDeleter {
        journal: Arc<Journal>,
    }

    impl oio::Delete for StubDeleter {
        async fn delete(&mut self, path: &str, _args: OpDelete) -> Result<()> {
            locked(&self.journal.deletes).push(path.to_owned());
            Ok(())
        }

        async fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    fn unsupported() -> Error {
        Error::new(ErrorKind::Unsupported, "the stub disk does not do this")
    }

    impl Service for StubDisk {
        type Reader = StubReader;
        type Writer = StubWriter;
        type Lister = oio::Lister;
        type Deleter = StubDeleter;
        type Copier = oio::Copier;

        fn info(&self) -> ServiceInfo {
            self.info.clone()
        }

        fn capability(&self) -> Capability {
            Capability {
                read: true,
                stat: true,
                write: true,
                delete: true,
                rename: self.spec.renames,
                read_with_version: true,
                read_with_if_match: true,
                read_with_if_none_match: true,
                read_with_if_modified_since: true,
                read_with_if_unmodified_since: true,
                write_with_if_not_exists: true,
                write_with_content_type: true,
                write_with_cache_control: true,
                write_with_content_disposition: true,
                write_with_content_encoding: true,
                ..Default::default()
            }
        }

        async fn create_dir(
            &self,
            _ctx: &OperationContext,
            _path: &str,
            _args: OpCreateDir,
        ) -> Result<RpCreateDir> {
            Err(unsupported())
        }

        async fn stat(
            &self,
            _ctx: &OperationContext,
            _path: &str,
            _args: OpStat,
        ) -> Result<RpStat> {
            if self.spec.stat_fails {
                return Err(Error::new(
                    ErrorKind::Unexpected,
                    "the stub disk cannot answer a stat right now",
                ));
            }
            match &self.contents {
                Some(contents) => {
                    let mut metadata =
                        Metadata::new(EntryMode::FILE).with_content_length(contents.len() as u64);
                    if let Some(content_type) = self.spec.content_type {
                        metadata.set_content_type(content_type);
                    }
                    Ok(RpStat::new(metadata))
                }
                None => Err(self.missing()),
            }
        }

        fn read(&self, _ctx: &OperationContext, _path: &str, args: OpRead) -> Result<Self::Reader> {
            locked(&self.journal.reads).push(args);
            // Real backends hand back a lazy reader and only fail once a range
            // is asked for, which is what lets the layer build the primary's
            // reader before it knows whether the primary holds the object.
            Ok(StubReader(self.contents.clone().unwrap_or_default()))
        }

        fn write(
            &self,
            _ctx: &OperationContext,
            path: &str,
            args: OpWrite,
        ) -> Result<Self::Writer> {
            locked(&self.journal.writes).push((path.to_owned(), args));
            match self.spec.writes {
                WriteBehaviour::Refuse => Err(unsupported()),
                WriteBehaviour::FailAfterOpen => Ok(StubWriter { fails: true }),
                WriteBehaviour::Accept => Ok(StubWriter { fails: false }),
            }
        }

        fn delete(&self, _ctx: &OperationContext) -> Result<Self::Deleter> {
            Ok(StubDeleter {
                journal: Arc::clone(&self.journal),
            })
        }

        fn list(
            &self,
            _ctx: &OperationContext,
            _path: &str,
            _args: OpList,
        ) -> Result<Self::Lister> {
            Err(unsupported())
        }

        fn copy(
            &self,
            _ctx: &OperationContext,
            _from: &str,
            _to: &str,
            _args: OpCopy,
            _opts: OpCopier,
        ) -> Result<Self::Copier> {
            Err(unsupported())
        }

        async fn rename(
            &self,
            _ctx: &OperationContext,
            from: &str,
            to: &str,
            _args: OpRename,
        ) -> Result<RpRename> {
            locked(&self.journal.renames).push((from.to_owned(), to.to_owned()));
            Ok(RpRename::default())
        }

        async fn presign(
            &self,
            _ctx: &OperationContext,
            _path: &str,
            _args: OpPresign,
        ) -> Result<RpPresign> {
            Err(unsupported())
        }
    }

    fn memory() -> Operator {
        Operator::new(services::Memory::default()).expect("memory service is infallible")
    }

    /// Compose a read-through disk over two stubs. Returns the composite and
    /// the primary's and fallback's journals.
    fn read_through(
        primary_spec: StubSpec,
        fallback_spec: StubSpec,
        throw_on_promotion_failure: bool,
    ) -> (Operator, Arc<Journal>, Arc<Journal>) {
        let (primary, primary_journal) = StubDisk::operator(primary_spec);
        let (fallback, fallback_journal) = StubDisk::operator(fallback_spec);
        let assets = primary.clone().layer(ReadThroughLayer {
            primary,
            fallback,
            throw_on_promotion_failure,
        });
        (assets, primary_journal, fallback_journal)
    }

    /// The fallback holds `cold bytes` under `cold.txt`; the primary holds
    /// nothing and refuses writes.
    fn stub_read_through() -> (Operator, Arc<Journal>, Arc<Journal>) {
        read_through(
            StubSpec::default(),
            StubSpec {
                contents: Some("cold bytes"),
                ..Default::default()
            },
            false,
        )
    }

    #[tokio::test]
    async fn a_plain_read_reaches_the_fallback_unconditionally_and_is_promoted() {
        let (assets, primary, fallback) = stub_read_through();

        let bytes = assets.read("cold.txt").await.expect("read resolves");
        assert_eq!(&bytes.to_vec(), b"cold bytes");

        let reads = fallback.reads();
        assert_eq!(reads.len(), 1, "the fallback was read once");
        assert!(
            reads[0].version().is_none()
                && reads[0].if_match().is_none()
                && reads[0].if_none_match().is_none()
                && reads[0].if_modified_since().is_none()
                && reads[0].if_unmodified_since().is_none(),
            "a plain read must not invent a version or a condition"
        );

        let writes = primary.writes();
        assert_eq!(
            primary.write_paths(),
            vec!["cold.txt".to_string()],
            "a plain fallback hit is promoted to the requested path"
        );
        assert!(
            writes[0].1.if_not_exists(),
            "a primary without a rename publishes with the no-clobber condition"
        );
    }

    #[tokio::test]
    async fn a_versioned_read_reaches_the_fallback_and_is_not_promoted() {
        let (assets, primary, fallback) = stub_read_through();

        let bytes = assets
            .read_with("cold.txt")
            .version("v7")
            .await
            .expect("a versioned read is still served");
        assert_eq!(&bytes.to_vec(), b"cold bytes");

        let reads = fallback.reads();
        assert_eq!(reads.len(), 1, "the fallback was read once");
        assert_eq!(
            reads[0].version(),
            Some("v7"),
            "the caller's version must reach the fallback, or it answers with \
             whatever is current there"
        );
        assert!(
            primary.write_paths().is_empty(),
            "a versioned read asks for one historical object; publishing it as \
             the primary's live copy would answer later plain reads with it"
        );
    }

    #[tokio::test]
    async fn a_conditional_read_reaches_the_fallback_and_is_not_promoted() {
        let (assets, primary, fallback) = stub_read_through();

        // Two distinct instants, so transposing the two fields fails here.
        let floor = Timestamp::MIN;
        let now = Timestamp::now();
        assert_ne!(floor, now, "the fixture needs two distinct instants");

        assets
            .read_with("cold.txt")
            .if_match("\"etag-live\"")
            .if_none_match("\"etag-cached\"")
            .if_modified_since(floor)
            .if_unmodified_since(now)
            .await
            .expect("the stub disk ignores conditions, so this resolves");

        let reads = fallback.reads();
        assert_eq!(reads.len(), 1, "the fallback was read once");
        assert_eq!(
            reads[0].if_match(),
            Some("\"etag-live\""),
            "if_match must reach the fallback, or a read the caller expected to \
             fail comes back with a body"
        );
        assert_eq!(
            reads[0].if_none_match(),
            Some("\"etag-cached\""),
            "if_none_match must reach the fallback"
        );
        assert_eq!(
            reads[0].if_modified_since(),
            Some(floor),
            "if_modified_since must reach the fallback unswapped"
        );
        assert_eq!(
            reads[0].if_unmodified_since(),
            Some(now),
            "if_unmodified_since must reach the fallback unswapped"
        );
        assert!(
            primary.write_paths().is_empty(),
            "a conditional hit is served from the fallback but never promoted"
        );
    }

    /// A primary that renames, over a fallback holding a typed object.
    fn rename_capable_read_through(
        writes: WriteBehaviour,
        fallback_stat_fails: bool,
        throw_on_promotion_failure: bool,
    ) -> (Operator, Arc<Journal>, Arc<Journal>) {
        read_through(
            StubSpec {
                renames: true,
                writes,
                ..Default::default()
            },
            StubSpec {
                contents: Some("cold bytes"),
                content_type: Some("image/png"),
                stat_fails: fallback_stat_fails,
                ..Default::default()
            },
            throw_on_promotion_failure,
        )
    }

    #[tokio::test]
    async fn a_promotion_on_a_rename_capable_primary_is_staged_and_renamed() {
        let (assets, primary, _fallback) =
            rename_capable_read_through(WriteBehaviour::Accept, false, false);

        let bytes = assets.read("cold.txt").await.expect("read resolves");
        assert_eq!(&bytes.to_vec(), b"cold bytes");

        let writes = primary.writes();
        assert_eq!(writes.len(), 1, "a promotion writes exactly once");
        let (staged, args) = &writes[0];
        assert!(
            staged.starts_with("cold.txt.suprnova-promote-") && staged.ends_with(".tmp"),
            "the bytes must be staged at a sibling of the target, got: {staged}"
        );
        assert_ne!(
            staged, "cold.txt",
            "the target must never be written in place on a backend that fills \
             a file after creating it"
        );
        assert_eq!(
            args.content_type(),
            Some("image/png"),
            "the staged write carries the fallback object's content metadata"
        );
        assert!(
            !args.if_not_exists(),
            "a staging path is unique, so a no-clobber condition on it would be \
             vacuous"
        );
        assert_eq!(
            primary.renames(),
            vec![(staged.clone(), "cold.txt".to_string())],
            "the staged object is published by renaming it onto the target"
        );
    }

    #[tokio::test]
    async fn a_failed_staged_write_removes_the_staging_object() {
        let (assets, primary, _fallback) =
            rename_capable_read_through(WriteBehaviour::FailAfterOpen, false, false);

        let bytes = assets
            .read("cold.txt")
            .await
            .expect("a failed promotion must not fail the read");
        assert_eq!(&bytes.to_vec(), b"cold bytes");

        let staged = primary.write_paths();
        assert_eq!(staged.len(), 1, "the promotion attempted one staged write");
        assert_eq!(
            primary.deletes(),
            staged,
            "a staging object left behind by a failed write must be removed; \
             nothing else ever sweeps it and a listing shows it forever"
        );
        assert!(
            primary.renames().is_empty(),
            "nothing was published, so nothing was renamed"
        );
    }

    #[tokio::test]
    async fn a_fallback_stat_failure_leaves_a_resolved_read_intact() {
        let (assets, primary, _fallback) =
            rename_capable_read_through(WriteBehaviour::Accept, true, false);

        let bytes = assets
            .read("cold.txt")
            .await
            .expect("the bytes were already in hand when the promotion failed");
        assert_eq!(&bytes.to_vec(), b"cold bytes");
        assert!(
            primary.write_paths().is_empty(),
            "the promotion never got as far as a write"
        );
    }

    #[tokio::test]
    async fn a_fallback_stat_failure_surfaces_when_promotion_failures_are_fatal() {
        let (assets, _primary, _fallback) =
            rename_capable_read_through(WriteBehaviour::Accept, true, true);

        let err = assets
            .read("cold.txt")
            .await
            .expect_err("throw_on_promotion_failure surfaces the failure");
        let message = err.to_string();
        assert!(
            message.contains("promotion") && message.contains("cold.txt"),
            "the error must name the failure and the path, got: {message}"
        );
    }
}
