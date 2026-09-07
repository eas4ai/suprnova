//! Database-backed L1 [`RenderStore`]: one row per key in
//! `suprnova_render_entries`, shared by every node pointed at the same
//! database.
//!
//! # What the row holds
//!
//! | Column | Meaning |
//! |---|---|
//! | `render_key` | [`RenderKey::to_base64url`] (`rk1.` plus 43 base64url characters), the lookup key itself and never a second hash of it |
//! | `bytes` | the encoded entry, opaque here: its integrity is the codec's to check |
//! | `epoch`, `token`, `generation_digest` | the [`PublicationFence`] the row was published under |
//! | `published_at_ms` | the publisher's own publication instant, carried back in [`StoredEntry`] |
//! | `expires_at_ms` | store time at publication plus the caller's `retention_ms` |
//!
//! # One clock
//!
//! `expires_at_ms` is written from the *database's* clock
//! ([`store_now_ms`], read inside the publishing transaction) and compared
//! against the database's clock inside the reading statement
//! ([`sql_now_ms`], inlined). A node whose clock runs fast can therefore
//! neither hide a live entry from its peers nor keep a dead one alive for
//! them. `retention_ms` is the caller's own Dead edge, exactly as
//! [`RenderStore::publish`] documents it; `u64::MAX` means "never
//! age-swept" and saturates at the largest storable instant rather than
//! wrapping into the past.
//!
//! # Fencing
//!
//! [`PublicationFence::supersedes`] decides every replacement, and it
//! decides it twice. A publication first reads the stored `(epoch, token)`
//! in its own transaction, `FOR UPDATE` on the dialects that have it, and
//! answers [`PublishOutcome::Fenced`] without writing when that row already
//! supersedes it. Then the upsert itself carries the same comparison into
//! SQL (`ON CONFLICT ... DO UPDATE ... WHERE`, or `IF(...)` per column on
//! MySQL), because the read cannot lock a row that does not exist yet: two
//! nodes publishing the same brand-new key can both find it absent, and
//! without the guard in the write the loser's conflict branch would
//! overwrite the winner. The same transaction then re-reads `(epoch,
//! token)` to see which fence holds the key, so the outcome is a stored
//! fact rather than a dialect-specific affected-row count. Either way, a
//! fence that does not supersede leaves the row exactly as it was.
//!
//! # Bounds
//!
//! `max_bytes` bounds one entry, checked before any statement runs, and
//! reclamation is bounded too: [`SqlRenderStore::sweep`] removes at most
//! `batch` expired rows per call and reports whether more remain. Unlike
//! the file-backed tier, this store keeps no in-memory tally and evicts
//! nothing to make room: the table is shared by every node, so no single
//! process can hold an accurate picture of it, and growth is bounded by
//! retention rather than by eviction.

use async_trait::async_trait;
use bytes::Bytes;
use sea_orm::{DbBackend, Value};
use suprnova_live::render_cache::RenderCacheError;
use suprnova_live::render_cache::key::RenderKey;
use suprnova_live::render_cache::store::{
    PublicationFence, PublishOutcome, RenderStore, StoreInspection, StoredEntry,
};

use super::{provider_error, sql_now_ms, store_now_ms};
use crate::database::transaction::ExecutorChoice;
use crate::render_cache::SweepOutcome;
use crate::{DB, FrameworkError, PRIMARY_CONNECTION_NAME, Transaction};

/// Expired rows removed by one [`super::super::RenderCache::sweep`] call
/// against this store. Bounded for the same reason the file tier's own
/// limit is: sweeping is housekeeping, and one call must never turn into a
/// table-sized delete.
pub(crate) const SWEEP_BATCH: usize = 64;

/// Database-backed L1 store. See the module documentation for the row
/// layout, the clock, and the fence.
///
/// `Debug` prints the bound and the test offset only - there is no entry
/// state in this type to leak.
#[derive(Debug)]
pub struct SqlRenderStore {
    max_bytes: u64,
    /// Milliseconds added to every store-time read, for tests alone. Per
    /// instance rather than process-wide: several stores share one test
    /// binary. See [`Self::set_time_offset_for_test`].
    time_offset_ms: std::sync::atomic::AtomicU64,
}

