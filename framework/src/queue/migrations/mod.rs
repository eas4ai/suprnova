//! Framework-owned migrations for the database queue's tables.
//!
//! The tables take Laravel 13's layouts (`jobs.stub`, `batches.stub`,
//! `failed_jobs.stub`), so a Suprnova application runs on a database
//! Laravel created, and its rows read the same to both:
//!
//! - [`CreateJobsTable`] creates `jobs`, and the reservations table the
//!   database driver keeps beside it (see [`crate::queue::database`]).
//! - [`CreateJobBatchesTable`] creates `job_batches`, and
//!   `job_batch_settlements`, where the batch repository records which jobs
//!   settled.
//! - [`CreateFailedJobsTable`] creates `failed_jobs`.
//!
//! Each one creates its tables when they are missing, leaves a table that
//! is already in Laravel's layout exactly as it is (Laravel's own migration
//! may have created it), and reshapes a table an earlier Suprnova release
//! documented, with its rows: a queued, delayed or reserved job stays
//! queued, delayed or reserved, a failed job keeps its id, and a batch
//! keeps its counts. A reshape that stops part way resumes on the next
//! `migrate`, and moves no row twice.
//!
//! The jobs and failed-jobs tables are named by `QUEUE_DB_TABLE` and
//! `QUEUE_FAILED_DB_TABLE`, as the queue reads them at boot. Applications
//! register the three in their own `Migrator`, after their own
//! migrations:
//!
//! ```rust,no_run
//! use sea_orm_migration::MigratorTrait;
//! use suprnova::queue::migrations::{
//!     CreateFailedJobsTable, CreateJobBatchesTable, CreateJobsTable,
//! };
//!
//! pub struct Migrator;
//!
//! impl MigratorTrait for Migrator {
//!     fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
//!         vec![
//!             // ... the app's own migrations ...
//!             Box::new(CreateJobsTable),
//!             Box::new(CreateJobBatchesTable),
//!             Box::new(CreateFailedJobsTable),
//!         ]
//!     }
//! }
//! ```

use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::{QueryResult, Statement, Value};

use crate::database::catalog::{CatalogColumn, column};
use crate::database::migration_guard::{
    MovedRow, UpgradeState, earlier_table_name, move_earlier_rows, quote, resume_set_aside,
    set_aside, upgrade_state,
};
use crate::database::placeholder::placeholder_list;
use crate::queue::envelope::Envelope;
use crate::schema::Schema;

/// The jobs table the queue reads: `QUEUE_DB_TABLE`, or `jobs`.
fn jobs_table() -> String {
    std::env::var("QUEUE_DB_TABLE").unwrap_or_else(|_| "jobs".to_owned())
}

/// The failed-jobs table the queue reads: `QUEUE_FAILED_DB_TABLE`, or
/// `failed_jobs`.
fn failed_jobs_table() -> String {
    std::env::var("QUEUE_FAILED_DB_TABLE").unwrap_or_else(|_| "failed_jobs".to_owned())
}

fn checked(table: String) -> Result<String, DbErr> {
    crate::database::validate_identifier(&table).map_err(|e| DbErr::Migration(e.to_string()))?;
    Ok(table)
}

/// The earlier layout of each queue table held the envelope in
/// `envelope_json` (jobs, failed jobs) or the options in `options_json`
/// (batches); Laravel's names neither.
fn has_column(columns: &[CatalogColumn], name: &str) -> bool {
    column(columns, name).is_some()
}

/// Read an integer however wide the earlier table stored it.
fn int(row: &QueryResult, name: &str) -> Result<Option<i64>, DbErr> {
    if let Ok(value) = row.try_get::<Option<i64>>("", name) {
        return Ok(value);
    }
    if let Ok(value) = row.try_get::<Option<i32>>("", name) {
        return Ok(value.map(i64::from));
    }
    row.try_get::<Option<u64>>("", name)?
        .map(|value| {
            <i64 as TryFrom<u64>>::try_from(value)
                .map_err(|_| DbErr::Migration(format!("{name} {value} is out of range")))
        })
        .transpose()
}

fn insert(
    backend: sea_orm_migration::sea_orm::DbBackend,
    table: &str,
    columns: &[&str],
    values: Vec<Value>,
) -> Result<Statement, DbErr> {
    let placeholders =
        placeholder_list(backend, 1, columns.len()).map_err(|e| DbErr::Migration(e.to_string()))?;
    Ok(Statement::from_sql_and_values(
        backend,
        format!(
            "INSERT INTO {} ({}) VALUES ({placeholders})",
            quote(backend, table),
            columns.join(", ")
        ),
        values,
    ))
}

