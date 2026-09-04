//! File-backed L1 [`RenderStore`]: one file per key under a directory the
//! process owns, published atomically so a reader never observes a partial
//! write. Entries survive a process restart, and a completed publication
//! also survives a hard crash or power loss in the ordinary case, since the
//! directory entry created by its rename is itself synced before the call
//! returns (see below) - if that sync fails, the publication has still
//! succeeded (the rename already landed), so a durability warning is
//! logged rather than the publication being reported as failed.
//! [`FileRenderStore::open`] rebuilds the in-memory byte tally by scanning
//! the directory once, removing anything left behind by a publication that
//! never completed, and every later publication keeps that tally in step
//! with the directory so no publish or eviction ever needs to re-read the
//! directory from disk.
//!
//! # File frame
//!
//! Each `<key.to_base64url()>.snrc` file holds one frame:
//!
//! | Field | Bytes |
//! |---|---|
//! | magic `SNRF` | 4 |
//! | fence epoch | 8 |
//! | fence token | 8 |
//! | fence generation digest | 32 |
//! | published_at_ms | 8 |
//! | stale_on_error_ms | 8 |
//! | entry length | 4 |
//! | entry bytes | entry length |
//! | SHA-256 of everything before it | 32 |
//!
//! `stale_on_error_ms` is the total milliseconds after `published_at_ms`
//! beyond which an entry can never be served again by any freshness band
//! (see [`suprnova_live::render_cache::coherence::evaluate_freshness`]:
//! for a non-private class, `Dead` starts at `fresh_ms + stale_on_error_ms`
//! past publication). [`FileRenderStore::sweep`] reads it, alongside each
//! entry's fence epoch, to remove files that can never be served again -
//! disk hygiene, not a correctness gate, since [`FileRenderStore::get`]
//! re-evaluates freshness independently on every read regardless of
//! whether sweep has run yet. [`RenderStore::publish`] (used generically,
//! including by this file's own tests) always frames zero here, since a
//! generic caller has no policy in scope to compute a real value from;
//! [`FileRenderStore::publish_with_retention`] is the one real caller
//! that does, and is what the middleware's L1 publish path uses.
//!
//! Publication writes a temporary file (`<name>.<pid>.<token>.tmp`),
//! `fsync`s it, renames it over the target, then `fsync`s the parent
//! directory: a reader either sees the previous complete file or the new
//! complete file, never a partial one, and the rename itself is not lost
//! to a crash or power loss either. A process that dies between creating
//! the temporary file and the rename leaves it behind with the `.tmp`
//! suffix, never the entry extension; [`FileRenderStore::open`] removes
//! any it finds. Any file that fails the frame check - wrong magic, a
//! truncated or tampered body, a bad digest - is treated as a miss and
//! removed, so a torn write self-heals on the next read rather than
//! living forever as a poisoned entry.
//!
//! The byte bound (`max_bytes`, passed to [`FileRenderStore::open`])
//! counts entry payload bytes, not the larger on-disk frame: sizing the
//! bound on what the caller actually asked to cache keeps it meaningful
//! even if the frame's fixed overhead changes later.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use bytes::Bytes;
use sha2::{Digest as _, Sha256};
use suprnova_live::render_cache::key::RenderKey;
use suprnova_live::render_cache::store::{
    PublicationFence, PublishOutcome, RenderStore, StoreInspection, StoredEntry,
};
use suprnova_live::render_cache::{RenderCacheError, RenderCacheErrorKind};
use tokio::sync::Mutex as AsyncMutex;

use crate::FrameworkError;

const MAGIC: &[u8; 4] = b"SNRF";
const FENCE_DIGEST_LEN: usize = 32;
const TRAILING_DIGEST_LEN: usize = 32;
/// Bytes of frame overhead before the entry bytes: magic (4) + fence epoch
/// (8) + fence token (8) + fence generation digest (32) + published_at_ms
/// (8) + stale_on_error_ms (8) + entry length (4).
const FRAME_HEADER_LEN: usize = 4 + 8 + 8 + FENCE_DIGEST_LEN + 8 + 8 + 4;

