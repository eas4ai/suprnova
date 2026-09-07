//! Database-backed [`InstanceRecordStore`]: one row per Live instance in
//! `suprnova_live_instances` and one per reserved retry identity in
//! `suprnova_live_promotions`, shared by every node pointed at the same
//! database.
//!
//! # What the rows hold
//!
//! | Column | Meaning |
//! |---|---|
//! | `scope` | lowercase hex of the trusted scope fingerprint, 64 characters |
//! | `instance` / `idempotency` | lowercase hex of the server-assigned instance identity or the bounded retry identity, 32 to 64 characters in a `VARCHAR(64)` (both identities accept 16 to 32 bytes) |
//! | `record` | the encoded record, opaque here: only the kernel's codec reads it |
//! | `version` | the version a compare-and-store must carry to replace this state (instances only) |
//! | `expires_at_ms` | store-time deadline after which the record is gone |
//!
//! Records are bytes and stay bytes. This adapter never decodes one, never
//! logs one, and never puts one in a message: the whole point of the port is
//! that authority is the kernel's and storage is the host's.
//!
//! # One clock
//!
//! `expires_at_ms` is the deadline the kernel computed on *its* clock and
//! handed over as a bound value; every comparison against it is
//! [`sql_now_ms`] inlined into the statement that reads or guards the row.
//! A record past that deadline answers exactly as one that was never
//! written, which is what the port promises.
//!
//! # The caller's transaction, when there is one
//!
//! Unlike the lease store, this one joins the host's ambient transaction
//! whenever one is open, and opens a short transaction of its own otherwise.
//! That coupling is the behaviour a database tier is chosen for: a claim
//! taken inside a request's transaction disappears with it if the request
//! rolls back, so authority and the host effects it was taken for commit or
//! fail together. An external key-value tier cannot offer it, and the Live
//! specification allows either.
//!
//! # Bounds
//!
//! A record over
//! [`MAX_RECORD_BYTES`] is refused
//! before any statement is built, and reclamation of elapsed records is
//! bounded to `RECLAIM_BATCH` rows per creating operation - the same rule,
//! and the same number, the in-memory reference store applies, so a burst of
//! expiries that arrive together is paid for over the operations that
//! follow rather than by whichever one is unlucky.
//!
//! Reclamation runs in the operation's transaction, which inside a host
//! transaction is the host's: its `DELETE` holds row locks until the host
//! commits, and is rolled back with the host if the host rolls back. That is
//! the price of the coupling above, it is bounded by the batch size, and it
//! costs correctness nothing - an elapsed record a rollback puts back is
//! still elapsed, still invisible to every read, and still the next creating
//! operation's to reclaim.

use async_trait::async_trait;
use sea_orm::{DbBackend, DbErr, Value};
use suprnova_live::identity::UnixMillis;
use suprnova_live::ledger::{
    CasOutcome, InstanceRecordKey, InstanceRecordStore, LedgerError, LedgerErrorKind,
    MAX_RECORD_BYTES, PromotionRecordKey, StoredRecord,
};

use super::{
    as_i64, as_u64, bind, db_error_kind, framework_error_kind, is_unique_violation, row_lock,
    sql_now_ms,
};
use crate::database::transaction::ExecutorChoice;
use crate::{DB, FrameworkError, PRIMARY_CONNECTION_NAME, Transaction};

/// The version a store gives a record it has just created. Versions belong
/// to a record rather than to a key, so a record created again at a key
/// whose previous record elapsed starts here again.
const FIRST_VERSION: u64 = 1;

/// Elapsed records one creating operation reclaims.
///
/// Bounded for the reason the in-memory reference store bounds it: a store
/// holding a million records that all elapse at once still answers in
/// predictable time, and the backlog drains over the operations that follow.
/// Reclamation is hygiene, never a correctness gate - every read and every
/// guard already refuses a record past its deadline whether or not a batch
/// has reached it.
pub(crate) const RECLAIM_BATCH: usize = 64;

/// Database-backed Live instance and promotion record store. See the module
/// documentation for the row layout, the clock, and the transaction rule.
///
/// `Debug` prints the test offset only. There is no record state in this
/// type, and if there were, it would not be printed: a record is instance
/// authority, and this crate redacts that everywhere.
#[derive(Debug, Default)]
pub struct SqlInstanceRecordStore {
    /// Milliseconds added to every store-time comparison, for tests alone.
    /// Per instance rather than process-wide: several stores share one test
    /// binary. See [`Self::set_time_offset_for_test`].
    time_offset_ms: std::sync::atomic::AtomicU64,
}

