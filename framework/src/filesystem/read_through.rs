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
//!
//! # Promotion streams
//!
//! A promotion streams the cold object from the fallback into the primary and
//! then answers the read from the primary. Nothing holds the whole object in
//! memory, so a multi-gigabyte cold object costs a transfer, not a buffer. A
//! read that is not promoted - `copy: false`, a versioned or conditional read,
//! or a read whose promotion already failed - fetches only the range it was
//! asked for. opendal's readers stop a read at its range, and its S3, Azure
//! Blob and GCS services refuse a response outside the requested range before
//! reading the body. This layer keeps its own bound on top, for a fallback
//! built without opendal's default layers.
//!
//! # A delete is never undone by a promotion
//!
//! A promotion that overlaps a delete or a move of the same path must not
//! republish the bytes it fetched before the delete. Within one process every
//! publish, delete and move of a path is serialized through [`Publications`],
//! and a promotion that started before a delete or a move of its path discards
//! its staged copy instead of publishing it.
//!
//! Across processes nothing is shared, so the storage itself decides. A delete
//! and a move remove the fallback copy first and the primary copy second. A
//! promotion checks that the fallback still holds the object it fetched
//! immediately before it publishes, and discards its own unpublished bytes
//! when it does not. It checks again after it publishes. A delete that
//! removed the fallback copy before the second check either still has its
//! primary step to run, which removes the promoted copy, or ran that step
//! before the publish, and then the promotion withdraws its copy itself.
//!
//! A withdrawal deletes only the exact object the promotion wrote, so a
//! writer that replaced it never loses its write. That takes a primary that
//! names each write by version and deletes one version on request, as a
//! versioned object store does. Elsewhere opendal has no delete that is
//! conditional on what the path holds, so the promotion keeps its copy and
//! logs a warning instead. On those primaries a delete on another node that
//! completes in the moment between the promotion's last check and its
//! publish can leave the promoted copy behind.

use super::streaming::WriterGuard;
use futures::TryStreamExt;
use opendal::options::{DeleteOptions, ReaderOptions, WriteOptions};
use opendal::raw::oio::{Copy as _, Read as _, ReadStream as _, ReadStreamDyn as _};
use opendal::raw::{
    Layer, OpCopier, OpCopy, OpCreateDir, OpDelete, OpList, OpPresign, OpRead, OpRename, OpStat,
    OpWrite, PresignOperation, RpCreateDir, RpPresign, RpRead, RpRename, RpStat, Service,
    ServiceInfo, Servicer, oio,
};
use opendal::{
    Buffer, BytesRange, Capability, Error, ErrorKind, Metadata, OperationContext, Operator, Result,
};
use sha2::{Digest as _, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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

/// Serializes, within one process, the moment a promotion publishes a path
/// with the deletes and moves that remove it.
///
/// A promotion fetches from the fallback, stages, and publishes some time
/// later. A delete that completes in between finds nothing on the primary to
/// remove, so without coordination the promotion would put the deleted object
/// back. Every publish, delete and move of a path therefore holds that path's
/// slot, and each delete or move advances the slot's generation. A promotion
/// records the generation before it fetches and publishes only if it is
/// unchanged. A recursive delete covers paths it cannot name one by one, so it
/// takes the whole table exclusively and advances a table-wide generation.
///
/// Slots exist only while something holds them, so the table stays as small
/// as the number of operations in flight.
#[derive(Debug, Default)]
pub(crate) struct Publications {
    /// Advanced by every recursive delete. Publishers hold it shared; a
    /// recursive delete holds it exclusively.
    tree: tokio::sync::RwLock<u64>,
    /// One slot per path some operation currently holds.
    slots: std::sync::Mutex<HashMap<String, Arc<PathSlot>>>,
}

/// The coordination state for one path.
#[derive(Debug, Default)]
struct PathSlot {
    /// Advanced by every delete or move of the path, and held across every
    /// publish, delete and move of it.
    generation: tokio::sync::Mutex<u64>,
}

/// A held claim on one path's slot. Dropping it releases the slot, and the
/// last claim removes the slot from the table.
struct SlotClaim<'a> {
    publications: &'a Publications,
    path: String,
    slot: Arc<PathSlot>,
}

impl Drop for SlotClaim<'_> {
    fn drop(&mut self) {
        let mut slots = self
            .publications
            .slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // The table's own reference plus this claim's: nobody else holds it.
        if Arc::strong_count(&self.slot) == 2 {
            slots.remove(&self.path);
        }
    }
}

/// What a promotion records before it fetches: the generations a later
/// delete or move would advance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Generations {
    tree: u64,
    path: u64,
}

impl Publications {
    fn claim(&self, path: &str) -> SlotClaim<'_> {
        let slot = self
            .slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(path.to_owned())
            .or_default()
            .clone();
        SlotClaim {
            publications: self,
            path: path.to_owned(),
            slot,
        }
    }

    /// The generations in force for `claim`'s path right now.
    async fn generations(&self, claim: &SlotClaim<'_>) -> Generations {
        let tree = *self.tree.read().await;
        let path = *claim.slot.generation.lock().await;
        Generations { tree, path }
    }

    /// Run `remove` - a delete or a move of `path` - so that no promotion
    /// that started before it can publish `path` afterwards.
    async fn retire<T>(
        &self,
        path: &str,
        recursive: bool,
        remove: impl std::future::Future<Output = Result<T>>,
    ) -> Result<T> {
        if recursive {
            let mut tree = self.tree.write().await;
            *tree += 1;
            return remove.await;
        }
        let claim = self.claim(path);
        let _tree = self.tree.read().await;
        let mut generation = claim.slot.generation.lock().await;
        *generation += 1;
        remove.await
    }

    /// Run `publish` only if no delete or move of the claimed path has run
    /// since `recorded`. Returns `None` when one has, and the publish was
    /// skipped.
    async fn publish<T>(
        &self,
        claim: &SlotClaim<'_>,
        recorded: Generations,
        publish: impl std::future::Future<Output = Result<T>>,
    ) -> Result<Option<T>> {
        let tree = self.tree.read().await;
        let generation = claim.slot.generation.lock().await;
        if (Generations {
            tree: *tree,
            path: *generation,
        }) != recorded
        {
            return Ok(None);
        }
        publish.await.map(Some)
    }
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
    /// Whether a failed promotion fails the read. See [`ReadThroughReader::degrade`].
    pub(crate) throw_on_promotion_failure: bool,
    /// Shared by every service this layer builds, so re-layering the
    /// composite never splits the coordination between two tables.
    pub(crate) publications: Arc<Publications>,
}

impl ReadThroughLayer {
    /// A read-through layer over `primary` and `fallback`.
    pub(crate) fn new(
        primary: Operator,
        fallback: Operator,
        copy: bool,
        throw_on_promotion_failure: bool,
    ) -> Self {
        Self {
            primary,
            fallback,
            copy,
            throw_on_promotion_failure,
            publications: Arc::default(),
        }
    }
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
            withdraw_by_version: capability.delete_with_version,
            publications: Arc::clone(&self.publications),
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
    /// Whether the primary can delete one version of an object, which is
    /// what lets a promotion withdraw exactly the copy it wrote.
    withdraw_by_version: bool,
    /// Keeps a promotion from republishing a path a delete or move removed.
    publications: Arc<Publications>,
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
        // A versioned or conditional read the primary cannot express is still
        // answered when the object lives only on the fallback, so these are
        // the union too. A primary that holds the object refuses them itself.
        capability.read_with_version |= fallback.read_with_version;
        capability.read_with_if_match |= fallback.read_with_if_match;
        capability.read_with_if_none_match |= fallback.read_with_if_none_match;
        capability.read_with_if_modified_since |= fallback.read_with_if_modified_since;
        capability.read_with_if_unmodified_since |= fallback.read_with_if_unmodified_since;
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
        //
        // A primary that cannot express those arguments refuses to build a
        // reader at all. That refusal only matters if the primary turns out to
        // hold the object, so it is kept and raised then, rather than here,
        // where it would stop a fallback that can answer from being asked.
        let primary_reader = self.inner.read(ctx, path, args.clone());
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
            withdraw_by_version: self.withdraw_by_version,
            publications: Arc::clone(&self.publications),
            promotion_failed: AtomicBool::new(false),
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
            publications: Arc::clone(&self.publications),
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
        // branch needs an existence probe first. Both branches retire `from`,
        // so a promotion of it that is already in flight cannot republish it
        // once the move has removed it.
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
            // The primary may also refuse the destination path itself - a
            // local disk's path guard refuses one that leaves its root or
            // names its staging directory. A `stat` of the destination runs
            // the same checks without changing anything: a miss is the
            // expected answer, and any other error is a refusal to report now,
            // while both disks still hold the source.
            match self.inner.stat(ctx, to, OpStat::new()).await {
                Ok(_) => {}
                Err(e) if e.kind() == ErrorKind::NotFound => {}
                Err(e) => return Err(move_failed(from, to, e)),
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
            self.publications
                .retire(from, false, async {
                    self.fallback
                        .delete(from)
                        .await
                        .map_err(|e| move_failed(from, to, e))?;
                    self.inner.rename(ctx, from, to, args).await
                })
                .await?;
            return Ok(RpRename::new());
        }

