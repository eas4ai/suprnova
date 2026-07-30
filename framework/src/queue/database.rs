//! SeaORM-backed queue driver. Portable across SQLite / MySQL / Postgres.
//!
//! On SQLite, uses transaction-level locking via `BEGIN` to serialize pop
//! attempts. On MySQL / Postgres, uses `SELECT ... FOR UPDATE SKIP LOCKED`.
//!
//! Every statement here is hand-written, so its placeholders are rendered
//! per backend by the crate-internal `database::placeholder` helpers —
//! Postgres rejects the `?` form outright, which made this whole driver a
//! parse error there.

use crate::database::placeholder::{placeholder, placeholder_list};
use crate::database::validate_identifier;
use crate::error::FrameworkError;
use crate::queue::driver::{QueueDriver, Reservation, ReservationToken};
use crate::queue::envelope::{DEFAULT_QUEUE, Envelope};
use async_trait::async_trait;
use chrono::Utc;
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement, TransactionTrait};
use std::time::Duration;
use uuid::Uuid;

/// SeaORM-backed [`QueueDriver`] that stores envelopes in a SQL `jobs`
/// table. Push, pop, ack, and nack are all transactional; visibility is
/// enforced via a `reserved_at` column the pop path conditionally
/// updates.
pub struct DatabaseQueueDriver {
    db: DatabaseConnection,
    table: String,
}

