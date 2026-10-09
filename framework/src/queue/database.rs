//! SeaORM-backed queue driver over Laravel 13's `jobs` table. Portable
//! across SQLite / MySQL / Postgres.
//!
//! On SQLite, uses transaction-level locking via `BEGIN` to serialize pop
//! attempts. On MySQL / Postgres, uses `SELECT ... FOR UPDATE SKIP LOCKED`.
//!
//! Every statement here is hand-written, so its placeholders are rendered
//! per backend by the crate-internal `database::placeholder` helpers -
//! Postgres rejects the `?` form outright, which made this whole driver a
//! parse error there.
//!
//! # The table
//!
//! The jobs table is Laravel's (`jobs.stub`): a big-integer `id`, `queue`,
//! a long-text `payload`, `attempts`, `reserved_at`, `available_at` and
//! `created_at`, the last three as epoch seconds.
//! [`CreateJobsTable`](crate::queue::migrations::CreateJobsTable) creates it,
//! and a table Laravel's own migration created works the same. A row's
//! `payload` is the envelope's JSON with the job's name added as
//! `displayName` and the envelope id as `uuid`, the keys Laravel's
//! `InspectedJob` reads.
//!
//! A reservation needs two things Laravel's layout has no column for: the
//! token a worker settles the job with, and the moment the reservation
//! lapses. They live in a table of their own,
//! `suprnova_<table>_reservations` (`job_id`, `token`, `reserved_until`),
//! written in the same transaction as the job's `reserved_at`. The token
//! fences every settlement, as `reserved_token` did: a worker whose
//! reservation lapsed and was reclaimed settles nothing.
//!
//! # Rows this driver did not write
//!
//! A Laravel application on the same database writes its own jobs into the
//! same table. Every envelope this driver writes starts with
//! `{"schema_version":`, which no Laravel payload does, and every pop,
//! listing and `clear` reads only those rows. So a Suprnova worker never
//! reserves a Laravel job, whatever its queue filter says.

use crate::database::clauses::quote_identifier;
use crate::database::placeholder::{placeholder, placeholder_list};
use crate::database::validate_identifier;
use crate::error::FrameworkError;
use crate::queue::driver::{QueueDriver, Reservation, ReservationToken, Settled};
use crate::queue::envelope::{DEFAULT_QUEUE, Envelope};
use crate::queue::inspect::InspectedJob;
use async_trait::async_trait;
use chrono::Utc;
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement, TransactionTrait};
use std::time::Duration;
use uuid::Uuid;

/// The text every payload this driver writes starts with: the first key of
/// a serialized [`Envelope`]. A `LIKE` on it tells this driver's rows from
/// the ones a Laravel application wrote into the same table.
const SUPRNOVA_PAYLOAD_PREFIX: &str = "{\"schema_version\":";

/// Largest value Laravel's `attempts` column holds on every engine: it is
/// an unsigned `SMALLINT` on MySQL and a signed one on Postgres. The
/// envelope keeps the true count; the column stops here.
pub(crate) const MAX_STORED_ATTEMPTS: u32 = 32_767;

/// SeaORM-backed [`QueueDriver`] that stores envelopes in Laravel's `jobs`
/// table. Push, pop, ack, and nack are all transactional; visibility is
/// enforced through the reservations table the pop path writes. See the
/// [module docs](self).
pub struct DatabaseQueueDriver {
    db: DatabaseConnection,
    /// The jobs table, quoted for the connection's backend. Every statement
    /// names it this way, as the migration that created it did, so Postgres
    /// does not fold `QueueJobs` to `queuejobs` and no engine reads `order`
    /// as a keyword.
    table: String,
    /// The reservations table, quoted the same way.
    reservations: String,
}

/// The name of the reservations table that goes with the jobs table
/// `table`: `suprnova_<table>_reservations`, in the same schema when the
/// name is schema-qualified.
pub fn reservations_table(table: &str) -> String {
    match table.rsplit_once('.') {
        Some((schema, name)) => format!("{schema}.suprnova_{name}_reservations"),
        None => format!("suprnova_{table}_reservations"),
    }
}