impl SqlInstanceRecordStore {
    /// A store over the primary connection's `suprnova_live_instances` and
    /// `suprnova_live_promotions` tables.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Moves this store's view of the database clock forward by `offset_ms`
    /// milliseconds, so a test can reach a record's deadline without waiting
    /// for one.
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

    /// The creation body for either table, run on one executor already
    /// inside this operation's transaction.
    ///
    /// The shape is: reclaim a bounded batch of elapsed records, take the
    /// locked read that decides whether an unexpired record holds the key,
    /// drop this key's own elapsed record if one is still there, and insert.
    /// The insert is plain rather than an upsert on purpose - a conflict
    /// means a peer created the record between this transaction's read and
    /// its write, and a peer's record is the one that stands. On PostgreSQL,
    /// where the read locks nothing when the row is absent, the unique key
    /// is the only thing that can decide that race.
    async fn create_through(
        &self,
        exec: &ExecutorChoice,
        table: RecordTable,
        key: RecordAddress,
        bytes: &[u8],
        expires_at: UnixMillis,
    ) -> Result<bool, LedgerError> {
        let backend = exec.backend();
        let offset = self.offset();
        self.reclaim(exec, table).await?;

        let held = exec
            .query_one(sea_orm::Statement::from_sql_and_values(
                backend,
                select_live_sql(backend, table).map_err(ledger_error)?,
                vec![
                    Value::from(key.scope.clone()),
                    Value::from(key.member.clone()),
                    Value::from(offset),
                ],
            ))
            .await
            .map_err(|error| ledger_db_error(&error))?;
        if held.is_some() {
            return Ok(false);
        }

        exec.run(sea_orm::Statement::from_sql_and_values(
            backend,
            delete_elapsed_sql(backend, table).map_err(ledger_error)?,
            vec![
                Value::from(key.scope.clone()),
                Value::from(key.member.clone()),
                Value::from(offset),
            ],
        ))
        .await
        .map_err(|error| ledger_db_error(&error))?;

        match exec
            .run(sea_orm::Statement::from_sql_and_values(
                backend,
                insert_record_sql(backend, table).map_err(ledger_error)?,
                vec![
                    Value::from(key.scope),
                    Value::from(key.member),
                    Value::from(bytes.to_vec()),
                    Value::from(as_i64(expires_at.get())),
                ],
            ))
            .await
        {
            Ok(_) => Ok(true),
            Err(error) if is_unique_violation(&error.to_string(), table.name()) => Ok(false),
            Err(error) => Err(ledger_db_error(&error)),
        }
    }

    /// Drops at most [`RECLAIM_BATCH`] elapsed records from `table`, oldest
    /// deadline first.
    ///
    /// The victims are chosen by an ordered read and then deleted by their
    /// own primary keys rather than by re-running the expiry predicate as a
    /// delete: that keeps the delete bounded to exactly the rows this
    /// operation looked at, on every dialect, and needs no subquery over the
    /// table a `DELETE` targets - which MySQL refuses outright (error 1093).
    ///
    /// It runs in whatever transaction the operation runs in, so inside a
    /// host transaction this delete takes row locks that are held until the
    /// *host* commits, and is rolled back with the host if it does not. That
    /// is why the batch is small and why reclamation is only ever hygiene:
    /// a rolled-back host transaction leaves the elapsed rows exactly where
    /// they were, and the next creating operation reclaims them instead.
    /// Nothing reads a reclaimed row in the meantime - every read and guard
    /// refuses a record past its deadline whether or not a batch has reached
    /// it.
    async fn reclaim(&self, exec: &ExecutorChoice, table: RecordTable) -> Result<(), LedgerError> {
        let backend = exec.backend();
        let limit = i64::try_from(RECLAIM_BATCH).unwrap_or(i64::MAX);
        let rows = exec
            .query_all(sea_orm::Statement::from_sql_and_values(
                backend,
                select_elapsed_sql(backend, table).map_err(ledger_error)?,
                vec![Value::from(self.offset()), Value::from(limit)],
            ))
            .await
            .map_err(|error| ledger_db_error(&error))?;
        if rows.is_empty() {
            return Ok(());
        }
        let mut victims: Vec<Value> = Vec::with_capacity(rows.len() * 2);
        for row in &rows {
            let scope: String = row
                .try_get_by_index(0)
                .map_err(|error| ledger_db_error(&error))?;
            let member: String = row
                .try_get_by_index(1)
                .map_err(|error| ledger_db_error(&error))?;
            victims.push(Value::from(scope));
            victims.push(Value::from(member));
        }
        exec.run(sea_orm::Statement::from_sql_and_values(
            backend,
            delete_batch_sql(backend, table, rows.len()).map_err(ledger_error)?,
            victims,
        ))
        .await
        .map_err(|error| ledger_db_error(&error))?;
        Ok(())
    }

