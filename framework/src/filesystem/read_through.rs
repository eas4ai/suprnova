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
//! | `read` | primary if it holds the object, else the fallback - promoting what it finds unless `copy` is `false` |
//! | `stat` (and everything built on it: `exists`, `size`, `last_modified`, `mime_type`) | primary if it holds the object, else the fallback |
//! | `write`, `create_dir` | primary only |
//! | `list` | primary only - fallback entries are invisible to a listing |
//! | `delete` | both, fallback first |
//! | `copy`, `rename` | destination on the primary always; the source comes from the primary if it holds it, else it is streamed across from the fallback. A `rename` also deletes the fallback's source |
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

use opendal::options::{DeleteOptions, ReadOptions, ReaderOptions, WriteOptions};
use opendal::raw::oio::Copy as _;
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
    /// Whether a fallback hit is written through to the primary. See
    /// [`crate::ReadThroughConfig::copy`].
    pub(crate) copy: bool,
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
            copy: self.copy,
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
    /// Whether a fallback hit is written through to the primary.
    copy: bool,
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
    // A one-shot copier, not a pass-through: `Service::copy` cannot await, and
    // resolving the source against two disks is an async question. See
    // [`ReadThroughService::copy`].
    type Copier = oio::OneShotCopier;

    fn info(&self) -> ServiceInfo {
        // The composite's identity is the primary's: that is where writes land
        // and what a listing describes.
        self.inner.info()
    }

    fn capability(&self) -> Capability {
        let mut capability = self.inner.capability();
        let fallback = self.fallback.service().capability();
        // Advertise the union for exactly the operations a caller can have
        // answered by either disk. Everything else stays the primary's answer,
        // because the primary is where the work lands: write, list and
        // create_dir touch nothing else, and `copy` / `rename` may read a
        // source off the fallback but still need the primary to accept the
        // destination.
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
            copy: self.copy,
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
        // `copy` cannot await, and "does the primary hold the source?" is an
        // async question, so the whole operation becomes a one-shot future the
        // copier drives on close.
        let inner = self.inner.clone();
        let ctx = ctx.clone();
        let primary = self.primary.clone();
        let fallback = self.fallback.clone();
        let from = from.to_owned();
        let to = to.to_owned();

        Ok(oio::OneShotCopier::new(async move {
            if primary.exists(&from).await? {
                // Drive the primary's own copier so the caller's OpCopy and
                // OpCopier reach the backend intact.
                let mut copier = inner.copy(&ctx, &from, &to, args, opts)?;
                return match copier.close().await {
                    Ok(meta) => Ok(meta),
                    Err(e) => {
                        let _ = copier.abort().await;
                        Err(e)
                    }
                };
            }

            // Nothing below this layer will apply the caller's conditions on
            // this branch: opendal's `CorrectnessCheckLayer` sits under the
            // primary's stack, and the primary's `copy` - the call it would
            // have checked - is exactly the call a fallback-only source cannot
            // make. So the conditions are honored here, by hand, or not at
            // all.
            if let Some(etag) = args.if_match() {
                // `if_match` on a copy is a condition on the destination
                // object's ETag, which the backend applies as part of its own
                // copy. A streaming write cannot stand in for that, and
                // silently dropping the condition would turn a guarded copy
                // into a clobber.
                return Err(Error::new(
                    ErrorKind::Unsupported,
                    format!(
                        "read-through copy of '{from}' to '{to}' cannot honor \
                         `if_match` ({etag}): the source lives only on the \
                         fallback disk, so the copy is a streaming write rather \
                         than a backend copy"
                    ),
                ));
            }

            let conditions = TransferConditions {
                source_version: args.source_version().map(str::to_owned),
                if_not_exists: args.if_not_exists(),
            };

            stream_across(&primary, &fallback, &from, &to, &conditions)
                .await
                .map_err(|e| copy_failed(&from, &to, e))
        }))
    }

    async fn rename(
        &self,
        ctx: &OperationContext,
        from: &str,
        to: &str,
        args: OpRename,
    ) -> Result<RpRename> {
        // The fallback's copy of the source goes on both branches: leaving it
        // behind would let the next read promote it straight back and undo the
        // move. Deleting a missing path is a success in opendal, so neither
        // branch needs an existence probe first.
        if self.primary.exists(from).await? {
            // Everything the primary would refuse this rename for has to be
            // established *before* the fallback source is deleted. A move that
            // is never attempted must leave both disks exactly as it found
            // them - the delete-first order below is what makes an attempted
            // move safe to retry, not a license to destroy the cold copy for a
            // move that was going to be rejected anyway. Nothing else will
            // catch these in time: opendal's correctness check sits under this
            // layer, so it only speaks once the rename is already running.
            //
            // The primary's own capability, not `Service::capability`'s union
            // with the fallback - the question is what the primary can be
            // asked to do.
            let capability = self.inner.capability();
            if !capability.rename {
                return Err(Error::new(
                    ErrorKind::Unsupported,
                    format!(
                        "read-through move of '{from}' to '{to}' cannot run: the \
                         primary disk has no `rename`"
                    ),
                ));
            }
            if args.if_not_exists() {
                if !capability.rename_with_if_not_exists {
                    return Err(Error::new(
                        ErrorKind::Unsupported,
                        format!(
                            "read-through move of '{from}' to '{to}' cannot \
                             honor `if_not_exists`: the primary disk has no \
                             conditional `rename`"
                        ),
                    ));
                }
                if self.primary.exists(to).await? {
                    return Err(Error::new(
                        ErrorKind::ConditionNotMatch,
                        format!(
                            "read-through move of '{from}' to '{to}' is refused \
                             by `if_not_exists`: the primary disk already holds \
                             '{to}'"
                        ),
                    ));
                }
            }

            // Now delete the fallback's copy, *before* the rename. While the
            // primary holds `from`, that copy is unreachable through this disk,
            // so removing it first changes nothing a caller can observe - and
            // it makes a retry safe. The other order does not: a rename that
            // succeeded and then lost its fallback delete to a transient fault
            // leaves a retry to find `from` gone from the primary, take the
            // streaming branch, and overwrite the destination it just moved
            // correctly with the fallback's stale bytes.
            //
            // A rename that fails *after* the delete leaves the primary still
            // holding `from`, the fallback copy gone, and `to` unwritten. A
            // retry re-enters this same branch, finds the fallback delete a
            // no-op, and runs the rename again - so the failure costs the cold
            // copy and nothing else.
            self.fallback
                .delete(from)
                .await
                .map_err(|e| move_failed(from, to, e))?;
            self.inner.rename(ctx, from, to, args).await?;
            return Ok(RpRename::new());
        }

        // On this branch the order has to be the other way round: the fallback
        // holds the only copy until the destination is in place.
        let conditions = TransferConditions {
            source_version: None,
            if_not_exists: args.if_not_exists(),
        };
        stream_across(&self.primary, &self.fallback, from, to, &conditions)
            .await
            .map_err(|e| move_failed(from, to, e))?;

        self.fallback
            .delete(from)
            .await
            .map_err(|e| move_failed(from, to, e))?;

        Ok(RpRename::new())
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
    /// Whether a fallback hit is written through to the primary.
    copy: bool,
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

    /// The options the fallback read runs under, fetching `range`.
    ///
    /// Everything the caller set on the original read that selects *which*
    /// object comes back is replayed here. Dropping any of it would answer a
    /// versioned read with the fallback's current object, or hand back a body
    /// where the caller expected `ConditionNotMatch`.
    ///
    /// The range is a parameter because the two read paths want different
    /// ones. A promoting read passes the default - the whole object, because
    /// that is what gets written through - and slices the caller's range out
    /// of it afterwards. A non-promoting read passes the caller's range, since
    /// nothing is written back and there is no reason to fetch more.
    fn fallback_read_options(&self, range: BytesRange) -> ReadOptions {
        ReadOptions {
            range,
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

        if !self.copy {
            // Nothing is written back, so there is nothing to fetch beyond what
            // the caller asked for - and no race re-check, because there is no
            // write to lose a race with.
            return self
                .fallback
                .read_options(&self.path, self.fallback_read_options(range))
                .await
                .map(Some);
        }

        // Promotion needs the whole object, so the whole object is what we
        // fetch. A fallback-resolved read therefore holds the object in memory
        // until the promotion write completes.
        let full = self
            .fallback
            .read_options(
                &self.path,
                self.fallback_read_options(BytesRange::default()),
            )
            .await?;

        // Re-check after the fetch: a writer that landed on the primary while
        // we were pulling the fallback bytes must win, not be overwritten by a
        // stale copy of the cold tier.
        match self.primary.exists(&self.path).await {
            Ok(true) => return Ok(None),
            Ok(false) => {
                if self.is_promotable() {
                    self.promote(&full).await?;
                }
            }
            // This probe exists only to protect the promotion write, and the
            // caller's bytes are already in hand, so a primary that cannot
            // answer it is a promotion failure like any other. Failing the read
            // here would put a permissions or transport fault on the promotion
            // side outside the degrade contract entirely.
            Err(e) => self.degrade(e)?,
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
            Err(e) => self.degrade(e),
        }
    }

    /// Apply the configured outcome to a promotion-side failure.
    ///
    /// Every operation that runs only because the read is promoting - the
    /// race re-check, the fallback `stat`, the staged write, the publish -
    /// routes its failure through here, so `throw_on_promotion_failure` means
    /// the same thing for all of them and no single step can fail a read that
    /// has already resolved.
    fn degrade(&self, e: Error) -> Result<()> {
        if self.throw_on_promotion_failure {
            return Err(Error::new(
                ErrorKind::Unexpected,
                format!(
                    "read-through promotion of '{}' to the primary disk failed",
                    self.path
                ),
            )
            .set_source(e));
        }

        tracing::warn!(
            path = %self.path,
            error = %e,
            "read-through promotion to the primary disk failed; serving the fallback bytes"
        );
        Ok(())
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

/// Streaming chunk size for a fallback-to-primary transfer. Matches
/// [`crate::filesystem::streaming`]'s 64 KiB for the same reason: it keeps
/// round-trips reasonable without materializing a whole object.
const CROSS_DISK_CHUNK_BYTES: usize = 64 * 1024;

/// The parts of a caller's `copy` / `rename` arguments that a fallback-spanning
/// transfer can carry across.
///
/// It is a struct rather than two parameters so that adding a condition later
/// forces every call site to decide what it means, instead of silently
/// defaulting the new one away - which is precisely how `if_not_exists` was
/// lost the first time.
#[derive(Debug, Default)]
struct TransferConditions {
    /// The source version to read from the fallback, from
    /// [`opendal::raw::OpCopy::source_version`]. A `rename` has no equivalent.
    source_version: Option<String>,
    /// Whether the destination write must fail if the primary already holds
    /// the object, from `OpCopy::if_not_exists` / `OpRename::if_not_exists`.
    if_not_exists: bool,
}

/// Stream `from` out of the fallback and into `to` on the primary.
///
/// Laravel's `copyFromFallback` buffers the source through `php://temp`;
/// streaming instead keeps a cold-tier object off the heap, which matters
/// because the fallback is where the large, rarely-touched objects live.
///
/// The caller's conditions ride along in `conditions`: the source version
/// selects which object the fallback hands over, and `if_not_exists` becomes a
/// conditional write so a guarded copy still refuses to clobber. A primary that
/// cannot express that condition fails the transfer through opendal's own
/// correctness check rather than quietly ignoring it.
///
/// A failure mid-stream must not be observable as a truncated destination, so
/// the writer is aborted and a destination this transfer created is deleted
/// before the error is returned - the same cleanup `copy_between_disks`
/// performs. A destination that was already there is left alone; see
/// [`discard_partial`].
async fn stream_across(
    primary: &Operator,
    fallback: &Operator,
    from: &str,
    to: &str,
    conditions: &TransferConditions,
) -> Result<Metadata> {
    let reader = fallback
        .reader_options(
            from,
            ReaderOptions {
                version: conditions.source_version.clone(),
                chunk: Some(CROSS_DISK_CHUNK_BYTES),
                ..Default::default()
            },
        )
        .await?;
    let mut stream = std::pin::pin!(reader.into_bytes_stream(..).await?);

    // Whether the destination is this transfer's to remove if it fails. An
    // object that was already there belongs to the caller, and a failed copy
    // must not be the thing that destroys it.
    let destination_existed = primary.exists(to).await?;

    let mut writer = primary
        .writer_options(
            to,
            WriteOptions {
                if_not_exists: conditions.if_not_exists,
                ..Default::default()
            },
        )
        .await?;

    loop {
        match futures::TryStreamExt::try_next(&mut stream).await {
            Ok(Some(chunk)) => {
                if let Err(e) = writer.write(chunk).await {
                    discard_partial(primary, &mut writer, to, destination_existed).await;
                    return Err(e);
                }
            }
            Ok(None) => break,
            Err(e) => {
                discard_partial(primary, &mut writer, to, destination_existed).await;
                return Err(Error::new(
                    ErrorKind::Unexpected,
                    format!("reading '{from}' from the fallback disk failed"),
                )
                .set_source(e));
            }
        }
    }

    match writer.close().await {
        Ok(meta) => Ok(meta),
        Err(e) => {
            discard_partial(primary, &mut writer, to, destination_existed).await;
            Err(e)
        }
    }
}

/// Best-effort cleanup of a half-written destination. `abort` discards staged
/// writes for backends that buffer them; `delete` removes an already-visible
/// partial object. Both are logged rather than propagated so the caller still
/// sees the failure that actually mattered.
///
/// `destination_existed` is what keeps the cleanup from becoming the worse
/// failure. On an object store - the tiering case this whole feature exists
/// for - a write is buffered until it is published, so an intact object sits at
/// `to` for the whole transfer and deleting it would destroy data the transfer
/// never wrote. When it was already there, it is left alone.
///
/// A local-filesystem primary registered through `Storage::register_fs` behaves
/// the same way, because it stages the write under
/// [`crate::filesystem::ATOMIC_STAGING_DIR`] and only renames on success; the
/// `abort` above is what removes the staged file. An fs operator built without
/// that staging directory is the asymmetry: it opens the target itself with
/// `O_TRUNC`, so a pre-existing destination is already gone by the time a
/// transfer can fail, and nothing here can bring it back.
async fn discard_partial(
    primary: &Operator,
    writer: &mut opendal::Writer,
    to: &str,
    destination_existed: bool,
) {
    if let Err(e) = writer.abort().await {
        tracing::warn!(
            path = %to,
            error = %e,
            "failed to abort the writer while cleaning up a failed read-through transfer"
        );
    }

    if destination_existed {
        tracing::warn!(
            path = %to,
            "a read-through transfer failed onto a destination that already \
             existed; leaving it in place, though a primary that opens the \
             target in place rather than staging the write will have \
             truncated it when the writer opened"
        );
        return;
    }

    if let Err(e) = primary.delete(to).await {
        tracing::warn!(
            path = %to,
            error = %e,
            "failed to delete the partial destination while cleaning up a failed read-through transfer"
        );
    }
}

/// Rewrap a fallback-spanning copy failure. Mirrors Laravel's
/// `UnableToCopyFile`, so a caller can tell a failed copy from a failed move.
///
/// The source's kind is kept rather than flattened to `Unexpected`: a caller
/// that set `if_not_exists` has to see `ConditionNotMatch`, and one that copied
/// a path on neither disk has to see `NotFound`, exactly as a single-disk copy
/// would report them. Only the message says the failure was a read-through one.
fn copy_failed(from: &str, to: &str, source: Error) -> Error {
    Error::new(
        source.kind(),
        format!("read-through copy of '{from}' to '{to}' failed"),
    )
    .set_source(source)
}

/// Rewrap a fallback-spanning move failure. Mirrors Laravel's
/// `UnableToMoveFile`. Keeps the source's kind for the same reason
/// [`copy_failed`] does.
fn move_failed(from: &str, to: &str, source: Error) -> Error {
    Error::new(
        source.kind(),
        format!("read-through move of '{from}' to '{to}' failed"),
    )
    .set_source(source)
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
        ranges: Mutex<Vec<BytesRange>>,
        stats: Mutex<Vec<String>>,
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

        fn ranges(&self) -> Vec<BytesRange> {
            locked(&self.ranges).clone()
        }

        fn stats(&self) -> Vec<String> {
            locked(&self.stats).clone()
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
    enum WriteBehavior {
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
        /// A generated body of this many `.` bytes, used instead of
        /// `contents`. Exists so a transfer can be made to span more than one
        /// 64 KiB chunk, which is the only way to walk the streaming loop.
        generated_bytes: Option<usize>,
        content_type: Option<&'static str>,
        /// Whether `stat` fails rather than answering.
        stat_fails: bool,
        /// How many `read` calls answer normally before every later one
        /// fails. `None` never fails on a count. It is what puts a failure
        /// *after* the destination writer is open, which is the only state in
        /// which the transfer cleanup runs.
        read_fails_after: Option<usize>,
        /// How many `stat` calls answer normally before every later one fails.
        ///
        /// `None` never fails on a count. It exists to reach the promotion's
        /// race re-check, which is the *second* existence probe of a read: a
        /// disk that failed every `stat` would fail the first probe instead
        /// and never get there.
        stat_fails_after: Option<usize>,
        /// How many leading `delete` calls fail before the disk starts
        /// accepting them. Stands in for a transient fault on the fallback's
        /// delete, which is the only way to observe *when* a move removes the
        /// source relative to moving it.
        delete_failures: usize,
        /// Whether the disk advertises and implements a rename.
        renames: bool,
        /// Whether the disk advertises a *conditional* rename. Separate from
        /// `renames` because a backend can have one without the other - the
        /// local filesystem is exactly that case - and the two refusals it
        /// produces are different.
        renames_conditionally: bool,
        writes: WriteBehavior,
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
                contents: spec
                    .generated_bytes
                    .map(|len| Buffer::from(vec![b'.'; len]))
                    .or_else(|| spec.contents.map(Buffer::from)),
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

    /// A reader over a fixed body that records the range it was asked for.
    ///
    /// The range does not travel in `OpRead` - opendal splits it out and hands
    /// it to the reader - so journaling it here is the only way to observe how
    /// much of the fallback object a read actually fetches.
    struct StubReader {
        contents: Buffer,
        read_fails_after: Option<usize>,
        journal: Arc<Journal>,
    }

    impl StubReader {
        fn slice(&self, range: BytesRange) -> Result<Buffer> {
            let answered = {
                let mut ranges = locked(&self.journal.ranges);
                ranges.push(range);
                ranges.len() - 1
            };
            if self.read_fails_after.is_some_and(|after| answered >= after) {
                return Err(Error::new(
                    ErrorKind::Unexpected,
                    "the stub disk dropped the transfer part-way through",
                ));
            }
            let slice = range.to_content_range(self.contents.len())?;
            Ok(self.contents.slice(slice))
        }
    }

    impl oio::Read for StubReader {
        async fn open(&self, range: BytesRange) -> Result<(RpRead, Box<dyn oio::ReadStreamDyn>)> {
            Ok((
                RpRead::default(),
                Box::new(self.slice(range)?) as Box<dyn oio::ReadStreamDyn>,
            ))
        }

        async fn read(&self, range: BytesRange) -> Result<(RpRead, Buffer)> {
            Ok((RpRead::default(), self.slice(range)?))
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
        failures: usize,
        journal: Arc<Journal>,
    }

    impl oio::Delete for StubDeleter {
        async fn delete(&mut self, path: &str, _args: OpDelete) -> Result<()> {
            let attempt = {
                let mut deletes = locked(&self.journal.deletes);
                deletes.push(path.to_owned());
                deletes.len() - 1
            };
            if attempt < self.failures {
                return Err(Error::new(
                    ErrorKind::Unexpected,
                    "the stub disk cannot delete right now",
                ));
            }
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
                rename_with_if_not_exists: self.spec.renames_conditionally,
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

        async fn stat(&self, _ctx: &OperationContext, path: &str, _args: OpStat) -> Result<RpStat> {
            let answered = {
                let mut stats = locked(&self.journal.stats);
                stats.push(path.to_owned());
                stats.len() - 1
            };
            if self.spec.stat_fails
                || self
                    .spec
                    .stat_fails_after
                    .is_some_and(|after| answered >= after)
            {
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
            Ok(StubReader {
                contents: self.contents.clone().unwrap_or_default(),
                read_fails_after: self.spec.read_fails_after,
                journal: Arc::clone(&self.journal),
            })
        }

        fn write(
            &self,
            _ctx: &OperationContext,
            path: &str,
            args: OpWrite,
        ) -> Result<Self::Writer> {
            locked(&self.journal.writes).push((path.to_owned(), args));
            match self.spec.writes {
                WriteBehavior::Refuse => Err(unsupported()),
                WriteBehavior::FailAfterOpen => Ok(StubWriter { fails: true }),
                WriteBehavior::Accept => Ok(StubWriter { fails: false }),
            }
        }

        fn delete(&self, _ctx: &OperationContext) -> Result<Self::Deleter> {
            Ok(StubDeleter {
                failures: self.spec.delete_failures,
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
        read_through_with_copy(
            primary_spec,
            fallback_spec,
            true,
            throw_on_promotion_failure,
        )
    }

    /// Compose a read-through disk over two stubs with an explicit `copy`
    /// flag. Returns the composite and the primary's and fallback's journals.
    fn read_through_with_copy(
        primary_spec: StubSpec,
        fallback_spec: StubSpec,
        copy: bool,
        throw_on_promotion_failure: bool,
    ) -> (Operator, Arc<Journal>, Arc<Journal>) {
        let (primary, primary_journal) = StubDisk::operator(primary_spec);
        let (fallback, fallback_journal) = StubDisk::operator(fallback_spec);
        let assets = primary.clone().layer(ReadThroughLayer {
            primary,
            fallback,
            copy,
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
        writes: WriteBehavior,
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
            rename_capable_read_through(WriteBehavior::Accept, false, false);

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
            rename_capable_read_through(WriteBehavior::FailAfterOpen, false, false);

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
            rename_capable_read_through(WriteBehavior::Accept, true, false);

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
            rename_capable_read_through(WriteBehavior::Accept, true, true);

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

    /// A promoting disk over a primary that answers the first existence probe
    /// and then cannot answer the race re-check.
    fn re_check_fails_read_through(
        throw_on_promotion_failure: bool,
    ) -> (Operator, Arc<Journal>, Arc<Journal>) {
        read_through(
            StubSpec {
                stat_fails_after: Some(1),
                writes: WriteBehavior::Accept,
                ..Default::default()
            },
            StubSpec {
                contents: Some("cold bytes"),
                ..Default::default()
            },
            throw_on_promotion_failure,
        )
    }

    #[tokio::test]
    async fn a_failed_race_re_check_leaves_a_resolved_read_intact() {
        let (assets, primary, _fallback) = re_check_fails_read_through(false);

        let bytes = assets
            .read("cold.txt")
            .await
            .expect("the bytes were already in hand when the re-check failed");
        assert_eq!(&bytes.to_vec(), b"cold bytes");
        assert_eq!(
            primary.stats().len(),
            2,
            "the read probed the primary once to route and once to re-check"
        );
        assert!(
            primary.write_paths().is_empty(),
            "a re-check that cannot answer must not promote over whatever is \
             there"
        );
    }

    #[tokio::test]
    async fn a_failed_race_re_check_surfaces_when_promotion_failures_are_fatal() {
        let (assets, _primary, _fallback) = re_check_fails_read_through(true);

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

    #[tokio::test]
    async fn copy_false_fetches_only_the_requested_range_and_probes_once() {
        let (assets, primary, fallback) = read_through_with_copy(
            StubSpec::default(),
            StubSpec {
                contents: Some("cold bytes"),
                ..Default::default()
            },
            false,
            false,
        );

        let bytes = assets
            .read_with("cold.txt")
            .range(5..10)
            .await
            .expect("a non-promoting read resolves from the fallback");
        assert_eq!(&bytes.to_vec(), b"bytes");

        let ranges = fallback.ranges();
        assert_eq!(ranges.len(), 1, "the fallback was read once");
        assert_eq!(ranges[0].offset(), 5);
        assert_eq!(
            ranges[0].size(),
            Some(5),
            "with nothing written back there is no reason to fetch more than \
             the caller asked for"
        );
        assert_eq!(
            primary.stats().len(),
            1,
            "the race re-check exists to protect a promotion write; with none \
             to protect it must not run"
        );
        assert!(
            primary.write_paths().is_empty(),
            "copy: false must never write through"
        );
    }

    #[tokio::test]
    async fn copy_false_still_replays_the_caller_conditions_onto_the_fallback() {
        let (assets, _primary, fallback) = read_through_with_copy(
            StubSpec::default(),
            StubSpec {
                contents: Some("cold bytes"),
                ..Default::default()
            },
            false,
            false,
        );

        assets
            .read_with("cold.txt")
            .version("v7")
            .if_match("\"etag-live\"")
            .await
            .expect("the stub disk ignores conditions, so this resolves");

        let reads = fallback.reads();
        assert_eq!(reads.len(), 1, "the fallback was read once");
        assert_eq!(
            reads[0].version(),
            Some("v7"),
            "not promoting is no reason to drop the caller's version; the \
             fallback would answer with whatever is current there"
        );
        assert_eq!(
            reads[0].if_match(),
            Some("\"etag-live\""),
            "a condition the caller set must still reach the fallback"
        );
    }

    #[tokio::test]
    async fn a_promoting_read_still_fetches_the_whole_fallback_object() {
        let (assets, _primary, fallback) = stub_read_through();

        let bytes = assets
            .read_with("cold.txt")
            .range(5..10)
            .await
            .expect("a ranged read resolves");
        assert_eq!(&bytes.to_vec(), b"bytes");

        let ranges = fallback.ranges();
        assert_eq!(ranges.len(), 1, "the fallback was read once");
        assert!(
            ranges[0].is_full(),
            "promotion writes the whole object through, so the whole object is \
             what a promoting read fetches"
        );
    }

    /// A body that spans two transfer chunks, so the streaming loop runs more
    /// than once and a failure can land between chunks.
    const TWO_CHUNK_BYTES: usize = CROSS_DISK_CHUNK_BYTES + 4096;

    /// An operator over a real local-filesystem directory, configured exactly
    /// as `Storage::register_fs` configures one - atomic staging included, so
    /// these tests see the same write path an application does.
    fn fs_operator(root: &std::path::Path) -> Operator {
        let service = crate::filesystem::atomic_fs_service(
            root.to_str().expect("a tempdir path is valid UTF-8"),
        )
        .expect("a tempdir path stays valid UTF-8 once the staging name is joined");
        Operator::new(service).expect("the fs service builds over an existing directory")
    }

    /// Compose a read-through disk over a *real* primary and a stub fallback.
    ///
    /// The transfer tests need a primary that really stores what it is given -
    /// what sits at the destination after a failed transfer is the whole
    /// assertion - while still needing a fallback that can drop a transfer
    /// part-way or lose a delete, which only the stub can do.
    fn read_through_over(primary: Operator, fallback_spec: StubSpec) -> (Operator, Arc<Journal>) {
        let (fallback, fallback_journal) = StubDisk::operator(fallback_spec);
        let assets = primary.clone().layer(ReadThroughLayer {
            primary,
            fallback,
            copy: true,
            throw_on_promotion_failure: false,
        });
        (assets, fallback_journal)
    }

    /// A fallback holding a two-chunk body that fails after handing over the
    /// first chunk - a transfer that dies with the destination writer open.
    fn interrupted_fallback() -> StubSpec {
        StubSpec {
            generated_bytes: Some(TWO_CHUNK_BYTES),
            read_fails_after: Some(1),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn a_failed_transfer_does_not_destroy_a_pre_existing_destination() {
        // An in-memory primary buffers a write until it is published, which is
        // how every object store behaves and why the destination is intact for
        // the whole transfer - and therefore destroyable by a careless cleanup.
        let primary = memory();
        primary
            .write("warm.txt", "the destination that was already there")
            .await
            .expect("seed the destination");

        let (assets, _fallback) = read_through_over(primary.clone(), interrupted_fallback());

        let err = assets
            .copy("cold.txt", "warm.txt")
            .await
            .expect_err("the fallback drops the transfer part-way through");
        assert!(
            err.to_string().contains("cold.txt"),
            "the failure must name the source, got: {err}"
        );

        assert_eq!(
            &primary
                .read("warm.txt")
                .await
                .expect("the destination is still there")
                .to_vec(),
            b"the destination that was already there",
            "a failed transfer must not be the thing that destroys an object it \
             never wrote"
        );
    }

    #[tokio::test]
    async fn a_failed_transfer_cleans_up_a_partial_destination() {
        // A local-filesystem primary stages every non-append write under
        // `ATOMIC_STAGING_DIR` and renames it into place, so a transfer that
        // dies part-way never publishes the destination at all. What it can
        // leave behind is the temp file it opened, and only the writer's
        // `abort` removes that - so both are the assertion.
        let tmp = tempfile::tempdir().expect("tempdir");
        let primary = fs_operator(tmp.path());

        let (assets, _fallback) = read_through_over(primary.clone(), interrupted_fallback());

        assets
            .copy("cold.txt", "warm.txt")
            .await
            .expect_err("the fallback drops the transfer part-way through");

        assert!(
            !primary
                .exists("warm.txt")
                .await
                .expect("primary exists answers"),
            "a failed transfer must publish nothing at the destination; \
             nothing else sweeps a partial and a listing shows it forever"
        );

        let staging = tmp.path().join(crate::filesystem::ATOMIC_STAGING_DIR);
        let staged: Vec<String> = std::fs::read_dir(&staging)
            .unwrap_or_else(|e| panic!("the staging directory at {staging:?} must exist: {e}"))
            .map(|entry| {
                entry
                    .expect("a staging directory entry reads")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert!(
            staged.is_empty(),
            "a failed transfer must not leave its temp file under the staging \
             directory either, found: {staged:?}"
        );
    }

    #[tokio::test]
    async fn a_copy_replays_the_source_version_onto_the_fallback() {
        let (assets, fallback) = read_through_over(
            memory(),
            StubSpec {
                contents: Some("cold bytes"),
                ..Default::default()
            },
        );

        assets
            .copy_with("cold.txt", "warm.txt")
            .source_version("v7")
            .await
            .expect("the stub disk ignores versions, so this resolves");

        let reads = fallback.reads();
        assert_eq!(reads.len(), 1, "the fallback was read once");
        assert_eq!(
            reads[0].version(),
            Some("v7"),
            "a copy that names a source version must get that version; the \
             fallback would otherwise hand over whatever is current"
        );
    }

    #[tokio::test]
    async fn a_retried_move_does_not_overwrite_the_destination_with_stale_bytes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let primary = fs_operator(tmp.path());
        primary
            .write("both.txt", "primary copy")
            .await
            .expect("seed the primary");

        // The fallback holds a stale copy of the same path and loses its first
        // delete to a transient fault.
        let (assets, _fallback) = read_through_over(
            primary.clone(),
            StubSpec {
                contents: Some("stale fallback copy"),
                delete_failures: 1,
                ..Default::default()
            },
        );

        assets
            .rename("both.txt", "moved.txt")
            .await
            .expect_err("the fallback delete fails on the first attempt");
        assert!(
            !primary
                .exists("moved.txt")
                .await
                .expect("primary exists answers"),
            "the source goes before the rename, so a move that loses its \
             delete has not moved anything yet"
        );

        assets
            .rename("both.txt", "moved.txt")
            .await
            .expect("the retry finds the source still on the primary");

        assert_eq!(
            &primary
                .read("moved.txt")
                .await
                .expect("the destination is on the primary")
                .to_vec(),
            b"primary copy",
            "a retry must move the primary's object, not resurrect the \
             fallback's stale copy over a destination the first attempt \
             already wrote correctly"
        );
    }

    /// A read-through disk whose primary holds the source and whose fallback
    /// holds a stale copy of it, over a primary with the given rename
    /// capabilities. Returns the composite and the fallback's journal.
    fn move_refusal_read_through(
        renames: bool,
        renames_conditionally: bool,
    ) -> (Operator, Arc<Journal>) {
        let (assets, _primary, fallback) = read_through(
            StubSpec {
                contents: Some("primary copy"),
                renames,
                renames_conditionally,
                ..Default::default()
            },
            StubSpec {
                contents: Some("stale fallback copy"),
                ..Default::default()
            },
            false,
        );
        (assets, fallback)
    }

    #[tokio::test]
    async fn a_move_a_primary_cannot_perform_leaves_the_fallback_source_alone() {
        let (assets, fallback) = move_refusal_read_through(false, false);

        let err = assets
            .rename("both.txt", "moved.txt")
            .await
            .expect_err("a primary without a rename cannot move anything");
        assert_eq!(
            err.kind(),
            ErrorKind::Unsupported,
            "the caller has to see the refusal for what it is, got: {err}"
        );
        assert!(
            err.to_string().contains("rename"),
            "the error must name what the primary cannot do, got: {err}"
        );
        assert!(
            fallback.deletes().is_empty(),
            "a move that was never attempted must not have removed its source \
             from the fallback; the cold copy would be gone with nothing moved"
        );
    }

    #[tokio::test]
    async fn a_conditional_move_a_primary_cannot_guard_leaves_the_fallback_source_alone() {
        let (assets, fallback) = move_refusal_read_through(true, false);

        let err = assets
            .rename_with("both.txt", "moved.txt")
            .if_not_exists(true)
            .await
            .expect_err("a primary without a conditional rename cannot guard one");
        assert_eq!(
            err.kind(),
            ErrorKind::Unsupported,
            "the caller has to see the refusal for what it is, got: {err}"
        );
        assert!(
            err.to_string().contains("if_not_exists"),
            "the error must name the condition it cannot honor, got: {err}"
        );
        assert!(
            fallback.deletes().is_empty(),
            "the rename would have been rejected under this layer, after the \
             fallback source was already gone"
        );
    }

    #[tokio::test]
    async fn a_conditional_move_onto_an_existing_destination_leaves_the_fallback_source_alone() {
        // The stub answers every path from one body, so the destination exists
        // as far as the condition is concerned.
        let (assets, fallback) = move_refusal_read_through(true, true);

        let err = assets
            .rename_with("both.txt", "moved.txt")
            .if_not_exists(true)
            .await
            .expect_err("if_not_exists must refuse an existing destination");
        assert_eq!(
            err.kind(),
            ErrorKind::ConditionNotMatch,
            "a refused condition must not reach the caller as anything else, got: {err}"
        );
        assert!(
            fallback.deletes().is_empty(),
            "a move the condition refuses never happens, so its source stays \
             where it is on both disks"
        );
    }
}
