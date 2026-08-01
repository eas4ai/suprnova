//! Jobs that exist to be measured, not to do work.
//!
//! Each one is the instrument for one question about the queue under it
//! — signal handling, worker loss, claim exclusivity. They live in the
//! dogfood app because those questions have to drive the real queue
//! across real processes: a job that runs under a test harness proves
//! things about the harness.

use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, Value};
use serde::{Deserialize, Serialize};
use suprnova::{FrameworkError, Job, async_trait};

/// Render a placeholder for the backend in hand.
///
/// The dogfood app runs on SQLite locally and Postgres under the
/// benchmark compose stack, and the two disagree about placeholder
/// syntax. Getting this wrong shows up as a runtime SQL error inside a
/// worker, which is a miserable thing to debug through a queue.
fn placeholder(backend: DatabaseBackend, n: usize) -> String {
    match backend {
        DatabaseBackend::Postgres => format!("${n}"),
        _ => "?".into(),
    }
}

/// Sleeps, so a signal can land while it is genuinely in flight.
///
/// Phase 1.4. The experiment sends SIGTERM to a worker holding this job
/// and asks whether the in-flight work drains or dies. A job that
/// finished instantly would make every outcome look identical.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BenchSleep {
    pub seconds: u64,
}

#[async_trait]
impl Job for BenchSleep {
    fn job_name() -> &'static str {
        "BenchSleep"
    }

    async fn handle(self) -> Result<(), FrameworkError> {
        // `warn` rather than `info`, and not by accident. These two lines
        // are the experiment's only direct evidence that the in-flight job
        // ran to completion rather than being cut short by the drain — and
        // the stack runs at `LOG_LEVEL=warn`, which silently filtered them
        // out. The first run after the SIGTERM fix reported "job_finished:
        // no" for that reason alone, which reads as a framework failure and
        // is not one. Evidence a verdict depends on has to survive the log
        // level the system under test actually runs at.
        tracing::warn!(seconds = self.seconds, "bench sleep job started");
        tokio::time::sleep(std::time::Duration::from_secs(self.seconds)).await;
        tracing::warn!(seconds = self.seconds, "bench sleep job finished");
        Ok(())
    }
}

/// Kills its own process, without unwinding.
///
/// Phase 1.3. The claim under test is that a worker lost mid-job is
/// reclaimed without the durable `attempts` counter advancing, so a poison
/// job cycles forever and never dead-letters.
///
/// `abort()` rather than `panic!` deliberately: a panic is caught by the
/// framework's panic boundary and settled as a normal failure, which is
/// the path that already works. Only an abrupt death — the process
/// vanishing without settling anything — exercises reclaim, and that is
/// what a real crash, OOM kill, or `docker kill` looks like.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BenchAbort {
    /// Correlates the enqueue with whatever survived in the `jobs` table.
    pub marker: String,
}

#[async_trait]
impl Job for BenchAbort {
    fn job_name() -> &'static str {
        "BenchAbort"
    }

    async fn handle(self) -> Result<(), FrameworkError> {
        tracing::error!(
            marker = %self.marker,
            "bench abort job is killing this process on purpose (phase 1.3)"
        );
        // Flush what tracing has buffered; the next line does not return.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        std::process::abort();
    }
}

/// Records that it ran, exactly once.
///
/// Phase 1.5. `bench_job_runs.job_id` carries a UNIQUE index, so a second
/// worker claiming the same job fails its insert rather than quietly
/// writing a duplicate. The assertion is enforced by the database at the
/// moment of the defect, not by a count run afterwards.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BenchRecord {
    pub job_id: i64,
}

#[async_trait]
impl Job for BenchRecord {
    fn job_name() -> &'static str {
        "BenchRecord"
    }

    async fn handle(self) -> Result<(), FrameworkError> {
        let conn = suprnova::DB::connection()?;
        let backend = conn.inner().get_database_backend();

        // Which worker got it, so a duplicate can be attributed to a pair
        // of workers rather than merely observed. Scaled compose replicas
        // all share one environment block, so the container hostname is
        // the only per-replica identity available without hand-writing a
        // service per worker.
        let worker_id = crate::bench_identity::process_id();

        let sql = format!(
            "INSERT INTO bench_job_runs (job_id, worker_id, ran_at) VALUES ({}, {}, {})",
            placeholder(backend, 1),
            placeholder(backend, 2),
            placeholder(backend, 3),
        );

        conn.inner()
            .execute(Statement::from_sql_and_values(
                backend,
                sql,
                [
                    Value::from(self.job_id),
                    Value::from(worker_id),
                    Value::from(chrono::Utc::now()),
                ],
            ))
            .await
            .map_err(|e| {
                // A UNIQUE violation here IS the finding, so it must reach
                // the operator rather than being swallowed as a retry.
                FrameworkError::internal(format!(
                    "bench_job_runs insert failed for job_id={} (a UNIQUE violation means \
                     the job was claimed twice, which is the defect 1.5 tests for): {e}",
                    self.job_id
                ))
            })?;

        Ok(())
    }
}
