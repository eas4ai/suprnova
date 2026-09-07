//! Database-backed [`LeaseStore`]: one row per key in
//! `suprnova_render_leases`, so exactly one node at a time leads the rebuild
//! of a key across every process pointed at the same database.
//!
//! # What the row holds
//!
//! | Column | Meaning |
//! |---|---|
//! | `render_key` | [`RenderKey::to_base64url`] (`rk1.` plus 43 base64url characters), the lookup key itself and never a second hash of it |
//! | `epoch` | the authority epoch the current or last tenure was taken at, recorded for inspection and never a condition of acquisition |
//! | `lease_id` | this row's own tenure counter: one on creation, one more on every takeover |
//! | `expires_at_ms` | store time when the current tenure ends; zero once released |
//! | `next_token` | the last publication token minted for this key |
//!
//! # One clock, and it is never this node's
//!
//! Every expiry decision - whether a lease may be taken over, whether a
//! token may still be minted - is a comparison against
//! [`sql_now_ms`] inlined into the statement that guards
//! the row. A node whose clock runs fast can neither extend its own lease
//! nor declare a peer's expired, which is the whole reason leadership lives
//! in the database rather than in a process.
//!
//! # Its own transaction, always
//!
//! Unlike [`SqlRenderStore`](super::SqlRenderStore), this store never joins
//! the caller's ambient transaction. A lease taken inside someone else's
//! transaction would become visible to other nodes only at that
//! transaction's commit - so two nodes could both believe they lead - and a
//! rollback would silently drop a lease a peer had already observed as held.
//! Leadership is not the host's to roll back, so each operation opens, and
//! closes, a short transaction of its own.
//!
//! # Release never deletes
//!
//! Releasing sets `expires_at_ms` to zero and leaves the row where it is,
//! because `next_token` has to outlive the tenure that minted from it. Were
//! the row deleted and recreated, tokens would restart at one, and a
//! fenced-out leader's already-minted token could outrank the publication
//! that replaced it - exactly the inversion the fence exists to prevent.

use async_trait::async_trait;
use sea_orm::{DbBackend, Value};
use suprnova_live::render_cache::RenderCacheError;
use suprnova_live::render_cache::key::RenderKey;
use suprnova_live::render_cache::{LeaseAttempt, LeaseStore};

use super::{as_i64, as_u64, bind, is_unique_violation, provider_error, row_lock, sql_now_ms};
use crate::database::transaction::ExecutorChoice;
use crate::{DB, FrameworkError, Transaction};

/// The table this store owns. Named once so the statements and the
/// collision classifier that reads the backend's message cannot drift apart.
const LEASES: &str = "suprnova_render_leases";

/// The lease id a key's first tenure is created with. Each takeover writes
/// one more than the row currently carries, so a tenure id is never reused
/// and a fenced-out holder is always detectable.
const FIRST_LEASE_ID: u64 = 1;

/// The token counter a key's row starts at, so the first mint returns one.
const FIRST_TOKEN: u64 = 0;

/// Database-backed rebuild lease store. See the module documentation for the
/// row layout, the clock, and why release keeps the row.
///
/// `Debug` prints the test offset only - there is no lease state in this
/// type, which is the point: the database holds it.
#[derive(Debug, Default)]
pub struct SqlLeaseStore {
    /// Milliseconds added to every store-time comparison, for tests alone.
    /// Per instance rather than process-wide: several stores share one test
    /// binary. See [`Self::set_time_offset_for_test`].
    time_offset_ms: std::sync::atomic::AtomicU64,
}

impl SqlLeaseStore {
    /// A store over the primary connection's `suprnova_render_leases` table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Moves this store's view of the database clock forward by `offset_ms`
    /// milliseconds, so a test can reach a lease expiry without waiting for
    /// one.
    ///
    /// Never called by production code, and deliberately not a process-wide
    /// switch: parallel tests in one binary would otherwise move each
    /// other's store clocks. Not part of the public contract: doc-hidden,
    /// the same shape as
    /// [`SqlRenderStore::set_time_offset_for_test`](super::SqlRenderStore::set_time_offset_for_test).
    #[doc(hidden)]
    pub fn set_time_offset_for_test(&self, offset_ms: u64) {
        self.time_offset_ms
            .store(offset_ms, std::sync::atomic::Ordering::Relaxed);
    }