impl SqlRenderStore {
    /// A store over the primary connection's `suprnova_render_entries`
    /// table, refusing any single entry larger than `max_bytes`.
    #[must_use]
    pub fn new(max_bytes: u64) -> Self {
        Self {
            max_bytes,
            time_offset_ms: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Moves this store's view of the database clock forward by `offset_ms`
    /// milliseconds, so a test can reach an expiry without waiting for one.
    ///
    /// Never called by production code, and deliberately not a process-wide
    /// switch: parallel tests in one binary would otherwise move each
    /// other's store clocks. Not part of the public contract: doc-hidden,
    /// the same shape as
    /// [`mark_installed`](super::super::mark_installed).
    #[doc(hidden)]
    pub fn set_time_offset_for_test(&self, offset_ms: u64) {
        self.time_offset_ms
            .store(offset_ms, std::sync::atomic::Ordering::Relaxed);
    }

    fn offset(&self) -> u64 {
        self.time_offset_ms
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Removes at most `batch` rows whose `expires_at_ms` has passed by
    /// store time, oldest first, and reports whether any expired row
    /// remains afterwards.
    ///
    /// Bounded by intent: this table is shared, an operator may run this on
    /// a live system, and one call must stay a small, predictable delete
    /// rather than growing with however large L1 has become. A backlog
    /// larger than `batch` drains across calls - the caller checks
    /// `more_remain`. Reclamation is hygiene, never a correctness gate:
    /// [`RenderStore::get`] already refuses a row past its expiry whether
    /// or not a sweep has reached it.
    ///
    /// # Errors
    ///
    /// Returns [`RenderCacheError`] with the provider-unavailable kind when
    /// the backend is unsupported or the delete fails.
    pub async fn sweep(&self, batch: usize) -> Result<SweepOutcome, RenderCacheError> {
        let exec = ExecutorChoice::resolve_write(None, Some(PRIMARY_CONNECTION_NAME), None)
            .await
            .map_err(provider_error)?;
        let backend = exec.backend();
        let offset = as_i64(self.offset());
        let limit = i64::try_from(batch).unwrap_or(i64::MAX);
        let removed = exec
            .run(sea_orm::Statement::from_sql_and_values(
                backend,
                delete_expired_sql(backend).map_err(provider_error)?,
                vec![Value::from(offset), Value::from(limit)],
            ))
            .await
            .map_err(|error| provider_error(FrameworkError::database(error.to_string())))?
            .rows_affected();
        let more_remain = exec
            .query_one(sea_orm::Statement::from_sql_and_values(
                backend,
                select_expired_sql(backend).map_err(provider_error)?,
                vec![Value::from(offset)],
            ))
            .await
            .map_err(|error| provider_error(FrameworkError::database(error.to_string())))?
            .is_some();
        Ok(SweepOutcome {
            removed: usize::try_from(removed).unwrap_or(usize::MAX),
            more_remain,
        })
    }

    /// The publication body, run on one executor that is already inside a
    /// transaction: read the stored fence, compare, then run the guarded
    /// upsert.
    ///
    /// The read is the cheap path, not the guard. It answers `Fenced`
    /// without writing anything when a stored row already supersedes this
    /// publication, and on MySQL and SQLite its `FOR UPDATE` (or the
    /// engine's own write serialisation) keeps two publishers for an
    /// existing key in line. What it cannot do is lock a row that is not
    /// there, so [`Self::upsert_fenced`] carries the same comparison into
    /// the write itself.
    async fn publish_through(
        &self,
        exec: &ExecutorChoice,
        key: &RenderKey,
        bytes: Bytes,
        fence: PublicationFence,
        now_ms: u64,
        retention_ms: u64,
    ) -> Result<PublishOutcome, RenderCacheError> {
        let backend = exec.backend();
        if let Some(current) = read_fence(exec, backend, &key.to_base64url()).await?
            && !fence.supersedes(&current)
        {
            return Ok(PublishOutcome::Fenced);
        }
        self.upsert_fenced(exec, key, bytes, fence, now_ms, retention_ms)
            .await
    }

    /// The guarded write and its confirmation, with no read-compare in
    /// front of it: the conflict branch of [`upsert_sql`] replaces the
    /// stored row only when this fence supersedes it, and the same
    /// transaction then re-reads `(epoch, token)` to see which fence
    /// actually holds the key.
    ///
    /// The confirmation is a re-read rather than an affected-row count on
    /// purpose: what "affected" means differs between dialects and even by
    /// driver flag (MySQL reports 2 for a row it changed, 0 for one it did
    /// not, and 0 again for an update whose values matched), while the
    /// stored fence is the fact the caller actually needs. Equal to this
    /// publication's fence means it stands; anything else means another
    /// publication holds the key and this one is
    /// [`PublishOutcome::Fenced`].
    async fn upsert_fenced(
        &self,
        exec: &ExecutorChoice,
        key: &RenderKey,
        bytes: Bytes,
        fence: PublicationFence,
        now_ms: u64,
        retention_ms: u64,
    ) -> Result<PublishOutcome, RenderCacheError> {
        let backend = exec.backend();
        let name = key.to_base64url();
        // Read on this executor, so it is the database's clock, taken
        // inside the same transaction as the write it bounds.
        let now = store_now_ms(exec, backend, self.offset()).await?;
        exec.run(sea_orm::Statement::from_sql_and_values(
            backend,
            upsert_sql(backend).map_err(provider_error)?,
            vec![
                Value::from(name.clone()),
                Value::from(bytes.to_vec()),
                Value::from(as_i64(fence.epoch)),
                Value::from(as_i64(fence.token)),
                Value::from(hex::encode(fence.generation_digest)),
                Value::from(as_i64(now_ms)),
                Value::from(as_i64(now.saturating_add(retention_ms))),
            ],
        ))
        .await
        .map_err(|error| provider_error(FrameworkError::database(error.to_string())))?;
        let held = read_fence(exec, backend, &name).await?.ok_or_else(|| {
            // The row was written in this very transaction, so it cannot be
            // absent: a missing row here is a broken store, not a lost
            // fence, and saying "fenced" would hide it.
            provider_error(FrameworkError::database(
                "render cache L1 row is absent immediately after its own upsert".to_owned(),
            ))
        })?;
        if held.epoch == fence.epoch && held.token == fence.token {
            Ok(PublishOutcome::Published)
        } else {
            Ok(PublishOutcome::Fenced)
        }
    }
}

/// The `(epoch, token)` currently stored for `name`, locked for the
/// publication about to replace it where the dialect can lock it.
///
/// The returned fence's digest is a placeholder: nothing this function
/// serves reads it. [`PublicationFence::supersedes`] compares the epoch and
/// the token alone, and the digest column is fetched only by
/// [`RenderStore::get`], which needs the real one.
async fn read_fence(
    exec: &ExecutorChoice,
    backend: DbBackend,
    name: &str,
) -> Result<Option<PublicationFence>, RenderCacheError> {
    let Some(row) = exec
        .query_one(sea_orm::Statement::from_sql_and_values(
            backend,
            select_fence_sql(backend).map_err(provider_error)?,
            vec![Value::from(name.to_owned())],
        ))
        .await
        .map_err(|error| provider_error(FrameworkError::database(error.to_string())))?
    else {
        return Ok(None);
    };
    let epoch: i64 = row
        .try_get_by_index(0)
        .map_err(|error| provider_error(FrameworkError::database(error.to_string())))?;
    let token: i64 = row
        .try_get_by_index(1)
        .map_err(|error| provider_error(FrameworkError::database(error.to_string())))?;
    Ok(Some(PublicationFence {
        epoch: as_u64(epoch),
        generation_digest: [0; 32],
        token: as_u64(token),
    }))
}

#[async_trait]
impl RenderStore for SqlRenderStore {
    async fn get(&self, key: &RenderKey) -> Result<Option<StoredEntry>, RenderCacheError> {
        // Pinned to the primary for the same reason the generation ledger
        // is: a replica lagging behind the publication that just landed
        // would report a miss for an entry the primary already holds, and
        // - worse - could still hold a row a takeover has already
        // replaced.
        let exec = ExecutorChoice::resolve_read(None, Some(PRIMARY_CONNECTION_NAME), None)
            .await
            .map_err(provider_error)?;
        let backend = exec.backend();
        let Some(row) = exec
            .query_one(sea_orm::Statement::from_sql_and_values(
                backend,
                select_entry_sql(backend).map_err(provider_error)?,
                vec![
                    Value::from(key.to_base64url()),
                    Value::from(as_i64(self.offset())),
                ],
            ))
            .await
            .map_err(|error| provider_error(FrameworkError::database(error.to_string())))?
        else {
            return Ok(None);
        };
        let bytes: Vec<u8> = row
            .try_get_by_index(0)
            .map_err(|error| provider_error(FrameworkError::database(error.to_string())))?;
        let epoch: i64 = row
            .try_get_by_index(1)
            .map_err(|error| provider_error(FrameworkError::database(error.to_string())))?;
        let token: i64 = row
            .try_get_by_index(2)
            .map_err(|error| provider_error(FrameworkError::database(error.to_string())))?;
        let digest_hex: String = row
            .try_get_by_index(3)
            .map_err(|error| provider_error(FrameworkError::database(error.to_string())))?;
        let published_at_ms: i64 = row
            .try_get_by_index(4)
            .map_err(|error| provider_error(FrameworkError::database(error.to_string())))?;
        let Some(generation_digest) = decode_digest(&digest_hex) else {
            // A fence digest that is not 64 hex characters is a row this
            // build cannot reconstruct a fence from, so there is nothing to
            // serve: a miss, exactly like the file tier's own treatment of
            // a frame it cannot decode. Sweeping it is retention's job.
            tracing::warn!(
                target: "suprnova::render_cache",
                "render cache L1 row carries an unreadable fence digest and is treated as a miss",
            );
            return Ok(None);
        };
        Ok(Some(StoredEntry {
            bytes: Bytes::from(bytes),
            published_at_ms: as_u64(published_at_ms),
            fence: PublicationFence {
                epoch: as_u64(epoch),
                generation_digest,
                token: as_u64(token),
            },
        }))
    }

    async fn publish(
        &self,
        key: &RenderKey,
        bytes: Bytes,
        fence: PublicationFence,
        now_ms: u64,
        retention_ms: u64,
    ) -> Result<PublishOutcome, RenderCacheError> {
        // Bounds before allocation: an entry over the bound is refused
        // before a statement is built, a transaction is opened, or its
        // bytes are copied for binding. A store bounded to zero bytes holds
        // nothing at all, matching both sibling stores (`MemoryRenderStore`
        // and `FileRenderStore` both special-case it, since `0 > 0` is
        // false).
        if self.max_bytes == 0 || bytes.len() as u64 > self.max_bytes {
            return Ok(PublishOutcome::Rejected);
        }
        // The read and the write have to see the same row, so they run in
        // one transaction. The caller's own transaction is that transaction
        // when there is one: opening a second would take a second pooled
        // connection - a deadlock on a single-connection database - and
        // would commit independently of the write the caller is publishing
        // alongside.
        if let Some(tx) = Transaction::current() {
            return self
                .publish_through(
                    &ExecutorChoice::from_tx(&tx),
                    key,
                    bytes,
                    fence,
                    now_ms,
                    retention_ms,
                )
                .await;
        }
        let tx = DB::begin_transaction().await.map_err(provider_error)?;
        let outcome = {
            // Dropped before the commit below: `Transaction::commit` unwraps
            // the shared handle and refuses while a clone is still alive.
            let exec = ExecutorChoice::from_tx(&tx);
            self.publish_through(&exec, key, bytes, fence, now_ms, retention_ms)
                .await
        };
        match outcome {
            Ok(outcome) => {
                tx.commit().await.map_err(provider_error)?;
                Ok(outcome)
            }
            Err(error) => {
                // The publication failed; nothing it wrote may stand.
                let _ = tx.rollback().await;
                Err(error)
            }
        }
    }

    async fn evict(&self, key: &RenderKey) -> Result<(), RenderCacheError> {
        let exec = ExecutorChoice::resolve_write(None, Some(PRIMARY_CONNECTION_NAME), None)
            .await
            .map_err(provider_error)?;
        let backend = exec.backend();
        exec.run(sea_orm::Statement::from_sql_and_values(
            backend,
            delete_entry_sql(backend).map_err(provider_error)?,
            vec![Value::from(key.to_base64url())],
        ))
        .await
        .map_err(|error| provider_error(FrameworkError::database(error.to_string())))?;
        Ok(())
    }

    async fn inspect(&self) -> Result<StoreInspection, RenderCacheError> {
        let exec = ExecutorChoice::resolve_read(None, Some(PRIMARY_CONNECTION_NAME), None)
            .await
            .map_err(provider_error)?;
        let backend = exec.backend();
        let row = exec
            .query_one(sea_orm::Statement::from_sql_and_values(
                backend,
                inspect_sql(backend).map_err(provider_error)?,
                vec![],
            ))
            .await
            .map_err(|error| provider_error(FrameworkError::database(error.to_string())))?
            .ok_or_else(|| {
                provider_error(FrameworkError::database(
                    "render cache L1 inspection returned no row".to_owned(),
                ))
            })?;
        let entries: i64 = row
            .try_get_by_index(0)
            .map_err(|error| provider_error(FrameworkError::database(error.to_string())))?;
        let bytes: i64 = row
            .try_get_by_index(1)
            .map_err(|error| provider_error(FrameworkError::database(error.to_string())))?;
        // Occupancy, so every row the table holds counts - including one
        // whose retention has passed and which no sweep has reached yet.
        // That is what an operator asking how large L1 has grown needs to
        // see.
        Ok(StoreInspection {
            entries: usize::try_from(entries).unwrap_or(usize::MAX),
            bytes: usize::try_from(bytes).unwrap_or(usize::MAX),
        })
    }
}

/// Milliseconds as the column stores them. Epochs, tokens, and instants
/// above `i64::MAX` are unreachable - epochs count emergency invalidations,
/// tokens count publications, and the instant is milliseconds since 1970 -
/// so clamping keeps the conversion total without a panic and without
/// wrapping a saturated `u64::MAX` retention into the distant past.
fn as_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// The inverse of [`as_i64`]. A negative column value cannot occur: every
/// writer here is [`as_i64`].
fn as_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

/// A 64-character lowercase hex fence digest, or `None` when the column
/// does not hold one.
fn decode_digest(text: &str) -> Option<[u8; 32]> {
    let bytes = hex::decode(text).ok()?;
    bytes.try_into().ok()
}

/// `SELECT` for one live entry: the row exists and its expiry has not
/// passed by store time.
///
/// The store-time expression is interpolated because it is one of three
/// constants chosen by a closed match on the backend - never a caller
/// value. Every caller value in this module is bound.
fn select_entry_sql(backend: DbBackend) -> Result<String, FrameworkError> {
    let now = sql_now_ms(backend)?;
    Ok(match backend {
        DbBackend::Postgres => format!(
            "SELECT bytes, epoch, token, generation_digest, published_at_ms \
             FROM suprnova_render_entries \
             WHERE render_key = $1 AND expires_at_ms > ({now}) + $2"
        ),
        _ => format!(
            "SELECT bytes, epoch, token, generation_digest, published_at_ms \
             FROM suprnova_render_entries \
             WHERE render_key = ? AND expires_at_ms > ({now}) + ?"
        ),
    })
}

/// `SELECT` for the stored fence, locked for the publication that is about
/// to replace it where the dialect can lock it. SQLite has no `FOR UPDATE`
/// and needs none: it serialises writers.
fn select_fence_sql(backend: DbBackend) -> Result<&'static str, FrameworkError> {
    match backend {
        DbBackend::Postgres => {
            Ok("SELECT epoch, token FROM suprnova_render_entries WHERE render_key = $1 FOR UPDATE")
        }
        DbBackend::MySql => {
            Ok("SELECT epoch, token FROM suprnova_render_entries WHERE render_key = ? FOR UPDATE")
        }
        DbBackend::Sqlite => {
            Ok("SELECT epoch, token FROM suprnova_render_entries WHERE render_key = ?")
        }
        _ => Err(crate::database::unsupported_database_backend(backend)),
    }
}

/// The per-backend insert-or-replace for one entry, with
/// [`PublicationFence::supersedes`]'s own comparison carried into the
/// conflict branch: the stored row is overwritten only when its
/// `(epoch, token)` is below the incoming one, and left untouched otherwise.
///
/// This condition is not redundant with the `FOR UPDATE` read that precedes
/// it. That read locks a row that exists; it locks nothing when the key is
/// absent, so two nodes publishing the same brand-new key can both read
/// "absent" and both insert - and with an unconditional conflict branch the
/// loser would then overwrite the winner, standing a lower fence up as
/// current. With the condition, the write itself is the fence, on every
/// dialect. The caller confirms which side won by re-reading `(epoch,
/// token)` in the same transaction rather than by an affected-row count,
/// which differs by dialect and by driver flag.
///
/// MySQL says the same thing with `IF(...)` per column, since
/// `ON DUPLICATE KEY UPDATE` takes no `WHERE`, and keeps the `VALUES(col)`
/// spelling rather than the newer row alias (the alias form needs MySQL
/// 8.0.19 and MariaDB does not have it at all, and this framework supports
/// both). The order of those assignments is load-bearing: MySQL applies them
/// left to right and a later expression sees a column already assigned, so
/// `token` and `epoch` come last, after every condition that reads their
/// stored values has been evaluated.
fn upsert_sql(backend: DbBackend) -> Result<&'static str, FrameworkError> {
    match backend {
        DbBackend::Postgres => Ok("INSERT INTO suprnova_render_entries \
             (render_key, bytes, epoch, token, generation_digest, published_at_ms, expires_at_ms) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (render_key) DO UPDATE SET bytes = EXCLUDED.bytes, \
             epoch = EXCLUDED.epoch, token = EXCLUDED.token, \
             generation_digest = EXCLUDED.generation_digest, \
             published_at_ms = EXCLUDED.published_at_ms, \
             expires_at_ms = EXCLUDED.expires_at_ms \
             WHERE suprnova_render_entries.epoch < EXCLUDED.epoch \
             OR (suprnova_render_entries.epoch = EXCLUDED.epoch \
             AND suprnova_render_entries.token < EXCLUDED.token)"),
        DbBackend::Sqlite => Ok("INSERT INTO suprnova_render_entries \
             (render_key, bytes, epoch, token, generation_digest, published_at_ms, expires_at_ms) \
             VALUES (?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (render_key) DO UPDATE SET bytes = excluded.bytes, \
             epoch = excluded.epoch, token = excluded.token, \
             generation_digest = excluded.generation_digest, \
             published_at_ms = excluded.published_at_ms, \
             expires_at_ms = excluded.expires_at_ms \
             WHERE suprnova_render_entries.epoch < excluded.epoch \
             OR (suprnova_render_entries.epoch = excluded.epoch \
             AND suprnova_render_entries.token < excluded.token)"),
        DbBackend::MySql => Ok("INSERT INTO suprnova_render_entries \
             (render_key, bytes, epoch, token, generation_digest, published_at_ms, expires_at_ms) \
             VALUES (?, ?, ?, ?, ?, ?, ?) \
             ON DUPLICATE KEY UPDATE \
             bytes = IF(epoch < VALUES(epoch) \
             OR (epoch = VALUES(epoch) AND token < VALUES(token)), VALUES(bytes), bytes), \
             generation_digest = IF(epoch < VALUES(epoch) \
             OR (epoch = VALUES(epoch) AND token < VALUES(token)), \
             VALUES(generation_digest), generation_digest), \
             published_at_ms = IF(epoch < VALUES(epoch) \
             OR (epoch = VALUES(epoch) AND token < VALUES(token)), \
             VALUES(published_at_ms), published_at_ms), \
             expires_at_ms = IF(epoch < VALUES(epoch) \
             OR (epoch = VALUES(epoch) AND token < VALUES(token)), \
             VALUES(expires_at_ms), expires_at_ms), \
             token = IF(epoch < VALUES(epoch) \
             OR (epoch = VALUES(epoch) AND token < VALUES(token)), VALUES(token), token), \
             epoch = IF(epoch < VALUES(epoch) \
             OR (epoch = VALUES(epoch) AND token < VALUES(token)), VALUES(epoch), epoch)"),
        _ => Err(crate::database::unsupported_database_backend(backend)),
    }
}

/// `DELETE` for one key.
fn delete_entry_sql(backend: DbBackend) -> Result<&'static str, FrameworkError> {
    match backend {
        DbBackend::Postgres => Ok("DELETE FROM suprnova_render_entries WHERE render_key = $1"),
        DbBackend::MySql | DbBackend::Sqlite => {
            Ok("DELETE FROM suprnova_render_entries WHERE render_key = ?")
        }
        _ => Err(crate::database::unsupported_database_backend(backend)),
    }
}