        // On this branch the order has to be the other way round: the fallback
        // holds the only copy until the destination is in place.
        let conditions = TransferConditions {
            source_version: None,
            if_not_exists: args.if_not_exists(),
        };
        self.publications
            .retire(from, false, async {
                stream_across(&self.primary, &self.fallback, from, to, &conditions)
                    .await
                    .map_err(|e| move_failed(from, to, e))?;

                self.fallback
                    .delete(from)
                    .await
                    .map_err(|e| move_failed(from, to, e))?;
                // A promotion of `from` on another node may have published it
                // while the bytes streamed across. Removing the primary's
                // `from` after the fallback copy is what lets that promotion's
                // own check, or this step, take it back out; see
                // `ReadThroughReader::confirm_or_withdraw`.
                self.primary
                    .delete(from)
                    .await
                    .map_err(|e| move_failed(from, to, e))
            })
            .await?;

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
    /// The primary's lazy reader, used whenever the primary owns the object,
    /// or the refusal building it returned. See [`ReadThroughService::read`].
    primary_reader: Result<oio::Reader>,
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
    /// Whether the primary can delete one version of an object.
    withdraw_by_version: bool,
    /// Keeps a promotion from republishing a path a delete or move removed.
    publications: Arc<Publications>,
    /// Set once a promotion for this read has failed. A chunked read asks
    /// for one range at a time; retrying the promotion for each would move
    /// the whole object once per chunk, so the rest of the read takes ranged
    /// reads from the fallback instead.
    promotion_failed: AtomicBool,
}

/// Which disk answers a range.
enum Route {
    /// The primary holds the object, or a promotion just put it there.
    Primary,
    /// Only the fallback holds it, and nothing is promoted for this range.
    Fallback,
}

/// How a promotion ended, when it did not fail.
enum Promotion {
    /// The object is on the primary now: this promotion published it, or a
    /// writer or another promotion got there first.
    OnPrimary,
    /// A delete or move of the path ran while the promotion was fetching, so
    /// its bytes were discarded instead of published.
    Superseded,
}

/// Whether `now` describes the same fallback object as `fetched`, by the
/// strongest validator both carry: an ETag, a version, or a modification
/// time with the length. `None` when they share none.
///
/// A length alone is not one. A same-size overwrite keeps it, so taking it
/// as proof published the old bytes over the new.
fn same_source(fetched: &Metadata, now: &Metadata) -> Option<bool> {
    if let (Some(a), Some(b)) = (fetched.etag(), now.etag()) {
        return Some(a == b);
    }
    if let (Some(a), Some(b)) = (fetched.version(), now.version()) {
        return Some(a == b);
    }
    match (fetched.last_modified(), now.last_modified()) {
        (Some(a), Some(b)) => Some(a == b && fetched.content_length() == now.content_length()),
        _ => None,
    }
}

/// Whether `metadata` carries anything [`same_source`] can compare.
fn has_validator(metadata: &Metadata) -> bool {
    metadata.etag().is_some() || metadata.version().is_some() || metadata.last_modified().is_some()
}

/// The SHA-256 of an object's bytes.
type Digest = [u8; 32];

/// What a promotion knows about the fallback object it fetched, kept to
/// tell later whether the fallback still holds that object.
#[derive(Clone)]
struct Fetched {
    /// The fallback's metadata from before the fetch.
    metadata: Metadata,
    /// The digest of the bytes the promotion streamed. Kept only when the
    /// metadata carries no validator, since then the bytes are the only
    /// identity there is.
    digest: Option<Digest>,
}