/// One key's tracked disk footprint, kept in memory so [`FileRenderStore`]
/// can compare fences and enforce the byte bound without reading a file
/// back from disk on every call. Kept in step with the directory by every
/// method that changes what is stored.
struct TrackedEntry {
    fence: PublicationFence,
    payload_bytes: u64,
    published_at_ms: u64,
    /// Total milliseconds after `published_at_ms` beyond which this entry
    /// is dead; see the module documentation. Mirrors the frame's own
    /// field so [`FileRenderStore::sweep`] never needs to re-read a file
    /// from disk to decide whether it is retired.
    stale_on_error_ms: u64,
}

/// In-memory byte tally, guarded by an async mutex so a publish can hold it
/// across the `fsync` and rename it performs.
struct TallyState {
    /// Keyed by `key.to_base64url()`, which is also the file stem.
    entries: BTreeMap<String, TrackedEntry>,
    total_bytes: u64,
}

/// File-backed L1 store. See the module documentation for the on-disk frame
/// and the atomicity guarantee.
pub struct FileRenderStore {
    directory: PathBuf,
    max_bytes: u64,
    state: AsyncMutex<TallyState>,
    /// Publications since open, for the every-256th automatic [`Self::sweep`]
    /// (see the module documentation). A plain atomic, not guarded by
    /// `state`: each call's `fetch_add` returns a value no other call can
    /// also observe, so exactly one call crosses each multiple-of-256
    /// boundary regardless of concurrent publishers.
    publish_count: std::sync::atomic::AtomicU64,
}