/// `DELETE` for at most one batch of expired rows, oldest expiry first.
///
/// The victims are selected through a derived table rather than a bare
/// subquery over the same table: MySQL refuses to read the table a `DELETE`
/// targets in a plain subquery (error 1093), and a derived table is the
/// portable way to say the same thing. `LIMIT` sits inside it because
/// neither Postgres nor a stock SQLite build accepts `LIMIT` on `DELETE`
/// itself.
fn delete_expired_sql(backend: DbBackend) -> Result<String, FrameworkError> {
    let now = sql_now_ms(backend)?;
    Ok(match backend {
        DbBackend::Postgres => format!(
            "DELETE FROM suprnova_render_entries WHERE render_key IN \
             (SELECT render_key FROM (SELECT render_key FROM suprnova_render_entries \
             WHERE expires_at_ms <= ({now}) + $1 ORDER BY expires_at_ms LIMIT $2) AS due)"
        ),
        _ => format!(
            "DELETE FROM suprnova_render_entries WHERE render_key IN \
             (SELECT render_key FROM (SELECT render_key FROM suprnova_render_entries \
             WHERE expires_at_ms <= ({now}) + ? ORDER BY expires_at_ms LIMIT ?) AS due)"
        ),
    })
}

/// Whether any expired row remains, asked as cheaply as the dialect allows.
fn select_expired_sql(backend: DbBackend) -> Result<String, FrameworkError> {
    let now = sql_now_ms(backend)?;
    Ok(match backend {
        DbBackend::Postgres => format!(
            "SELECT 1 FROM suprnova_render_entries WHERE expires_at_ms <= ({now}) + $1 LIMIT 1"
        ),
        _ => format!(
            "SELECT 1 FROM suprnova_render_entries WHERE expires_at_ms <= ({now}) + ? LIMIT 1"
        ),
    })
}

