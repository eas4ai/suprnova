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

use opendal::options::{DeleteOptions, WriteOptions};
use opendal::raw::{
    Layer, OpCopier, OpCopy, OpCreateDir, OpDelete, OpList, OpPresign, OpRead, OpRename, OpStat,
    OpWrite, PresignOperation, RpCreateDir, RpPresign, RpRead, RpRename, RpStat, Service,
    ServiceInfo, Servicer, oio,
};
use opendal::{
    Buffer, BytesRange, Capability, Error, ErrorKind, OperationContext, Operator, Result,
};
use std::sync::Arc;

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
        // Read the conditional-write capability once here rather than per read:
        // it is a property of the backend, not of any single operation.
        let promote_conditionally = inner.capability().write_with_if_not_exists;
        Arc::new(ReadThroughService {
            inner,
            primary: self.primary.clone(),
            fallback: self.fallback.clone(),
            throw_on_promotion_failure: self.throw_on_promotion_failure,
            promote_conditionally,
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
        // for.
        let primary_reader = self.inner.read(ctx, path, args)?;
        Ok(ReadThroughReader {
            primary_reader,
            primary: self.primary.clone(),
            fallback: self.fallback.clone(),
            path: path.to_owned(),
            throw_on_promotion_failure: self.throw_on_promotion_failure,
            promote_conditionally: self.promote_conditionally,
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
    /// Whether a failed promotion fails the read.
    throw_on_promotion_failure: bool,
    /// Whether the promotion write can be made conditional on absence.
    promote_conditionally: bool,
}

impl ReadThroughReader {
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
        let full = self.fallback.read(&self.path).await?;

        // Re-check after the fetch: a writer that landed on the primary while
        // we were pulling the fallback bytes must win, not be overwritten by a
        // stale copy of the cold tier.
        if self.primary.exists(&self.path).await? {
            return Ok(None);
        }

        self.promote(&full).await?;

        let slice = range.to_content_range(full.len())?;
        Ok(Some(full.slice(slice)))
    }

    /// Write a fallback hit through to the primary.
    ///
    /// When the primary can express it, the write is conditional on the object
    /// not already existing. That closes the gap the re-check above leaves open:
    /// the re-check makes a concurrent *writer* win, but two concurrent readers
    /// can both pass it, and only a conditional write stops both from issuing
    /// the promotion. Losing that condition is success, not failure - the object
    /// that won came from the same fallback read and holds the same bytes.
    async fn promote(&self, contents: &Buffer) -> Result<()> {
        let options = WriteOptions {
            if_not_exists: self.promote_conditionally,
            ..Default::default()
        };

        match self
            .primary
            .write_options(&self.path, contents.clone(), options)
            .await
        {
            Ok(_) => Ok(()),
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
                // The caller already has the bytes it asked for, so a failed
                // promotion is a performance problem, not a read failure. An
                // unwritable primary degrades the disk to "read the fallback
                // every time" instead of taking the application down.
                tracing::warn!(
                    path = %self.path,
                    error = %e,
                    "read-through promotion to the primary disk failed; serving the fallback bytes"
                );
                Ok(())
            }
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
