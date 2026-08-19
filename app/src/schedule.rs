//! Scheduled tasks.
//!
//! Registered from `cmd/main.rs` via `Application::schedule`. The app had
//! no scheduled work at all before these, which is why the replica
//! question — three `schedule:work` processes against one database — had
//! never been exercised.

use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, Value};
use suprnova::{FrameworkError, Schedule};

fn placeholder(backend: DatabaseBackend, n: usize) -> String {
    match backend {
        DatabaseBackend::Postgres => format!("${n}"),
        _ => "?".into(),
    }
}

/// Append `(task_name, instance_id, tick_minute)` on every run.
///
/// The instrument for the scheduler experiment. Run N `schedule:work`
/// processes against one database and group by `(task_name, tick_minute)`:
///
/// * exactly one row per minute — one replica ran that tick;
/// * N rows per minute — every replica independently decided the tick was
///   theirs, so every scheduled side effect is multiplied by the replica
///   count.
///
/// The minute comes from the wall clock rather than being passed in,
/// because the question is whether N processes independently decide the
/// *same* tick is theirs to run.
async fn record_tick(task_name: &str) -> Result<(), FrameworkError> {
    let conn = suprnova::DB::connection()?;
    let backend = conn.inner().get_database_backend();

    let instance_id = crate::bench_identity::process_id();
    let now = chrono::Utc::now();
    let tick_minute = now.format("%Y-%m-%dT%H:%M").to_string();

    let sql = format!(
        "INSERT INTO bench_scheduler_ticks (task_name, instance_id, tick_minute, ran_at) \
         VALUES ({}, {}, {}, {})",
        placeholder(backend, 1),
        placeholder(backend, 2),
        placeholder(backend, 3),
        placeholder(backend, 4),
    );

    conn.inner()
        .execute(Statement::from_sql_and_values(
            backend,
            sql,
            [
                Value::from(task_name.to_string()),
                Value::from(instance_id.clone()),
                Value::from(tick_minute.clone()),
                Value::from(now),
            ],
        ))
        .await
        .map_err(|e| {
            FrameworkError::from_external_with("bench_scheduler_ticks insert failed", e)
        })?;

    tracing::info!(
        task = %task_name,
        instance_id = %instance_id,
        tick_minute = %tick_minute,
        "bench scheduler tick recorded"
    );
    Ok(())
}

/// The task name recorded by the arm that asks for no coordination.
pub const PLAIN_TASK: &str = "bench:tick-plain";

/// The task name recorded by the arm that calls `.on_one_server()`.
pub const ONE_SERVER_TASK: &str = "bench:tick-one-server";

/// Register the app's scheduled tasks.
///
/// Two tasks, deliberately, running in the same window against the same
/// replicas and the same clock — so the only difference between the arms
/// is the one builder call under test.
///
/// Single-server execution is opt-in, so the plain arm is expected to
/// record one row *per replica* per minute and that is not a regression.
/// Measuring only the elected arm would let a reader conclude the default
/// had changed; measuring only the plain arm would say nothing about the
/// fix.
pub fn register(schedule: &mut Schedule) {
    // Every minute is the shortest interval that still exercises the
    // cross-replica question, and it keeps a ten-minute run to ten rows
    // per replica rather than something that needs sampling.
    //
    // `call` returns a builder; `add` is what registers it. A chain that
    // ends at `.every_minute()` compiles, runs, and schedules nothing.
    let plain = schedule
        .call(|| async { record_tick(PLAIN_TASK).await })
        .name(PLAIN_TASK)
        .description("control arm — no coordination requested")
        .every_minute();
    schedule.add(plain);

    let elected = schedule
        .call(|| async { record_tick(ONE_SERVER_TASK).await })
        .name(ONE_SERVER_TASK)
        .description("the arm under test — one execution per tick across replicas")
        .every_minute()
        .on_one_server();
    schedule.add(elected);
}