// ---------------------------------------------------------------------------
// failed_jobs
// ---------------------------------------------------------------------------

/// Creates the `failed_jobs` table in Laravel 13's layout, or reshapes one
/// in the earlier Suprnova layout (`id` text, `job_name`, `envelope_json`,
/// epoch `failed_at`) with its rows. Each record keeps its id as `uuid`, so
/// `queue:retry <id>` and `queue:forget <id>` still find it.
///
/// `failed_at` is `DATETIME` on MySQL, where Laravel's migration says
/// `TIMESTAMP`, the framework's rule for a time column: `TIMESTAMP` refuses
/// any time after 2038-01-19.
pub struct CreateFailedJobsTable;

impl MigrationName for CreateFailedJobsTable {
    fn name(&self) -> &str {
        "m20261005_000001_create_failed_jobs_table"
    }
}

fn failed_jobs_is_earlier(columns: &[CatalogColumn]) -> bool {
    has_column(columns, "envelope_json")
}

async fn create_failed_jobs(manager: &SchemaManager<'_>, table: &str) -> Result<(), DbErr> {
    Schema::create(manager, table, |t| {
        t.unsigned_id();
        t.string("uuid").unique();
        t.string("connection");
        t.string("queue");
        t.long_text("payload");
        t.long_text("exception");
        t.date_time("failed_at").precision(0).use_current();
        t.index(&["connection", "queue", "failed_at"]);
    })
    .await
}

#[async_trait::async_trait]
impl MigrationTrait for CreateFailedJobsTable {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let table = checked(failed_jobs_table())?;
        match upgrade_state(manager, &table, failed_jobs_is_earlier).await? {
            UpgradeState::Untouched => return Ok(()),
            UpgradeState::Fresh => return create_failed_jobs(manager, &table).await,
            UpgradeState::Earlier => {
                set_aside(manager, &table).await?;
                create_failed_jobs(manager, &table).await?;
            }
            UpgradeState::Resume => {
                resume_set_aside(manager, &table, failed_jobs_is_earlier).await?;
                if !manager.has_table(&table).await? {
                    create_failed_jobs(manager, &table).await?;
                }
            }
        }
        let backend = manager.get_database_backend();
        let target = table.clone();
        move_earlier_rows(
            manager,
            &table,
            "id, connection, queue, job_name, envelope_json, exception, failed_at",
            "id",
            |row| {
                let id: String = row.try_get("", "id")?;
                let connection: String = row.try_get("", "connection")?;
                let queue: String = row.try_get("", "queue")?;
                let job_name: String = row.try_get("", "job_name")?;
                let envelope_json: String = row.try_get("", "envelope_json")?;
                let exception: String = row.try_get("", "exception")?;
                let failed_at = int(row, "failed_at")?.unwrap_or_default();
                let failed_at = chrono::DateTime::<chrono::Utc>::from_timestamp(failed_at, 0)
                    .ok_or_else(|| {
                        DbErr::Migration(format!(
                            "failed job {id}: failed_at {failed_at} is not a time"
                        ))
                    })?
                    .naive_utc();
                let connection =
                    if connection.starts_with(crate::queue::failed::SUPRNOVA_CONNECTION_PREFIX) {
                        connection
                    } else {
                        format!(
                            "{}{connection}",
                            crate::queue::failed::SUPRNOVA_CONNECTION_PREFIX
                        )
                    };
                Ok(MovedRow {
                    key: id.clone().into(),
                    writes: vec![insert(
                        backend,
                        &target,
                        &[
                            "uuid",
                            "connection",
                            "queue",
                            "payload",
                            "exception",
                            "failed_at",
                        ],
                        vec![
                            id.clone().into(),
                            connection.into(),
                            queue.into(),
                            earlier_failed_payload(&envelope_json, &job_name, &id).into(),
                            exception.into(),
                            failed_at.into(),
                        ],
                    )?],
                })
            },
        )
        .await?;
        Ok(())
    }

    /// Leaves the table: it may hold failed jobs, and Laravel may read it.
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}

/// An earlier record's envelope with `displayName` and `uuid` added, as
/// the store writes a payload now. Text that is no JSON object is kept as
/// it is, so a damaged record still moves.
fn earlier_failed_payload(envelope_json: &str, job_name: &str, uuid: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(envelope_json) {
        Ok(serde_json::Value::Object(mut object)) => {
            object.insert(
                "displayName".to_owned(),
                serde_json::Value::String(job_name.to_owned()),
            );
            object.insert(
                "uuid".to_owned(),
                serde_json::Value::String(uuid.to_owned()),
            );
            serde_json::to_string(&serde_json::Value::Object(object))
                .unwrap_or_else(|_| envelope_json.to_owned())
        }
        _ => envelope_json.to_owned(),
    }
}

