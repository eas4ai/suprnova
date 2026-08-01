//! Bring `failed_jobs` in line with what `DatabaseFailedJobStore` writes.
//!
//! The table shipped in `m_2026_08_01_queue_tables` was transcribed by
//! hand and got it wrong: no `connection` column, and `queue` nullable
//! where the store always supplies a value. The store's INSERT names all
//! seven columns, so every dead-letter failed with
//!
//! ```text
//! column "connection" of relation "failed_jobs" does not exist
//! ```
//!
//! and the failure mode is nasty. `handle_dead_letter` deliberately does
//! *not* ack when the store rejects a record — leaving the reservation
//! intact is the safe choice, since the alternative is deleting work
//! nobody recorded. But the job is out of attempts, so the next worker
//! re-claims it on visibility expiry, hits the same guard, fails the same
//! insert, and returns it again. Measured: `attempts` climbing 2 → 9 → 16
//! across two 60-second rounds, a job that would never leave, and a table
//! that stayed empty.
//!
//! It surfaced only once the daemons had a tracing subscriber. Before
//! that the error existed and had nowhere to be printed.

use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m_2026_08_02_failed_jobs_schema"
    }
}

#[derive(DeriveIden)]
enum FailedJobs {
    Table,
    Connection,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(FailedJobs::Table)
                    // Defaulted rather than nullable: the store always
                    // writes a value, and a NULL here would only ever mean
                    // "row inserted by something that was not the store".
                    .add_column(
                        ColumnDef::new(FailedJobs::Connection)
                            .string()
                            .not_null()
                            .default("default"),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(FailedJobs::Table)
                    .drop_column(FailedJobs::Connection)
                    .to_owned(),
            )
            .await
    }
}