/// Row and byte occupancy.
///
/// The sum is cast per dialect because an aggregate's own type is not the
/// column's: MySQL's `SUM` returns `DECIMAL` and Postgres promotes a sum
/// past `int4`, neither of which decodes into `i64` on its own.
fn inspect_sql(backend: DbBackend) -> Result<&'static str, FrameworkError> {
    match backend {
        DbBackend::Postgres => Ok(
            "SELECT COUNT(*), COALESCE(SUM(LENGTH(bytes)), 0)::BIGINT FROM suprnova_render_entries",
        ),
        DbBackend::MySql => Ok(
            "SELECT COUNT(*), CAST(COALESCE(SUM(LENGTH(bytes)), 0) AS SIGNED) \
             FROM suprnova_render_entries",
        ),
        DbBackend::Sqlite => {
            Ok("SELECT COUNT(*), COALESCE(SUM(LENGTH(bytes)), 0) FROM suprnova_render_entries")
        }
        _ => Err(crate::database::unsupported_database_backend(backend)),
    }
}

#[cfg(test)]
mod tests {
    //! The publication fence as the database enforces it.
    //!
    //! [`SqlRenderStore::publish`]'s read-compare answers `Fenced` cheaply,
    //! but it cannot lock a row that does not exist yet, so the guard that
    //! actually has to hold is the one inside the upsert. These tests pin
    //! its shape on all three dialects and its behaviour on SQLite, calling
    //! [`SqlRenderStore::upsert_fenced`] directly - no read-compare in front
    //! of it, which is exactly the situation two nodes racing on a
    //! brand-new key produce.
    use super::*;
    use crate::container::testing::{TestContainer, TestContainerGuard};
    use crate::database::testing::TestDatabase;
    use sea_orm_migration::{MigrationTrait, MigratorTrait};
    use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
    use suprnova_live::identity::{KeyId, UnixMillis};