    fn offset(&self) -> i64 {
        as_i64(
            self.time_offset_ms
                .load(std::sync::atomic::Ordering::Relaxed),
        )
    }

    /// The acquisition body, run on one executor already inside this
    /// operation's own transaction.
    ///
    /// The shape is a locked read, one guarded write, and a re-read that
    /// decides. The read locks the row where the dialect can lock one, so
    /// nothing changes it under this transaction; what it cannot do is lock
    /// a row that is not there, which is why an absent key is a plain
    /// `INSERT` whose unique key is the guard and whose violation is the
    /// answer.
    async fn acquire_through(
        &self,
        exec: &ExecutorChoice,
        key: &RenderKey,
        epoch: u64,
        ttl_ms: u64,
    ) -> Result<AcquireStep, RenderCacheError> {
        let backend = exec.backend();
        let name = key.to_base64url();
        let offset = self.offset();
        let ttl = as_i64(ttl_ms);

        let expected_lease_id = match read_lease(exec, backend, &name).await? {
            None => {
                match exec
                    .run(sea_orm::Statement::from_sql_and_values(
                        backend,
                        insert_lease_sql(backend).map_err(provider_error)?,
                        vec![
                            Value::from(name.clone()),
                            Value::from(as_i64(epoch)),
                            Value::from(offset),
                            Value::from(ttl),
                        ],
                    ))
                    .await
                {
                    Ok(_) => FIRST_LEASE_ID,
                    // Two nodes reached a brand-new key together. On
                    // PostgreSQL the read above locked nothing, because
                    // there was nothing to lock, so the unique key is what
                    // decides - and the node that lost it leads nothing.
                    Err(error) if is_unique_violation(&error.to_string(), LEASES) => {
                        return Ok(AcquireStep::Collided);
                    }
                    Err(error) => {
                        return Err(provider_error(FrameworkError::database(error.to_string())));
                    }
                }
            }
            Some(prior) => {
                exec.run(sea_orm::Statement::from_sql_and_values(
                    backend,
                    takeover_lease_sql(backend).map_err(provider_error)?,
                    vec![
                        Value::from(as_i64(epoch)),
                        Value::from(offset),
                        Value::from(ttl),
                        Value::from(name.clone()),
                        Value::from(offset),
                    ],
                ))
                .await
                .map_err(|error| provider_error(FrameworkError::database(error.to_string())))?;
                // A takeover writes exactly one more than the row carried
                // when this transaction locked it, so the re-read below can
                // tell "my write landed" from "the guard refused it" without
                // an affected-row count, whose meaning differs by dialect
                // and by driver flag. Saturating is unreachable - the column
                // is a signed 64-bit tenure counter - and reports a
                // held lease rather than panicking if it ever were not.
                prior.lease_id.saturating_add(1)
            }
        };

        let Some(held) = read_lease(exec, backend, &name).await? else {
            // The row was written in this very transaction, so it cannot be
            // absent: a missing row here is a broken store, not a lost
            // lease, and answering "held" would hide it.
            return Err(provider_error(FrameworkError::database(
                "render cache lease row is absent immediately after its own write".to_owned(),
            )));
        };
        if held.lease_id == expected_lease_id {
            Ok(AcquireStep::Won(LeaseAttempt::Acquired {
                lease_id: held.lease_id,
                expires_at_ms: held.expires_at_ms,
            }))
        } else {
            Ok(AcquireStep::Lost)
        }
    }
}

/// What one acquisition attempt did, and therefore how its transaction ends.
enum AcquireStep {
    /// The row is this caller's tenure now, and the transaction has to
    /// commit for any peer to see it.
    Won(LeaseAttempt),
    /// An unexpired lease holds the key. Nothing was written, so the
    /// transaction is rolled back.
    Lost,
    /// The insert collided with a peer's. Nothing of this caller's stands,
    /// and on PostgreSQL the transaction is aborted, so a rollback is the
    /// only way out of it.
    Collided,
}

/// One lease row's tenure and expiry.
struct LeaseRow {
    lease_id: u64,
    expires_at_ms: u64,
}