    /// The compare-and-store body, run on one executor already inside this
    /// operation's transaction.
    ///
    /// A locked read decides the cheap answers - nothing there, or elapsed,
    /// is [`CasOutcome::Missing`]; a different version is
    /// [`CasOutcome::Conflict`] - and holds the row for the guarded update
    /// that follows. The update's outcome is then confirmed by re-reading
    /// the version rather than by an affected-row count, whose meaning
    /// differs by dialect and even by driver flag. The re-read is
    /// unambiguous because the version it must see is one past the version
    /// the locked read observed, and nothing else can move the row while
    /// this transaction holds it.
    ///
    /// The re-read carries the update's own store-time guard, so a record
    /// that elapses between the locked read and the update - which
    /// PostgreSQL reaches inside one transaction, its `clock_timestamp()`
    /// advancing within it - answers [`CasOutcome::Missing`]. Nothing was
    /// written in that case, and "missing" is what the port promises for a
    /// record that is no longer there.
    async fn replace_through(
        &self,
        exec: &ExecutorChoice,
        key: &InstanceRecordKey,
        expected_version: u64,
        bytes: &[u8],
        expires_at: UnixMillis,
    ) -> Result<CasOutcome, LedgerError> {
        let backend = exec.backend();
        let address = RecordAddress::instance(key);
        let offset = self.offset();

        let Some(row) = exec
            .query_one(sea_orm::Statement::from_sql_and_values(
                backend,
                select_live_sql(backend, RecordTable::Instances).map_err(ledger_error)?,
                vec![
                    Value::from(address.scope.clone()),
                    Value::from(address.member.clone()),
                    Value::from(offset),
                ],
            ))
            .await
            .map_err(|error| ledger_db_error(&error))?
        else {
            return Ok(CasOutcome::Missing);
        };
        let current: i64 = row
            .try_get_by_index(0)
            .map_err(|error| ledger_db_error(&error))?;
        if as_u64(current) != expected_version {
            return Ok(CasOutcome::Conflict);
        }

        exec.run(sea_orm::Statement::from_sql_and_values(
            backend,
            compare_and_store_sql(backend).map_err(ledger_error)?,
            vec![
                Value::from(bytes.to_vec()),
                Value::from(as_i64(expires_at.get())),
                Value::from(address.scope.clone()),
                Value::from(address.member.clone()),
                Value::from(as_i64(expected_version)),
                Value::from(offset),
            ],
        ))
        .await
        .map_err(|error| ledger_db_error(&error))?;

        let Some(row) = exec
            .query_one(sea_orm::Statement::from_sql_and_values(
                backend,
                select_version_sql(backend).map_err(ledger_error)?,
                vec![
                    Value::from(address.scope),
                    Value::from(address.member),
                    Value::from(offset),
                ],
            ))
            .await
            .map_err(|error| ledger_db_error(&error))?
        else {
            // The record elapsed between the locked read and the guarded
            // update, which PostgreSQL can genuinely reach inside one
            // transaction because `clock_timestamp()` advances within it.
            // Nothing was written - the update carries the same guard - so
            // this is the same answer the port gives for a record that was
            // never there.
            return Ok(CasOutcome::Missing);
        };
        let stored: i64 = row
            .try_get_by_index(0)
            .map_err(|error| ledger_db_error(&error))?;
        let stored = as_u64(stored);
        if stored == expected_version.saturating_add(1) {
            Ok(CasOutcome::Stored { version: stored })
        } else {
            Ok(CasOutcome::Conflict)
        }
    }

    /// Reads one record from either table under the store-time guard.
    async fn load_from(
        &self,
        table: RecordTable,
        key: RecordAddress,
    ) -> Result<Option<StoredRecord>, LedgerError> {
        let exec = read_executor().await?;
        let backend = exec.backend();
        let Some(row) = exec
            .query_one(sea_orm::Statement::from_sql_and_values(
                backend,
                select_record_sql(backend, table).map_err(ledger_error)?,
                vec![
                    Value::from(key.scope),
                    Value::from(key.member),
                    Value::from(self.offset()),
                ],
            ))
            .await
            .map_err(|error| ledger_db_error(&error))?
        else {
            return Ok(None);
        };
        let bytes: Vec<u8> = row
            .try_get_by_index(0)
            .map_err(|error| ledger_db_error(&error))?;
        let expires_at_ms: i64 = row
            .try_get_by_index(1)
            .map_err(|error| ledger_db_error(&error))?;
        // A reservation has no version column: nothing ever replaces one, it
        // is created and it elapses, so every read of one reports the
        // version a creation gives.
        let version = match table {
            RecordTable::Instances => {
                let version: i64 = row
                    .try_get_by_index(2)
                    .map_err(|error| ledger_db_error(&error))?;
                as_u64(version)
            }
            RecordTable::Promotions => FIRST_VERSION,
        };
        Ok(Some(StoredRecord {
            bytes,
            version,
            expires_at: UnixMillis::new(as_u64(expires_at_ms)),
        }))
    }