impl FileRenderStore {
    /// Opens (creating if needed) a file store rooted at `directory`,
    /// bounded to `max_bytes` of entry payload.
    ///
    /// Scans `directory` once, keeping every `.snrc` file whose frame
    /// checks out. A `.snrc` file that fails its frame check is a torn
    /// publication and is removed here rather than waiting for a `get`
    /// that may never come; a `.tmp` file is a temporary file left behind
    /// by a crash between its creation and the rename that would have
    /// made it a `.snrc` file, and is removed too, since it was never part
    /// of any published entry.
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError`] when `directory` cannot be created or
    /// listed.
    pub fn open(directory: impl AsRef<Path>, max_bytes: u64) -> Result<Self, FrameworkError> {
        let directory = directory.as_ref().to_path_buf();
        std::fs::create_dir_all(&directory).map_err(|err| {
            FrameworkError::from_external_with("creating the render cache L1 directory", err)
        })?;
        let mut entries = BTreeMap::new();
        let mut total_bytes: u64 = 0;
        let read_dir = std::fs::read_dir(&directory).map_err(|err| {
            FrameworkError::from_external_with("reading the render cache L1 directory", err)
        })?;
        for dir_entry in read_dir {
            let dir_entry = dir_entry.map_err(|err| {
                FrameworkError::from_external_with("reading the render cache L1 directory", err)
            })?;
            let path = dir_entry.path();
            let extension = path.extension().and_then(std::ffi::OsStr::to_str);
            if extension == Some("tmp") {
                // A process that dies between a publish's temporary file
                // and its rename leaves this behind. It was never part of
                // any published entry - the in-call cleanup on write
                // failure only runs when the call itself returns an error,
                // never after a hard stop - so without this it would sit
                // here consuming disk outside the tracked bound across
                // every future restart.
                let _ = std::fs::remove_file(&path);
                continue;
            }
            if extension != Some("snrc") {
                continue;
            }
            let Some(name) = path.file_stem().and_then(std::ffi::OsStr::to_str) else {
                continue;
            };
            let Ok(data) = std::fs::read(&path) else {
                continue;
            };
            match decode_frame(&Bytes::from(data)) {
                Ok(frame) => {
                    let payload_bytes = frame.payload.len() as u64;
                    total_bytes += payload_bytes;
                    entries.insert(
                        name.to_owned(),
                        TrackedEntry {
                            fence: frame.fence,
                            payload_bytes,
                            published_at_ms: frame.published_at_ms,
                            stale_on_error_ms: frame.stale_on_error_ms,
                        },
                    );
                }
                Err(_) => {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
        Ok(Self {
            directory,
            max_bytes,
            state: AsyncMutex::new(TallyState {
                entries,
                total_bytes,
            }),
            publish_count: std::sync::atomic::AtomicU64::new(0),
        })
    }

    fn path_for_name(&self, name: &str) -> PathBuf {
        self.directory.join(format!("{name}.snrc"))
    }

    fn path_for(&self, key: &RenderKey) -> PathBuf {
        self.path_for_name(&key.to_base64url())
    }

    fn temp_path_for(&self, name: &str, token: u64) -> PathBuf {
        self.directory
            .join(format!("{name}.{}.{token}.tmp", std::process::id()))
    }

    /// The path a key would be stored at, for tests that reach past the
    /// [`RenderStore`] contract to corrupt a file directly.
    #[doc(hidden)]
    #[must_use]
    pub fn path_for_test(&self, key: &RenderKey) -> PathBuf {
        self.path_for(key)
    }
}

#[async_trait]
impl RenderStore for FileRenderStore {
    async fn get(&self, key: &RenderKey) -> Result<Option<StoredEntry>, RenderCacheError> {
        let name = key.to_base64url();
        let path = self.path_for_name(&name);
        let data = match tokio::fs::read(&path).await {
            Ok(data) => data,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => {
                return Err(RenderCacheError::new(
                    RenderCacheErrorKind::ProviderUnavailable,
                ));
            }
        };
        if let Ok(frame) = decode_frame(&Bytes::from(data)) {
            return Ok(Some(StoredEntry {
                bytes: frame.payload,
                published_at_ms: frame.published_at_ms,
                fence: frame.fence,
            }));
        }
        // The read above happened before any lock was held, so a
        // concurrent publish for this key may already have replaced what
        // looked like a torn file with a valid one by the time cleanup
        // gets here. Re-read under the lock before deleting anything:
        // deleting by path alone on the strength of the earlier,
        // now-possibly-stale read would risk destroying a publish that
        // completed in between. (`evict`'s removal does not need this
        // because it is unconditional by intent - the caller asked for
        // this key gone regardless of what currently occupies it.)
        let mut state = self.state.lock().await;
        match tokio::fs::read(&path).await {
            Ok(data) => match decode_frame(&Bytes::from(data)) {
                Ok(frame) => Ok(Some(StoredEntry {
                    bytes: frame.payload,
                    published_at_ms: frame.published_at_ms,
                    fence: frame.fence,
                })),
                Err(_) => {
                    // Any frame defect - wrong magic, a truncated body, a
                    // bad digest - makes this file a miss either way: a
                    // file that fails decode can never be served. Mirrors
                    // `evict` for the cleanup itself: drop the tally entry
                    // only when the removal succeeded or the file was
                    // already absent. A real removal failure leaves the
                    // tally entry intact, so the entry stays counted
                    // rather than leaking disk outside the tracked bound
                    // forever.
                    match tokio::fs::remove_file(&path).await {
                        Ok(()) => {}
                        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                        Err(_) => return Ok(None),
                    }
                    if let Some(removed) = state.entries.remove(&name) {
                        state.total_bytes -= removed.payload_bytes;
                    }
                    Ok(None)
                }
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                if let Some(removed) = state.entries.remove(&name) {
                    state.total_bytes -= removed.payload_bytes;
                }
                Ok(None)
            }
            Err(_) => Err(RenderCacheError::new(
                RenderCacheErrorKind::ProviderUnavailable,
            )),
        }
    }

    async fn publish(
        &self,
        key: &RenderKey,
        bytes: Bytes,
        fence: PublicationFence,
        now_ms: u64,
    ) -> Result<PublishOutcome, RenderCacheError> {
        // Zero: a generic caller (this trait method's every caller besides
        // the middleware's L1 publish path, including this file's own
        // tests) has no policy in scope to compute a real retention from.
        // See [`Self::publish_with_retention`] and the module documentation.
        self.publish_framed(key, bytes, fence, now_ms, 0).await
    }

    async fn evict(&self, key: &RenderKey) -> Result<(), RenderCacheError> {
        let name = key.to_base64url();
        let path = self.path_for_name(&name);
        // Held across the removal and the tally update, the same way
        // `publish` holds it across its write and rename. Taking the lock
        // only after removing the file let a concurrent publish for this
        // key complete its own locked section (write, rename, tally
        // insert) in between: this method would then take the lock and
        // remove that fresh tally entry, leaving a file genuinely on disk
        // that the tally says is absent, forever exempt from the byte
        // bound.
        let mut state = self.state.lock().await;
        match tokio::fs::remove_file(&path).await {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(RenderCacheError::new(
                    RenderCacheErrorKind::ProviderUnavailable,
                ));
            }
        }
        if let Some(removed) = state.entries.remove(&name) {
            state.total_bytes -= removed.payload_bytes;
        }
        Ok(())
    }

    async fn inspect(&self) -> Result<StoreInspection, RenderCacheError> {
        let state = self.state.lock().await;
        Ok(StoreInspection {
            entries: state.entries.len(),
            bytes: usize::try_from(state.total_bytes).unwrap_or(usize::MAX),
        })
    }
}

impl FileRenderStore {
    /// Publishes exactly like [`RenderStore::publish`], but frames a real
    /// `stale_on_error_ms` (total milliseconds after `now_ms` beyond which
    /// the entry is dead - see the module documentation) so [`Self::sweep`]
    /// can later remove it once its retention window elapses. The
    /// middleware's L1 publish path is the only caller with a route policy
    /// in scope to compute this from
    /// (`entry.header().fresh_ms + entry.header().stale_on_error_ms`);
    /// every other caller, including this file's own tests, uses the
    /// generic [`RenderStore::publish`], which always frames zero.
    pub async fn publish_with_retention(
        &self,
        key: &RenderKey,
        bytes: Bytes,
        fence: PublicationFence,
        now_ms: u64,
        stale_on_error_ms: u64,
    ) -> Result<PublishOutcome, RenderCacheError> {
        self.publish_framed(key, bytes, fence, now_ms, stale_on_error_ms)
            .await
    }

    /// Removes entries that are dead by retention
    /// (`published_at_ms + stale_on_error_ms` has elapsed at `now_ms`) or
    /// dead by epoch (the entry's fence epoch is below `epoch`, the current
    /// ledger epoch). Returns how many files were removed.
    ///
    /// Disk hygiene, not a correctness gate: [`RenderStore::get`]
    /// independently re-evaluates freshness on every read, so an entry
    /// sweep has not yet reached is never served past what the
    /// application's own freshness policy allows - see the module
    /// documentation.
    ///
    /// Holds the tally lock across every removal and its matching tally
    /// update, one entry at a time, the same discipline
    /// [`RenderStore::publish`] and [`RenderStore::evict`] use: a file is
    /// removed from disk, and only then - and only if that removal
    /// actually succeeded or the file was already gone - is its tally
    /// entry dropped. A removal that fails for a real reason leaves that
    /// entry tracked and counted, exactly like `publish`'s own eviction
    /// loop and `evict`, rather than letting the tally and the directory
    /// disagree about what is still on disk.
    ///
    /// # Errors
    ///
    /// This method itself cannot fail; the `Result` matches the store's
    /// other operations and leaves room for a future provider-level
    /// failure without a signature change.
    pub async fn sweep(&self, now_ms: u64, epoch: u64) -> Result<usize, RenderCacheError> {
        let mut state = self.state.lock().await;
        let dead: Vec<String> = state
            .entries
            .iter()
            .filter(|(_, tracked)| {
                now_ms.saturating_sub(tracked.published_at_ms) >= tracked.stale_on_error_ms
                    || tracked.fence.epoch < epoch
            })
            .map(|(name, _)| name.clone())
            .collect();
        let mut removed = 0_usize;
        for name in dead {
            match tokio::fs::remove_file(self.path_for_name(&name)).await {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => continue,
            }
            if let Some(tracked) = state.entries.remove(&name) {
                state.total_bytes -= tracked.payload_bytes;
                removed += 1;
            }
        }
        Ok(removed)
    }

    async fn publish_framed(
        &self,
        key: &RenderKey,
        bytes: Bytes,
        fence: PublicationFence,
        now_ms: u64,
        stale_on_error_ms: u64,
    ) -> Result<PublishOutcome, RenderCacheError> {
        let payload_len = bytes.len() as u64;
        // The frame's entry-length field is a u32 regardless of how large
        // `max_bytes` is configured, so an entry that could never fit in
        // that field is rejected before anything else is touched, the same
        // as any other bound violation.
        let max_payload = self.max_bytes.min(u64::from(u32::MAX));
        // `payload_len > max_payload` alone is not enough: a store bounded
        // to zero bytes must reject an empty payload too, and `0 > 0` is
        // false, matching the in-process sibling's `max_bytes == 0` guard
        // in `crates/suprnova-live/src/render_cache/store.rs`.
        if self.max_bytes == 0 || payload_len > max_payload {
            return Ok(PublishOutcome::Rejected);
        }
        let name = key.to_base64url();
        let mut state = self.state.lock().await;
        if let Some(existing) = state.entries.get(&name)
            && !fence.supersedes(&existing.fence)
        {
            return Ok(PublishOutcome::Fenced);
        }
        // The entry being replaced (if any) is not removed from the tally
        // yet: its file is untouched on disk until the write below
        // succeeds, so the tally keeps counting it until then. A write
        // failure must leave both the disk and the tally exactly as they
        // were for this key.
        let existing_len = state
            .entries
            .get(&name)
            .map_or(0, |entry| entry.payload_bytes);
        if state.total_bytes - existing_len + payload_len > self.max_bytes {
            let mut others: Vec<(String, u64, u64)> = state
                .entries
                .iter()
                .filter(|(candidate, _)| **candidate != name)
                .map(|(candidate, entry)| {
                    (
                        candidate.clone(),
                        entry.published_at_ms,
                        entry.payload_bytes,
                    )
                })
                .collect();
            others.sort_by_key(|&(_, published_at_ms, _)| published_at_ms);

            // A cheap pre-check before touching anything: if evicting
            // every eligible candidate still could not free enough room,
            // no combination can, so decline immediately rather than
            // deleting real files only to discover the same thing
            // afterwards. `total_evictable` is exactly
            // `state.total_bytes - existing_len` by construction (it sums
            // every entry this method just excluded from that
            // subtraction), so this condition reduces to `payload_len >
            // self.max_bytes`, already ruled out by the guard at the top
            // of this method - unreachable today, but cheap, and it keeps
            // the "a bound too small for the entry regardless of what is
            // evicted" invariant explicit at the point where eviction
            // actually happens rather than resting on a proof tied to a
            // guard many lines away that could silently stop holding if
            // that guard's arithmetic ever changes.
            let total_evictable: u64 = others.iter().map(|&(_, _, bytes)| bytes).sum();
            if state.total_bytes - existing_len + payload_len - total_evictable > self.max_bytes {
                return Ok(PublishOutcome::Rejected);
            }