impl DatabaseQueueDriver {
    /// Construct a driver bound to the given connection and `jobs` table.
    ///
    /// The `table` argument is interpolated directly into every SQL
    /// statement (push/pop/ack/nack), so it MUST validate as a SQL
    /// identifier — operator-controlled env input doesn't excuse the
    /// composition. Validation happens once, here, rather than on every
    /// query.
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError::param`] when `table` fails
    /// [`validate_identifier`] (empty, too long, bad characters, multiple
    /// schema separators).
    pub fn new(db: DatabaseConnection, table: String) -> Result<Self, FrameworkError> {
        validate_identifier(&table)?;
        Ok(Self { db, table })
    }

    fn backend(&self) -> DatabaseBackend {
        self.db.get_database_backend()
    }
}

#[async_trait]
impl QueueDriver for DatabaseQueueDriver {
    async fn push(&self, env: Envelope) -> Result<(), FrameworkError> {
        let envelope_json = env
            .to_json()
            .map_err(|e| FrameworkError::internal(format!("envelope encode: {e}")))?;
        let stmt = Statement::from_sql_and_values(
            self.backend(),
            format!(
                "INSERT INTO {} \
                 (id, job_name, queue, envelope_json, available_at, attempts, created_at) \
                 VALUES ({})",
                self.table,
                placeholder_list(self.backend(), 1, 7)
            ),
            vec![
                sea_orm::Value::from(env.id.to_string()),
                sea_orm::Value::from(env.job_name.clone()),
                // NULL, not "default" — an unrouted job stays indistinguishable
                // from one written before the column existed, so a mixed-version
                // fleet drains the same rows during a rolling upgrade.
                sea_orm::Value::from(env.queue.clone()),
                sea_orm::Value::from(envelope_json),
                sea_orm::Value::from(env.available_at.timestamp()),
                sea_orm::Value::from(env.attempts as i64),
                sea_orm::Value::from(env.dispatched_at.timestamp()),
            ],
        );
        self.db
            .execute(stmt)
            .await
            .map_err(|e| FrameworkError::internal(format!("queue push: {e}")))?;
        Ok(())
    }

    async fn pop_from(
        &self,
        visibility_timeout: Duration,
        queues: &[String],
    ) -> Result<Option<Reservation>, FrameworkError> {
        self.pop_filtered(visibility_timeout, queues).await
    }

    async fn pop(
        &self,
        visibility_timeout: Duration,
    ) -> Result<Option<Reservation>, FrameworkError> {
        self.pop_filtered(visibility_timeout, &[]).await
    }
    async fn ack(&self, token: &ReservationToken) -> Result<(), FrameworkError> {
        let stmt = Statement::from_sql_and_values(
            self.backend(),
            format!(
                "DELETE FROM {} WHERE reserved_token = {}",
                self.table,
                placeholder(self.backend(), 1)
            ),
            vec![sea_orm::Value::from(token.0.to_string())],
        );
        self.db
            .execute(stmt)
            .await
            .map_err(|e| FrameworkError::internal(format!("queue ack: {e}")))?;
        Ok(())
    }

    async fn nack(
        &self,
        token: &ReservationToken,
        requeue_delay: Duration,
    ) -> Result<(), FrameworkError> {
        self.requeue(token, requeue_delay, AttemptPolicy::Consume, "nack")
            .await
    }

    async fn release(
        &self,
        token: &ReservationToken,
        _env: &Envelope,
        delay: Duration,
    ) -> Result<(), FrameworkError> {
        // The stored row is the source of truth and its `attempts` was never
        // bumped for this run — only the worker's local copy was — so
        // requeuing it in place preserves the count without touching `_env`.
        self.requeue(token, delay, AttemptPolicy::Preserve, "release")
            .await
    }

    async fn size(&self) -> Result<u64, FrameworkError> {
        let row = self
            .db
            .query_one(Statement::from_string(
                self.backend(),
                format!("SELECT COUNT(*) FROM {}", self.table),
            ))
            .await
            .map_err(|e| FrameworkError::internal(format!("queue size: {e}")))?;
        let n: i64 = match row {
            Some(r) => r
                .try_get_by_index(0)
                .map_err(|e| FrameworkError::internal(format!("queue size col: {e}")))?,
            None => 0,
        };
        Ok(n.max(0) as u64)
    }

    async fn pending_size(&self) -> Result<u64, FrameworkError> {
        let now = Utc::now().timestamp();
        let row = self
            .db
            .query_one(Statement::from_sql_and_values(
                self.backend(),
                format!(
                    "SELECT COUNT(*) FROM {} \
                     WHERE available_at <= {} \
                       AND (reserved_until IS NULL OR reserved_until <= {})",
                    self.table,
                    placeholder(self.backend(), 1),
                    placeholder(self.backend(), 2)
                ),
                vec![sea_orm::Value::from(now), sea_orm::Value::from(now)],
            ))
            .await
            .map_err(|e| FrameworkError::internal(format!("queue pending_size: {e}")))?;
        let n: i64 = match row {
            Some(r) => r
                .try_get_by_index(0)
                .map_err(|e| FrameworkError::internal(format!("queue pending_size col: {e}")))?,
            None => 0,
        };
        Ok(n.max(0) as u64)
    }

    async fn delayed_size(&self) -> Result<u64, FrameworkError> {
        let now = Utc::now().timestamp();
        let row = self
            .db
            .query_one(Statement::from_sql_and_values(
                self.backend(),
                format!(
                    "SELECT COUNT(*) FROM {} WHERE available_at > {}",
                    self.table,
                    placeholder(self.backend(), 1)
                ),
                vec![sea_orm::Value::from(now)],
            ))
            .await
            .map_err(|e| FrameworkError::internal(format!("queue delayed_size: {e}")))?;
        let n: i64 = match row {
            Some(r) => r
                .try_get_by_index(0)
                .map_err(|e| FrameworkError::internal(format!("queue delayed_size col: {e}")))?,
            None => 0,
        };
        Ok(n.max(0) as u64)
    }

    async fn reserved_size(&self) -> Result<u64, FrameworkError> {
        let now = Utc::now().timestamp();
        let row = self
            .db
            .query_one(Statement::from_sql_and_values(
                self.backend(),
                format!(
                    "SELECT COUNT(*) FROM {} \
                     WHERE reserved_until IS NOT NULL AND reserved_until > {}",
                    self.table,
                    placeholder(self.backend(), 1)
                ),
                vec![sea_orm::Value::from(now)],
            ))
            .await
            .map_err(|e| FrameworkError::internal(format!("queue reserved_size: {e}")))?;
        let n: i64 = match row {
            Some(r) => r
                .try_get_by_index(0)
                .map_err(|e| FrameworkError::internal(format!("queue reserved_size col: {e}")))?,
            None => 0,
        };
        Ok(n.max(0) as u64)
    }

    async fn clear(&self) -> Result<u64, FrameworkError> {
        let r = self
            .db
            .execute(Statement::from_string(
                self.backend(),
                format!("DELETE FROM {}", self.table),
            ))
            .await
            .map_err(|e| FrameworkError::internal(format!("queue clear: {e}")))?;
        Ok(r.rows_affected())
    }

    fn name(&self) -> &'static str {
        "database"
    }
}

/// Whether a requeue consumes one of the envelope's `max_tries`.
///
/// `nack` and `release` differ in exactly this and nothing else, so they share
/// [`DatabaseQueueDriver::requeue`] rather than two near-identical copies of a
/// read-modify-write that has to stay consistent across three backends.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AttemptPolicy {
    /// `nack`: the attempt is spent, so the stored count advances.
    Consume,
    /// `release`: the job goes back untouched and the next delivery sees the
    /// same `attempts` this one did.
    Preserve,
}

impl DatabaseQueueDriver {
    /// Put a reserved row back on the queue `delay` from now, in place.
    ///
    /// Runs inside a transaction so the read of `envelope_json` and the write
    /// that clears the reservation cannot interleave with another worker's
    /// reclaim of the same row: without it, a visibility expiry landing between
    /// the two statements lets a second worker reserve the row and then have
    /// its reservation silently cleared by this update.
    async fn requeue(
        &self,
        token: &ReservationToken,
        delay: Duration,
        attempts: AttemptPolicy,
        op: &'static str,
    ) -> Result<(), FrameworkError> {
        let now = Utc::now().timestamp();
        let new_available = now + delay.as_secs().min(i64::MAX as u64) as i64;

        let txn = self
            .db
            .begin()
            .await
            .map_err(|e| FrameworkError::internal(format!("queue {op} txn: {e}")))?;

        // Step 1: Read the stored envelope.
        let select_sql = format!(
            "SELECT envelope_json FROM {} WHERE reserved_token = {}",
            self.table,
            placeholder(self.backend(), 1)
        );
        let row = txn
            .query_one(Statement::from_sql_and_values(
                self.backend(),
                &select_sql,
                vec![sea_orm::Value::from(token.0.to_string())],
            ))
            .await
            .map_err(|e| FrameworkError::internal(format!("queue {op} lookup: {e}")))?;

        // Idempotent on unknown token.
        let Some(row) = row else {
            txn.commit()
                .await
                .map_err(|e| FrameworkError::internal(format!("queue {op} txn commit: {e}")))?;
            return Ok(());
        };

        let envelope_json: String = row
            .try_get_by_index::<String>(0)
            .map_err(|e| FrameworkError::internal(format!("queue envelope col: {e}")))?;

        // Step 2: Apply the attempt policy and the new availability in Rust,
        // then write the envelope back so the stored JSON and the columns
        // never disagree — `pop` decodes the JSON, so a column-only update
        // would hand the next worker a stale `available_at`.
        let mut env = Envelope::from_json(&envelope_json)
            .map_err(|e| FrameworkError::internal(format!("envelope decode: {e}")))?;
        if attempts == AttemptPolicy::Consume {
            env.attempts += 1;
        }
        env.available_at = chrono::DateTime::<Utc>::from_timestamp(new_available, 0)
            .ok_or_else(|| FrameworkError::internal(format!("{op}: invalid timestamp")))?;
        let new_json = env
            .to_json()
            .map_err(|e| FrameworkError::internal(format!("envelope encode: {e}")))?;

        // Step 3: Clear the reservation, update available_at, write new JSON.
        let attempts_clause = match attempts {
            AttemptPolicy::Consume => "attempts = attempts + 1, ",
            AttemptPolicy::Preserve => "",
        };
        let stmt = Statement::from_sql_and_values(
            self.backend(),
            format!(
                "UPDATE {} \
                 SET reserved_until = NULL, reserved_token = NULL, \
                     available_at = {}, {}envelope_json = {} \
                 WHERE reserved_token = {}",
                self.table,
                placeholder(self.backend(), 1),
                attempts_clause,
                placeholder(self.backend(), 2),
                placeholder(self.backend(), 3)
            ),
            vec![
                sea_orm::Value::from(new_available),
                sea_orm::Value::from(new_json),
                sea_orm::Value::from(token.0.to_string()),
            ],
        );
        txn.execute(stmt)
            .await
            .map_err(|e| FrameworkError::internal(format!("queue {op}: {e}")))?;
        txn.commit()
            .await
            .map_err(|e| FrameworkError::internal(format!("queue {op} txn commit: {e}")))?;
        Ok(())
    }

    /// Shared body of [`QueueDriver::pop`] and [`QueueDriver::pop_from`].
    ///
    /// With an empty `queues` the emitted SQL is byte-identical to the
    /// pre-routing query, so a deployment that never routes anything is
    /// unaffected — including one whose `jobs` table predates the `queue`
    /// column.
    async fn pop_filtered(
        &self,
        visibility_timeout: Duration,
        queues: &[String],
    ) -> Result<Option<Reservation>, FrameworkError> {
        let now = Utc::now().timestamp();
        let token = Uuid::new_v4().to_string();
        let reserved_until = now + visibility_timeout.as_secs().min(i64::MAX as u64) as i64;

        let lock_clause = match self.backend() {
            DatabaseBackend::Postgres | DatabaseBackend::MySql => "FOR UPDATE SKIP LOCKED",
            DatabaseBackend::Sqlite => "",
        };

        let txn = self
            .db
            .begin()
            .await
            .map_err(|e| FrameworkError::internal(format!("queue txn: {e}")))?;

        // Bind the queue names as parameters rather than interpolating them:
        // queue names reach here from CLI arguments, so interpolation would be
        // a SQL-injection path straight through the worker's `--queue` flag.
        let mut params = vec![sea_orm::Value::from(now), sea_orm::Value::from(now)];
        let queue_clause = if queues.is_empty() {
            String::new()
        } else {
            // Ordinal 3 onwards: the two timestamp binds above already claimed
            // $1 and $2, and Postgres reads the wrong parameter (or errors on a
            // missing one) if the list restarts its own numbering.
            let placeholders = placeholder_list(self.backend(), 3, queues.len());
            // An envelope written before routing has NULL here and belongs to
            // the default queue, so a worker draining "default" must still see it.
            let null_clause = if queues.iter().any(|q| q == DEFAULT_QUEUE) {
                " OR queue IS NULL"
            } else {
                ""
            };
            for q in queues {
                params.push(sea_orm::Value::from(q.clone()));
            }
            format!(" AND (queue IN ({placeholders}){null_clause})")
        };

        let select_sql = format!(
            "SELECT id, envelope_json FROM {} \
             WHERE available_at <= {} \
               AND (reserved_until IS NULL OR reserved_until <= {}){} \
             ORDER BY available_at ASC \
             LIMIT 1 {}",
            self.table,
            placeholder(self.backend(), 1),
            placeholder(self.backend(), 2),
            queue_clause,
            lock_clause
        );
        let row = txn
            .query_one(Statement::from_sql_and_values(
                self.backend(),
                &select_sql,
                params,
            ))
            .await
            .map_err(|e| FrameworkError::internal(format!("queue select: {e}")))?;

        let Some(row) = row else {
            txn.commit().await.ok();
            return Ok(None);
        };

        // Use index-based access — raw SQL column names may not be introspectable.
        let id: String = row
            .try_get_by_index::<String>(0)
            .map_err(|e| FrameworkError::internal(format!("queue id col: {e}")))?;
        let envelope_json: String = row
            .try_get_by_index::<String>(1)
            .map_err(|e| FrameworkError::internal(format!("queue envelope col: {e}")))?;

        // Conditional UPDATE — re-asserts the same "this row is unreserved or
        // its reservation has expired" predicate the SELECT used. Without the
        // predicate, two consumers that observed the same visible row could
        // both stamp their reservation tokens onto it; the loser would walk
        // away with a token that doesn't match what's stored, and a later
        // `ack`/`nack` (which keys on `reserved_token`) would no-op silently,
        // running the job twice. With the predicate the loser sees
        // `rows_affected == 0` and reports an empty pop, so the worker polls
        // again instead of holding a stale reservation.
        //
        // On SQLite, correctness relies on the connection's `busy_timeout`
        // being non-zero so consumer B's UPDATE blocks waiting for A's
        // writer lock (then sees the row reserved, fails the predicate, and
        // affects zero rows). sqlx-sqlite defaults `busy_timeout` to 5s, so
        // this holds in practice; if a future sqlx version drops that
        // default to 0, two concurrent pops could each get `SQLITE_BUSY` and
        // the race would surface as a transient error rather than a
        // double-reservation.
        let exec = txn
            .execute(Statement::from_sql_and_values(
                self.backend(),
                format!(
                    "UPDATE {} SET reserved_until = {}, reserved_token = {} \
                     WHERE id = {} \
                       AND (reserved_until IS NULL OR reserved_until <= {})",
                    self.table,
                    placeholder(self.backend(), 1),
                    placeholder(self.backend(), 2),
                    placeholder(self.backend(), 3),
                    placeholder(self.backend(), 4)
                ),
                vec![
                    sea_orm::Value::from(reserved_until),
                    sea_orm::Value::from(token.clone()),
                    sea_orm::Value::from(id),
                    sea_orm::Value::from(now),
                ],
            ))
            .await
            .map_err(|e| FrameworkError::internal(format!("queue reserve: {e}")))?;

        if exec.rows_affected() == 0 {
            // Another consumer reserved this row in the gap between our SELECT
            // and our UPDATE. Commit the empty txn (nothing to roll back) and
            // tell the caller the queue had nothing for us.
            txn.commit()
                .await
                .map_err(|e| FrameworkError::internal(format!("queue txn commit: {e}")))?;
            return Ok(None);
        }

        txn.commit()
            .await
            .map_err(|e| FrameworkError::internal(format!("queue txn commit: {e}")))?;

        let env = Envelope::from_json(&envelope_json)
            .map_err(|e| FrameworkError::internal(format!("envelope decode: {e}")))?;

        let reservation_token = ReservationToken(
            Uuid::parse_str(&token)
                .map_err(|e| FrameworkError::internal(format!("uuid parse: {e}")))?,
        );

        Ok(Some(Reservation {
            envelope: env,
            token: reservation_token,
        }))
    }
}