    /// Creates one record in either table, inside the caller's transaction
    /// when one is open and a short transaction of this operation's own
    /// otherwise.
    async fn create(
        &self,
        table: RecordTable,
        key: RecordAddress,
        bytes: &[u8],
        expires_at: UnixMillis,
    ) -> Result<bool, LedgerError> {
        // Bounds before allocation: a record over the codec's bound is
        // refused before a statement is built, a transaction is opened, or
        // its bytes are copied for binding.
        if bytes.len() > MAX_RECORD_BYTES {
            return Err(LedgerError::new(LedgerErrorKind::CapacityExceeded));
        }
        let transaction = OperationTransaction::begin().await?;
        let created = {
            // Dropped before the commit below: `Transaction::commit` unwraps
            // the shared handle and refuses while a clone is still alive.
            let exec = transaction.executor();
            self.create_through(&exec, table, key, bytes, expires_at)
                .await
        };
        transaction.finish(created).await
    }
}

/// Which of the two record tables an operation addresses.
///
/// The two hold different columns and answer different questions, but the
/// statements that create, read, and reclaim them are the same statement
/// with two names in it, so they are written once and named here.
#[derive(Clone, Copy, Eq, PartialEq)]
enum RecordTable {
    /// `suprnova_live_instances`: instance authority, versioned.
    Instances,
    /// `suprnova_live_promotions`: reserved retry identities, unversioned.
    Promotions,
}

impl RecordTable {
    /// The table name, which is a constant of this module and never a caller
    /// value.
    const fn name(self) -> &'static str {
        match self {
            Self::Instances => "suprnova_live_instances",
            Self::Promotions => "suprnova_live_promotions",
        }
    }

    /// The column naming the member within a scope.
    const fn member_column(self) -> &'static str {
        match self {
            Self::Instances => "instance",
            Self::Promotions => "idempotency",
        }
    }
}

/// One record's address as the columns store it: lowercase hex of the scope
/// and of the member identity within it.
///
/// Hex rather than raw bytes for the reason the generation ledger's own
/// identity column is hex: a fixed-width lowercase string compares and
/// indexes identically on all three backends, and it is exactly as wide as
/// the `CHAR` column the migration declares.
struct RecordAddress {
    scope: String,
    member: String,
}

impl RecordAddress {
    fn instance(key: &InstanceRecordKey) -> Self {
        Self {
            scope: hex::encode(key.scope.as_bytes()),
            member: hex::encode(key.instance_id.as_bytes()),
        }
    }

    fn promotion(key: &PromotionRecordKey) -> Self {
        Self {
            scope: hex::encode(key.scope.as_bytes()),
            member: hex::encode(key.idempotency_key.as_bytes()),
        }
    }
}

/// The transaction one record operation runs in.
enum OperationTransaction {
    /// The host's own, joined rather than opened. It is the host's to commit
    /// or roll back, and this store neither ends it nor may end it: that is
    /// the coupling a database tier is chosen for.
    Ambient(Transaction),
    /// This operation's own, opened because no host transaction was in
    /// scope, and closed before the operation returns.
    Owned(Transaction),
}

impl OperationTransaction {
    async fn begin() -> Result<Self, LedgerError> {
        match Transaction::current() {
            Some(tx) => Ok(Self::Ambient(tx)),
            None => Ok(Self::Owned(
                DB::begin_transaction().await.map_err(ledger_error)?,
            )),
        }
    }

    fn executor(&self) -> ExecutorChoice {
        match self {
            Self::Ambient(tx) | Self::Owned(tx) => ExecutorChoice::from_tx(tx),
        }
    }

    /// Ends this operation's own transaction around `outcome`, and leaves
    /// the host's alone.
    async fn finish<T>(self, outcome: Result<T, LedgerError>) -> Result<T, LedgerError> {
        match self {
            Self::Ambient(_) => outcome,
            Self::Owned(tx) => match outcome {
                Ok(value) => {
                    tx.commit().await.map_err(ledger_error)?;
                    Ok(value)
                }
                Err(error) => {
                    // The operation failed; nothing it wrote may stand.
                    let _ = tx.rollback().await;
                    Err(error)
                }
            },
        }
    }
}

