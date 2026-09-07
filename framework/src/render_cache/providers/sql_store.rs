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
//! A publication reads the stored `(epoch, token)` and writes in one
//! transaction, `FOR UPDATE` on the dialects that have it, so
//! [`PublicationFence::supersedes`] decides every replacement: a fence that
//! does not supersede the stored one is [`PublishOutcome::Fenced`] and the
//! row is left exactly as it was.
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
    /// transaction: read the stored fence, compare, then upsert.
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
        let name = key.to_base64url();
        let stored = exec
            .query_one(sea_orm::Statement::from_sql_and_values(
                backend,
                select_fence_sql(backend).map_err(provider_error)?,
                vec![Value::from(name.clone())],
            ))
            .await
            .map_err(|error| provider_error(FrameworkError::database(error.to_string())))?;
        if let Some(row) = stored {
            let epoch: i64 = row
                .try_get_by_index(0)
                .map_err(|error| provider_error(FrameworkError::database(error.to_string())))?;
            let token: i64 = row
                .try_get_by_index(1)
                .map_err(|error| provider_error(FrameworkError::database(error.to_string())))?;
            let current = PublicationFence {
                epoch: as_u64(epoch),
                // Never read for the comparison: `supersedes` comes down to
                // the epoch and the token alone, so the stored digest is
                // not fetched here at all.
                generation_digest: [0; 32],
                token: as_u64(token),
            };
            if !fence.supersedes(&current) {
                return Ok(PublishOutcome::Fenced);
            }
        }
        // Read on this executor, so it is the database's clock, taken
        // inside the same transaction as the write it bounds.
        let now = store_now_ms(exec, backend, self.offset()).await?;
        exec.run(sea_orm::Statement::from_sql_and_values(
            backend,
            upsert_sql(backend).map_err(provider_error)?,
            vec![
                Value::from(name),
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
        Ok(PublishOutcome::Published)
    }
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

/// The per-backend insert-or-replace for one entry. The caller has already
/// decided the stored fence is superseded, so the conflict branch replaces
/// unconditionally.
///
/// MySQL keeps the `VALUES(col)` spelling rather than the newer row alias:
/// the alias form needs MySQL 8.0.19 and MariaDB does not have it at all,
/// and this framework supports both.
fn upsert_sql(backend: DbBackend) -> Result<&'static str, FrameworkError> {
    match backend {
        DbBackend::Postgres => Ok("INSERT INTO suprnova_render_entries \
             (render_key, bytes, epoch, token, generation_digest, published_at_ms, expires_at_ms) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (render_key) DO UPDATE SET bytes = EXCLUDED.bytes, \
             epoch = EXCLUDED.epoch, token = EXCLUDED.token, \
             generation_digest = EXCLUDED.generation_digest, \
             published_at_ms = EXCLUDED.published_at_ms, \
             expires_at_ms = EXCLUDED.expires_at_ms"),
        DbBackend::Sqlite => Ok("INSERT INTO suprnova_render_entries \
             (render_key, bytes, epoch, token, generation_digest, published_at_ms, expires_at_ms) \
             VALUES (?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (render_key) DO UPDATE SET bytes = excluded.bytes, \
             epoch = excluded.epoch, token = excluded.token, \
             generation_digest = excluded.generation_digest, \
             published_at_ms = excluded.published_at_ms, \
             expires_at_ms = excluded.expires_at_ms"),
        DbBackend::MySql => Ok("INSERT INTO suprnova_render_entries \
             (render_key, bytes, epoch, token, generation_digest, published_at_ms, expires_at_ms) \
             VALUES (?, ?, ?, ?, ?, ?, ?) \
             ON DUPLICATE KEY UPDATE bytes = VALUES(bytes), epoch = VALUES(epoch), \
             token = VALUES(token), generation_digest = VALUES(generation_digest), \
             published_at_ms = VALUES(published_at_ms), expires_at_ms = VALUES(expires_at_ms)"),
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
