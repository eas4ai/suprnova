//! Benchmark Phase 1 support tables.
//!
//! Two experiments in `BENCHMARK-PLAN.md` need somewhere durable to write
//! evidence, and the evidence has to survive the process that produced it
//! — both experiments work by killing workers.
//!
//! `bench_scheduler_ticks` (1.2) — every scheduled run appends
//! `(instance_id, tick_minute)`. Three `schedule:work` replicas against one
//! database should produce exactly one row per `tick_minute`; three rows
//! means the per-process dedup is doing nothing across replicas.
//!
//! `bench_job_runs` (1.5) — each job inserts its own id. The UNIQUE index
//! on `job_id` is the assertion: if two workers ever claim the same job,
//! the second insert fails loudly instead of leaving a duplicate row for
//! someone to notice later. A benchmark that can only detect a defect by
//! counting afterwards is weaker than one the database refuses to record.
//!
//! Both tables exist only for the harness. They are cheap, empty in normal
//! operation, and dropping them costs nothing.

use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m_2026_08_01_bench_tables"
    }
}

#[derive(DeriveIden)]
enum BenchSchedulerTicks {
    Table,
    Id,
    InstanceId,
    TickMinute,
    RanAt,
}

#[derive(DeriveIden)]
enum BenchJobRuns {
    Table,
    Id,
    JobId,
    WorkerId,
    RanAt,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(BenchSchedulerTicks::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(BenchSchedulerTicks::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(BenchSchedulerTicks::InstanceId)
                            .string()
                            .not_null(),
                    )
                    // The minute the task fired, as the scheduler saw it.
                    // Grouping on this is the whole experiment.
                    .col(
                        ColumnDef::new(BenchSchedulerTicks::TickMinute)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(BenchSchedulerTicks::RanAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(BenchJobRuns::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(BenchJobRuns::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(BenchJobRuns::JobId).big_integer().not_null())
                    .col(ColumnDef::new(BenchJobRuns::WorkerId).string().not_null())
                    .col(
                        ColumnDef::new(BenchJobRuns::RanAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // The exactly-once assertion, enforced by the database rather than
        // by a query someone remembers to run.
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_bench_job_runs_job_id")
                    .table(BenchJobRuns::Table)
                    .col(BenchJobRuns::JobId)
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(BenchJobRuns::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(BenchSchedulerTicks::Table).to_owned())
            .await
    }
}