/// A read executor, pinned to the primary and joining the host's ambient
/// transaction when one is open.
///
/// Pinned for the same reason the generation ledger is: a replica lagging
/// behind the claim that just landed would report a record that is no longer
/// current, and instance authority read from behind is authority that can be
/// granted twice.
async fn read_executor() -> Result<ExecutorChoice, LedgerError> {
    ExecutorChoice::resolve_read(None, Some(PRIMARY_CONNECTION_NAME), None)
        .await
        .map_err(ledger_error)
}

/// A write executor, pinned to the primary and joining the host's ambient
/// transaction when one is open. Used by the single-statement operations,
/// which need no transaction of their own.
async fn write_executor() -> Result<ExecutorChoice, LedgerError> {
    ExecutorChoice::resolve_write(None, Some(PRIMARY_CONNECTION_NAME), None)
        .await
        .map_err(ledger_error)
}

/// Collapses a backend failure into the one closed provider kind the ledger
/// contract exposes, logging the cause's closed-set kind first.
///
/// The cause never reaches the caller: [`LedgerError`] carries a kind and
/// nothing else, exactly so a driver message - which can echo bound values,
/// and whose bound values here include an encoded record - can never travel
/// back into a response. It does not reach the log either. What is logged
/// is the failure's own variant name, which keeps "no primary connection
/// registered" (`kind="service_not_found"`) distinguishable from "the
/// database is down" (`kind="database"`) in whatever collects these logs
/// without a record travelling with it. Mirrors
/// [`provider_error`](super::provider_error), the same collapse for the
/// render cache's own closed kind.
fn ledger_error(error: FrameworkError) -> LedgerError {
    ledger_error_kind(framework_error_kind(&error))
}

/// [`ledger_error`] for a SeaORM failure, which is the case where an
/// encoded record would otherwise travel into the log:
/// [`db_error_kind`](super::db_error_kind) names the variant and drops the
/// driver's message.
fn ledger_db_error(error: &DbErr) -> LedgerError {
    ledger_error_kind(db_error_kind(error))
}

/// The one `warn` site behind [`ledger_error`] and [`ledger_db_error`].
///
/// `kind` is `&'static str` by signature, and that signature is the whole
/// guarantee: nothing a backend, a driver, or a caller composed at runtime
/// can be handed to it.
fn ledger_error_kind(kind: &'static str) -> LedgerError {
    tracing::warn!(
        target: "suprnova::render_cache",
        kind,
        "live instance record store provider failure",
    );
    LedgerError::new(LedgerErrorKind::ProviderUnavailable)
}

#[async_trait]
impl InstanceRecordStore for SqlInstanceRecordStore {
    async fn load(&self, key: &InstanceRecordKey) -> Result<Option<StoredRecord>, LedgerError> {
        self.load_from(RecordTable::Instances, RecordAddress::instance(key))
            .await
    }

    async fn insert_if_absent(
        &self,
        key: &InstanceRecordKey,
        bytes: &[u8],
        expires_at: UnixMillis,
    ) -> Result<bool, LedgerError> {
        self.create(
            RecordTable::Instances,
            RecordAddress::instance(key),
            bytes,
            expires_at,
        )
        .await
    }

    async fn compare_and_store(
        &self,
        key: &InstanceRecordKey,
        expected_version: u64,
        bytes: &[u8],
        expires_at: UnixMillis,
    ) -> Result<CasOutcome, LedgerError> {
        if bytes.len() > MAX_RECORD_BYTES {
            return Err(LedgerError::new(LedgerErrorKind::CapacityExceeded));
        }
        let transaction = OperationTransaction::begin().await?;
        let outcome = {
            let exec = transaction.executor();
            self.replace_through(&exec, key, expected_version, bytes, expires_at)
                .await
        };
        transaction.finish(outcome).await
    }

    async fn remove(&self, key: &InstanceRecordKey) -> Result<(), LedgerError> {
        let exec = write_executor().await?;
        let backend = exec.backend();
        let address = RecordAddress::instance(key);
        exec.run(sea_orm::Statement::from_sql_and_values(
            backend,
            delete_record_sql(backend, RecordTable::Instances).map_err(ledger_error)?,
            vec![Value::from(address.scope), Value::from(address.member)],
        ))
        .await
        .map_err(|error| ledger_db_error(&error))?;
        Ok(())
    }

    async fn load_promotion(
        &self,
        key: &PromotionRecordKey,
    ) -> Result<Option<StoredRecord>, LedgerError> {
        self.load_from(RecordTable::Promotions, RecordAddress::promotion(key))
            .await
    }