/// Whether `fallback` still holds the object `fetched` describes at `path`.
///
/// The fallback's validators decide when it has any. A fallback without
/// them - the in-memory disk reports only a length - is read again and its
/// bytes compared by digest, which is the one check a length cannot fool.
async fn fallback_holds(fallback: &Operator, path: &str, fetched: &Fetched) -> Result<bool> {
    let now = match fallback.stat(path).await {
        Ok(now) => now,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    if let Some(same) = same_source(&fetched.metadata, &now) {
        return Ok(same);
    }
    // Validators the fetch saw and this answer lacks prove nothing either
    // way, and nothing was hashed to fall back on.
    let Some(digest) = fetched.digest else {
        return Ok(false);
    };
    if now.content_length() != fetched.metadata.content_length() {
        return Ok(false);
    }
    match content_digest(fallback, path).await {
        Ok(now) => Ok(now == digest),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// Hash the whole object at `path`, one chunk at a time, without holding it.
async fn content_digest(disk: &Operator, path: &str) -> Result<Digest> {
    let reader = disk.service().read(disk.context(), path, OpRead::new())?;
    let (_, mut stream) = reader.open(BytesRange::default()).await?;
    let mut hasher = Sha256::new();
    loop {
        let chunk = stream.read_dyn().await?;
        if chunk.is_empty() {
            return Ok(hasher.finalize().into());
        }
        for bytes in chunk {
            hasher.update(&bytes);
        }
    }
}

/// The check a promotion makes after it publishes, and the withdrawal that
/// follows when the fallback no longer holds what it fetched.
///
/// It owns everything it needs so it can run on a task of its own: the
/// published copy has to be checked and, if need be, withdrawn even when the
/// read that promoted it is cancelled half-way.
struct Confirmation {
    primary: Operator,
    fallback: Operator,
    path: String,
    /// What the promotion fetched.
    fetched: Fetched,
    /// The version that names the published copy, when the primary reports
    /// one and can delete a single version. `None` means the copy cannot be
    /// withdrawn without risking a writer's newer object.
    version: Option<String>,
}

impl Confirmation {
    /// Check the fallback, and withdraw the published copy when it no
    /// longer holds the object.
    ///
    /// A fallback that cannot be asked counts as one that no longer holds the
    /// object wherever the copy can be withdrawn exactly: a promotion lost to
    /// a transient fault costs only a later re-read. Where it cannot, the
    /// copy is kept and served, since the fallback held the same object just
    /// before the publish.
    async fn run(self) -> Result<Promotion> {
        let still_cold = fallback_holds(&self.fallback, &self.path, &self.fetched).await;
        match (still_cold, self.version) {
            (Ok(true), _) => Ok(Promotion::OnPrimary),
            (_, Some(version)) => {
                // A versioned delete removes this one object. A writer that
                // replaced it holds a different version and keeps it.
                let options = DeleteOptions {
                    version: Some(version),
                    recursive: false,
                };
                match self.primary.delete_options(&self.path, options).await {
                    Ok(()) => {}
                    Err(e) if e.kind() == ErrorKind::NotFound => {}
                    Err(e) => return Err(e),
                }
                Ok(Promotion::Superseded)
            }
            (Ok(false), None) => {
                tracing::warn!(
                    path = %self.path,
                    "read-through promotion published a copy whose fallback object went away \
                     while it published; the primary cannot delete that one copy by version, \
                     so it is kept rather than risk deleting a newer write"
                );
                Ok(Promotion::Superseded)
            }
            (Err(e), None) => {
                tracing::warn!(
                    path = %self.path,
                    error = %e,
                    "read-through promotion could not re-check the fallback after it \
                     published; keeping the copy, which the fallback held just before"
                );
                Ok(Promotion::OnPrimary)
            }
        }
    }
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

    /// The primary's reader, or the refusal building it returned.
    fn primary_reader(&self) -> Result<&oio::Reader> {
        self.primary_reader
            .as_ref()
            .map_err(|e| Error::new(e.kind(), e.to_string()))
    }

    /// The arguments a fallback read runs under.
    ///
    /// Everything the caller set on the original read that selects *which*
    /// object comes back is replayed here. Dropping any of it would answer a
    /// versioned read with the fallback's current object, or hand back a body
    /// where the caller expected `ConditionNotMatch`.
    fn fallback_args(&self) -> OpRead {
        let mut args = OpRead::new();
        if let Some(version) = self.args.version() {
            args = args.with_version(version);
        }
        if let Some(etag) = self.args.if_match() {
            args = args.with_if_match(etag);
        }
        if let Some(etag) = self.args.if_none_match() {
            args = args.with_if_none_match(etag);
        }
        if let Some(at) = self.args.if_modified_since() {
            args = args.with_if_modified_since(at);
        }
        if let Some(at) = self.args.if_unmodified_since() {
            args = args.with_if_unmodified_since(at);
        }
        args
    }

    /// Open `range` of the fallback object as a stream that never yields more
    /// than the range holds.
    ///
    /// opendal stops a read at its range in its completion layer and in its
    /// stream reader, and its S3, Azure Blob and GCS services refuse a
    /// response outside the requested range before reading the body. A disk
    /// built with `Operator::from_parts` skips the completion layer, and a
    /// custom service may not check, so this bound stays as the backstop: a
    /// backend that ignores `Range` and answers with the whole body costs one
    /// chunk here, not the whole body held in memory.
    async fn open_fallback(&self, range: BytesRange) -> Result<(RpRead, BoundedStream)> {
        let reader = self.fallback.service().read(
            self.fallback.context(),
            &self.path,
            self.fallback_args(),
        )?;
        let (reply, stream) = reader.open(range).await?;
        // Only a range that names its size can be held to a length here. An
        // open-ended range (`5..`) has none; on S3, Azure Blob and GCS the
        // service itself refuses a response that does not start at the
        // requested offset, and a whole-object read is the common case of
        // that shape.
        let limit = if range.is_suffix() {
            None
        } else {
            range.size()
        };
        let stream = BoundedStream {
            inner: stream,
            limit,
            read: 0,
            path: self.path.clone(),
            range,
        };
        Ok((reply, stream))
    }

    /// Decide which disk answers, promoting the object first when this read
    /// should and can.
    async fn route(&self) -> Result<Route> {
        if self.primary.exists(&self.path).await? {
            return Ok(Route::Primary);
        }
        if !self.copy || !self.is_promotable() || self.promotion_failed.load(Ordering::Acquire) {
            return Ok(Route::Fallback);
        }

        // The fallback's own metadata rides along with the bytes. Without it an
        // S3-to-S3 read-through would silently drop `Content-Type` the first
        // time each object crossed over, and nothing would ever restore it. A
        // miss here is not a promotion failure: the object is on neither disk.
        let metadata = match self.fallback.stat(&self.path).await {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == ErrorKind::NotFound => return Err(e),
            Err(e) => {
                self.degrade(e)?;
                return Ok(Route::Fallback);
            }
        };

        match self.promote(&metadata).await {
            Ok(Promotion::OnPrimary) => Ok(Route::Primary),
            Ok(Promotion::Superseded) => Ok(Route::Fallback),
            Err(e) => {
                self.degrade(e)?;
                Ok(Route::Fallback)
            }
        }
    }

    /// Apply the configured outcome to a promotion-side failure.
    ///
    /// Every operation that runs only because the read is promoting - the
    /// fallback `stat`, the staged write, the race re-check, the publish -
    /// routes its failure through here, so `throw_on_promotion_failure` means
    /// the same thing for all of them. Failure is a performance problem rather
    /// than a read failure unless that flag is set: the fallback can still
    /// answer, so an unwritable primary degrades the disk to "read the
    /// fallback every time" instead of taking the application down.
    fn degrade(&self, e: Error) -> Result<()> {
        self.promotion_failed.store(true, Ordering::Release);
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
            "read-through promotion to the primary disk failed; serving from the fallback"
        );
        Ok(())
    }

    /// Stream the fallback object into the primary so that no reader can
    /// observe it half-written, and no delete that overlaps it is undone.
    ///
    /// Where the primary advertises a `rename`, the bytes are staged at a
    /// unique sibling and renamed onto the target. That covers any primary
    /// whose write is not itself an indivisible publish, whatever the backend
    /// stages underneath. Where the primary has no `rename`, the write itself
    /// is the atomic publish and runs directly, conditional on the object not
    /// already existing so two concurrent readers do not both promote.
    ///
    /// The staged form cannot use that condition: its path is unique, so the
    /// condition would be vacuous. It re-checks the primary immediately before
    /// the rename instead, and a write that landed in the meantime wins.
    ///
    /// Either way the publish runs through [`Publications::publish`], and a
    /// promotion that a delete or move of the path overtook discards its bytes.
    /// So does one whose fallback object went away while it streamed, which
    /// is how a delete on another node shows: the fallback is asked again
    /// just before the publish.
    async fn promote(&self, metadata: &Metadata) -> Result<Promotion> {
        let claim = self.publications.claim(&self.path);
        // Recorded before the first byte is fetched: a delete that removes the
        // fallback copy after this point advances it.
        let recorded = self.publications.generations(&claim).await;
        // Without a validator, the checks below compare the bytes themselves.
        let by_content = !has_validator(metadata);
        if by_content {
            tracing::debug!(
                path = %self.path,
                "read-through fallback reports no ETag, version or modification time; \
                 the promotion is checked against the fallback by content"
            );
        }

        if !self.promote_atomically {
            let options = self.promotion_options(metadata, self.promote_conditionally);
            let writer = self.primary.writer_options(&self.path, options).await?;
            // Abort multipart state on failure, but never delete a published
            // object: another reader may have won the condition.
            let mut guard = WriterGuard::new(
                self.primary.clone(),
                "read-through primary",
                &self.path,
                writer,
            )
            .preserve_destination();
            let fetched = match self.stream_into(guard.writer(), metadata, by_content).await {
                Ok(fetched) => fetched,
                Err(e) => return guard.settle(Err(e)).await,
            };
            match self.still_holds(&fetched).await {
                Ok(true) => {}
                Ok(false) => {
                    guard.cleanup().await;
                    return Ok(Promotion::Superseded);
                }
                Err(e) => return guard.settle(Err(e)).await,
            }
            // The close is the publish on these backends, so it is what runs
            // under the coordination.
            let closed = self
                .publications
                .publish(&claim, recorded, guard.writer().close())
                .await;
            return match closed {
                Ok(Some(written)) => {
                    guard.settle(Ok::<(), Error>(())).await?;
                    // The write is the publish here, so the version it
                    // reports names the object now at the path.
                    let version = written
                        .version()
                        .filter(|_| self.withdraw_by_version)
                        .map(str::to_owned);
                    self.confirm_or_withdraw(fetched, version).await
                }
                Ok(None) => {
                    guard.cleanup().await;
                    Ok(Promotion::Superseded)
                }
                // Losing the condition is not a failure: the object that won
                // came from the same fallback and holds the same bytes.
                Err(e) if e.kind() == ErrorKind::ConditionNotMatch => {
                    guard.cleanup().await;
                    Ok(Promotion::OnPrimary)
                }
                Err(e) => guard.settle(Err(e)).await,
            };
        }

        let staged = staging_path(&self.path);
        let options = self.promotion_options(metadata, false);
        let writer = self.primary.writer_options(&staged, options).await?;
        // Own both the writer's staging and the unique sibling until the
        // final rename completes. Cleanup never targets the published path.
        let mut guard = WriterGuard::new(
            self.primary.clone(),
            "read-through primary",
            &staged,
            writer,
        );
        let staged_result: Result<Fetched> = async {
            let fetched = self
                .stream_into(guard.writer(), metadata, by_content)
                .await?;
            guard.writer().close().await?;
            Ok(fetched)
        }
        .await;
        let fetched = match staged_result {
            Ok(fetched) => fetched,
            Err(e) => return guard.settle(Err(e)).await,
        };
        match self.still_holds(&fetched).await {
            Ok(true) => {}
            Ok(false) => {
                guard.cleanup().await;
                return Ok(Promotion::Superseded);
            }
            Err(e) => return guard.settle(Err(e)).await,
        }

        let published = self
            .publications
            .publish(&claim, recorded, async {
                if self.primary.exists(&self.path).await? {
                    // Somebody published while we were staging. Their object
                    // wins.
                    return Ok(false);
                }
                self.primary.rename(&staged, &self.path).await?;
                Ok(true)
            })
            .await;
        match published {
            Ok(Some(true)) => {
                guard.settle(Ok::<(), Error>(())).await?;
                // A version the staged write reported names the staged
                // object, not what the rename published, so it proves
                // nothing about the copy at the path.
                self.confirm_or_withdraw(fetched, None).await
            }
            Ok(Some(false)) => {
                guard.cleanup().await;
                Ok(Promotion::OnPrimary)
            }
            Ok(None) => {
                guard.cleanup().await;
                Ok(Promotion::Superseded)
            }
            Err(e) => guard.settle(Err(e)).await,
        }
    }

    /// Copy the whole fallback object into `writer`, one fallback chunk at a
    /// time, without closing it. Returns what was fetched, hashing the bytes
    /// on the way when `by_content` says the metadata cannot identify them.
    async fn stream_into(
        &self,
        writer: &mut opendal::Writer,
        metadata: &Metadata,
        by_content: bool,
    ) -> Result<Fetched> {
        let (_, mut stream) = self.open_fallback(BytesRange::default()).await?;
        let mut hasher = by_content.then(Sha256::new);
        loop {
            let chunk = stream.read().await?;
            if chunk.is_empty() {
                return Ok(Fetched {
                    metadata: metadata.clone(),
                    digest: hasher.map(|hasher| hasher.finalize().into()),
                });
            }
            if let Some(hasher) = hasher.as_mut() {
                for bytes in chunk.clone() {
                    hasher.update(&bytes);
                }
            }
            writer.write(chunk).await?;
        }
    }

    /// Whether the fallback still holds the object this promotion fetched.
    /// Asked just before the promotion publishes, so a delete, a move or an
    /// overwrite on another node while the bytes streamed is seen before
    /// anything reaches the path.
    async fn still_holds(&self, fetched: &Fetched) -> Result<bool> {
        fallback_holds(&self.fallback, &self.path, fetched).await
    }

    /// After this promotion published, check that the fallback still holds
    /// the object it fetched, and withdraw the published copy when it does
    /// not. `version` names the published copy when the primary can delete
    /// that one version; see [`Confirmation`].
    ///
    /// This closes the moment between the check in [`Self::still_holds`] and
    /// the publish. A delete or a move on another node removes the fallback
    /// copy before the primary copy, so this check either runs before the
    /// fallback copy went - and the delete's primary step, still to come,
    /// removes the promoted copy - or after it, and the promotion withdraws
    /// its own copy here.
    ///
    /// The check runs on a task of its own and this awaits it. A read
    /// cancelled now would otherwise drop the check with it, and leave the
    /// copy of a deleted object on the primary.
    async fn confirm_or_withdraw(
        &self,
        fetched: Fetched,
        version: Option<String>,
    ) -> Result<Promotion> {
        let confirmation = Confirmation {
            primary: self.primary.clone(),
            fallback: self.fallback.clone(),
            path: self.path.clone(),
            fetched,
            version,
        };
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle.spawn(confirmation.run()).await.map_err(|e| {
                Error::new(
                    ErrorKind::Unexpected,
                    "read-through promotion check did not finish",
                )
                .set_source(e)
            })?,
            // Foreign executors have no runtime to spawn onto.
            Err(_) => confirmation.run().await,
        }
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
}

impl oio::Read for ReadThroughReader {
    async fn open(&self, range: BytesRange) -> Result<(RpRead, Box<dyn oio::ReadStreamDyn>)> {
        match self.route().await? {
            Route::Primary => self.primary_reader()?.open(range).await,
            Route::Fallback => {
                let (reply, stream) = self.open_fallback(range).await?;
                Ok((reply, Box::new(stream) as Box<dyn oio::ReadStreamDyn>))
            }
        }
    }

    async fn read(&self, range: BytesRange) -> Result<(RpRead, Buffer)> {
        match self.route().await? {
            Route::Primary => self.primary_reader()?.read(range).await,
            Route::Fallback => {
                let (reply, mut stream) = self.open_fallback(range).await?;
                let buffer = stream.read_all().await?;
                Ok((reply, buffer))
            }
        }
    }
}

/// A fallback stream that fails as soon as it would yield more bytes than the
/// range asked for, or ends with fewer.
///
/// It is what keeps a ranged fallback read bounded by its range: the chunk
/// that crosses the limit is refused rather than handed on, so a backend that
/// ignores `Range` costs one chunk, not the whole body.
pub(crate) struct BoundedStream {
    inner: Box<dyn oio::ReadStreamDyn>,
    /// The bytes the range holds, when it says.
    limit: Option<u64>,
    /// The bytes yielded so far.
    read: u64,
    path: String,
    range: BytesRange,
}

impl oio::ReadStream for BoundedStream {
    async fn read(&mut self) -> Result<Buffer> {
        let chunk = self.inner.read_dyn().await?;
        let Some(limit) = self.limit else {
            return Ok(chunk);
        };
        if chunk.is_empty() {
            if self.read < limit {
                return Err(Error::new(
                    ErrorKind::Unexpected,
                    format!(
                        "the fallback disk answered {} bytes of '{}' for the range {}, which \
                         holds {limit}",
                        self.read, self.path, self.range
                    ),
                ));
            }
            return Ok(chunk);
        }
        self.read += chunk.len() as u64;
        if self.read > limit {
            return Err(range_ignored(&self.path, self.range));
        }
        Ok(chunk)
    }
}

/// The error for a fallback that answered a range with bytes outside it.
fn range_ignored(path: &str, range: BytesRange) -> Error {
    Error::new(
        ErrorKind::Unexpected,
        format!(
            "the fallback disk answered the range {range} of '{path}' with bytes outside \
             it; the backend ignores range requests"
        ),
    )
}

/// The deleter produced by [`ReadThroughService`]. Removes the object from the
/// fallback as well as the primary, so a delete cannot be undone by the next
/// read promoting the cold copy back.
pub(crate) struct ReadThroughDeleter {
    /// The primary's deleter.
    inner: oio::Deleter,
    /// The disk the same delete is replayed against first.
    fallback: Operator,
    /// Keeps a promotion in flight from republishing what this removes.
    publications: Arc<Publications>,
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
        let Self {
            inner,
            fallback,
            publications,
        } = self;
        publications
            .retire(path, args.recursive(), async {
                fallback.delete_options(path, options).await?;
                inner.delete(path, args).await
            })
            .await
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
/// Failure aborts only the writer's owned state, as `copy_between_disks` does.
/// A public destination can belong to another writer even if it was absent
/// when this transfer began. Cancellation diverts abort to a detached task.
/// A backend that truncates in place cannot restore old bytes.
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

    let writer = primary
        .writer_options(
            to,
            WriteOptions {
                if_not_exists: conditions.if_not_exists,
                ..Default::default()
            },
        )
        .await?;

    let mut guard = WriterGuard::new(primary.clone(), "read-through primary", to, writer)
        .preserve_destination();
    let result = async {
        while let Some(chunk) = stream.try_next().await.map_err(|e| {
            Error::new(
                ErrorKind::Unexpected,
                format!("reading '{from}' from the fallback disk failed"),
            )
            .set_source(e)
        })? {
            guard.writer().write(chunk).await?;
        }
        guard.writer().close().await
    }
    .await;
    guard.settle(result).await
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
        /// The size of every buffer a writer was handed, in order.
        write_sizes: Mutex<Vec<usize>>,
        /// How many pieces a piecewise stream has handed out.
        pieces_pulled: Mutex<usize>,
        /// Held by the next read until the test releases it.
        read_gate: Mutex<Option<ReadGate>>,
        /// What closed writers stored, by path, moved by a rename and
        /// removed by a delete. A path found here answers ahead of the
        /// stub's fixed body, so a promotion can be read back.
        stored: Mutex<std::collections::HashMap<String, Buffer>>,
        /// Paths deleted since anything was last stored there. A deleted
        /// path answers as missing even where the fixed body would have
        /// answered, so a delete on the stub is observable afterwards.
        deleted: Mutex<std::collections::HashSet<String>>,
        /// When set, every `stat` hands the test a release down this channel
        /// and answers only once the test sends or drops it. A test acts at
        /// the stat it cares about by looking at the disks, not by counting
        /// calls.
        stat_hook: Mutex<Option<StatHook>>,
        /// Held by the next delete until the test releases it.
        delete_gate: Mutex<Option<ReadGate>>,
        /// Told after every delete, so a test can wait for one that runs on
        /// a task it does not own.
        deleted_once: tokio::sync::Notify,
        /// The version each stored path was last written under, on a disk
        /// that keeps versions.
        versions: Mutex<std::collections::HashMap<String, String>>,
        /// How many versions the disk has handed out.
        next_version: Mutex<u64>,
    }

    /// The sending end of [`Journal::stat_hook`].
    type StatHook = tokio::sync::mpsc::UnboundedSender<tokio::sync::oneshot::Sender<()>>;

    /// The receiving end of [`Journal::stat_hook`]: one release per held stat.
    type HeldStats = tokio::sync::mpsc::UnboundedReceiver<tokio::sync::oneshot::Sender<()>>;

    /// Stops one call part-way so a test can act while it is in flight.
    struct ReadGate {
        /// Told when the read reaches the gate.
        entered: tokio::sync::oneshot::Sender<()>,
        /// Awaited before the read goes on.
        release: tokio::sync::oneshot::Receiver<()>,
    }

    impl std::fmt::Debug for ReadGate {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("ReadGate")
        }
    }

    /// Wait at the journal's read gate, if a test set one.
    async fn pass_read_gate(journal: &Journal) {
        pass_gate(&journal.read_gate).await;
    }

    /// Wait at `gate`, if a test set it.
    async fn pass_gate(gate: &Mutex<Option<ReadGate>>) {
        let gate = locked(gate).take();
        if let Some(gate) = gate {
            let _ = gate.entered.send(());
            let _ = gate.release.await;
        }
    }

    /// Wait for the test to release this stat, if it holds stats.
    async fn pass_stat_hook(journal: &Journal) {
        let hook = locked(&journal.stat_hook).clone();
        if let Some(hook) = hook {
            let (release, released) = tokio::sync::oneshot::channel();
            if hook.send(release).is_ok() {
                let _ = released.await;
            }
        }
    }

    /// The bytes a list of fetched ranges covers over an object of `len`.
    fn fetched_bytes(ranges: &[BytesRange], len: u64) -> u64 {
        ranges
            .iter()
            .map(|range| {
                range
                    .size()
                    .unwrap_or_else(|| len.saturating_sub(range.offset()))
            })
            .sum()
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
        /// Answer every read with the whole body whatever range was asked
        /// for, the way an HTTP backend that ignores `Range` answers `200`.
        ignores_range: bool,
        /// Hand an opened stream out in pieces of this many bytes, the way a
        /// network body arrives, instead of in one buffer.
        stream_piece: Option<usize>,
        /// Keep a version for every write, report it, and delete a single
        /// version on request, the way a versioned object store does.
        versions: bool,
        /// Report no ETag for an object, only its length, the way the
        /// in-memory disk does. By default the stub reports an ETag derived
        /// from the object's bytes, as an object store does.
        reports_no_validators: bool,
    }

    /// An ETag that changes whenever the bytes do.
    fn content_etag(contents: &Buffer) -> String {
        use std::hash::{Hash as _, Hasher as _};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        contents.to_vec().hash(&mut hasher);
        format!("\"{:016x}\"", hasher.finish())
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
        path: String,
        contents: Buffer,
        read_fails_after: Option<usize>,
        ignores_range: bool,
        stream_piece: Option<usize>,
        journal: Arc<Journal>,
    }

    /// A body handed out a piece at a time, counting the pieces.
    struct PieceStream {
        rest: Buffer,
        piece: usize,
        journal: Arc<Journal>,
    }

    impl oio::ReadStream for PieceStream {
        async fn read(&mut self) -> Result<Buffer> {
            if self.rest.is_empty() {
                return Ok(Buffer::new());
            }
            let take = self.piece.min(self.rest.len());
            let piece = self.rest.slice(0..take);
            self.rest = self.rest.slice(take..self.rest.len());
            *locked(&self.journal.pieces_pulled) += 1;
            Ok(piece)
        }
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
            // Looked up per range, not when the reader was built: a
            // promotion stores its object after the reader exists.
            let stored = locked(&self.journal.stored).get(&self.path).cloned();
            let contents = match stored {
                Some(stored) => stored,
                None if locked(&self.journal.deleted).contains(&self.path) => {
                    return Err(Error::new(
                        ErrorKind::NotFound,
                        "the stub disk deleted this path",
                    ));
                }
                None => self.contents.clone(),
            };
            if self.ignores_range {
                return Ok(contents);
            }
            let slice = range.to_content_range(contents.len())?;
            Ok(contents.slice(slice))
        }
    }

    impl oio::Read for StubReader {
        async fn open(&self, range: BytesRange) -> Result<(RpRead, Box<dyn oio::ReadStreamDyn>)> {
            // The body is taken before the gate, so a read held there has
            // already fetched what it will hand on: whatever the test does
            // while it waits cannot take the bytes back.
            let body = self.slice(range)?;
            pass_read_gate(&self.journal).await;
            let stream: Box<dyn oio::ReadStreamDyn> = match self.stream_piece {
                Some(piece) => Box::new(PieceStream {
                    rest: body,
                    piece,
                    journal: Arc::clone(&self.journal),
                }),
                None => Box::new(body),
            };
            Ok((RpRead::default(), stream))
        }

        async fn read(&self, range: BytesRange) -> Result<(RpRead, Buffer)> {
            let body = self.slice(range)?;
            pass_read_gate(&self.journal).await;
            Ok((RpRead::default(), body))
        }
    }

    struct StubWriter {
        fails: bool,
        versions: bool,
        path: String,
        body: Vec<Buffer>,
        journal: Arc<Journal>,
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
        async fn write(&mut self, buffer: Buffer) -> Result<()> {
            locked(&self.journal.write_sizes).push(buffer.len());
            if self.fails {
                return Err(Self::failure());
            }
            self.body.push(buffer);
            Ok(())
        }

        async fn close(&mut self) -> Result<Metadata> {
            if self.fails {
                return Err(Self::failure());
            }
            let body: Buffer = std::mem::take(&mut self.body)
                .into_iter()
                .flatten()
                .collect();
            locked(&self.journal.deleted).remove(&self.path);
            locked(&self.journal.stored).insert(self.path.clone(), body);
            let mut metadata = Metadata::new(EntryMode::FILE);
            if self.versions {
                let version = {
                    let mut next = locked(&self.journal.next_version);
                    *next += 1;
                    format!("v{next}")
                };
                locked(&self.journal.versions).insert(self.path.clone(), version.clone());
                metadata.set_version(&version);
            }
            Ok(metadata)
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
        async fn delete(&mut self, path: &str, args: OpDelete) -> Result<()> {
            let attempt = {
                let mut deletes = locked(&self.journal.deletes);
                deletes.push(path.to_owned());
                deletes.len() - 1
            };
            pass_gate(&self.journal.delete_gate).await;
            if attempt < self.failures {
                return Err(Error::new(
                    ErrorKind::Unexpected,
                    "the stub disk cannot delete right now",
                ));
            }
            // A versioned delete removes that one version: when the path now
            // holds a newer one, the newer one stays.
            let current = locked(&self.journal.versions).get(path).cloned();
            let removes = match args.version() {
                Some(version) => current.as_deref() == Some(version),
                None => true,
            };
            if removes {
                locked(&self.journal.stored).remove(path);
                locked(&self.journal.versions).remove(path);
                locked(&self.journal.deleted).insert(path.to_owned());
            }
            self.journal.deleted_once.notify_one();
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
                delete_with_version: self.spec.versions,
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
            // Before anything is read, so what the test changes while it
            // holds the stat is what the stat answers.
            pass_stat_hook(&self.journal).await;
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
            let stored = locked(&self.journal.stored).get(path).cloned();
            let deleted = locked(&self.journal.deleted).contains(path);
            let fixed = if deleted {
                None
            } else {
                self.contents.as_ref()
            };
            match stored.as_ref().or(fixed) {
                Some(contents) => {
                    let mut metadata =
                        Metadata::new(EntryMode::FILE).with_content_length(contents.len() as u64);
                    if let Some(content_type) = self.spec.content_type {
                        metadata.set_content_type(content_type);
                    }
                    if !self.spec.reports_no_validators {
                        metadata.set_etag(&content_etag(contents));
                    }
                    if let Some(version) = locked(&self.journal.versions).get(path) {
                        metadata.set_version(version);
                    }
                    Ok(RpStat::new(metadata))
                }
                None => Err(self.missing()),
            }
        }

        fn read(&self, _ctx: &OperationContext, path: &str, args: OpRead) -> Result<Self::Reader> {
            locked(&self.journal.reads).push(args);
            // Real backends hand back a lazy reader and only fail once a range
            // is asked for, which is what lets the layer build the primary's
            // reader before it knows whether the primary holds the object.
            Ok(StubReader {
                path: path.to_owned(),
                contents: self.contents.clone().unwrap_or_default(),
                read_fails_after: self.spec.read_fails_after,
                ignores_range: self.spec.ignores_range,
                stream_piece: self.spec.stream_piece,
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
                WriteBehavior::FailAfterOpen => Ok(StubWriter {
                    fails: true,
                    versions: self.spec.versions,
                    path: path.to_owned(),
                    body: Vec::new(),
                    journal: Arc::clone(&self.journal),
                }),
                WriteBehavior::Accept => Ok(StubWriter {
                    fails: false,
                    versions: self.spec.versions,
                    path: path.to_owned(),
                    body: Vec::new(),
                    journal: Arc::clone(&self.journal),
                }),
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
            let mut stored = locked(&self.journal.stored);
            if let Some(body) = stored.remove(from) {
                locked(&self.journal.deleted).remove(to);
                stored.insert(to.to_owned(), body);
            }
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
        let assets = primary.clone().layer(ReadThroughLayer::new(
            primary,
            fallback,
            copy,
            throw_on_promotion_failure,
        ));
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

    /// A promoting disk over a rename-capable primary that answers the first
    /// existence probe and then cannot answer the race re-check, which runs
    /// just before the staged copy is renamed into place. A primary without a
    /// rename has no re-check: its conditional write is the guard.
    fn re_check_fails_read_through(
        throw_on_promotion_failure: bool,
    ) -> (Operator, Arc<Journal>, Arc<Journal>) {
        read_through(
            StubSpec {
                stat_fails_after: Some(1),
                renames: true,
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
            .expect("a failed re-check degrades to reading the fallback");
        assert_eq!(&bytes.to_vec(), b"cold bytes");
        assert_eq!(
            primary.stats().len(),
            2,
            "the read probed the primary once to route and once to re-check"
        );
        assert!(
            primary.renames().is_empty(),
            "a re-check that cannot answer must not promote over whatever is \
             there"
        );
        assert_eq!(
            primary.deletes(),
            primary.write_paths(),
            "the staged copy that was never published is removed"
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
    async fn a_promoting_read_streams_the_whole_object_once_then_reads_the_primary() {
        let (assets, primary, fallback) =
            rename_capable_read_through(WriteBehavior::Accept, false, false);

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
             what a promoting read streams from the fallback"
        );
        assert_eq!(
            primary.renames().len(),
            1,
            "the object was published, and the range was then read from the primary"
        );
    }

    /// A body that spans two transfer chunks, so the streaming loop runs more
    /// than once and a failure can land between chunks.
    const TWO_CHUNK_BYTES: usize = CROSS_DISK_CHUNK_BYTES + 4096;

    /// An operator over a real local-filesystem directory, configured exactly
    /// as `Storage::register_fs` configures one - atomic staging included, so
    /// these tests see the same write path an application does.
    fn fs_operator(root: &std::path::Path) -> Operator {
        crate::filesystem::local_fs_operator(root.to_str().expect("a tempdir path is valid UTF-8"))
            .expect("the fs service builds over an existing directory")
    }

    /// Compose a read-through disk over a *real* primary and a stub fallback.
    ///
    /// The transfer tests need a primary that really stores what it is given -
    /// what sits at the destination after a failed transfer is the whole
    /// assertion - while still needing a fallback that can drop a transfer
    /// part-way or lose a delete, which only the stub can do.
    fn read_through_over(primary: Operator, fallback_spec: StubSpec) -> (Operator, Arc<Journal>) {
        let (fallback, fallback_journal) = StubDisk::operator(fallback_spec);
        let assets = primary
            .clone()
            .layer(ReadThroughLayer::new(primary, fallback, true, false));
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

    /// One MiB, in sixteen transfer chunks.
    const ONE_MIB: usize = 1024 * 1024;

    /// DRIVERS-018: a read that is never promoted fetches only the range it
    /// asked for, even on a disk with `copy: true`. A versioned read used to
    /// fetch the whole object although nothing was written back.
    #[tokio::test]
    async fn a_versioned_ranged_read_on_a_copying_disk_fetches_only_the_range() {
        let (assets, primary, fallback) = stub_read_through();

        let bytes = assets
            .read_with("cold.txt")
            .range(5..10)
            .version("v7")
            .await
            .expect("a versioned ranged read resolves from the fallback");
        assert_eq!(&bytes.to_vec(), b"bytes");

        let ranges = fallback.ranges();
        assert_eq!(ranges.len(), 1, "the fallback was read once");
        assert_eq!(
            (ranges[0].offset(), ranges[0].size()),
            (5, Some(5)),
            "a read that is never promoted has no reason to fetch the whole object"
        );
        assert!(primary.write_paths().is_empty());
    }

    /// DRIVERS-018: once a promotion has failed, the rest of the read takes
    /// ranged reads from the fallback. Every chunk of a chunked read used to
    /// retry the promotion and fetch the whole object again, so the bytes
    /// moved grew with the square of the object size.
    #[tokio::test]
    async fn a_failed_promotion_is_not_retried_by_every_chunk_of_the_read() {
        let (assets, _primary, fallback) = read_through(
            StubSpec::default(),
            StubSpec {
                generated_bytes: Some(ONE_MIB),
                ..Default::default()
            },
            false,
        );

        let reader = assets
            .reader_with("cold.bin")
            .chunk(CROSS_DISK_CHUNK_BYTES)
            .await
            .expect("a chunked reader opens");
        let chunks: Vec<_> = reader
            .into_bytes_stream(0..ONE_MIB as u64)
            .await
            .expect("the stream opens")
            .try_collect()
            .await
            .expect("every chunk resolves from the fallback");
        let total: usize = chunks.iter().map(|chunk| chunk.len()).sum();
        assert_eq!(total, ONE_MIB, "the whole object was served");

        let fetched = fetched_bytes(&fallback.ranges(), ONE_MIB as u64);
        assert!(
            fetched <= (ONE_MIB + CROSS_DISK_CHUNK_BYTES) as u64,
            "serving one MiB moved {fetched} bytes from the fallback"
        );
    }

    /// DRIVERS-018: a promotion streams the object into the primary instead
    /// of holding all of it in memory. It used to fetch the whole object into
    /// one buffer and write that buffer through in a single call.
    #[tokio::test]
    async fn a_promotion_streams_the_object_instead_of_buffering_it() {
        let (assets, primary, _fallback) = read_through(
            StubSpec {
                renames: true,
                writes: WriteBehavior::Accept,
                ..Default::default()
            },
            StubSpec {
                generated_bytes: Some(ONE_MIB),
                stream_piece: Some(16 * 1024),
                ..Default::default()
            },
            false,
        );

        let _ = assets.read_with("cold.bin").range(5..10).await;

        let sizes = locked(&primary.write_sizes).clone();
        assert!(!sizes.is_empty(), "the promotion wrote the object through");
        assert_eq!(
            sizes.iter().sum::<usize>(),
            ONE_MIB,
            "the promotion wrote the whole object"
        );
        let largest = sizes.iter().copied().max().unwrap_or_default();
        assert!(
            largest <= CROSS_DISK_CHUNK_BYTES,
            "the promotion handed the primary a {largest}-byte buffer: the object was \
             held in memory whole instead of streamed"
        );
    }

    /// The DRIVERS-018 follow-up: a fallback that ignores `Range` and answers
    /// with the whole body cannot make a ranged read hold the whole body.
    /// The read used to collect every byte the backend sent before anything
    /// noticed the length was wrong, and returned them.
    #[tokio::test]
    async fn a_fallback_that_ignores_the_range_cannot_make_a_ranged_read_hold_the_body() {
        let (assets, _primary, fallback) = read_through_with_copy(
            StubSpec::default(),
            StubSpec {
                generated_bytes: Some(ONE_MIB),
                ignores_range: true,
                stream_piece: Some(16 * 1024),
                ..Default::default()
            },
            false,
            false,
        );

        let result = assets.read_with("cold.bin").range(5..10).await;
        assert!(
            result.is_err(),
            "a ranged read answered with the whole body must fail, got {} bytes",
            result.as_ref().map_or(0, |bytes| bytes.len())
        );
        let pulled = *locked(&fallback.pieces_pulled);
        assert!(
            pulled <= 2,
            "the read pulled {pulled} pieces of a body it could never use"
        );
    }

    /// DRIVERS-019: a delete that completes while a promotion is in flight is
    /// not undone by that promotion. The promotion used to publish whatever
    /// it had fetched as soon as the primary looked empty, which is exactly
    /// how the primary looks right after a delete.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_delete_during_a_promotion_is_not_undone_by_it() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let primary = fs_operator(tmp.path());
        let (fallback, fallback_journal) = StubDisk::operator(StubSpec {
            contents: Some("cold bytes"),
            ..Default::default()
        });
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        *locked(&fallback_journal.read_gate) = Some(ReadGate {
            entered: entered_tx,
            release: release_rx,
        });
        let assets = primary.clone().layer(ReadThroughLayer::new(
            primary.clone(),
            fallback,
            true,
            false,
        ));

        let reader = tokio::spawn({
            let assets = assets.clone();
            async move { assets.read("cold.txt").await }
        });
        entered_rx
            .await
            .expect("the promotion reached the fallback");
        assets
            .delete("cold.txt")
            .await
            .expect("the delete completes while the promotion is held");
        release_tx.send(()).expect("release the promotion");
        // The overlapping read may still answer with the old bytes; what it
        // may not do is put them back.
        let _ = reader.await.expect("the read task");

        assert!(
            !primary.exists("cold.txt").await.expect("exists answers"),
            "the promotion republished an object a completed delete had removed"
        );
    }

    /// DRIVERS-020: a versioned read reaches a fallback that can answer it
    /// even when the primary cannot read versions at all. The read used to
    /// build the primary's reader first, and the primary's correctness check
    /// refused the version before the fallback was ever asked.
    #[tokio::test]
    async fn a_versioned_read_reaches_the_fallback_through_a_primary_without_versions() {
        let primary = memory();
        let (fallback, fallback_journal) = StubDisk::operator(StubSpec {
            contents: Some("cold bytes"),
            ..Default::default()
        });
        let assets = primary
            .clone()
            .layer(ReadThroughLayer::new(primary, fallback, true, false));

        let bytes = assets
            .read_with("cold.txt")
            .version("v7")
            .await
            .expect("the fallback answers a versioned read the primary cannot");
        assert_eq!(&bytes.to_vec(), b"cold bytes");
        assert_eq!(
            fallback_journal
                .reads()
                .first()
                .and_then(|read| read.version()),
            Some("v7"),
            "the version must reach the fallback"
        );
    }

    /// Two read-through disks over one shared primary and one fallback, the
    /// way two nodes of one deployment see them. Each has its own in-process
    /// coordination, so nothing either holds in memory reaches the other.
    ///
    /// The fallback holds `cold bytes` under `cold.txt`, and its next read
    /// stops at a gate after it has fetched its bytes. Node A's promotion is
    /// the read that stops there. Returns both disks and the gate's two ends.
    fn two_nodes(
        primary: &Operator,
    ) -> (
        Operator,
        Operator,
        tokio::sync::oneshot::Receiver<()>,
        tokio::sync::oneshot::Sender<()>,
    ) {
        let (fallback, journal) = StubDisk::operator(StubSpec {
            contents: Some("cold bytes"),
            ..Default::default()
        });
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        *locked(&journal.read_gate) = Some(ReadGate {
            entered: entered_tx,
            release: release_rx,
        });
        let node_a = primary.clone().layer(ReadThroughLayer::new(
            primary.clone(),
            fallback.clone(),
            true,
            false,
        ));
        let node_b = primary.clone().layer(ReadThroughLayer::new(
            primary.clone(),
            fallback,
            true,
            false,
        ));
        (node_a, node_b, entered_rx, release_tx)
    }

    /// DRIVERS-019, across nodes: a delete that completes on node B while
    /// node A's promotion holds the bytes it fetched leaves the object
    /// deleted. Node A used to publish what it had fetched, because only its
    /// own process could tell it a delete had run.
    async fn a_delete_on_another_node_wins_over_a_promotion(primary: Operator) {
        let (node_a, node_b, entered, release) = two_nodes(&primary);

        let reader = tokio::spawn({
            let node_a = node_a.clone();
            async move { node_a.read("cold.txt").await }
        });
        entered
            .await
            .expect("node A's promotion fetched the object");
        node_b
            .delete("cold.txt")
            .await
            .expect("node B's delete completes while node A is held");
        release.send(()).expect("release node A");
        // Node A's own read may fail or answer with the old bytes; what it
        // may not do is leave them on the shared primary.
        let _ = reader.await.expect("node A's read task");

        assert!(
            !primary.exists("cold.txt").await.expect("exists answers"),
            "node A's promotion republished an object node B's delete had removed"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_delete_on_another_node_wins_over_a_promotion_on_a_local_primary() {
        let tmp = tempfile::tempdir().expect("tempdir");
        a_delete_on_another_node_wins_over_a_promotion(fs_operator(tmp.path())).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_delete_on_another_node_wins_over_a_promotion_on_a_conditional_primary() {
        a_delete_on_another_node_wins_over_a_promotion(memory()).await;
    }

    /// DRIVERS-019, across nodes, for a move: node B moving `cold.txt` while
    /// node A promotes it leaves no `cold.txt` behind on the shared primary.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_move_on_another_node_wins_over_a_promotion() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let primary = fs_operator(tmp.path());
        let (node_a, node_b, entered, release) = two_nodes(&primary);

        let reader = tokio::spawn({
            let node_a = node_a.clone();
            async move { node_a.read("cold.txt").await }
        });
        entered
            .await
            .expect("node A's promotion fetched the object");
        node_b
            .rename("cold.txt", "moved.txt")
            .await
            .expect("node B's move completes while node A is held");
        release.send(()).expect("release node A");
        let _ = reader.await.expect("node A's read task");

        assert!(
            !primary.exists("cold.txt").await.expect("exists answers"),
            "node A's promotion republished the source of node B's move"
        );
        assert_eq!(
            &primary
                .read("moved.txt")
                .await
                .expect("the move landed")
                .to_vec(),
            b"cold bytes"
        );
    }

    /// Make every `stat` on `journal`'s disk wait for the test. Each one
    /// sends a release down the returned channel and answers once the test
    /// sends or drops it.
    fn hold_stats(journal: &Journal) -> HeldStats {
        let (hook, held) = tokio::sync::mpsc::unbounded_channel();
        *locked(&journal.stat_hook) = Some(hook);
        held
    }

    /// Let held fallback stats through until one runs while the primary
    /// holds `path`: the check a promotion makes after it publishes. Returns
    /// that stat's release, still unsent, and stops holding later stats.
    async fn hold_the_confirmation(
        held: &mut HeldStats,
        fallback: &Journal,
        primary: &Operator,
        path: &str,
    ) -> tokio::sync::oneshot::Sender<()> {
        loop {
            let release = held
                .recv()
                .await
                .expect("the promotion checks the fallback after it publishes");
            if primary.exists(path).await.expect("exists answers") {
                *locked(&fallback.stat_hook) = None;
                return release;
            }
            let _ = release.send(());
        }
    }

    /// The fallback holds `cold bytes` under `cold.txt`.
    fn cold_fallback() -> (Operator, Arc<Journal>) {
        StubDisk::operator(StubSpec {
            contents: Some("cold bytes"),
            ..Default::default()
        })
    }

    /// A primary that keeps a version per write and can delete one version.
    fn versioned_primary() -> (Operator, Arc<Journal>) {
        StubDisk::operator(StubSpec {
            writes: WriteBehavior::Accept,
            versions: true,
            ..Default::default()
        })
    }

    /// A length alone identifies nothing; an ETag, a version, or a
    /// modification time with the length does.
    #[test]
    fn a_length_alone_is_not_an_identity() {
        let sized = |len: u64| Metadata::new(EntryMode::FILE).with_content_length(len);
        assert_eq!(same_source(&sized(10), &sized(10)), None);
        assert!(!has_validator(&sized(10)));

        let tagged = |etag: &str| sized(10).with_etag(etag.to_owned());
        assert_eq!(same_source(&tagged("a"), &tagged("a")), Some(true));
        assert_eq!(same_source(&tagged("a"), &tagged("b")), Some(false));

        let at = Timestamp::from_second(1_700_000_000).expect("a valid time");
        let stamped = |len: u64| sized(len).with_last_modified(at);
        assert_eq!(same_source(&stamped(10), &stamped(10)), Some(true));
        assert_eq!(same_source(&stamped(10), &stamped(11)), Some(false));
        // A validator only one side reports proves nothing.
        assert_eq!(same_source(&tagged("a"), &sized(10)), None);
    }

    /// A fallback that reports only lengths holds `cold bytes` under
    /// `cold.txt`.
    fn cold_fallback_without_validators() -> (Operator, Arc<Journal>) {
        StubDisk::operator(StubSpec {
            contents: Some("cold bytes"),
            reports_no_validators: true,
            ..Default::default()
        })
    }

    /// Overwrite `cold.txt` on a stub fallback with bytes of the same length.
    fn overwrite_with_same_size(fallback: &Journal) {
        locked(&fallback.stored).insert("cold.txt".to_owned(), Buffer::from("COLD BYTES"));
    }

    /// Sol review follow-up: a fallback that reports no ETag, version or
    /// modification time cannot vouch for an object by its length. A
    /// same-size overwrite while the promotion streamed used to pass the
    /// check before the publish, and the old bytes landed on the primary.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_same_size_overwrite_during_a_promotion_is_not_published() {
        let primary = memory();
        let (fallback, fallback_journal) = cold_fallback_without_validators();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        *locked(&fallback_journal.read_gate) = Some(ReadGate {
            entered: entered_tx,
            release: release_rx,
        });
        let assets = primary.clone().layer(ReadThroughLayer::new(
            primary.clone(),
            fallback,
            true,
            false,
        ));

        let reader = tokio::spawn({
            let assets = assets.clone();
            async move { assets.read("cold.txt").await }
        });
        entered_rx
            .await
            .expect("the promotion fetched the old bytes");
        overwrite_with_same_size(&fallback_journal);
        release_tx.send(()).expect("release the promotion");
        reader
            .await
            .expect("the read task")
            .expect("the read is still served");

        assert!(
            !primary.exists("cold.txt").await.expect("exists answers"),
            "the promotion published bytes the fallback no longer holds"
        );
    }

    /// Sol review follow-up: the check after the publish does not take a
    /// length as proof either. A same-size overwrite between the publish and
    /// that check used to keep the old bytes on the primary.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_same_size_overwrite_after_the_publish_is_withdrawn() {
        let (primary, _primary_journal) = versioned_primary();
        let (fallback, fallback_journal) = cold_fallback_without_validators();
        let mut held = hold_stats(&fallback_journal);
        let assets = primary.clone().layer(ReadThroughLayer::new(
            primary.clone(),
            fallback,
            true,
            false,
        ));

        let reader = tokio::spawn({
            let assets = assets.clone();
            async move { assets.read("cold.txt").await }
        });
        let confirmation =
            hold_the_confirmation(&mut held, &fallback_journal, &primary, "cold.txt").await;
        overwrite_with_same_size(&fallback_journal);
        let _ = confirmation.send(());
        let _ = reader.await.expect("the read task");

        assert!(
            !primary.exists("cold.txt").await.expect("exists answers"),
            "the promotion kept bytes the fallback no longer holds"
        );
    }

    /// Sol review: a withdrawal never removes what a writer put on the
    /// primary after the promotion published, even an object of the same
    /// size on a primary that reports nothing else about it. The withdrawal
    /// used to take an equal length as proof that the object was its own
    /// copy, and deleted the writer's object.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_withdrawal_leaves_a_same_sized_replacement_alone() {
        let primary = memory();
        let (fallback, fallback_journal) = cold_fallback();
        let mut held = hold_stats(&fallback_journal);
        let assets = primary.clone().layer(ReadThroughLayer::new(
            primary.clone(),
            fallback,
            true,
            false,
        ));

        let reader = tokio::spawn({
            let assets = assets.clone();
            async move { assets.read("cold.txt").await }
        });
        let confirmation =
            hold_the_confirmation(&mut held, &fallback_journal, &primary, "cold.txt").await;
        primary
            .write("cold.txt", "COLD BYTES")
            .await
            .expect("a writer replaces the promoted copy");
        // The fallback copy is gone, so the promotion wants its copy back.
        locked(&fallback_journal.deleted).insert("cold.txt".to_owned());
        let _ = confirmation.send(());
        let _ = reader.await.expect("the read task");

        assert_eq!(
            &primary
                .read("cold.txt")
                .await
                .expect("the writer's object is still on the primary")
                .to_vec(),
            b"COLD BYTES",
            "the withdrawal deleted an object a writer had put on the primary"
        );
    }

    /// Sol review: a withdrawal removes only the version the promotion
    /// wrote, so a writer that replaces the object after the withdrawal
    /// decided to run keeps its object. The withdrawal used to check the
    /// primary and then delete the path outright, so whatever replaced the
    /// copy in between went instead.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_withdrawal_deletes_only_the_version_the_promotion_wrote() {
        let (primary, primary_journal) = versioned_primary();
        let (fallback, fallback_journal) = cold_fallback();
        let mut held = hold_stats(&fallback_journal);
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        *locked(&primary_journal.delete_gate) = Some(ReadGate {
            entered: entered_tx,
            release: release_rx,
        });
        let assets = primary.clone().layer(ReadThroughLayer::new(
            primary.clone(),
            fallback,
            true,
            false,
        ));

        let reader = tokio::spawn({
            let assets = assets.clone();
            async move { assets.read("cold.txt").await }
        });
        let confirmation =
            hold_the_confirmation(&mut held, &fallback_journal, &primary, "cold.txt").await;
        locked(&fallback_journal.deleted).insert("cold.txt".to_owned());
        let _ = confirmation.send(());
        entered_rx
            .await
            .expect("the withdrawal reaches the primary's delete");
        primary
            .write("cold.txt", "COLD BYTES")
            .await
            .expect("a writer replaces the promoted copy");
        release_tx.send(()).expect("release the withdrawal");
        let _ = reader.await.expect("the read task");

        assert_eq!(
            &primary
                .read("cold.txt")
                .await
                .expect("the writer's object is still on the primary")
                .to_vec(),
            b"COLD BYTES",
            "the withdrawal deleted an object a writer put on the primary after it decided to \
             run"
        );
    }

    /// DRIVERS-019, Sol review: a read cancelled while its promotion checks
    /// the fallback still withdraws the copy when that check finds the
    /// object deleted. The check ran on the read's own task, so cancelling
    /// the read dropped it and left a deleted object on the primary.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_cancelled_read_still_withdraws_a_promotion_a_delete_overtook() {
        let (primary, primary_journal) = versioned_primary();
        let (fallback, fallback_journal) = cold_fallback();
        let mut held = hold_stats(&fallback_journal);
        let assets = primary.clone().layer(ReadThroughLayer::new(
            primary.clone(),
            fallback,
            true,
            false,
        ));

        let reader = tokio::spawn({
            let assets = assets.clone();
            async move { assets.read("cold.txt").await }
        });
        let confirmation =
            hold_the_confirmation(&mut held, &fallback_journal, &primary, "cold.txt").await;
        // Another node deleted the object: its fallback copy first, then the
        // primary's, before this promotion published.
        locked(&fallback_journal.deleted).insert("cold.txt".to_owned());
        reader.abort();
        let _ = reader.await;
        assert!(
            confirmation.send(()).is_ok(),
            "the check died with the cancelled read, so nothing withdraws the copy of a \
             deleted object"
        );
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            primary_journal.deleted_once.notified(),
        )
        .await
        .expect("the withdrawal runs to the end after the read is cancelled");
        assert!(
            !primary.exists("cold.txt").await.expect("exists answers"),
            "the promotion left a deleted object on the primary"
        );
    }
}