/// The tenure currently recorded for `name`, locked for the acquisition
/// about to replace it where the dialect can lock it.
async fn read_lease(
    exec: &ExecutorChoice,
    backend: DbBackend,
    name: &str,
) -> Result<Option<LeaseRow>, RenderCacheError> {
    let Some(row) = exec
        .query_one(sea_orm::Statement::from_sql_and_values(
            backend,
            select_lease_sql(backend).map_err(provider_error)?,
            vec![Value::from(name.to_owned())],
        ))
        .await
        .map_err(|error| provider_error(FrameworkError::database(error.to_string())))?
    else {
        return Ok(None);
    };
    let lease_id: i64 = row
        .try_get_by_index(0)
        .map_err(|error| provider_error(FrameworkError::database(error.to_string())))?;
    let expires_at_ms: i64 = row
        .try_get_by_index(1)
        .map_err(|error| provider_error(FrameworkError::database(error.to_string())))?;
    Ok(Some(LeaseRow {
        lease_id: as_u64(lease_id),
        expires_at_ms: as_u64(expires_at_ms),
    }))
}

/// Opens the short transaction one lease operation runs in.
///
/// Deliberately [`DB::begin_transaction`] rather than a resolved executor:
/// the resolver would hand back the host's ambient transaction, and this
/// store must never write a lease inside one. See the module documentation.
async fn own_transaction() -> Result<Transaction, RenderCacheError> {
    DB::begin_transaction().await.map_err(provider_error)
}

#[async_trait]
impl LeaseStore for SqlLeaseStore {
    async fn try_acquire(
        &self,
        key: &RenderKey,
        epoch: u64,
        ttl_ms: u64,
    ) -> Result<LeaseAttempt, RenderCacheError> {
        let tx = own_transaction().await?;
        let step = {
            // Dropped before the commit below: `Transaction::commit` unwraps
            // the shared handle and refuses while a clone is still alive.
            let exec = ExecutorChoice::from_tx(&tx);
            self.acquire_through(&exec, key, epoch, ttl_ms).await
        };
        match step {
            Ok(AcquireStep::Won(attempt)) => {
                tx.commit().await.map_err(provider_error)?;
                Ok(attempt)
            }
            Ok(AcquireStep::Lost | AcquireStep::Collided) => {
                // Nothing of this caller's may stand, and after a collision
                // a rollback is the only statement the backend will accept.
                let _ = tx.rollback().await;
                Ok(LeaseAttempt::Held)
            }
            Err(error) => {
                let _ = tx.rollback().await;
                Err(error)
            }
        }
    }

    async fn mint_token(
        &self,
        key: &RenderKey,
        lease_id: u64,
    ) -> Result<Option<u64>, RenderCacheError> {
        let tx = own_transaction().await?;
        let minted = {
            let exec = ExecutorChoice::from_tx(&tx);
            mint_through(&exec, key, lease_id, self.offset()).await
        };
        match minted {
            Ok(token) => {
                tx.commit().await.map_err(provider_error)?;
                Ok(token)
            }
            Err(error) => {
                let _ = tx.rollback().await;
                Err(error)
            }
        }
    }

    async fn release(&self, key: &RenderKey, lease_id: u64) -> Result<(), RenderCacheError> {
        let tx = own_transaction().await?;
        let released = {
            let exec = ExecutorChoice::from_tx(&tx);
            let backend = exec.backend();
            exec.run(sea_orm::Statement::from_sql_and_values(
                backend,
                release_lease_sql(backend).map_err(provider_error)?,
                vec![
                    Value::from(key.to_base64url()),
                    Value::from(as_i64(lease_id)),
                ],
            ))
            .await
            .map_err(|error| provider_error(FrameworkError::database(error.to_string())))
        };
        match released {
            Ok(_) => tx.commit().await.map_err(provider_error),
            Err(error) => {
                let _ = tx.rollback().await;
                Err(error)
            }
        }
    }
}