    struct Migrator;

    #[async_trait::async_trait]
    impl MigratorTrait for Migrator {
        fn migrations() -> Vec<Box<dyn MigrationTrait>> {
            vec![Box::new(crate::render_cache::migration::TierMigration)]
        }
    }

    fn keys() -> SnapshotKeyRing {
        let active = KeyRecord::new(
            KeyId::parse("render-cache-test").expect("key id"),
            RootKey::new(vec![4; 32]).expect("root key"),
            UnixMillis::new(0),
            UnixMillis::new(u64::MAX / 2),
            UnixMillis::new(u64::MAX),
        )
        .expect("key record");
        SnapshotKeyRing::new(active, Vec::new()).expect("key ring")
    }

    fn test_key() -> RenderKey {
        RenderKey::for_test(&keys(), "/guarded")
    }

    fn test_fence(epoch: u64, token: u64) -> PublicationFence {
        PublicationFence {
            epoch,
            generation_digest: [u8::try_from(token % 251).expect("a byte"); 32],
            token,
        }
    }

    async fn write_executor() -> ExecutorChoice {
        ExecutorChoice::resolve_write(None, Some(PRIMARY_CONNECTION_NAME), None)
            .await
            .expect("a write executor over the test database")
    }

    #[test]
    fn the_upsert_carries_the_fence_into_the_conflict_branch_on_every_dialect() {
        let postgres = upsert_sql(DbBackend::Postgres).expect("postgres");
        assert!(postgres.contains("VALUES ($1, $2, $3, $4, $5, $6, $7)"));
        assert!(
            postgres.contains(
                "ON CONFLICT (render_key) DO UPDATE SET bytes = EXCLUDED.bytes, \
                 epoch = EXCLUDED.epoch, token = EXCLUDED.token"
            ),
            "{postgres}"
        );
        assert!(
            postgres.contains(
                "WHERE suprnova_render_entries.epoch < EXCLUDED.epoch \
                 OR (suprnova_render_entries.epoch = EXCLUDED.epoch \
                 AND suprnova_render_entries.token < EXCLUDED.token)"
            ),
            "the conflict branch must replace only what this fence supersedes: {postgres}"
        );

        let sqlite = upsert_sql(DbBackend::Sqlite).expect("sqlite");
        assert!(sqlite.contains("VALUES (?, ?, ?, ?, ?, ?, ?)"));
        assert!(
            sqlite.contains(
                "WHERE suprnova_render_entries.epoch < excluded.epoch \
                 OR (suprnova_render_entries.epoch = excluded.epoch \
                 AND suprnova_render_entries.token < excluded.token)"
            ),
            "{sqlite}"
        );

        // MySQL has no `WHERE` on `ON DUPLICATE KEY UPDATE`, so every
        // written column carries the same condition through `IF(...)`.
        let mysql = upsert_sql(DbBackend::MySql).expect("mysql");
        assert!(mysql.contains("VALUES (?, ?, ?, ?, ?, ?, ?)"));
        assert_eq!(
            mysql.matches("IF(epoch < VALUES(epoch)").count(),
            6,
            "all six written columns must be guarded, not just some: {mysql}"
        );
        for column in [
            "bytes",
            "generation_digest",
            "published_at_ms",
            "expires_at_ms",
            "token",
            "epoch",
        ] {
            assert!(
                mysql.contains(&format!("{column} = IF(epoch < VALUES(epoch)")),
                "{column} is written unguarded: {mysql}"
            );
        }
        // Load-bearing order: MySQL applies these assignments left to right
        // and a later condition sees a column already assigned, so the two
        // columns every condition reads must be assigned last.
        let bytes_at = mysql.find("bytes = IF(").expect("bytes assignment");
        let token_at = mysql.find("token = IF(").expect("token assignment");
        let epoch_at = mysql.find("epoch = IF(").expect("epoch assignment");
        assert!(
            bytes_at < token_at && token_at < epoch_at,
            "epoch and token must be assigned after every condition that reads them: {mysql}"
        );
    }

