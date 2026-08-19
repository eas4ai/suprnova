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
            .map_err(|e| FrameworkError::from_external_with("verify query failed", e))?
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

/// Assert the scheduler experiment's two arms behaved as designed.
#[derive(Parser, Command, Debug)]
#[console(
    name = "bench:verify-ticks",
    description = "assert the elected arm fired once per tick and the control arm fired per replica"
)]
pub struct VerifyTicks {
    /// How many `schedule:work` replicas were running.
    #[arg(long, default_value_t = 3)]
    pub replicas: i64,
}

#[async_trait]
impl TypedCommand for VerifyTicks {
    async fn run(self) -> Result<(), FrameworkError> {
        let conn = suprnova::DB::connection()?;
        let backend = conn.inner().get_database_backend();

        let rows = conn
            .inner()
            .query_all(Statement::from_string(
                backend,
                "SELECT task_name, tick_minute, COUNT(*) AS runs, \
                        COUNT(DISTINCT instance_id) AS instances \
                 FROM bench_scheduler_ticks \
                 GROUP BY task_name, tick_minute ORDER BY task_name, tick_minute"
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

        // The first and last minute of a window can legitimately hold
        // fewer rows than the replica count, because replicas start and
        // stop inside them. Interior minutes are the only ones where the
        // control arm's count is a real measurement, so boundary minutes
        // are reported and excluded rather than silently averaged in.
        let mut minutes: Vec<String> = rows
            .iter()
            .map(|r| r.try_get_by("tick_minute").unwrap_or_default())
            .collect();
        minutes.sort();
        minutes.dedup();
        let (first, last) = (minutes.first().cloned(), minutes.last().cloned());

        let mut elected_violations = Vec::new();
        let mut control_ran = false;
        let mut control_saw_all_replicas = false;

        println!("  task                   tick              runs  instances");
        for row in &rows {
            let task: String = row.try_get_by("task_name").unwrap_or_default();
            let minute: String = row.try_get_by("tick_minute").unwrap_or_default();
            let runs: i64 = row.try_get_by("runs").unwrap_or(0);
            let instances: i64 = row.try_get_by("instances").unwrap_or(0);
            let boundary = Some(&minute) == first.as_ref() || Some(&minute) == last.as_ref();
            let mark = if boundary { " (boundary)" } else { "" };
            println!("  {task:<22} {minute}  {runs:>4}  {instances:>9}{mark}");

            if task == crate::schedule::ONE_SERVER_TASK {
                // Every minute counts here, boundary or not: a boundary
                // cannot manufacture an *extra* execution, only a missing
                // one, so >1 is always a real violation.
                if runs > 1 {
                    elected_violations.push(minute.clone());
                }
            } else if task == crate::schedule::PLAIN_TASK {
                control_ran = true;
                if !boundary && runs >= self.replicas {
                    control_saw_all_replicas = true;
                }
            }
        }

        if !elected_violations.is_empty() {
            return Err(FrameworkError::internal(format!(
                "FAIL: the on_one_server() arm fired more than once on {} tick(s) ({}). \
                 Replicas are not coordinating, so every scheduled side effect is \
                 multiplied by the replica count.",
                elected_violations.len(),
                elected_violations.join(", "),
            )));
        }

        // Without this the experiment could pass by never running at all,
        // or by running against one replica, and nobody would know.
        if !control_ran {
            return Err(FrameworkError::internal(
                "FAIL: the control arm recorded nothing, so there is no evidence the \
                 replicas were live and contending. The elected arm's clean result \
                 cannot be trusted.",
            ));
        }
        if !control_saw_all_replicas {
            return Err(FrameworkError::internal(format!(
                "FAIL: no interior minute shows the control arm running on all {} \
                 replicas. Either fewer replicas were up than expected or the window \
                 was too short — either way the elected arm was never actually \
                 contended, so a single execution proves nothing.",
                self.replicas,
            )));
        }

        println!(
            "PASS: on_one_server() fired exactly once per tick while the uncoordinated \
             control arm fired on all {} replicas",
            self.replicas
        );
        Ok(())
    }
}

/// Emit a password hash for the benchmark seeder.
///
/// The seeder writes one hash to all N million rows so any seeded account
/// can authenticate with a known password. That hash has to come from the
/// framework's configured hasher, not from a hash pasted in from
/// elsewhere: driver and cost live in config, and a mismatch does not
/// surface until warmup tries to log in — at the far end of a multi-hour
/// load, with the whole dataset already written.
///
/// So this verifies its own output before printing. A hash that does not
/// round-trip is the one failure mode this command exists to prevent, and
/// discovering it here costs a second.
#[derive(Parser, Command, Debug)]
#[console(
    name = "bench:password-hash",
    description = "Emit a seeder password hash from the configured hasher, verified"
)]
pub struct PasswordHash {
    /// The plaintext every seeded account will accept.
    #[arg(long, default_value = "bench-password")]
    pub password: String,
}

#[async_trait]
impl TypedCommand for PasswordHash {
    async fn run(self) -> Result<(), FrameworkError> {
        let hash = suprnova::hashing::hash(&self.password)?;

        if !suprnova::hashing::verify(&self.password, &hash)? {
            return Err(FrameworkError::internal(
                "the configured hasher produced a hash that does not verify against \
                 its own input; seeding with it would leave every account unable to \
                 log in",
            ));
        }

        // A wrong password must also be rejected. A verifier that returns
        // true for everything would sail through the check above.
        if suprnova::hashing::verify(&format!("{}-wrong", self.password), &hash)? {
            return Err(FrameworkError::internal(
                "the configured hasher verified a password it should have rejected",
            ));
        }

        eprintln!("verified: round-trips, and rejects a wrong password");
        println!("{hash}");
        Ok(())
    }
}