// ---------------------------------------------------------------------------
// jobs
// ---------------------------------------------------------------------------

/// Creates the `jobs` table in Laravel 13's layout and the reservations
/// table the database driver keeps beside it, or reshapes a `jobs` table
/// in the earlier Suprnova layout (`id` text, `job_name`, `envelope_json`,
/// `reserved_until`, `reserved_token`) with its rows.
///
/// A job reserved when the reshape runs stays reserved by the same token
/// until the same moment, so the worker holding it settles it as before
/// and no other worker takes it first. An unrouted job lands on the queue
/// [`LaravelDatabase::default_queue`](crate::LaravelDatabase::default_queue)
/// names.
pub struct CreateJobsTable;

impl MigrationName for CreateJobsTable {
    fn name(&self) -> &str {
        "m20261005_000002_create_jobs_table"
    }
}

fn jobs_is_earlier(columns: &[CatalogColumn]) -> bool {
    has_column(columns, "envelope_json")
}

async fn create_jobs(manager: &SchemaManager<'_>, table: &str) -> Result<(), DbErr> {
    Schema::create(manager, table, |t| {
        t.unsigned_id();
        t.string("queue").index();
        t.long_text("payload");
        t.unsigned_small_integer("attempts");
        t.unsigned_integer("reserved_at").nullable();
        t.unsigned_integer("available_at");
        t.unsigned_integer("created_at");
    })
    .await
}

async fn create_reservations(manager: &SchemaManager<'_>, jobs: &str) -> Result<(), DbErr> {
    let reservations = crate::queue::database::reservations_table(jobs);
    if manager.has_table(&reservations).await? {
        return Ok(());
    }
    Schema::create(manager, &reservations, |t| {
        t.unsigned_big_integer("job_id").primary();
        t.char("token", 36).unique();
        t.big_integer("reserved_until");
    })
    .await
}

#[async_trait::async_trait]
impl MigrationTrait for CreateJobsTable {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let table = checked(jobs_table())?;
        let state = upgrade_state(manager, &table, jobs_is_earlier).await?;
        match state {
            UpgradeState::Untouched => return create_reservations(manager, &table).await,
            UpgradeState::Fresh => {
                create_jobs(manager, &table).await?;
                return create_reservations(manager, &table).await;
            }
            UpgradeState::Earlier => {
                set_aside(manager, &table).await?;
                create_jobs(manager, &table).await?;
            }
            UpgradeState::Resume => {
                resume_set_aside(manager, &table, jobs_is_earlier).await?;
                if !manager.has_table(&table).await? {
                    create_jobs(manager, &table).await?;
                }
            }
        }
        create_reservations(manager, &table).await?;