    /// Seeds a row at `(2, 5)`, then proves the guard both ways with no
    /// read-compare in front of it: a lower fence changes nothing, a higher
    /// one replaces everything.
    async fn assert_the_guarded_upsert_refuses_a_lower_fence_and_takes_a_higher_one() {
        let store = SqlRenderStore::new(1024 * 1024);
        let key = test_key();
        let exec = write_executor().await;

        assert_eq!(
            store
                .upsert_fenced(
                    &exec,
                    &key,
                    Bytes::from_static(b"held"),
                    test_fence(2, 5),
                    1_000,
                    60_000
                )
                .await
                .expect("the first publication"),
            PublishOutcome::Published
        );

        assert_eq!(
            store
                .upsert_fenced(
                    &exec,
                    &key,
                    Bytes::from_static(b"lower"),
                    test_fence(2, 4),
                    9_000,
                    60_000
                )
                .await
                .expect("the lower publication"),
            PublishOutcome::Fenced,
            "the SQL guard is the only thing between this write and the row"
        );
        let held = store.get(&key).await.expect("get").expect("a hit");
        assert_eq!(held.bytes.as_ref(), b"held", "no column may have moved");
        assert_eq!(held.fence.epoch, 2);
        assert_eq!(held.fence.token, 5);
        assert_eq!(held.published_at_ms, 1_000);

        assert_eq!(
            store
                .upsert_fenced(
                    &exec,
                    &key,
                    Bytes::from_static(b"higher"),
                    test_fence(3, 1),
                    9_000,
                    60_000
                )
                .await
                .expect("the higher publication"),
            PublishOutcome::Published,
            "a superseding fence must still replace the row"
        );
        let taken = store.get(&key).await.expect("get").expect("a hit");
        assert_eq!(taken.bytes.as_ref(), b"higher");
        assert_eq!(taken.fence.epoch, 3);
        assert_eq!(taken.fence.token, 1);
        assert_eq!(taken.published_at_ms, 9_000);
    }