            for (victim, _, _) in others {
                if state.total_bytes - existing_len + payload_len <= self.max_bytes {
                    break;
                }
                // Mirrors `evict`: remove the file first, and drop the
                // tally entry only when the removal succeeded or the file
                // was already absent. A victim whose removal fails for a
                // real reason stays tracked and counted instead of being
                // written off as freed space that was never actually
                // given back - the same disk-versus-tally divergence
                // `evict` and `get` were fixed to avoid, reached a third
                // way through this loop. Move on to the next candidate
                // rather than proceeding as if room had been made.
                match tokio::fs::remove_file(self.path_for_name(&victim)).await {
                    Ok(()) => {}
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => continue,
                }
                if let Some(removed) = state.entries.remove(&victim) {
                    state.total_bytes -= removed.payload_bytes;
                }
            }
            if state.total_bytes - existing_len + payload_len > self.max_bytes {
                // The pre-check above found enough room theoretically
                // achievable, but eviction still could not free it: at
                // least one candidate's removal failed for a real reason
                // partway through the loop, after others had already been
                // genuinely evicted. This is a documented limitation, not
                // the disk-versus-tally divergence fixed elsewhere in this
                // file - the tally still accurately reflects what is
                // actually on disk, it is just smaller than it was a
                // moment ago. Rolling those evictions back, or staging
                // them through temporary names so they could be, is more
                // machinery than this tier's design calls for: a
                // filesystem error during eviction can leave the store
                // holding fewer entries than before while still declining
                // the publication that triggered it, which is a degraded
                // but consistent state rather than a divergence.
                return Ok(PublishOutcome::Rejected);
            }
        }
        let framed = encode_frame(&fence, now_ms, stale_on_error_ms, &bytes);
        let final_path = self.path_for_name(&name);
        let temp_path = self.temp_path_for(&name, fence.token);
        match write_frame_atomically(temp_path, final_path, framed).await {
            Ok(Published::Durable) => {}
            Ok(Published::RenamedWithoutDirectorySync(sync_error)) => {
                // The rename already landed: the frame is live and correct
                // on disk, so the publication succeeded. Only the
                // directory entry's own durability is in question, which
                // is a warning, not a failure - propagating an error here
                // instead of falling through to the tally update below
                // would leave the disk and the tally disagreeing about a
                // key that is genuinely present, exactly the divergence
                // `evict` and `get` were fixed to avoid on the race side.
                tracing::warn!(
                    target: "suprnova::render_cache",
                    error = %sync_error,
                    "render cache L1 directory sync failed after a successful rename; \
                     the entry is live but its directory entry may not survive a crash",
                );
            }
            Err(_) => {
                return Err(RenderCacheError::new(
                    RenderCacheErrorKind::ProviderUnavailable,
                ));
            }
        }
        state.total_bytes = state.total_bytes - existing_len + payload_len;
        state.entries.insert(
            name,
            TrackedEntry {
                fence,
                payload_bytes: payload_len,
                published_at_ms: now_ms,
                stale_on_error_ms,
            },
        );
        drop(state);
        // Every 256th publication (across both the generic path and the
        // retention-aware one) triggers a sweep using the epoch this very
        // publication was fenced under - `key_input` reads the ledger
        // epoch fresh on every dispatch (see `middleware.rs`'s own note on
        // this), so `fence.epoch` here is exactly "the current ledger
        // epoch" from this call's point of view. A publication must never
        // fail because its own housekeeping sweep did, so any error is
        // discarded; the lock above is already released, so this cannot
        // deadlock against `sweep`'s own locking.
        let count = self
            .publish_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        if count.is_multiple_of(256) {
            let _ = self.sweep(now_ms, fence.epoch).await;
        }
        Ok(PublishOutcome::Published)
    }
}