    async fn insert_promotion_if_absent(
        &self,
        key: &PromotionRecordKey,
        bytes: &[u8],
        expires_at: UnixMillis,
    ) -> Result<bool, LedgerError> {
        self.create(
            RecordTable::Promotions,
            RecordAddress::promotion(key),
            bytes,
            expires_at,
        )
        .await
    }

    async fn count_instances(&self) -> Result<usize, LedgerError> {
        let exec = read_executor().await?;
        let backend = exec.backend();
        let row = exec
            .query_one(sea_orm::Statement::from_sql_and_values(
                backend,
                count_live_sql(backend).map_err(ledger_error)?,
                vec![Value::from(self.offset())],
            ))
            .await
            .map_err(|error| ledger_db_error(&error))?
            .ok_or_else(|| {
                ledger_error(FrameworkError::database(
                    "live instance count returned no row".to_owned(),
                ))
            })?;
        let live: i64 = row
            .try_get_by_index(0)
            .map_err(|error| ledger_db_error(&error))?;
        Ok(usize::try_from(live).unwrap_or(usize::MAX))
    }
}

/// `SELECT` for one live record's payload: the row exists and its deadline
/// has not passed by store time.
///
/// The store-time expression is interpolated because it is one of three
/// constants chosen by a closed match on the backend, and the table and
/// column names because they are constants of this module. Every caller
/// value is bound.
fn select_record_sql(backend: DbBackend, table: RecordTable) -> Result<String, FrameworkError> {
    let now = sql_now_ms(backend)?;
    let (name, member) = (table.name(), table.member_column());
    let columns = match table {
        RecordTable::Instances => "record, expires_at_ms, version",
        RecordTable::Promotions => "record, expires_at_ms",
    };
    Ok(format!(
        "SELECT {columns} FROM {name} WHERE scope = {} AND {member} = {} \
         AND expires_at_ms > ({now}) + {}",
        bind(backend, 1)?,
        bind(backend, 2)?,
        bind(backend, 3)?
    ))
}

/// `SELECT` that answers "an unexpired record holds this key", locked for
/// the write about to follow where the dialect can lock it.
///
/// It reads `version` on the instances table because compare-and-store needs
/// the version it is about to replace, and a constant on the promotions
/// table, which has no version to read: what both callers need from it is
/// whether there is a row at all.
fn select_live_sql(backend: DbBackend, table: RecordTable) -> Result<String, FrameworkError> {
    let now = sql_now_ms(backend)?;
    let (name, member) = (table.name(), table.member_column());
    let column = match table {
        RecordTable::Instances => "version",
        RecordTable::Promotions => "1",
    };
    Ok(format!(
        "SELECT {column} FROM {name} WHERE scope = {} AND {member} = {} \
         AND expires_at_ms > ({now}) + {}{}",
        bind(backend, 1)?,
        bind(backend, 2)?,
        bind(backend, 3)?,
        row_lock(backend)
    ))
}

/// `SELECT` for the version a compare-and-store just wrote, under the same
/// store-time guard as the update it confirms.
///
/// The guard is not redundant. PostgreSQL's `clock_timestamp()` advances
/// inside a transaction, so a record whose deadline sits between the locked
/// read and the update genuinely elapses mid-operation; the update refuses
/// it, and this read must then say `Missing` rather than compare a version
/// that no longer holds anything and call it a conflict.
fn select_version_sql(backend: DbBackend) -> Result<String, FrameworkError> {
    let now = sql_now_ms(backend)?;
    Ok(format!(
        "SELECT version FROM suprnova_live_instances \
         WHERE scope = {} AND instance = {} AND expires_at_ms > ({now}) + {}",
        bind(backend, 1)?,
        bind(backend, 2)?,
        bind(backend, 3)?
    ))
}

/// `INSERT` for a record at a key nothing holds.
///
/// Deliberately not an upsert. A conflict means a peer created the record
/// between this transaction's read and its write, and the answer to that is
/// that the peer's record stands - not that this caller should overwrite it.
fn insert_record_sql(backend: DbBackend, table: RecordTable) -> Result<String, FrameworkError> {
    let (name, member) = (table.name(), table.member_column());
    let (columns, version) = match table {
        RecordTable::Instances => (", version", format!(", {FIRST_VERSION}")),
        RecordTable::Promotions => ("", String::new()),
    };
    Ok(format!(
        "INSERT INTO {name} (scope, {member}, record{columns}, expires_at_ms) \
         VALUES ({}, {}, {}{version}, {})",
        bind(backend, 1)?,
        bind(backend, 2)?,
        bind(backend, 3)?,
        bind(backend, 4)?
    ))
}