        // The oldest layouts lacked `queue`, and the reservation columns
        // came later still; each one missing reads as NULL.
        let earlier = crate::database::catalog::table_columns(
            manager.get_connection(),
            &earlier_table_name(&table),
        )
        .await?;
        let optional = |name: &str| {
            if has_column(&earlier, name) {
                name.to_owned()
            } else {
                format!("NULL AS {name}")
            }
        };
        let select = format!(
            "id, envelope_json, available_at, attempts, created_at, {}, {}, {}",
            optional("queue"),
            optional("reserved_until"),
            optional("reserved_token"),
        );
        let backend = manager.get_database_backend();
        let reservations = crate::queue::database::reservations_table(&table);
        let now = crate::clock::now().timestamp();
        let target = table.clone();
        move_earlier_rows(manager, &table, &select, "id", |row| {
            let id: String = row.try_get("", "id")?;
            let envelope_json: String = row.try_get("", "envelope_json")?;
            let queue: Option<String> = row.try_get("", "queue")?;
            let available_at = int(row, "available_at")?.unwrap_or(now);
            let created_at = int(row, "created_at")?.unwrap_or(available_at);
            let attempts = Ord::max(int(row, "attempts")?.unwrap_or_default(), 0);
            let reserved_until = int(row, "reserved_until")?;
            let reserved_token: Option<String> = row.try_get("", "reserved_token")?;
            let payload = match Envelope::from_json(&envelope_json) {
                Ok(env) => crate::queue::database::job_payload(&env)
                    .map_err(|e| DbErr::Migration(format!("job {id}: {e}")))?,
                // A row that does not decode is kept as it was; the driver
                // reports it as unparseable rather than losing it.
                Err(_) => envelope_json,
            };
            let queue = match queue.as_deref() {
                None | Some(crate::queue::envelope::DEFAULT_QUEUE) => {
                    crate::LaravelDatabase::default_queue().to_owned()
                }
                Some(name) => name.to_owned(),
            };
            let reservation = match (reserved_until, reserved_token) {
                (Some(until), Some(token)) => Some((until, token)),
                _ => None,
            };
            let mut writes = vec![insert(
                backend,
                &target,
                &[
                    "queue",
                    "payload",
                    "attempts",
                    "reserved_at",
                    "available_at",
                    "created_at",
                ],
                vec![
                    queue.into(),
                    payload.clone().into(),
                    Ord::min(
                        attempts,
                        i64::from(crate::queue::database::MAX_STORED_ATTEMPTS),
                    )
                    .into(),
                    reservation.as_ref().map(|_| now).into(),
                    available_at.into(),
                    created_at.into(),
                ],
            )?];
            if let Some((until, token)) = reservation {
                // The new row's id is not known here; its payload is unique,
                // since it carries the envelope id.
                writes.push(Statement::from_sql_and_values(
                    backend,
                    format!(
                        "INSERT INTO {} (job_id, token, reserved_until) \
                         SELECT id, {} FROM {} WHERE payload = {}",
                        quote(backend, &reservations),
                        placeholder_list(backend, 1, 2)
                            .map_err(|e| DbErr::Migration(e.to_string()))?,
                        quote(backend, &target),
                        placeholder_list(backend, 3, 1)
                            .map_err(|e| DbErr::Migration(e.to_string()))?,
                    ),
                    vec![token.into(), until.into(), payload.into()],
                ));
            }
            Ok(MovedRow {
                key: id.into(),
                writes,
            })
        })
        .await?;
        Ok(())
    }

    /// Leaves the tables: they may hold queued jobs, and Laravel may read
    /// `jobs`.
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// job_batches
// ---------------------------------------------------------------------------

/// Creates `job_batches` in Laravel 13's layout and the framework's
/// `job_batch_settlements`, or reshapes a `job_batches` table in the
/// earlier Suprnova layout (`options_json`, no counters) with its rows.
/// Each batch's `pending_jobs`, `failed_jobs` and `failed_job_ids` are
/// filled from its settlement rows, which stay where they are.
pub struct CreateJobBatchesTable;

impl MigrationName for CreateJobBatchesTable {
    fn name(&self) -> &str {
        "m20261005_000003_create_job_batches_table"
    }
}

fn batches_is_earlier(columns: &[CatalogColumn]) -> bool {
    has_column(columns, "options_json")
}

async fn create_batches(manager: &SchemaManager<'_>, table: &str) -> Result<(), DbErr> {
    Schema::create(manager, table, |t| {
        t.string("id").primary();
        t.string("name");
        t.integer("total_jobs");
        t.integer("pending_jobs");
        t.integer("failed_jobs");
        t.long_text("failed_job_ids");
        t.medium_text("options").nullable();
        t.integer("cancelled_at").nullable();
        t.integer("created_at");
        t.integer("finished_at").nullable();
    })
    .await
}

async fn create_settlements(manager: &SchemaManager<'_>, table: &str) -> Result<(), DbErr> {
    if !manager.has_table(table).await? {
        Schema::create(manager, table, |t| {
            t.string("batch_id");
            t.string("job_id");
            t.integer("failed");
            t.big_integer("settled_at");
            t.primary(&["batch_id", "job_id"]);
        })
        .await?;
    }
    match_batch_collation(manager, table).await
}

/// On MySQL, give `job_batch_settlements` the collation of
/// `job_batches.id`, which the batch repository joins its `batch_id` with.
/// Laravel creates `job_batches` as `utf8mb4_unicode_ci`, and MySQL 8
/// refuses to compare that with its own default collation, so every
/// settlement count failed on a table Laravel created. The settlements
/// table is the framework's own, so converting it alters nothing of
/// Laravel's.
async fn match_batch_collation(manager: &SchemaManager<'_>, table: &str) -> Result<(), DbErr> {
    let connection = manager.get_connection();
    if connection.get_database_backend() != sea_orm_migration::sea_orm::DbBackend::MySql {
        return Ok(());
    }
    let collation = |table: &str, column: &str| {
        Statement::from_sql_and_values(
            sea_orm_migration::sea_orm::DbBackend::MySql,
            "SELECT COLLATION_NAME AS collation FROM information_schema.COLUMNS \
             WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ? AND COLUMN_NAME = ?",
            [table.into(), column.into()],
        )
    };
    let read = |row: Option<QueryResult>| -> Result<Option<String>, DbErr> {
        match row {
            Some(row) => row.try_get::<Option<String>>("", "collation"),
            None => Ok(None),
        }
    };
    let batches = read(
        connection
            .query_one_raw(collation(crate::queue::DEFAULT_BATCHES_TABLE, "id"))
            .await?,
    )?;
    let settlements = read(
        connection
            .query_one_raw(collation(table, "batch_id"))
            .await?,
    )?;
    let (Some(batches), Some(settlements)) = (batches, settlements) else {
        return Ok(());
    };
    if batches == settlements {
        return Ok(());
    }
    // A collation name is `<charset>_...`; the charset is what precedes the
    // first underscore. Both come from the catalog, never from input.
    let charset = batches.split('_').next().unwrap_or("utf8mb4");
    connection
        .execute_unprepared(&format!(
            "ALTER TABLE {} CONVERT TO CHARACTER SET {charset} COLLATE {batches}",
            quote(sea_orm_migration::sea_orm::DbBackend::MySql, table)
        ))
        .await?;
    Ok(())
}