/// A decoded frame's fields, ready to become a [`StoredEntry`] (the caller
/// picks which fields the public contract carries forward).
struct DecodedFrame {
    fence: PublicationFence,
    published_at_ms: u64,
    /// Read by [`FileRenderStore::open`]'s directory scan into
    /// [`TrackedEntry::stale_on_error_ms`], which [`FileRenderStore::sweep`]
    /// then reads to decide whether an entry's retention window has
    /// elapsed.
    stale_on_error_ms: u64,
    payload: Bytes,
}

/// Encodes one frame. See the module documentation for the byte layout.
fn encode_frame(
    fence: &PublicationFence,
    published_at_ms: u64,
    stale_on_error_ms: u64,
    payload: &[u8],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(FRAME_HEADER_LEN + payload.len() + TRAILING_DIGEST_LEN);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&fence.epoch.to_be_bytes());
    out.extend_from_slice(&fence.token.to_be_bytes());
    out.extend_from_slice(&fence.generation_digest);
    out.extend_from_slice(&published_at_ms.to_be_bytes());
    out.extend_from_slice(&stale_on_error_ms.to_be_bytes());
    // `publish` rejects any payload above `u32::MAX` before this is ever
    // reached, so the length always fits.
    let entry_len = u32::try_from(payload.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&entry_len.to_be_bytes());
    out.extend_from_slice(payload);
    let digest: [u8; TRAILING_DIGEST_LEN] = Sha256::digest(&out).into();
    out.extend_from_slice(&digest);
    out
}

