//! Scheduled tasks.
//!
//! Registered from `cmd/main.rs` via `Application::schedule`. Everything
//! here today exists for benchmark Phase 1.2; the app had no scheduled
//! work before, which is also why the replica question had never been
//! exercised.

use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, Value};
use suprnova::{FrameworkError, Schedule};

fn placeholder(backend: DatabaseBackend, n: usize) -> String {
    match backend {
        DatabaseBackend::Postgres => format!("${n}"),
        _ => "?".into(),
    }
}

/// Append `(instance_id, tick_minute)` on every run.
///
/// Phase 1.2's instrument. Run three `schedule:work` processes with
/// distinct `BENCH_INSTANCE_ID` values against one database and group by
/// `tick_minute`:
///
/// * exactly one row per minute — the replicas are coordinating;
/// * three rows per minute — the per-process `AtomicI64` dedup is doing
///   nothing across processes, and whichever cache driver is configured
///   granted every replica the lock.
///
/// The minute is taken from wall clock rather than passed in, because the
/// question is whether N processes independently decide the *same* tick is
/// theirs to run.
async fn record_tick() -> Result<(), FrameworkError> {
    let conn = suprnova::DB::connection()?;
    let backend = conn.inner().get_database_backend();

    let instance_id = std::env::var("BENCH_INSTANCE_ID").unwrap_or_else(|_| "unset".into());
    let now = chrono::Utc::now();
    let tick_minute = now.format("%Y-%m-%dT%H:%M").to_string();

    let sql = format!(
        "INSERT INTO bench_scheduler_ticks (instance_id, tick_minute, ran_at) \
         VALUES ({}, {}, {})",
        placeholder(backend, 1),
        placeholder(backend, 2),
        placeholder(backend, 3),
    );

    conn.inner()
        .execute(Statement::from_sql_and_values(
            backend,
            sql,
            [
                Value::from(instance_id.clone()),
                Value::from(tick_minute.clone()),
                Value::from(now),
            ],
        ))
        .await
        .map_err(|e| {
            FrameworkError::internal(format!("bench_scheduler_ticks insert failed: {e}"))
        })?;

    tracing::info!(
        instance_id = %instance_id,
        tick_minute = %tick_minute,
        "bench scheduler tick recorded"
    );
    Ok(())
}

/// Register the app's scheduled tasks.
pub fn register(schedule: &mut Schedule) {
    // Every minute is the shortest interval that still exercises the
    // cross-replica question, and it keeps a ten-minute run to ten rows
    // per replica rather than something that needs sampling.
    //
    // `call` returns a builder; `add` is what registers it. A chain that
    // ends at `.every_minute()` compiles, runs, and schedules nothing.
    let tick = schedule
        .call(|| async { record_tick().await })
        .name("bench:scheduler-tick")
        .description("Phase 1.2 — append (instance_id, tick_minute) on each run")
        .every_minute();
    schedule.add(tick);
}