/// The guarded replacement: the record moves only while it still carries the
/// version the caller read and is still there by store time.
///
/// The expiry guard is not redundant with the version guard. Without it an
/// elapsed record would be replaced back into life at a fresh deadline,
/// which would turn "this record is gone" into "this record is current"
/// across every node reading it.
fn compare_and_store_sql(backend: DbBackend) -> Result<String, FrameworkError> {
    let now = sql_now_ms(backend)?;
    Ok(format!(
        "UPDATE suprnova_live_instances SET record = {}, version = version + 1, \
         expires_at_ms = {} WHERE scope = {} AND instance = {} AND version = {} \
         AND expires_at_ms > ({now}) + {}",
        bind(backend, 1)?,
        bind(backend, 2)?,
        bind(backend, 3)?,
        bind(backend, 4)?,
        bind(backend, 5)?,
        bind(backend, 6)?
    ))
}

/// `DELETE` for one record, whatever its deadline.
fn delete_record_sql(backend: DbBackend, table: RecordTable) -> Result<String, FrameworkError> {
    let (name, member) = (table.name(), table.member_column());
    Ok(format!(
        "DELETE FROM {name} WHERE scope = {} AND {member} = {}",
        bind(backend, 1)?,
        bind(backend, 2)?
    ))
}

/// `DELETE` for one record only if store time has passed its deadline, so a
/// creation can reuse a key its previous record no longer holds without ever
/// dropping one that is still live.
fn delete_elapsed_sql(backend: DbBackend, table: RecordTable) -> Result<String, FrameworkError> {
    let now = sql_now_ms(backend)?;
    let (name, member) = (table.name(), table.member_column());
    Ok(format!(
        "DELETE FROM {name} WHERE scope = {} AND {member} = {} \
         AND expires_at_ms <= ({now}) + {}",
        bind(backend, 1)?,
        bind(backend, 2)?,
        bind(backend, 3)?
    ))
}

/// `SELECT` for one bounded batch of elapsed records, oldest deadline first.
/// The migration's index on `expires_at_ms` is what keeps this ordered read
/// from scanning the table.
fn select_elapsed_sql(backend: DbBackend, table: RecordTable) -> Result<String, FrameworkError> {
    let now = sql_now_ms(backend)?;
    let (name, member) = (table.name(), table.member_column());
    Ok(format!(
        "SELECT scope, {member} FROM {name} WHERE expires_at_ms <= ({now}) + {} \
         ORDER BY expires_at_ms LIMIT {}",
        bind(backend, 1)?,
        bind(backend, 2)?
    ))
}

/// `DELETE` for exactly the `count` records a batch selected, addressed by
/// their own primary keys.
///
/// One statement rather than `count` of them, and an explicit list of keys
/// rather than a subquery over the table being deleted from: the first keeps
/// reclamation to one round trip, and the second is what MySQL requires
/// (error 1093 refuses to read the target table in a subquery). Every key is
/// bound.
fn delete_batch_sql(
    backend: DbBackend,
    table: RecordTable,
    count: usize,
) -> Result<String, FrameworkError> {
    let (name, member) = (table.name(), table.member_column());
    let mut clauses = Vec::with_capacity(count);
    for row in 0..count {
        clauses.push(format!(
            "(scope = {} AND {member} = {})",
            bind(backend, row * 2 + 1)?,
            bind(backend, row * 2 + 2)?
        ));
    }
    Ok(format!("DELETE FROM {name} WHERE {}", clauses.join(" OR ")))
}

/// `SELECT` counting the instance records that are still there by store
/// time, which is what the kernel's configured capacity is measured against.
fn count_live_sql(backend: DbBackend) -> Result<String, FrameworkError> {
    let now = sql_now_ms(backend)?;
    Ok(format!(
        "SELECT COUNT(*) FROM suprnova_live_instances WHERE expires_at_ms > ({now}) + {}",
        bind(backend, 1)?
    ))
}

#[cfg(test)]
mod tests {
    //! The record statements' shape per dialect.
    //!
    //! Behaviour is proven against a real database in
    //! `framework/tests/render_cache/tiers/sql.rs`, including the whole engine
    //! ledger conformance suite over this store. What cannot be proven there
    //! on SQLite alone is that the PostgreSQL and MySQL spellings say the
    //! same thing: the numbered placeholders, the row-locking read, and the
    //! store-time guards that decide what a record still is.
    use super::*;

    const DIALECTS: [DbBackend; 3] = [DbBackend::Postgres, DbBackend::MySql, DbBackend::Sqlite];