fn read_u64(bytes: &[u8], at: usize) -> Result<u64, RenderCacheError> {
    let invalid = || RenderCacheError::new(RenderCacheErrorKind::EntryInvalid);
    let slice = bytes.get(at..at + 8).ok_or_else(invalid)?;
    Ok(u64::from_be_bytes(slice.try_into().map_err(|_| invalid())?))
}

fn read_u32(bytes: &[u8], at: usize) -> Result<u32, RenderCacheError> {
    let invalid = || RenderCacheError::new(RenderCacheErrorKind::EntryInvalid);
    let slice = bytes.get(at..at + 4).ok_or_else(invalid)?;
    Ok(u32::from_be_bytes(slice.try_into().map_err(|_| invalid())?))
}

/// Decodes and verifies one frame; any structural or integrity defect is
/// [`RenderCacheErrorKind::EntryInvalid`], which every call site in this
/// module treats as a miss and a reason to remove the file.
fn decode_frame(bytes: &Bytes) -> Result<DecodedFrame, RenderCacheError> {
    let invalid = || RenderCacheError::new(RenderCacheErrorKind::EntryInvalid);
    if bytes.len() < FRAME_HEADER_LEN + TRAILING_DIGEST_LEN || &bytes[..4] != MAGIC {
        return Err(invalid());
    }
    let (payload_section, trailing_digest) = bytes.split_at(bytes.len() - TRAILING_DIGEST_LEN);
    let expected: [u8; TRAILING_DIGEST_LEN] = Sha256::digest(payload_section).into();
    if expected != trailing_digest {
        return Err(invalid());
    }
    let epoch = read_u64(bytes, 4)?;
    let token = read_u64(bytes, 12)?;
    let generation_digest: [u8; FENCE_DIGEST_LEN] = bytes
        .get(20..20 + FENCE_DIGEST_LEN)
        .ok_or_else(invalid)?
        .try_into()
        .map_err(|_| invalid())?;
    let published_at_ms = read_u64(bytes, 20 + FENCE_DIGEST_LEN)?;
    let stale_on_error_ms = read_u64(bytes, 28 + FENCE_DIGEST_LEN)?;
    let entry_len = read_u32(bytes, 36 + FENCE_DIGEST_LEN)? as usize;
    let payload_start = FRAME_HEADER_LEN;
    let payload_end = payload_start.checked_add(entry_len).ok_or_else(invalid)?;
    if payload_end != payload_section.len() {
        return Err(invalid());
    }
    Ok(DecodedFrame {
        fence: PublicationFence {
            epoch,
            token,
            generation_digest,
        },
        published_at_ms,
        stale_on_error_ms,
        payload: bytes.slice(payload_start..payload_end),
    })
}

