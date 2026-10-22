//! Record *which* scheduled task produced a tick.
//!
//! The scheduler experiment now runs two tasks side by side in one
//! window: a plain one and an `.on_one_server()` one. That is the only
//! honest way to measure the fix, because single-server execution is
//! opt-in - a run that showed one row per minute without saying which task
//! it came from would look like the default had changed, and it has not.
//!
//! Both arms in one window rather than two sequential runs: the replicas,
//! the clock, and the cache are then identical for both, so the only
//! difference left is the builder call under test.

use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m_2026_08_02_bench_tick_task"
    }
}

#[derive(DeriveIden)]
enum BenchSchedulerTicks {
    Table,
    TaskName,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(BenchSchedulerTicks::Table)
                    // Defaulted rather than nullable: rows written before
                    // this column existed came from the single unnamed
                    // task, and leaving them NULL would make them look
                    // like a third arm.
                    .add_column(
                        ColumnDef::new(BenchSchedulerTicks::TaskName)
                            .string()
                            .not_null()
                            .default("legacy"),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(BenchSchedulerTicks::Table)
                    .drop_column(BenchSchedulerTicks::TaskName)
                    .to_owned(),
            )
            .await
    }
}
