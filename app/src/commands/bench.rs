//! Console commands driving benchmark Phase 1.
//!
//! Enqueue side and verify side both live here so an experiment script
//! stays a few lines of shell rather than embedded SQL. The verify
//! commands exit non-zero on a failed assertion, so a script can gate on
//! them directly.

use async_trait::async_trait;
use clap::Parser;
use sea_orm::{ConnectionTrait, Statement};
use suprnova::{Command, FrameworkError, Queue, TypedCommand};

use crate::jobs::bench::{BenchAbort, BenchRecord, BenchSleep};

/// Enqueue one long-running job (Phase 1.4).
#[derive(Parser, Command, Debug)]
#[console(
    name = "bench:enqueue-sleep",
    description = "Phase 1.4 — enqueue a job that sleeps, so SIGTERM can land mid-flight"
)]
pub struct EnqueueSleep {
    /// How long the job occupies a worker.
    #[arg(long, default_value_t = 20)]
    pub seconds: u64,
}

#[async_trait]
impl TypedCommand for EnqueueSleep {
    async fn run(self) -> Result<(), FrameworkError> {
        Queue::push(BenchSleep {
            seconds: self.seconds,
        })
        .await?;
        println!("enqueued BenchSleep({}s)", self.seconds);
        Ok(())
    }
}

/// Enqueue a job that kills its worker (Phase 1.3).
#[derive(Parser, Command, Debug)]
#[console(
    name = "bench:enqueue-abort",
    description = "Phase 1.3 — enqueue a job whose handler aborts its own process"
)]
pub struct EnqueueAbort {
    /// Correlates the enqueue with the surviving `jobs` row.
    #[arg(long, default_value = "phase-1.3")]
    pub marker: String,
}

#[async_trait]
impl TypedCommand for EnqueueAbort {
    async fn run(self) -> Result<(), FrameworkError> {
        Queue::push(BenchAbort {
            marker: self.marker.clone(),
        })
        .await?;
        println!("enqueued BenchAbort(marker={})", self.marker);
        Ok(())
    }
}

/// Enqueue N recording jobs (Phase 1.5).
#[derive(Parser, Command, Debug)]
#[console(
    name = "bench:enqueue-records",
    description = "Phase 1.5 — enqueue N jobs that each record their id exactly once"
)]
pub struct EnqueueRecords {
    #[arg(long, default_value_t = 1000)]
    pub count: i64,
}

#[async_trait]
impl TypedCommand for EnqueueRecords {
    async fn run(self) -> Result<(), FrameworkError> {
        for job_id in 1..=self.count {
            Queue::push(BenchRecord { job_id }).await?;
        }
        println!("enqueued {} BenchRecord jobs", self.count);
        Ok(())
    }
}

/// Assert every job ran exactly once (Phase 1.5).
#[derive(Parser, Command, Debug)]
#[console(
    name = "bench:verify-records",
    description = "Phase 1.5 — assert N distinct jobs ran, none twice"
)]
pub struct VerifyRecords {
    #[arg(long, default_value_t = 1000)]
    pub expect: i64,
}

#[async_trait]
impl TypedCommand for VerifyRecords {
    async fn run(self) -> Result<(), FrameworkError> {
        let conn = suprnova::DB::connection()?;
        let backend = conn.inner().get_database_backend();

        let row = conn
            .inner()
            .query_one(Statement::from_string(
                backend,
                "SELECT COUNT(*) AS total, COUNT(DISTINCT job_id) AS distinct_ids \
                 FROM bench_job_runs"
                    .to_string(),
            ))
            .await
            .map_err(|e| FrameworkError::internal(format!("verify query failed: {e}")))?
            .ok_or_else(|| FrameworkError::internal("verify query returned no row"))?;

        let total: i64 = row.try_get_by("total").unwrap_or(0);
        let distinct: i64 = row.try_get_by("distinct_ids").unwrap_or(0);

        println!(
            "bench_job_runs: total={total} distinct={distinct} expected={}",
            self.expect
        );

        // Two distinct failures, and they mean different things: a
        // duplicate is a claiming defect, a shortfall is jobs that never
        // ran at all. Reporting them separately keeps the second from
        // being read as the first.
        if total != distinct {
            return Err(FrameworkError::internal(format!(
                "FAIL: {} rows for {distinct} distinct jobs — a job was claimed more than once",
                total
            )));
        }
        if distinct != self.expect {
            return Err(FrameworkError::internal(format!(
                "FAIL: {distinct} distinct jobs ran, expected {} — the rest never executed",
                self.expect
            )));
        }

        println!("PASS: {distinct} jobs, each executed exactly once");
        Ok(())
    }
}

/// Assert one execution per scheduled tick (Phase 1.2).
#[derive(Parser, Command, Debug)]
#[console(
    name = "bench:verify-ticks",
    description = "Phase 1.2 — assert each scheduled tick fired exactly once across replicas"
)]
pub struct VerifyTicks;

#[async_trait]
impl TypedCommand for VerifyTicks {
    async fn run(self) -> Result<(), FrameworkError> {
        let conn = suprnova::DB::connection()?;
        let backend = conn.inner().get_database_backend();

        let rows = conn
            .inner()
            .query_all(Statement::from_string(
                backend,
                "SELECT tick_minute, COUNT(*) AS runs, COUNT(DISTINCT instance_id) AS instances \
                 FROM bench_scheduler_ticks GROUP BY tick_minute ORDER BY tick_minute"
                    .to_string(),
            ))
            .await
            .map_err(|e| FrameworkError::internal(format!("verify query failed: {e}")))?;

        if rows.is_empty() {
            return Err(FrameworkError::internal(
                "FAIL: no ticks recorded at all — the scheduler never ran, so this run \
                 proves nothing either way",
            ));
        }

        let mut duplicated = 0usize;
        for row in &rows {
            let minute: String = row.try_get_by("tick_minute").unwrap_or_default();
            let runs: i64 = row.try_get_by("runs").unwrap_or(0);
            let instances: i64 = row.try_get_by("instances").unwrap_or(0);
            println!("  {minute}  runs={runs}  instances={instances}");
            if runs > 1 {
                duplicated += 1;
            }
        }

        if duplicated > 0 {
            return Err(FrameworkError::internal(format!(
                "FAIL: {duplicated} of {} ticks fired more than once — replicas are not \
                 coordinating, so every scheduled task runs once per replica",
                rows.len()
            )));
        }

        println!("PASS: {} ticks, each fired exactly once", rows.len());
        Ok(())
    }
}
