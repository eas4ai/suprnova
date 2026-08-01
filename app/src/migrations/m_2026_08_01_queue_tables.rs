//! `jobs` and `failed_jobs` — the schema `QUEUE_DRIVER=database` expects.
//!
//! The framework ships the driver but no migration for the tables it
//! reads, so today the schema exists only in `framework/tests/
//! queue_database.rs` and in a doc comment on `queue::failed`. An app that
//! sets `QUEUE_DRIVER=database` gets "no such table: jobs" at the first
//! enqueue and has to transcribe the schema by hand from source.
//!
//! This migration is that transcription for the dogfood app, and it is
//! also the argument that the framework should own it: a driver whose
//! storage nobody can provision is a driver most people will not reach.
//!
//! Column choices are the driver's, not ours — `available_at`,
//! `reserved_until` and `created_at` are epoch integers rather than
//! timestamps because that is what `DatabaseQueueDriver` reads and writes.

use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m_2026_08_01_queue_tables"
    }
}

#[derive(DeriveIden)]
enum Jobs {
    Table,
    Id,
    JobName,
    Queue,
    EnvelopeJson,
    AvailableAt,
    ReservedUntil,
    ReservedToken,
    Attempts,
    CreatedAt,
}

#[derive(DeriveIden)]
enum FailedJobs {
    Table,
    Id,
    Connection,
    JobName,
    Queue,
    EnvelopeJson,
    Exception,
    FailedAt,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Jobs::Table)
                    .if_not_exists()
                    // The envelope's UUID, stored as text — the driver
                    // generates it, so this is not auto-increment.
                    .col(ColumnDef::new(Jobs::Id).string().not_null().primary_key())
                    .col(ColumnDef::new(Jobs::JobName).string().not_null())
                    .col(ColumnDef::new(Jobs::Queue).string().null())
                    .col(ColumnDef::new(Jobs::EnvelopeJson).text().not_null())
                    .col(ColumnDef::new(Jobs::AvailableAt).big_integer().not_null())
                    .col(ColumnDef::new(Jobs::ReservedUntil).big_integer().null())
                    .col(ColumnDef::new(Jobs::ReservedToken).string().null())
                    .col(
                        ColumnDef::new(Jobs::Attempts)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(ColumnDef::new(Jobs::CreatedAt).big_integer().not_null())
                    .to_owned(),
            )
            .await?;

        // Every claim scans for due work; without this the queue degrades
        // into a table scan per poll.
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_jobs_available_at")
                    .table(Jobs::Table)
                    .col(Jobs::AvailableAt)
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(FailedJobs::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(FailedJobs::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    // `connection` is not optional — `DatabaseFailedJobStore`
                    // names all seven columns in its INSERT, and omitting one
                    // makes every dead-letter fail rather than fall back.
                    .col(ColumnDef::new(FailedJobs::Connection).string().not_null())
                    .col(ColumnDef::new(FailedJobs::JobName).string().not_null())
                    .col(ColumnDef::new(FailedJobs::Queue).string().not_null())
                    .col(ColumnDef::new(FailedJobs::EnvelopeJson).text().not_null())
                    .col(ColumnDef::new(FailedJobs::Exception).text().not_null())
                    .col(
                        ColumnDef::new(FailedJobs::FailedAt)
                            .big_integer()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(FailedJobs::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Jobs::Table).to_owned())
            .await
    }
}