impl DatabaseQueueDriver {
    /// Construct a driver bound to the given connection and `jobs` table.
    ///
    /// The `table` argument is interpolated into every SQL statement
    /// (push/pop/ack/nack), so it MUST validate as a SQL identifier -
    /// operator-controlled env input doesn't excuse the composition.
    /// Validation happens once, here, rather than on every query, and so
    /// does the quoting every statement uses: each segment of the name is
    /// quoted for the backend, as the migrations quote it. The
    /// reservations table, [`reservations_table`] of `table`, is validated
    /// and quoted the same way.
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError::param`] when `table` fails
    /// [`validate_identifier`] (empty, too long, bad characters, multiple
    /// schema separators), or when the reservations table's name does.
    pub fn new(db: DatabaseConnection, table: String) -> Result<Self, FrameworkError> {
        validate_identifier(&table)?;
        let reservations = reservations_table(&table);
        validate_identifier(&reservations)?;
        let backend = db.get_database_backend();
        Ok(Self {
            table: quote_identifier(backend, &table),
            reservations: quote_identifier(backend, &reservations),
            db,
        })
    }

    fn backend(&self) -> DatabaseBackend {
        self.db.get_database_backend()
    }

    /// The `queue` column value for an envelope routed to `queue`. An
    /// unrouted envelope, or one routed to `default`, is stored under
    /// [`LaravelDatabase::default_queue`](crate::LaravelDatabase::default_queue),
    /// which is `default` unless the application shares its database with
    /// a Laravel application.
    fn stored_queue(queue: Option<&str>) -> String {
        match queue {
            None => crate::LaravelDatabase::default_queue().to_owned(),
            Some(name) if name == DEFAULT_QUEUE => {
                crate::LaravelDatabase::default_queue().to_owned()
            }
            Some(name) => name.to_owned(),
        }
    }

    /// `payload LIKE '{"schema_version":%'` at `ordinal`, with its bound
    /// value. See the [module docs](self).
    fn own_rows_clause(
        &self,
        column: &str,
        ordinal: usize,
    ) -> Result<(String, sea_orm::Value), FrameworkError> {
        Ok((
            format!("{column} LIKE {}", placeholder(self.backend(), ordinal)?),
            sea_orm::Value::from(format!("{SUPRNOVA_PAYLOAD_PREFIX}%")),
        ))
    }
}

/// The `payload` a jobs row stores for `env`: the envelope's JSON with
/// `displayName` and `uuid` appended, so it still starts with
/// [`SUPRNOVA_PAYLOAD_PREFIX`] and still decodes into the envelope.
pub(crate) fn job_payload(env: &Envelope) -> Result<String, FrameworkError> {
    let json = env
        .to_json()
        .map_err(|e| FrameworkError::internal(format!("envelope encode: {e}")))?;
    let body = json
        .strip_suffix('}')
        .filter(|body| body.starts_with(SUPRNOVA_PAYLOAD_PREFIX))
        .ok_or_else(|| {
            FrameworkError::internal("envelope encode: the envelope is not a JSON object")
        })?;
    let display_name = serde_json::to_string(&env.job_name)
        .map_err(|e| FrameworkError::internal(format!("envelope encode: {e}")))?;
    Ok(format!(
        "{body},\"displayName\":{display_name},\"uuid\":\"{}\"}}",
        env.id
    ))
}

/// Read a big-integer key. MySQL's `BIGINT UNSIGNED`, Laravel's `id()`
/// there, decodes only as `u64`; every other engine's as `i64`.
fn read_id(row: &sea_orm::QueryResult, index: usize) -> Result<i64, FrameworkError> {
    if let Ok(id) = row.try_get_by_index::<i64>(index) {
        return Ok(id);
    }
    let id = row
        .try_get_by_index::<u64>(index)
        .map_err(|e| FrameworkError::internal(format!("queue id col: {e}")))?;
    i64::try_from(id).map_err(|_| FrameworkError::internal(format!("queue id {id} out of range")))
}

fn visibility_deadline_seconds(now: chrono::DateTime<Utc>, timeout: Duration) -> i64 {
    if timeout.is_zero() {
        return now.timestamp();
    }

    // `reserved_until` stores whole seconds and readers compare it with
    // `floor(now)`. Ceil the absolute expiry instant so neither the current
    // fractional second nor a subsecond timeout shortens the requested lease.
    let fractional_nanos =
        u64::from(now.timestamp_subsec_nanos()) + u64::from(timeout.subsec_nanos());
    let ceiling_seconds = fractional_nanos.div_ceil(1_000_000_000);
    let deadline =
        i128::from(now.timestamp()) + i128::from(timeout.as_secs()) + i128::from(ceiling_seconds);

    deadline.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_second_visibility_near_a_second_boundary_never_rounds_down() {
        let now = chrono::DateTime::<Utc>::from_timestamp(1_700_000_000, 900_000_000)
            .expect("valid timestamp");

        assert_eq!(
            visibility_deadline_seconds(now, Duration::from_secs(1)),
            1_700_000_002,
            "a claim made 900ms into a second needs the following whole-second boundary",
        );
    }

    #[test]
    fn positive_subsecond_visibility_ceil_preserves_the_requested_interval() {
        let now = chrono::DateTime::<Utc>::from_timestamp(1_700_000_000, 100_000_000)
            .expect("valid timestamp");

        assert_eq!(
            visibility_deadline_seconds(now, Duration::from_millis(150)),
            1_700_000_001,
            "a positive subsecond lease must reach the next stored second",
        );
    }

    #[test]
    fn pre_epoch_large_duration_stays_finite_when_the_final_sum_fits() {
        let now = chrono::DateTime::<Utc>::from_timestamp(-10, 900_000_000)
            .expect("valid pre-epoch timestamp");

        assert_eq!(
            visibility_deadline_seconds(now, Duration::from_secs(i64::MAX as u64)),
            i64::MAX - 9,
        );
    }

    #[test]
    fn duration_max_from_pre_epoch_saturates_only_the_final_sum() {
        let now = chrono::DateTime::<Utc>::from_timestamp(-10, 900_000_000)
            .expect("valid pre-epoch timestamp");

        assert_eq!(visibility_deadline_seconds(now, Duration::MAX), i64::MAX);
    }
}

/// Read an integer column whatever width and sign the engine reports it
/// with: Laravel's layout mixes `INT UNSIGNED`, `SMALLINT` and `BIGINT`,
/// and each driver decodes only its own Rust type.
fn read_int(row: &sea_orm::QueryResult, index: usize) -> Result<Option<i64>, FrameworkError> {
    if let Ok(v) = row.try_get_by_index::<Option<i64>>(index) {
        return Ok(v);
    }
    if let Ok(v) = row.try_get_by_index::<Option<i32>>(index) {
        return Ok(v.map(i64::from));
    }
    if let Ok(v) = row.try_get_by_index::<Option<u32>>(index) {
        return Ok(v.map(i64::from));
    }
    if let Ok(v) = row.try_get_by_index::<Option<i16>>(index) {
        return Ok(v.map(i64::from));
    }
    match row.try_get_by_index::<Option<u64>>(index) {
        Ok(v) => v
            .map(|v| {
                i64::try_from(v)
                    .map_err(|_| FrameworkError::internal(format!("queue column {v} out of range")))
            })
            .transpose(),
        Err(e) => Err(FrameworkError::internal(format!(
            "queue integer col {index}: {e}"
        ))),
    }
}

#[async_trait]
impl QueueDriver for DatabaseQueueDriver {
    async fn push(&self, env: Envelope) -> Result<(), FrameworkError> {
        let (sql, values) = self.insert_statement(&env)?;
        self.db
            .execute_raw(Statement::from_sql_and_values(self.backend(), sql, values))
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

    fn queue_filter_capability(&self) -> crate::queue::driver::QueueFilterCapability {
        crate::queue::driver::QueueFilterCapability::Supported
    }

    async fn pop(
        &self,
        visibility_timeout: Duration,
    ) -> Result<Option<Reservation>, FrameworkError> {
        self.pop_filtered(visibility_timeout, &[]).await
    }

    async fn ack(&self, token: &ReservationToken) -> Result<(), FrameworkError> {
        let txn = self
            .db
            .begin()
            .await
            .map_err(|e| FrameworkError::internal(format!("queue ack txn: {e}")))?;
        self.delete_reserved(&txn, token, "ack").await?;
        txn.commit()
            .await
            .map_err(|e| FrameworkError::internal(format!("queue ack commit: {e}")))?;
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

    /// Atomic terminal settlement: the follow-ups land in the same `jobs`
    /// table, in the same transaction, as the delete that drops the
    /// reservation.
    ///
    /// The delete is the fence. It is keyed on the reservation token, so a
    /// worker whose reservation expired while it was busy affects zero
    /// rows, and the whole transaction - including the successor it was
    /// about to enqueue - rolls back. That is the case two-step settlement
    /// cannot handle at all: the stale worker's push succeeds, the new
    /// owner's push succeeds too, and the chain forks.
    async fn settle(
        &self,
        token: &ReservationToken,
        follow_ups: &[Envelope],
    ) -> Result<Settled, FrameworkError> {
        let txn = self
            .db
            .begin()
            .await
            .map_err(|e| FrameworkError::internal(format!("queue settle txn: {e}")))?;

        // The fence runs FIRST. Both statements commit together either way, so
        // ordering does not affect atomicity - it decides which outcome a
        // stale worker sees. Inserting first, a worker whose job had already
        // been reclaimed and settled by someone else would collide with the
        // successor that owner enqueued and surface a duplicate-key error;
        // deleting first, it reads zero rows and reports `Stale`, which is what
        // actually happened.
        let deleted = self.delete_reserved(&txn, token, "settle ack").await?;

        if deleted == 0 {
            // Not ours any more. Commit nothing: the follow-ups belong to
            // whichever consumer holds the reservation now.
            txn.rollback()
                .await
                .map_err(|e| FrameworkError::internal(format!("queue settle rollback: {e}")))?;
            return Ok(Settled::Stale);
        }

        for env in follow_ups {
            let (sql, values) = self.insert_statement(env)?;
            txn.execute_raw(Statement::from_sql_and_values(self.backend(), sql, values))
                .await
                .map_err(|e| FrameworkError::internal(format!("queue settle push: {e}")))?;
        }

        txn.commit()
            .await
            .map_err(|e| FrameworkError::internal(format!("queue settle commit: {e}")))?;
        Ok(Settled::Atomically)
    }

    async fn release(
        &self,
        token: &ReservationToken,
        _env: &Envelope,
        delay: Duration,
    ) -> Result<(), FrameworkError> {
        // The stored row is the source of truth and its `attempts` was never
        // bumped for this run - only the worker's local copy was - so
        // requeuing it in place preserves the count without touching `_env`.
        self.requeue(token, delay, AttemptPolicy::Preserve, "release")
            .await
    }

    async fn size(&self, queue: Option<&str>) -> Result<u64, FrameworkError> {
        let (own, own_value) = self.own_rows_clause("payload", 1)?;
        let (selected, queue_params) = self.queue_filter_clause(queue, &self.table, 2)?;
        let mut params = vec![own_value];
        params.extend(queue_params);
        self.count(
            format!("SELECT COUNT(*) FROM {} WHERE {own}{selected}", self.table),
            params,
            "size",
        )
        .await
    }

    async fn pending_size(&self, queue: Option<&str>) -> Result<u64, FrameworkError> {
        let now = crate::clock::now().timestamp();
        let (own, own_value) = self.own_rows_clause("j.payload", 3)?;
        let (selected, queue_params) = self.queue_filter_clause(queue, "j", 4)?;
        let mut params = vec![
            sea_orm::Value::from(now),
            sea_orm::Value::from(now),
            own_value,
        ];
        params.extend(queue_params);
        self.count(
            format!(
                "SELECT COUNT(*) FROM {} j WHERE j.available_at <= {} AND {} AND {own}{selected}",
                self.table,
                placeholder(self.backend(), 1)?,
                self.free_clause("j", 2)?,
            ),
            params,
            "pending_size",
        )
        .await
    }

    async fn delayed_size(&self, queue: Option<&str>) -> Result<u64, FrameworkError> {
        let now = crate::clock::now().timestamp();
        let (own, own_value) = self.own_rows_clause("payload", 2)?;
        let (selected, queue_params) = self.queue_filter_clause(queue, &self.table, 3)?;
        let mut params = vec![sea_orm::Value::from(now), own_value];
        params.extend(queue_params);
        self.count(
            format!(
                "SELECT COUNT(*) FROM {} WHERE available_at > {} AND {own}{selected}",
                self.table,
                placeholder(self.backend(), 1)?
            ),
            params,
            "delayed_size",
        )
        .await
    }

    async fn reserved_size(&self, queue: Option<&str>) -> Result<u64, FrameworkError> {
        let now = crate::clock::now().timestamp();
        let (own, own_value) = self.own_rows_clause("j.payload", 2)?;
        let (selected, queue_params) = self.queue_filter_clause(queue, "j", 3)?;
        let mut params = vec![sea_orm::Value::from(now), own_value];
        params.extend(queue_params);
        self.count(
            format!(
                "SELECT COUNT(*) FROM {} j WHERE NOT {} AND {own}{selected}",
                self.table,
                self.free_clause("j", 1)?,
            ),
            params,
            "reserved_size",
        )
        .await
    }

    async fn clear(&self, queue: Option<&str>) -> Result<u64, FrameworkError> {
        let txn = self
            .db
            .begin()
            .await
            .map_err(|e| FrameworkError::internal(format!("queue clear txn: {e}")))?;
        let (own, own_value) = self.own_rows_clause("payload", 1)?;
        let (selected, queue_params) = self.queue_filter_clause(queue, &self.table, 2)?;
        let mut params = vec![own_value];
        params.extend(queue_params);
        txn.execute_raw(Statement::from_sql_and_values(
            self.backend(),
            format!(
                "DELETE FROM {} WHERE job_id IN (SELECT id FROM {} WHERE {own}{selected})",
                self.reservations, self.table
            ),
            params.clone(),
        ))
        .await
        .map_err(|e| FrameworkError::internal(format!("queue clear reservations: {e}")))?;
        let r = txn
            .execute_raw(Statement::from_sql_and_values(
                self.backend(),
                format!("DELETE FROM {} WHERE {own}{selected}", self.table),
                params,
            ))
            .await
            .map_err(|e| FrameworkError::internal(format!("queue clear: {e}")))?;
        txn.commit()
            .await
            .map_err(|e| FrameworkError::internal(format!("queue clear commit: {e}")))?;
        Ok(r.rows_affected())
    }

    /// Rows matching [`pending_size`](Self::pending_size)'s exact predicate,
    /// decoded and ordered by `available_at`. See
    /// the `list_jobs` helper for the poison-row contract.
    async fn pending_jobs(&self, queue: Option<&str>) -> Result<Vec<InspectedJob>, FrameworkError> {
        let now = crate::clock::now().timestamp();
        let (own, own_value) = self.own_rows_clause("j.payload", 3)?;
        let (queue_clause, queue_params) = self.queue_filter_clause(queue, "j", 4)?;
        let mut params = vec![
            sea_orm::Value::from(now),
            sea_orm::Value::from(now),
            own_value,
        ];
        params.extend(queue_params);
        let sql = format!(
            "SELECT j.payload FROM {} j \
             WHERE j.available_at <= {} AND {} AND {own}{} \
             ORDER BY j.available_at ASC, j.id ASC",
            self.table,
            placeholder(self.backend(), 1)?,
            self.free_clause("j", 2)?,
            queue_clause,
        );
        self.list_jobs(sql, params).await
    }

    /// Rows matching [`delayed_size`](Self::delayed_size)'s exact predicate,
    /// decoded and ordered by `available_at`. See
    /// the `list_jobs` helper for the poison-row contract.
    async fn delayed_jobs(&self, queue: Option<&str>) -> Result<Vec<InspectedJob>, FrameworkError> {
        let now = crate::clock::now().timestamp();
        let (own, own_value) = self.own_rows_clause("j.payload", 2)?;
        let (queue_clause, queue_params) = self.queue_filter_clause(queue, "j", 3)?;
        let mut params = vec![sea_orm::Value::from(now), own_value];
        params.extend(queue_params);
        let sql = format!(
            "SELECT j.payload FROM {} j WHERE j.available_at > {} AND {own}{} \
             ORDER BY j.available_at ASC, j.id ASC",
            self.table,
            placeholder(self.backend(), 1)?,
            queue_clause,
        );
        self.list_jobs(sql, params).await
    }

    /// Rows matching [`reserved_size`](Self::reserved_size)'s exact
    /// predicate, decoded and ordered by `available_at`. See
    /// the `list_jobs` helper for the poison-row contract.
    async fn reserved_jobs(
        &self,
        queue: Option<&str>,
    ) -> Result<Vec<InspectedJob>, FrameworkError> {
        let now = crate::clock::now().timestamp();
        let (own, own_value) = self.own_rows_clause("j.payload", 2)?;
        let (queue_clause, queue_params) = self.queue_filter_clause(queue, "j", 3)?;
        let mut params = vec![sea_orm::Value::from(now), own_value];
        params.extend(queue_params);
        let sql = format!(
            "SELECT j.payload FROM {} j WHERE NOT {} AND {own}{} \
             ORDER BY j.available_at ASC, j.id ASC",
            self.table,
            self.free_clause("j", 1)?,
            queue_clause,
        );
        self.list_jobs(sql, params).await
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
    /// The `INSERT` that enqueues one envelope, as SQL plus bound values.
    ///
    /// Shared by [`QueueDriver::push`] and [`QueueDriver::settle`] so a
    /// follow-up enqueued inside the settlement transaction is written exactly
    /// the way a directly-pushed one is - a second copy of this statement
    /// would be free to drift in a column, a placeholder ordinal, or the
    /// stored queue name.
    fn insert_statement(
        &self,
        env: &Envelope,
    ) -> Result<(String, Vec<sea_orm::Value>), FrameworkError> {
        let payload = job_payload(env)?;
        let sql = format!(
            "INSERT INTO {} \
             (queue, payload, attempts, reserved_at, available_at, created_at) \
             VALUES ({}, NULL, {})",
            self.table,
            placeholder_list(self.backend(), 1, 3)?,
            placeholder_list(self.backend(), 4, 2)?
        );
        Ok((
            sql,
            vec![
                sea_orm::Value::from(Self::stored_queue(env.queue.as_deref())),
                sea_orm::Value::from(payload),
                sea_orm::Value::from(i64::from(env.attempts.min(MAX_STORED_ATTEMPTS))),
                sea_orm::Value::from(env.available_at.timestamp()),
                sea_orm::Value::from(env.dispatched_at.timestamp()),
            ],
        ))
    }

    /// `COUNT(*)` of `sql` as a `u64`.
    async fn count(
        &self,
        sql: String,
        params: Vec<sea_orm::Value>,
        op: &'static str,
    ) -> Result<u64, FrameworkError> {
        let row = self
            .db
            .query_one_raw(Statement::from_sql_and_values(self.backend(), sql, params))
            .await
            .map_err(|e| FrameworkError::internal(format!("queue {op}: {e}")))?;
        let n = match row {
            Some(r) => read_int(&r, 0)?.unwrap_or(0),
            None => 0,
        };
        Ok(n.max(0) as u64)
    }

    /// The jobs row `job` holds no live reservation: it was never reserved,
    /// or its reservation's `reserved_until` has passed. One bound value,
    /// the current time, at `ordinal`.
    ///
    /// A row reserved by something other than this driver (`reserved_at`
    /// set, no reservations row) is not free here: only a pop, which knows
    /// its visibility timeout, decides when such a reservation has lapsed.
    fn free_clause(&self, job: &str, ordinal: usize) -> Result<String, FrameworkError> {
        Ok(format!(
            "({job}.reserved_at IS NULL OR EXISTS (SELECT 1 FROM {res} r \
             WHERE r.job_id = {job}.id AND r.reserved_until <= {now}))",
            res = self.reservations,
            now = placeholder(self.backend(), ordinal)?,
        ))
    }

    /// [`Self::free_clause`] for a pop: also free is a row reserved by
    /// something other than this driver whose `reserved_at` is at or before
    /// `expired` (the pop's visibility timeout ago). Two bound values at
    /// `ordinal`: the current time, then `expired`.
    fn claimable_clause(&self, job: &str, ordinal: usize) -> Result<String, FrameworkError> {
        Ok(format!(
            "({job}.reserved_at IS NULL \
             OR EXISTS (SELECT 1 FROM {res} r WHERE r.job_id = {job}.id \
                        AND r.reserved_until <= {now}) \
             OR ({job}.reserved_at <= {expired} AND NOT EXISTS \
                 (SELECT 1 FROM {res} r2 WHERE r2.job_id = {job}.id)))",
            res = self.reservations,
            now = placeholder(self.backend(), ordinal)?,
            expired = placeholder(self.backend(), ordinal + 1)?,
        ))
    }

    /// Delete the job a reservation holds, and the reservation, inside
    /// `txn`. Returns how many job rows went: `0` when the token holds
    /// nothing any more.
    async fn delete_reserved(
        &self,
        txn: &sea_orm::DatabaseTransaction,
        token: &ReservationToken,
        op: &'static str,
    ) -> Result<u64, FrameworkError> {
        let token_value = sea_orm::Value::from(token.0.to_string());
        let deleted = txn
            .execute_raw(Statement::from_sql_and_values(
                self.backend(),
                format!(
                    "DELETE FROM {} WHERE id IN (SELECT job_id FROM {} WHERE token = {})",
                    self.table,
                    self.reservations,
                    placeholder(self.backend(), 1)?
                ),
                vec![token_value.clone()],
            ))
            .await
            .map_err(|e| FrameworkError::internal(format!("queue {op}: {e}")))?;
        txn.execute_raw(Statement::from_sql_and_values(
            self.backend(),
            format!(
                "DELETE FROM {} WHERE token = {}",
                self.reservations,
                placeholder(self.backend(), 1)?
            ),
            vec![token_value],
        ))
        .await
        .map_err(|e| FrameworkError::internal(format!("queue {op} reservation: {e}")))?;
        Ok(deleted.rows_affected())
    }

    /// Put a reserved row back on the queue `delay` from now, in place.
    ///
    /// Runs inside a transaction, and the update is fenced on the token, so
    /// the read of `payload` and the write that clears the reservation
    /// cannot interleave with another worker's reclaim of the same row:
    /// without the fence, a visibility expiry landing between the two
    /// statements lets a second worker reserve the row and then have its
    /// reservation silently cleared by this update.
    async fn requeue(
        &self,
        token: &ReservationToken,
        delay: Duration,
        attempts: AttemptPolicy,
        op: &'static str,
    ) -> Result<(), FrameworkError> {
        // Resolved before the transaction opens, so a delay no date can hold
        // fails with the job still reserved instead of overflowing.
        let new_available = crate::queue::driver::available_after(delay)?.timestamp();

        let txn = self
            .db
            .begin()
            .await
            .map_err(|e| FrameworkError::internal(format!("queue {op} txn: {e}")))?;

        // Step 1: Read the stored envelope.
        let token_value = sea_orm::Value::from(token.0.to_string());
        let row = txn
            .query_one_raw(Statement::from_sql_and_values(
                self.backend(),
                format!(
                    "SELECT j.id, j.payload FROM {} j INNER JOIN {} r ON r.job_id = j.id \
                     WHERE r.token = {}",
                    self.table,
                    self.reservations,
                    placeholder(self.backend(), 1)?
                ),
                vec![token_value.clone()],
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

        let id = read_id(&row, 0)?;
        let payload: String = row
            .try_get_by_index::<String>(1)
            .map_err(|e| FrameworkError::internal(format!("queue payload col: {e}")))?;

        // Step 2: Apply the attempt policy and the new availability in Rust,
        // then write the payload back so the stored JSON and the columns
        // never disagree - `pop` decodes the JSON, so a column-only update
        // would hand the next worker a stale `available_at`.
        let mut env = Envelope::from_json(&payload)
            .map_err(|e| FrameworkError::internal(format!("envelope decode: {e}")))?;
        if attempts == AttemptPolicy::Consume {
            env.attempts += 1;
        }
        env.available_at = chrono::DateTime::<Utc>::from_timestamp(new_available, 0)
            .ok_or_else(|| FrameworkError::internal(format!("{op}: invalid timestamp")))?;
        let new_payload = job_payload(&env)?;

        // Step 3: Clear the reservation, update available_at, write the new
        // payload, all fenced on the token.
        txn.execute_raw(Statement::from_sql_and_values(
            self.backend(),
            format!(
                "UPDATE {table} \
                 SET reserved_at = NULL, available_at = {}, attempts = {}, payload = {} \
                 WHERE id = {} AND EXISTS (SELECT 1 FROM {res} r \
                     WHERE r.job_id = {table}.id AND r.token = {})",
                placeholder(self.backend(), 1)?,
                placeholder(self.backend(), 2)?,
                placeholder(self.backend(), 3)?,
                placeholder(self.backend(), 4)?,
                placeholder(self.backend(), 5)?,
                table = self.table,
                res = self.reservations,
            ),
            vec![
                sea_orm::Value::from(new_available),
                sea_orm::Value::from(i64::from(env.attempts.min(MAX_STORED_ATTEMPTS))),
                sea_orm::Value::from(new_payload),
                sea_orm::Value::from(id),
                token_value.clone(),
            ],
        ))
        .await
        .map_err(|e| FrameworkError::internal(format!("queue {op}: {e}")))?;
        txn.execute_raw(Statement::from_sql_and_values(
            self.backend(),
            format!(
                "DELETE FROM {} WHERE token = {}",
                self.reservations,
                placeholder(self.backend(), 1)?
            ),
            vec![token_value],
        ))
        .await
        .map_err(|e| FrameworkError::internal(format!("queue {op} reservation: {e}")))?;
        txn.commit()
            .await
            .map_err(|e| FrameworkError::internal(format!("queue {op} txn commit: {e}")))?;
        Ok(())
    }

    /// `AND j.queue = ?` for a listing method's optional single-queue
    /// filter, with `default` read as the stored default queue the way
    /// `pop_filtered` reads it.
    ///
    /// Bound as a parameter at `ordinal`, never interpolated: `queue`
    /// reaches these listing methods from operator input (an admin
    /// dashboard, a CLI flag), the same trust boundary `pop_filtered`
    /// already treats as untrusted.
    fn queue_filter_clause(
        &self,
        queue: Option<&str>,
        job: &str,
        ordinal: usize,
    ) -> Result<(String, Vec<sea_orm::Value>), FrameworkError> {
        match queue {
            None => Ok((String::new(), Vec::new())),
            Some(q) => Ok((
                format!(
                    " AND {job}.queue = {}",
                    placeholder(self.backend(), ordinal)?
                ),
                vec![sea_orm::Value::from(Self::stored_queue(Some(q)))],
            )),
        }
    }

    /// Shared row-decoding for `pending_jobs`/`delayed_jobs`/`reserved_jobs`:
    /// run `sql`/`params` (which must select `payload` first) and decode
    /// each row into an [`InspectedJob`].
    ///
    /// A row whose payload fails to parse becomes a poison-marked entry
    /// (`id: None`, `payload: {"unparseable": true}`) instead of failing the
    /// whole listing - one corrupt row must not blind an operator to every
    /// other job on the queue.
    async fn list_jobs(
        &self,
        sql: String,
        params: Vec<sea_orm::Value>,
    ) -> Result<Vec<InspectedJob>, FrameworkError> {
        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(self.backend(), sql, params))
            .await
            .map_err(|e| FrameworkError::internal(format!("queue listing: {e}")))?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let payload: String = row
                .try_get_by_index(0)
                .map_err(|e| FrameworkError::internal(format!("queue listing payload col: {e}")))?;
            out.push(match Envelope::from_json(&payload) {
                Ok(env) => InspectedJob::from_envelope(&env),
                Err(e) => {
                    let name = serde_json::from_str::<serde_json::Value>(&payload)
                        .ok()
                        .and_then(|value| {
                            value
                                .get("displayName")
                                .and_then(serde_json::Value::as_str)
                                .map(str::to_owned)
                        })
                        .unwrap_or_default();
                    tracing::warn!(
                        job_name = %name,
                        error = %e,
                        "queue listing: payload failed to parse; \
                         reporting the row as unparseable instead of dropping it"
                    );
                    InspectedJob {
                        id: None,
                        queue: None,
                        name,
                        attempts: 0,
                        payload: serde_json::json!({ "unparseable": true }),
                        created_at: None,
                    }
                }
            });
        }
        Ok(out)
    }

    /// Shared body of [`QueueDriver::pop`] and [`QueueDriver::pop_from`].
    ///
    /// An empty `queues` reads every queue, but only rows this driver
    /// wrote (see the [module docs](self)).
    async fn pop_filtered(
        &self,
        visibility_timeout: Duration,
        queues: &[String],
    ) -> Result<Option<Reservation>, FrameworkError> {
        let claim_time = crate::clock::now();
        let now = claim_time.timestamp();
        let token = Uuid::new_v4();
        let reserved_until = visibility_deadline_seconds(claim_time, visibility_timeout);
        let expired =
            now.saturating_sub(i64::try_from(visibility_timeout.as_secs()).unwrap_or(i64::MAX));

        let lock_clause = match self.backend() {
            DatabaseBackend::Postgres | DatabaseBackend::MySql => "FOR UPDATE SKIP LOCKED",
            DatabaseBackend::Sqlite => "",
            backend => {
                return Err(crate::database::unsupported_database_backend(backend));
            }
        };

        let txn = self
            .db
            .begin()
            .await
            .map_err(|e| FrameworkError::internal(format!("queue txn: {e}")))?;

        // Bind the queue names as parameters rather than interpolating them:
        // queue names reach here from CLI arguments, so interpolation would be
        // a SQL-injection path straight through the worker's `--queue` flag.
        let (own, own_value) = self.own_rows_clause("j.payload", 4)?;
        let mut params = vec![
            sea_orm::Value::from(now),
            sea_orm::Value::from(now),
            sea_orm::Value::from(expired),
            own_value,
        ];
        let queue_clause = if queues.is_empty() {
            String::new()
        } else {
            let mut stored: Vec<String> = Vec::with_capacity(queues.len());
            for q in queues {
                let name = Self::stored_queue(Some(q));
                if !stored.contains(&name) {
                    stored.push(name);
                }
            }
            // Ordinal 5 onwards: the binds above already claimed $1 to $4,
            // and Postgres reads the wrong parameter (or errors on a missing
            // one) if the list restarts its own numbering.
            let placeholders = placeholder_list(self.backend(), 5, stored.len())?;
            params.extend(stored.into_iter().map(sea_orm::Value::from));
            format!(" AND j.queue IN ({placeholders})")
        };

        let select_sql = format!(
            "SELECT j.id, j.payload, j.reserved_at FROM {} j \
             WHERE j.available_at <= {} AND {} AND {own}{} \
             ORDER BY j.available_at ASC, j.id ASC \
             LIMIT 1 {}",
            self.table,
            placeholder(self.backend(), 1)?,
            self.claimable_clause("j", 2)?,
            queue_clause,
            lock_clause
        );
        let row = txn
            .query_one_raw(Statement::from_sql_and_values(
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

        // Use index-based access - raw SQL column names may not be introspectable.
        let id = read_id(&row, 0)?;
        let payload: String = row
            .try_get_by_index::<String>(1)
            .map_err(|e| FrameworkError::internal(format!("queue payload col: {e}")))?;
        // A reservation we can still see is a reservation that lapsed: the
        // SELECT predicate above admits a row never claimed, or one whose
        // holder's reservation has passed. So a non-NULL `reserved_at` here
        // means some worker took this job and did not come back - it died
        // mid-execution.
        //
        // That distinction is the whole fix. A job whose handler *fails*
        // is nacked, and `requeue(AttemptPolicy::Consume)` counts the
        // attempt. A job that *kills its worker* settles nothing, so
        // before this the reclaim returned it byte-identical and its
        // attempt count never moved. Such a job is immortal: it kills each
        // worker that claims it, comes back unchanged, and kills the next
        // one, for as long as anything restarts workers.
        let is_reclaim = read_int(&row, 2)?.is_some();

        let mut env = Envelope::from_json(&payload)
            .map_err(|e| FrameworkError::internal(format!("envelope decode: {e}")))?;
        if is_reclaim {
            env.attempts += 1;
        }

        // Conditional UPDATE - re-asserts the same "this row is unreserved or
        // its reservation has expired" predicate the SELECT used. Without the
        // predicate, two consumers that observed the same visible row could
        // both stamp their reservations onto it; the loser would walk away
        // with a token that does not match what is stored, and a later
        // `ack`/`nack` (which keys on the token) would no-op silently,
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
        // The attempt count and the payload rewrite are appended only on a
        // reclaim. A first claim is the hot path - every poll of a busy
        // queue takes it - and rewriting an identical payload on each one
        // would be a wasted column write per job.
        let mut set_clause = format!("reserved_at = {}", placeholder(self.backend(), 1)?);
        let mut update_params = vec![sea_orm::Value::from(now)];
        let mut next_ordinal = 2;
        if is_reclaim {
            // Column and payload together, the way `requeue` does it: `pop`
            // decodes the JSON, so bumping only the column would hand the
            // next worker an envelope whose `attempts` disagreed with the
            // row - and `worker.rs` reads the envelope to decide whether
            // `max_tries` is exhausted.
            set_clause.push_str(&format!(
                ", attempts = {}, payload = {}",
                placeholder(self.backend(), next_ordinal)?,
                placeholder(self.backend(), next_ordinal + 1)?
            ));
            update_params.push(sea_orm::Value::from(i64::from(
                env.attempts.min(MAX_STORED_ATTEMPTS),
            )));
            update_params.push(sea_orm::Value::from(job_payload(&env)?));
            next_ordinal += 2;
        }
        let update_sql = format!(
            "UPDATE {table} SET {set_clause} WHERE id = {} AND {}",
            placeholder(self.backend(), next_ordinal)?,
            self.claimable_clause(&self.table, next_ordinal + 1)?,
            table = self.table,
        );
        update_params.push(sea_orm::Value::from(id));
        update_params.push(sea_orm::Value::from(now));
        update_params.push(sea_orm::Value::from(expired));

        let exec = txn
            .execute_raw(Statement::from_sql_and_values(
                self.backend(),
                update_sql,
                update_params,
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

        // The reservation itself: a lapsed one for this job is replaced.
        txn.execute_raw(Statement::from_sql_and_values(
            self.backend(),
            format!(
                "DELETE FROM {} WHERE job_id = {}",
                self.reservations,
                placeholder(self.backend(), 1)?
            ),
            vec![sea_orm::Value::from(id)],
        ))
        .await
        .map_err(|e| FrameworkError::internal(format!("queue reserve: {e}")))?;
        txn.execute_raw(Statement::from_sql_and_values(
            self.backend(),
            format!(
                "INSERT INTO {} (job_id, token, reserved_until) VALUES ({})",
                self.reservations,
                placeholder_list(self.backend(), 1, 3)?
            ),
            vec![
                sea_orm::Value::from(id),
                sea_orm::Value::from(token.to_string()),
                sea_orm::Value::from(reserved_until),
            ],
        ))
        .await
        .map_err(|e| FrameworkError::internal(format!("queue reserve: {e}")))?;

        txn.commit()
            .await
            .map_err(|e| FrameworkError::internal(format!("queue txn commit: {e}")))?;

        Ok(Some(Reservation {
            envelope: env,
            token: ReservationToken(token),
        }))
    }
}