/// Increments the key's token counter while `lease_id` still holds it, and
/// reads back the value that increment produced.
///
/// The re-read carries the same guard as the update, so it answers `Some`
/// only for a caller whose write actually landed: a lease id that is not the
/// row's, or a tenure that store time has passed, reads nothing back and is
/// `None`. A tenure that elapses between the update and the re-read is
/// `None` too, and spends a token nobody uses - which costs nothing, because
/// the counter only has to be monotonic, and answering `None` for a lease
/// that has just expired is the truthful answer anyway.
async fn mint_through(
    exec: &ExecutorChoice,
    key: &RenderKey,
    lease_id: u64,
    offset: i64,
) -> Result<Option<u64>, RenderCacheError> {
    let backend = exec.backend();
    let name = key.to_base64url();
    let guard = vec![
        Value::from(name),
        Value::from(as_i64(lease_id)),
        Value::from(offset),
    ];
    exec.run(sea_orm::Statement::from_sql_and_values(
        backend,
        mint_token_sql(backend).map_err(provider_error)?,
        guard.clone(),
    ))
    .await
    .map_err(|error| provider_error(FrameworkError::database(error.to_string())))?;
    let Some(row) = exec
        .query_one(sea_orm::Statement::from_sql_and_values(
            backend,
            select_token_sql(backend).map_err(provider_error)?,
            guard,
        ))
        .await
        .map_err(|error| provider_error(FrameworkError::database(error.to_string())))?
    else {
        return Ok(None);
    };
    let token: i64 = row
        .try_get_by_index(0)
        .map_err(|error| provider_error(FrameworkError::database(error.to_string())))?;
    Ok(Some(as_u64(token)))
}

/// `SELECT` for one key's tenure, locked for the acquisition that is about
/// to replace it where the dialect can lock it. SQLite has no `FOR UPDATE`
/// and needs none: it serialises writers.
fn select_lease_sql(backend: DbBackend) -> Result<String, FrameworkError> {
    Ok(format!(
        "SELECT lease_id, expires_at_ms FROM {LEASES} WHERE render_key = {}{}",
        bind(backend, 1)?,
        row_lock(backend)
    ))
}

/// `INSERT` for a key that has never been leased: the first tenure, a token
/// counter that has minted nothing, and an expiry measured on the database's
/// clock.
///
/// Deliberately not an upsert. A conflict here means a peer created the row
/// between this transaction's read and its write, and the answer to that is
/// that the peer leads - not that this caller should overwrite it. The
/// store-time expression is interpolated because it is one of three
/// constants chosen by a closed match on the backend, and the tenure and
/// token the first row starts at are this module's own constants; every
/// caller value is bound.
fn insert_lease_sql(backend: DbBackend) -> Result<String, FrameworkError> {
    let now = sql_now_ms(backend)?;
    Ok(format!(
        "INSERT INTO {LEASES} (render_key, epoch, lease_id, expires_at_ms, next_token) \
         VALUES ({}, {}, {FIRST_LEASE_ID}, ({now}) + {} + {}, {FIRST_TOKEN})",
        bind(backend, 1)?,
        bind(backend, 2)?,
        bind(backend, 3)?,
        bind(backend, 4)?
    ))
}

/// `UPDATE` that takes an expired tenure over, guarded by store time so a
/// live one is never preempted.
///
/// `lease_id = lease_id + 1` keeps the counter per row rather than per
/// process or per table: no sequence to share, no identity to coordinate,
/// and a tenure id that a takeover always advances. `next_token` is left
/// exactly where it is, which is what stops a new tenure from reissuing a
/// token an older one already published under.
fn takeover_lease_sql(backend: DbBackend) -> Result<String, FrameworkError> {
    let now = sql_now_ms(backend)?;
    Ok(format!(
        "UPDATE {LEASES} SET epoch = {}, lease_id = lease_id + 1, \
         expires_at_ms = ({now}) + {} + {} \
         WHERE render_key = {} AND expires_at_ms <= ({now}) + {}",
        bind(backend, 1)?,
        bind(backend, 2)?,
        bind(backend, 3)?,
        bind(backend, 4)?,
        bind(backend, 5)?
    ))
}

/// `UPDATE` that mints the next token, guarded by the caller's tenure and by
/// store time.
fn mint_token_sql(backend: DbBackend) -> Result<String, FrameworkError> {
    let now = sql_now_ms(backend)?;
    Ok(format!(
        "UPDATE {LEASES} SET next_token = next_token + 1 \
         WHERE render_key = {} AND lease_id = {} AND expires_at_ms > ({now}) + {}",
        bind(backend, 1)?,
        bind(backend, 2)?,
        bind(backend, 3)?
    ))
}

/// `SELECT` for the token just minted, under the same guard as the update
/// that minted it.
fn select_token_sql(backend: DbBackend) -> Result<String, FrameworkError> {
    let now = sql_now_ms(backend)?;
    Ok(format!(
        "SELECT next_token FROM {LEASES} \
         WHERE render_key = {} AND lease_id = {} AND expires_at_ms > ({now}) + {}",
        bind(backend, 1)?,
        bind(backend, 2)?,
        bind(backend, 3)?
    ))
}