    #[tokio::test]
    async fn the_guarded_upsert_refuses_a_lower_fence_and_takes_a_higher_one() {
        let _db = TestDatabase::fresh::<Migrator>()
            .await
            .expect("the tier migration applies to a fresh SQLite database");
        assert_the_guarded_upsert_refuses_a_lower_fence_and_takes_a_higher_one().await;
    }

    // --- Live-DB tests (gated by `#[ignore]`) ---
    //
    // The guard exists for a race only a real server can have: on
    // PostgreSQL a `SELECT ... FOR UPDATE` for an absent key locks nothing,
    // so two publishers can both insert. These run the same direct upserts
    // there and on MySQL, whose `IF(...)` spelling of the condition is a
    // different statement altogether and cannot be proven by SQLite.
    //
    //   PG_TEST_URL=postgres://postgres:pw@127.0.0.1:55998/suprnova_test \
    //     cargo test -p suprnova --lib -- --ignored live_postgres
    //
    //   MYSQL_TEST_URL=mysql://root:pw@127.0.0.1:55997/suprnova_test \
    //     cargo test -p suprnova --lib -- --ignored live_mysql

    /// Drops the entries table if a prior failed run left it behind,
    /// applies the tier migration, and mounts the connection on the
    /// thread-local test container so `DB::*` resolves to it.
    async fn reset_and_migrate(url: &str) -> TestContainerGuard {
        use sea_orm::{ConnectOptions, ConnectionTrait};
        let mut options = ConnectOptions::new(url.to_owned());
        options
            .connect_timeout(std::time::Duration::from_secs(2))
            .acquire_timeout(std::time::Duration::from_secs(2));
        let conn = sea_orm::Database::connect(options)
            .await
            .expect("the live test database is not reachable - check the URL");
        for table in [
            "suprnova_render_entries",
            "suprnova_render_leases",
            "suprnova_live_instances",
            "suprnova_live_promotions",
        ] {
            let _ = conn
                .execute_raw(sea_orm::Statement::from_string(
                    conn.get_database_backend(),
                    format!("DROP TABLE IF EXISTS {table}"),
                ))
                .await;
        }
        let manager = sea_orm_migration::SchemaManager::new(&conn);
        crate::render_cache::migration::TierMigration
            .up(&manager)
            .await
            .expect("the tier migration applies to the live database");
        let guard = TestContainer::fake();
        TestContainer::singleton(crate::DbConnection::from_raw(conn));
        guard
    }

    #[tokio::test]
    #[ignore = "requires live Postgres; run with --ignored live_postgres"]
    async fn live_postgres_the_guarded_upsert_refuses_a_lower_fence_and_takes_a_higher_one() {
        let url = std::env::var("PG_TEST_URL").expect(
            "set PG_TEST_URL to a disposable Postgres - this test drops and recreates tables",
        );
        let _guard = reset_and_migrate(&url).await;
        assert_the_guarded_upsert_refuses_a_lower_fence_and_takes_a_higher_one().await;
    }

    #[tokio::test]
    #[ignore = "requires live MySQL; run with --ignored live_mysql"]
    async fn live_mysql_the_guarded_upsert_refuses_a_lower_fence_and_takes_a_higher_one() {
        let url = std::env::var("MYSQL_TEST_URL").expect(
            "set MYSQL_TEST_URL to a disposable MySQL - this test drops and recreates tables",
        );
        let _guard = reset_and_migrate(&url).await;
        assert_the_guarded_upsert_refuses_a_lower_fence_and_takes_a_higher_one().await;
    }
}