/// What the settlement rows say about one batch.
#[derive(Default)]
struct Settled {
    settled: i64,
    failed: i64,
    failed_ids: Vec<String>,
}

#[async_trait::async_trait]
impl MigrationTrait for CreateJobBatchesTable {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let table = crate::queue::DEFAULT_BATCHES_TABLE;
        let settlements = crate::queue::DEFAULT_BATCH_SETTLEMENTS_TABLE;
        match upgrade_state(manager, table, batches_is_earlier).await? {
            UpgradeState::Untouched => return create_settlements(manager, settlements).await,
            UpgradeState::Fresh => {
                create_batches(manager, table).await?;
                return create_settlements(manager, settlements).await;
            }
            UpgradeState::Earlier => {
                set_aside(manager, table).await?;
                create_batches(manager, table).await?;
            }
            UpgradeState::Resume => {
                resume_set_aside(manager, table, batches_is_earlier).await?;
                if !manager.has_table(table).await? {
                    create_batches(manager, table).await?;
                }
            }
        }
        create_settlements(manager, settlements).await?;

        let backend = manager.get_database_backend();
        let mut settled: std::collections::HashMap<String, Settled> =
            std::collections::HashMap::new();
        for row in manager
            .get_connection()
            .query_all_raw(Statement::from_string(
                backend,
                format!(
                    "SELECT batch_id, job_id, failed FROM {} ORDER BY settled_at, job_id",
                    quote(backend, settlements)
                ),
            ))
            .await?
        {
            let batch_id: String = row.try_get("", "batch_id")?;
            let job_id: String = row.try_get("", "job_id")?;
            let failed = int(&row, "failed")?.unwrap_or_default() != 0;
            let entry = settled.entry(batch_id).or_default();
            entry.settled += 1;
            if failed {
                entry.failed += 1;
                entry.failed_ids.push(job_id);
            }
        }

        move_earlier_rows(
            manager,
            table,
            "id, name, total_jobs, options_json, created_at, cancelled_at, finished_at",
            "id",
            |row| {
                let id: String = row.try_get("", "id")?;
                let name: String = row.try_get("", "name")?;
                let total = int(row, "total_jobs")?.unwrap_or_default();
                let options_json: String = row.try_get("", "options_json")?;
                let created_at = int(row, "created_at")?.unwrap_or_default();
                let cancelled_at = int(row, "cancelled_at")?;
                let finished_at = int(row, "finished_at")?;
                let counts = settled.remove(&id).unwrap_or_default();
                let failed_ids = serde_json::to_string(&counts.failed_ids)
                    .map_err(|e| DbErr::Migration(format!("batch {id}: {e}")))?;
                Ok(MovedRow {
                    key: id.clone().into(),
                    writes: vec![insert(
                        backend,
                        table,
                        &[
                            "id",
                            "name",
                            "total_jobs",
                            "pending_jobs",
                            "failed_jobs",
                            "failed_job_ids",
                            "options",
                            "created_at",
                            "cancelled_at",
                            "finished_at",
                        ],
                        vec![
                            id.into(),
                            name.into(),
                            total.into(),
                            Ord::max(total - counts.settled, 0).into(),
                            counts.failed.into(),
                            failed_ids.into(),
                            crate::queue::batch::php_wrap_options_json(&options_json, backend)
                                .into(),
                            created_at.into(),
                            cancelled_at.into(),
                            finished_at.into(),
                        ],
                    )?],
                })
            },
        )
        .await?;
        Ok(())
    }

    /// Leaves the tables: they may hold batches, and Laravel may read
    /// `job_batches`.
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