/// `UPDATE` that ends the caller's tenure by expiring it, never a `DELETE`.
/// See the module documentation for why the row has to stay.
fn release_lease_sql(backend: DbBackend) -> Result<String, FrameworkError> {
    Ok(format!(
        "UPDATE {LEASES} SET expires_at_ms = 0 WHERE render_key = {} AND lease_id = {}",
        bind(backend, 1)?,
        bind(backend, 2)?
    ))
}

#[cfg(test)]
mod tests {
    //! The lease statements' shape per dialect.
    //!
    //! Behaviour is proven against a real database in
    //! `framework/tests/render_cache/tiers.rs`; what cannot be proven there
    //! on SQLite alone is that the Postgres and MySQL spellings say the same
    //! thing - the row-locking read, the store-time guards, and the counter
    //! arithmetic that carries this store's whole contract.
    use super::*;

    #[test]
    fn the_read_locks_the_row_on_every_dialect_that_can_lock_one() {
        assert!(
            select_lease_sql(DbBackend::Postgres)
                .expect("postgres")
                .ends_with("FOR UPDATE")
        );
        assert!(
            select_lease_sql(DbBackend::MySql)
                .expect("mysql")
                .ends_with("FOR UPDATE")
        );
        // SQLite serialises writers, so there is no lock to take and no
        // syntax for taking one.
        assert!(
            !select_lease_sql(DbBackend::Sqlite)
                .expect("sqlite")
                .contains("FOR UPDATE")
        );
    }

    #[test]
    fn the_first_tenure_is_inserted_and_never_upserted() {
        for backend in [DbBackend::Postgres, DbBackend::MySql, DbBackend::Sqlite] {
            let sql = insert_lease_sql(backend).expect("a supported dialect");
            assert!(
                sql.starts_with("INSERT INTO suprnova_render_leases"),
                "{sql}"
            );
            assert!(
                !sql.contains("CONFLICT") && !sql.contains("DUPLICATE") && !sql.contains("IGNORE"),
                "a collision means a peer leads, so the unique key must be allowed to raise: {sql}"
            );
            assert!(sql.contains("lease_id, expires_at_ms, next_token"), "{sql}");
        }
    }

    #[test]
    fn a_takeover_advances_the_tenure_and_keeps_the_token_counter() {
        for backend in [DbBackend::Postgres, DbBackend::MySql, DbBackend::Sqlite] {
            let sql = takeover_lease_sql(backend).expect("a supported dialect");
            assert!(sql.contains("lease_id = lease_id + 1"), "{sql}");
            assert!(
                !sql.contains("next_token"),
                "a takeover must not touch the token counter: {sql}"
            );
            assert!(
                sql.contains(&format!(
                    "expires_at_ms <= ({})",
                    sql_now_ms(backend).expect("now")
                )),
                "the takeover guard is the database's clock: {sql}"
            );
        }
    }

    #[test]
    fn minting_and_its_confirmation_carry_the_same_guard() {
        for backend in [DbBackend::Postgres, DbBackend::MySql, DbBackend::Sqlite] {
            let now = sql_now_ms(backend).expect("now");
            let guard = format!("expires_at_ms > ({now})");
            let update = mint_token_sql(backend).expect("a supported dialect");
            let confirm = select_token_sql(backend).expect("a supported dialect");
            assert!(update.contains("next_token = next_token + 1"), "{update}");
            assert!(update.contains(&guard), "{update}");
            assert!(
                confirm.contains(&guard),
                "the confirming read must not answer for a tenure the update refused: {confirm}"
            );
        }
    }

    #[test]
    fn release_expires_the_row_rather_than_deleting_it() {
        for backend in [DbBackend::Postgres, DbBackend::MySql, DbBackend::Sqlite] {
            let sql = release_lease_sql(backend).expect("a supported dialect");
            assert!(sql.starts_with("UPDATE suprnova_render_leases"), "{sql}");
            assert!(sql.contains("SET expires_at_ms = 0"), "{sql}");
            assert!(
                !sql.contains("DELETE"),
                "the token counter has to outlive the tenure: {sql}"
            );
        }
    }
}