/// Whether a call to [`write_frame_atomically`] left the frame durably
/// synced, or merely renamed into place.
enum Published {
    /// The frame is live at the target path, and its directory entry is
    /// itself synced: durable across a crash or power loss, not only a
    /// plain process kill.
    Durable,
    /// The frame is live and correct at the target path - the rename
    /// already succeeded - but the directory entry could not also be
    /// synced. This is a durability warning, not a failed publication: the
    /// entry may not survive a crash or power loss, but everything a
    /// reader observes right now is exactly as if the sync had succeeded.
    RenamedWithoutDirectorySync(std::io::Error),
}

/// Writes `framed` to `temp_path`, `fsync`s it, renames it over
/// `final_path`, then `fsync`s the parent directory. Runs on a blocking
/// thread: `File::create`, `write_all`, `sync_all`, `rename`, and opening
/// the directory are all synchronous syscalls.
///
/// The rename alone makes the swap atomic - a reader never sees a torn
/// file - but the directory entry it creates is not durable across a crash
/// or power loss without also syncing the directory: without this, only a
/// plain process kill (where the page cache survives) is covered. Mirrors
/// `atomic_write_evidence` in
/// `crates/suprnova-live/benches/upload_framework_budget.rs`, which opens
/// the parent directory and calls `sync_all` immediately after its own
/// rename for the same reason.
///
/// A failure is returned only when nothing durable has landed yet - the
/// temporary file's creation, its write, its own `fsync`, or the rename
/// itself. Once the rename succeeds, the frame is live and correct, so a
/// directory-open or directory-`fsync` failure after that point is
/// reported as [`Published::RenamedWithoutDirectorySync`] rather than an
/// error: the caller must update its bookkeeping either way, since the
/// disk already has the new content.
async fn write_frame_atomically(
    temp_path: PathBuf,
    final_path: PathBuf,
    framed: Vec<u8>,
) -> std::io::Result<Published> {
    match tokio::task::spawn_blocking(move || -> std::io::Result<Published> {
        let result = (|| -> std::io::Result<Published> {
            let mut file = std::fs::File::create(&temp_path)?;
            file.write_all(&framed)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temp_path, &final_path)?;
            // The rename has landed: everything from here on is a
            // durability question about the directory entry, not about
            // whether the publication itself succeeded.
            let synced_directory: std::io::Result<()> = (|| {
                let parent = final_path.parent().ok_or_else(|| {
                    std::io::Error::other("render cache entry path has no parent directory")
                })?;
                std::fs::OpenOptions::new()
                    .read(true)
                    .open(parent)?
                    .sync_all()
            })();
            Ok(match synced_directory {
                Ok(()) => Published::Durable,
                Err(sync_error) => Published::RenamedWithoutDirectorySync(sync_error),
            })
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temp_path);
        }
        result
    })
    .await
    {
        Ok(result) => result,
        Err(join_err) => Err(std::io::Error::other(join_err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_round_trip_preserves_every_field_including_stale_on_error_ms() {
        let fence = PublicationFence {
            epoch: 7,
            token: 42,
            generation_digest: [9_u8; 32],
        };
        let payload = b"a payload that is not empty".to_vec();
        let framed = encode_frame(&fence, 1_234_567, 60_000, &payload);
        let decoded = decode_frame(&Bytes::from(framed)).expect("a well-formed frame decodes");
        assert_eq!(decoded.fence, fence, "every fence field round-trips");
        assert_eq!(decoded.published_at_ms, 1_234_567);
        assert_eq!(
            decoded.stale_on_error_ms, 60_000,
            "stale_on_error_ms round-trips even though no reader consumes it yet"
        );
        assert_eq!(decoded.payload.as_ref(), payload.as_slice());
    }

    #[test]
    fn an_empty_payload_and_all_zero_fields_also_round_trip() {
        let fence = PublicationFence {
            epoch: 0,
            token: 0,
            generation_digest: [0_u8; 32],
        };
        let framed = encode_frame(&fence, 0, 0, b"");
        let decoded = decode_frame(&Bytes::from(framed)).expect("an empty payload still decodes");
        assert_eq!(decoded.payload.len(), 0);
    }

    #[test]
    fn a_flipped_byte_anywhere_in_the_frame_fails_the_integrity_check() {
        let fence = PublicationFence {
            epoch: 1,
            token: 1,
            generation_digest: [3_u8; 32],
        };
        let template = encode_frame(&fence, 1_000, 5_000, b"payload-bytes-here");
        for at in 0..template.len() {
            let mut framed = template.clone();
            framed[at] ^= 0xFF;
            assert!(
                decode_frame(&Bytes::from(framed)).is_err(),
                "flipping byte {at} of {} must fail the frame check",
                template.len()
            );
        }
    }

    #[test]
    fn wrong_magic_and_truncated_frames_are_rejected() {
        let fence = PublicationFence {
            epoch: 1,
            token: 1,
            generation_digest: [1_u8; 32],
        };
        let mut framed = encode_frame(&fence, 1, 1, b"x");
        framed[0] = b'X';
        assert!(decode_frame(&Bytes::from(framed)).is_err(), "wrong magic");
        assert!(
            decode_frame(&Bytes::from_static(b"short")).is_err(),
            "too short to hold a frame"
        );
    }
}