    #[test]
    fn the_statements_render_their_placeholders_through_the_shared_binder() {
        let postgres =
            select_record_sql(DbBackend::Postgres, RecordTable::Instances).expect("postgres");
        assert!(postgres.contains("scope = $1"), "{postgres}");
        assert!(postgres.contains("instance = $2"), "{postgres}");
        assert!(postgres.contains("+ $3"), "{postgres}");

        for backend in [DbBackend::MySql, DbBackend::Sqlite] {
            let sql = select_record_sql(backend, RecordTable::Instances).expect("a dialect");
            assert!(sql.contains("scope = ?"), "{sql}");
            assert!(!sql.contains('$'), "{sql}");
        }
    }

    #[test]
    fn every_read_and_guard_measures_the_deadline_on_the_database_clock() {
        for backend in DIALECTS {
            let now = sql_now_ms(backend).expect("now");
            for table in [RecordTable::Instances, RecordTable::Promotions] {
                for sql in [
                    select_record_sql(backend, table).expect("a dialect"),
                    select_live_sql(backend, table).expect("a dialect"),
                    delete_elapsed_sql(backend, table).expect("a dialect"),
                    select_elapsed_sql(backend, table).expect("a dialect"),
                ] {
                    assert!(
                        sql.contains(&format!("({now})")),
                        "an expiry decided on a node clock is not an expiry: {sql}"
                    );
                }
            }
            assert!(
                compare_and_store_sql(backend)
                    .expect("a dialect")
                    .contains(&format!("({now})")),
                "a replacement must refuse an elapsed record"
            );
            assert!(
                select_version_sql(backend)
                    .expect("a dialect")
                    .contains(&format!("({now})")),
                "and its confirming re-read must refuse one too, or a record \
                 that elapsed mid-transaction reads back as a conflict"
            );
            assert!(
                count_live_sql(backend)
                    .expect("a dialect")
                    .contains(&format!("({now})"))
            );
        }
    }

    #[test]
    fn the_deciding_read_locks_the_row_where_the_dialect_can_lock_one() {
        for backend in [DbBackend::Postgres, DbBackend::MySql] {
            assert!(
                select_live_sql(backend, RecordTable::Instances)
                    .expect("a dialect")
                    .ends_with("FOR UPDATE")
            );
        }
        assert!(
            !select_live_sql(DbBackend::Sqlite, RecordTable::Instances)
                .expect("sqlite")
                .contains("FOR UPDATE")
        );
    }

    #[test]
    fn a_record_is_inserted_and_never_upserted() {
        for backend in DIALECTS {
            for table in [RecordTable::Instances, RecordTable::Promotions] {
                let sql = insert_record_sql(backend, table).expect("a dialect");
                assert!(
                    sql.starts_with(&format!("INSERT INTO {}", table.name())),
                    "{sql}"
                );
                assert!(
                    !sql.contains("CONFLICT")
                        && !sql.contains("DUPLICATE")
                        && !sql.contains("IGNORE"),
                    "a collision means a peer's record stands: {sql}"
                );
            }
            // A reservation has no version column, so neither has its insert.
            assert!(
                !insert_record_sql(backend, RecordTable::Promotions)
                    .expect("a dialect")
                    .contains("version"),
            );
            assert!(
                insert_record_sql(backend, RecordTable::Instances)
                    .expect("a dialect")
                    .contains(", version"),
            );
        }
    }

    #[test]
    fn the_replacement_carries_the_version_it_read_and_advances_it_by_one() {
        for backend in DIALECTS {
            let sql = compare_and_store_sql(backend).expect("a dialect");
            assert!(sql.contains("version = version + 1"), "{sql}");
            assert!(
                sql.contains(&format!("version = {}", bind(backend, 5).expect("a bind"))),
                "the update must replace exactly the version the caller read: {sql}"
            );
        }
    }

    #[test]
    fn reclamation_deletes_the_batch_it_selected_by_key() {
        for backend in DIALECTS {
            let selected = select_elapsed_sql(backend, RecordTable::Instances).expect("a dialect");
            assert!(selected.contains("ORDER BY expires_at_ms"), "{selected}");
            assert!(selected.contains("LIMIT"), "{selected}");

            let deleted = delete_batch_sql(backend, RecordTable::Instances, 2).expect("a dialect");
            assert_eq!(
                deleted.matches("scope = ").count(),
                2,
                "one key per selected row, and nothing else: {deleted}"
            );
            assert!(
                !deleted.contains("SELECT"),
                "MySQL refuses a subquery over the table a DELETE targets: {deleted}"
            );
            assert!(
                !deleted.contains("expires_at_ms"),
                "the batch is deleted by key, not by re-running the expiry predicate: {deleted}"
            );
        }
    }
}
