use chrono::Utc;
use sea_orm::{ConnectionTrait, Database};
use std::time::Duration;
use suprnova::queue::database::DatabaseQueueDriver;
use suprnova::queue::driver::{QueueDriver, Settled};
use suprnova::queue::{BackoffSchedule, CURRENT_SCHEMA_VERSION, Envelope};
use uuid::Uuid;

async fn fresh_db() -> sea_orm::DatabaseConnection {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    db.execute_unprepared(
        r"
        CREATE TABLE jobs (
            id TEXT PRIMARY KEY,
            job_name TEXT NOT NULL,
            queue TEXT NULL,
            envelope_json TEXT NOT NULL,
            available_at INTEGER NOT NULL,
            reserved_until INTEGER NULL,
            reserved_token TEXT NULL,
            attempts INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL
        )
    ",
    )
    .await
    .unwrap();
    db.execute_unprepared("CREATE INDEX idx_jobs_available_at ON jobs(available_at)")
        .await
        .unwrap();
    db
}

fn env(name: &str) -> Envelope {
    let now = Utc::now();
    Envelope {
        schema_version: CURRENT_SCHEMA_VERSION,
        id: Uuid::new_v4(),
        job_name: name.into(),
        queue: None,
        payload: serde_json::json!({}),
        dispatched_at: now,
        available_at: now,
        attempts: 0,
        max_tries: 3,
        backoff: BackoffSchedule::default(),
        timeout_secs: None,
        fail_on_timeout: false,
        idempotency_key: None,
        batch_id: None,
        chain_remaining: Vec::new(),
    }
}

#[tokio::test]
async fn database_driver_push_and_ack() {
    let db = fresh_db().await;
    let d = DatabaseQueueDriver::new(db, "jobs".to_string()).unwrap();

    d.push(env("A")).await.unwrap();
    d.push(env("B")).await.unwrap();

    let r1 = d.pop(Duration::from_secs(60)).await.unwrap().unwrap();
    let r2 = d.pop(Duration::from_secs(60)).await.unwrap().unwrap();
    assert_eq!(r1.envelope.job_name, "A");
    assert_eq!(r2.envelope.job_name, "B");

    d.ack(&r1.token).await.unwrap();
    d.ack(&r2.token).await.unwrap();

    let none = d.pop(Duration::from_millis(10)).await.unwrap();
    assert!(none.is_none(), "queue drained");
}

#[tokio::test]
async fn database_driver_reserved_rows_invisible_to_other_pops() {
    let db = fresh_db().await;
    let d = DatabaseQueueDriver::new(db, "jobs".to_string()).unwrap();

    d.push(env("A")).await.unwrap();

    let _r1 = d.pop(Duration::from_secs(60)).await.unwrap().unwrap();
    let r2 = d.pop(Duration::from_millis(10)).await.unwrap();
    assert!(r2.is_none(), "row reserved by r1 must not be popped again");
}

#[tokio::test]
async fn database_driver_nack_bumps_attempts() {
    let db = fresh_db().await;
    let d = DatabaseQueueDriver::new(db, "jobs".to_string()).unwrap();

    d.push(env("A")).await.unwrap();

    let r1 = d.pop(Duration::from_secs(60)).await.unwrap().unwrap();
    assert_eq!(r1.envelope.attempts, 0);

    d.nack(&r1.token, Duration::from_millis(0)).await.unwrap();

    let r2 = d.pop(Duration::from_secs(60)).await.unwrap().unwrap();
    assert_eq!(
        r2.envelope.attempts, 1,
        "nack must bump attempts (per trait contract)"
    );
}

/// Pins the conditional-UPDATE behavior the SQLite race fix introduced.
///
/// Two concurrent consumers can both observe the same visible row in the
/// gap between their SELECTs and their UPDATEs. Without a predicate, both
/// stamp their reservation tokens and the loser walks away with a token
/// that doesn't match the row's stored value — its later ack/nack silently
/// no-ops and the job runs twice. The fix re-asserts the same "unreserved
/// or expired" predicate on UPDATE; the loser sees zero rows affected and
/// reports an empty pop instead.
#[tokio::test]
async fn database_driver_pop_returns_none_when_row_was_reserved_concurrently() {
    let db = fresh_db().await;
    let d = DatabaseQueueDriver::new(db.clone(), "jobs".to_string()).unwrap();

    d.push(env("A")).await.unwrap();

    // Mimic "another consumer reserved this row between our SELECT and our
    // UPDATE" by stamping a fresh reservation onto the row directly.
    let now = chrono::Utc::now().timestamp();
    let future = now + 600;
    db.execute_raw(sea_orm::Statement::from_sql_and_values(sea_orm::DatabaseBackend::Sqlite,
    "UPDATE jobs SET reserved_until = ?, reserved_token = ?",
    vec![
        sea_orm::Value::from(future),
        sea_orm::Value::from("other-consumer-token".to_string()),
    ],))
    .await
    .unwrap();

    // Our pop now observes the row as reserved-in-future via its SELECT
    // filter; this path is the SELECT-side of the same predicate. The
    // conditional UPDATE matters when the SELECT happened *before* the
    // injected reservation — a case our test setup approximates by simply
    // observing that the driver respects the post-race state correctly.
    let r = d.pop(Duration::from_millis(50)).await.unwrap();
    assert!(
        r.is_none(),
        "pop must observe the concurrent reservation and yield None"
    );

    // The originally-injected reservation must still be intact (we did not
    // overwrite it with our own token).
    let row = db.query_one_raw(sea_orm::Statement::from_string(sea_orm::DatabaseBackend::Sqlite,
    "SELECT reserved_token FROM jobs",))
        .await
        .unwrap()
        .expect("row exists");
    let tok: String = row.try_get_by_index(0).unwrap();
    assert_eq!(
        tok, "other-consumer-token",
        "conditional UPDATE must not overwrite a still-valid reservation"
    );
}

#[tokio::test]
async fn database_driver_pop_releases_reservation_after_visibility_expiry() {
    let db = fresh_db().await;
    let d = DatabaseQueueDriver::new(db, "jobs".to_string()).unwrap();
    d.push(env("A")).await.unwrap();

    // First reservation with a near-zero visibility timeout.
    let r1 = d.pop(Duration::from_secs(0)).await.unwrap().unwrap();
    assert_eq!(r1.envelope.job_name, "A");

    // After visibility expires, a fresh pop must reclaim the row — and the
    // conditional UPDATE has to succeed against the *expired* reservation
    // because `reserved_until <= now` is true.
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let r2 = d.pop(Duration::from_millis(50)).await.unwrap();
    assert!(
        r2.is_some(),
        "expired reservations must be reclaimable by a later pop"
    );
}

#[tokio::test]
async fn database_driver_rejects_invalid_table_identifier() {
    let db = fresh_db().await;
    for bad in [
        "",
        "jobs; DROP TABLE users",
        "jobs--",
        "jobs'",
        "jobs/*",
        "1jobs",
        "jobs jobs",
    ] {
        let err = match DatabaseQueueDriver::new(db.clone(), bad.into()) {
            Err(e) => e,
            Ok(_) => panic!("expected validation error for {bad:?}, got Ok"),
        };
        assert!(
            err.to_string().to_lowercase().contains("identifier"),
            "expected an identifier-validation error for {bad:?}, got: {err}"
        );
    }
}

/// The SQL filter, including the NULL case. A `queue IS NULL` row was written
/// before routing existed (or by an unrouted push); a worker draining
/// `default` must still see it, or upgrading strands every in-flight job.
#[tokio::test]
async fn pop_from_filters_by_queue_and_treats_null_as_default() {
    let db = fresh_db().await;
    let d = DatabaseQueueDriver::new(db, "jobs".into()).unwrap();

    let mut billing = env("billing-job");
    billing.queue = Some("billing".into());
    let mut reports = env("reports-job");
    reports.queue = Some("reports".into());
    let legacy = env("legacy-job"); // queue stays None

    d.push(billing).await.unwrap();
    d.push(reports).await.unwrap();
    d.push(legacy).await.unwrap();

    // A billing worker sees only billing.
    let got = d
        .pop_from(Duration::from_secs(60), &["billing".to_string()])
        .await
        .unwrap()
        .expect("billing job");
    assert_eq!(got.envelope.job_name, "billing-job");
    d.ack(&got.token).await.unwrap();

    assert!(
        d.pop_from(Duration::from_secs(60), &["billing".to_string()])
            .await
            .unwrap()
            .is_none(),
        "billing worker must not consume reports or unrouted work"
    );

    // A default worker picks up the NULL-queue row.
    let got = d
        .pop_from(Duration::from_secs(60), &["default".to_string()])
        .await
        .unwrap()
        .expect("legacy job should be reachable as default");
    assert_eq!(got.envelope.job_name, "legacy-job");
    d.ack(&got.token).await.unwrap();

    // Unfiltered still drains the rest.
    let got = d
        .pop(Duration::from_secs(60))
        .await
        .unwrap()
        .expect("reports job");
    assert_eq!(got.envelope.job_name, "reports-job");
}

/// Queue names arrive from the `--queue` CLI flag, so they must be bound as
/// parameters rather than interpolated into the SQL.
#[tokio::test]
async fn queue_filter_is_parameterized_not_interpolated() {
    let db = fresh_db().await;
    let d = DatabaseQueueDriver::new(db, "jobs".into()).unwrap();

    let mut e = env("safe");
    e.queue = Some("billing".into());
    d.push(e).await.unwrap();

    // A hostile queue name must simply match nothing, not alter the statement.
    let hostile = vec!["billing') OR 1=1 --".to_string()];
    let got = d.pop_from(Duration::from_secs(60), &hostile).await.unwrap();
    assert!(
        got.is_none(),
        "injection attempt must not widen the result set"
    );

    // The real queue still works afterwards, proving the table survived.
    let got = d
        .pop_from(Duration::from_secs(60), &["billing".to_string()])
        .await
        .unwrap();
    assert_eq!(got.expect("billing job").envelope.job_name, "safe");
}

// ---------------------------------------------------------------------------
// DATA-02: release is a driver primitive, not push-then-ack
// ---------------------------------------------------------------------------
//
// `handle_released` used to re-push the envelope and then ack the original.
// `id` is this table's primary key, so the push collided with the row that
// still held the live reservation and came back
// `UNIQUE constraint failed: jobs.id`. The worker treated that as a lost push
// and declined to ack — the safe reading — so the release silently became a
// no-op: no delay applied, no `JobReleased` event, the job just sat reserved
// until visibility expiry redelivered it. Every release on a database-backed
// queue behaved that way.

/// The bug, stated as the sequence that produced it: the released copy used to
/// be pushed while the original was still reserved.
#[tokio::test]
async fn release_does_not_collide_with_the_live_reservation() {
    let db = fresh_db().await;
    let d = DatabaseQueueDriver::new(db, "jobs".to_string()).unwrap();
    d.push(env("A")).await.unwrap();
    let res = d.pop(Duration::from_secs(60)).await.unwrap().unwrap();

    d.release(&res.token, &res.envelope, Duration::from_secs(30))
        .await
        .expect("release must not fail — this is the primary-key collision");

    assert_eq!(
        d.size().await.unwrap(),
        1,
        "exactly one copy survives: the release requeues in place rather than \
         adding a second row"
    );
}

/// The released job must actually become invisible for the requested delay.
/// The old path never applied the delay at all.
#[tokio::test]
async fn release_applies_the_requested_delay() {
    let db = fresh_db().await;
    let d = DatabaseQueueDriver::new(db, "jobs".to_string()).unwrap();
    d.push(env("A")).await.unwrap();
    let res = d.pop(Duration::from_secs(60)).await.unwrap().unwrap();

    d.release(&res.token, &res.envelope, Duration::from_secs(3600))
        .await
        .unwrap();

    assert!(
        d.pop(Duration::from_secs(60)).await.unwrap().is_none(),
        "a job released for an hour must not be immediately poppable"
    );
    assert_eq!(
        d.delayed_size().await.unwrap(),
        1,
        "it is delayed, not gone"
    );
}

/// A zero delay makes the job immediately available again — the
/// `WithoutOverlapping`-style "someone else holds the lock, try again" case.
#[tokio::test]
async fn release_with_no_delay_is_immediately_poppable_again() {
    let db = fresh_db().await;
    let d = DatabaseQueueDriver::new(db, "jobs".to_string()).unwrap();
    d.push(env("A")).await.unwrap();
    let res = d.pop(Duration::from_secs(60)).await.unwrap().unwrap();

    d.release(&res.token, &res.envelope, Duration::ZERO)
        .await
        .unwrap();

    let again = d.pop(Duration::from_secs(60)).await.unwrap();
    assert_eq!(
        again.expect("released job is visible again").envelope.id,
        res.envelope.id,
        "the same envelope comes back, under the same id"
    );
}

/// The whole point of `release` over `nack`: the retry is free.
#[tokio::test]
async fn release_does_not_burn_an_attempt_but_nack_does() {
    let db = fresh_db().await;
    let d = DatabaseQueueDriver::new(db, "jobs".to_string()).unwrap();

    d.push(env("released")).await.unwrap();
    let res = d.pop(Duration::from_secs(60)).await.unwrap().unwrap();
    let before = res.envelope.attempts;
    d.release(&res.token, &res.envelope, Duration::ZERO)
        .await
        .unwrap();
    let after_release = d.pop(Duration::from_secs(60)).await.unwrap().unwrap();
    assert_eq!(
        after_release.envelope.attempts, before,
        "release means try again without spending an attempt"
    );

    d.nack(&after_release.token, Duration::ZERO).await.unwrap();
    let after_nack = d.pop(Duration::from_secs(60)).await.unwrap().unwrap();
    assert_eq!(
        after_nack.envelope.attempts,
        before + 1,
        "nack still spends one — the two must not have collapsed into each other"
    );
}

/// Settlement operations are called on tokens that may already be gone
/// (a redelivered job settling twice), so an unknown token is a no-op, not
/// an error that would stall the worker.
#[tokio::test]
async fn release_is_idempotent_on_an_unknown_token() {
    use suprnova::queue::driver::ReservationToken;
    let db = fresh_db().await;
    let d = DatabaseQueueDriver::new(db, "jobs".to_string()).unwrap();
    let stray = ReservationToken(Uuid::new_v4());
    d.release(&stray, &env("gone"), Duration::from_secs(5))
        .await
        .expect("an unknown token settles silently");
    assert_eq!(d.size().await.unwrap(), 0, "and enqueues nothing");
}

// ---------------------------------------------------------------------------
// DATA-02: atomic terminal settlement
// ---------------------------------------------------------------------------
//
// Finishing a chained job means enqueuing the successor AND releasing the job
// just finished. As two operations there is no safe order — ack-first loses
// the rest of the chain on a crash, push-first runs the successor twice — so
// the driver commits both or neither.

/// The happy path, stated as the invariant: after settling, the successor is
/// enqueued and the predecessor is gone. Both, or the transaction did nothing.
#[tokio::test]
async fn settle_commits_the_successor_and_the_ack_together() {
    let db = fresh_db().await;
    let d = DatabaseQueueDriver::new(db, "jobs".to_string()).unwrap();
    d.push(env("Head")).await.unwrap();
    let res = d.pop(Duration::from_secs(60)).await.unwrap().unwrap();

    let next = env("Tail");
    let next_id = next.id;
    let outcome = d
        .settle(&res.token, std::slice::from_ref(&next))
        .await
        .unwrap();

    assert_eq!(outcome, Settled::Atomically);
    assert_eq!(d.size().await.unwrap(), 1, "the predecessor is gone");
    let got = d.pop(Duration::from_secs(60)).await.unwrap().unwrap();
    assert_eq!(got.envelope.id, next_id, "and the successor is queued");
}

/// The fence. A worker whose visibility expired while it was busy must not
/// enqueue a chain successor for a message someone else now owns — that is
/// exactly how a chain forks.
#[tokio::test]
async fn settle_on_a_reclaimed_reservation_commits_nothing() {
    let db = fresh_db().await;
    let d = DatabaseQueueDriver::new(db, "jobs".to_string()).unwrap();
    d.push(env("Head")).await.unwrap();

    // Worker A reserves it, then its reservation lapses.
    let a = d.pop(Duration::from_secs(0)).await.unwrap().unwrap();
    // Worker B reclaims the expired reservation.
    let b = d.pop(Duration::from_secs(60)).await.unwrap().unwrap();
    assert_eq!(a.envelope.id, b.envelope.id, "same message, new owner");
    assert_ne!(a.token, b.token, "and a new reservation token");

    // A finishes late and tries to settle with its stale token.
    let orphan = env("Tail-from-A");
    let outcome = d
        .settle(&a.token, std::slice::from_ref(&orphan))
        .await
        .unwrap();

    assert_eq!(outcome, Settled::Stale);
    assert_eq!(
        d.size().await.unwrap(),
        1,
        "A enqueued nothing and dropped nothing — B still holds the one message"
    );

    // B settles normally and its successor is the only one that lands.
    let real = env("Tail-from-B");
    let real_id = real.id;
    assert_eq!(
        d.settle(&b.token, std::slice::from_ref(&real))
            .await
            .unwrap(),
        Settled::Atomically
    );
    let got = d.pop(Duration::from_secs(60)).await.unwrap().unwrap();
    assert_eq!(got.envelope.id, real_id, "the chain did not fork");
    assert_eq!(d.size().await.unwrap(), 1, "exactly one successor exists");
}

/// A settlement that cannot write its follow-up must not have dropped the
/// reservation either, or the chain is lost with nothing left to retry from.
#[tokio::test]
async fn a_failed_follow_up_rolls_back_the_ack_too() {
    let db = fresh_db().await;
    let d = DatabaseQueueDriver::new(db, "jobs".to_string()).unwrap();
    d.push(env("Head")).await.unwrap();

    // A successor whose id already exists makes the follow-up insert fail.
    let clash = env("Clash");
    d.push(clash.clone()).await.unwrap();

    let res = d
        .pop_from(Duration::from_secs(60), &[])
        .await
        .unwrap()
        .unwrap();

    let err = d.settle(&res.token, std::slice::from_ref(&clash)).await;
    assert!(err.is_err(), "the follow-up write failed");

    assert_eq!(
        d.size().await.unwrap(),
        2,
        "both original rows survive: the failed settlement dropped nothing"
    );
}

/// With no follow-ups, `settle` is a fenced acknowledgement. It still has to
/// report `Stale` rather than pretending success, because "the row is gone"
/// and "I dropped the row" are different facts for the caller.
#[tokio::test]
async fn settle_without_follow_ups_still_fences() {
    let db = fresh_db().await;
    let d = DatabaseQueueDriver::new(db, "jobs".to_string()).unwrap();
    d.push(env("Solo")).await.unwrap();
    let res = d.pop(Duration::from_secs(60)).await.unwrap().unwrap();

    assert_eq!(
        d.settle(&res.token, &[]).await.unwrap(),
        Settled::Atomically
    );
    assert_eq!(d.size().await.unwrap(), 0);

    assert_eq!(
        d.settle(&res.token, &[]).await.unwrap(),
        Settled::Stale,
        "settling the same reservation twice is not a second success"
    );
}

// ---------------------------------------------------------------------------
// F-2: a job that kills its worker must still exhaust its attempts
// ---------------------------------------------------------------------------
//
// Found by experiment, not by reading: one poison job was fed to three
// workers, killed all three, and came back with `attempts` still 0 every
// time.
//
// The asymmetry is the bug. A job whose handler *fails* is nacked, and
// `requeue(AttemptPolicy::Consume)` counts the attempt, so it dead-letters
// after `max_tries`. A job that *kills its worker* settles nothing — the
// reservation merely lapses — so the reclaim used to return it
// byte-identical. That job is immortal: it kills each worker that claims
// it, is reclaimed unchanged, and kills the next one, for as long as
// anything restarts workers. Which, after the SIGTERM fix, is every
// rolling deploy.

/// The bug, stated as the sequence that produced it.
#[tokio::test]
async fn reclaiming_a_lapsed_reservation_consumes_an_attempt() {
    let db = fresh_db().await;
    let d = DatabaseQueueDriver::new(db, "jobs".into()).unwrap();
    d.push(env("poison")).await.unwrap();

    // Claim it, then walk away without settling — exactly what a worker
    // that is SIGKILLed or aborts mid-handler leaves behind. A zero
    // visibility timeout makes the reservation lapse immediately, so the
    // test does not have to sleep through a real one.
    let first = d
        .pop(Duration::from_secs(0))
        .await
        .unwrap()
        .expect("the queued job");
    assert_eq!(
        first.envelope.attempts, 0,
        "a first claim is not a retry and must not consume an attempt"
    );

    let second = d
        .pop(Duration::from_secs(60))
        .await
        .unwrap()
        .expect("the lapsed reservation is reclaimable");
    assert_eq!(
        second.envelope.attempts, 1,
        "reclaiming after a worker died must count the attempt; leaving it at 0 \
         makes the job immortal"
    );
}

/// The durable column and the envelope must agree. `pop` decodes the JSON
/// and `worker.rs` reads *the envelope* to decide whether `max_tries` is
/// exhausted, so bumping only the column would leave the dead-letter
/// decision reading a stale count.
#[tokio::test]
async fn a_reclaim_advances_the_stored_column_and_the_envelope_together() {
    let db = fresh_db().await;
    let d = DatabaseQueueDriver::new(db.clone(), "jobs".into()).unwrap();
    d.push(env("poison")).await.unwrap();

    d.pop(Duration::from_secs(0)).await.unwrap().expect("claim");
    let reclaimed = d
        .pop(Duration::from_secs(60))
        .await
        .unwrap()
        .expect("reclaim");

    let row = db.query_one_raw(sea_orm::Statement::from_string(db.get_database_backend(),
    "SELECT attempts FROM jobs".to_string(),))
        .await
        .unwrap()
        .expect("the row is still there");
    let stored: i32 = row.try_get_by_index(0).unwrap();

    assert_eq!(stored, 1, "the durable column advanced");
    assert_eq!(
        i32::try_from(reclaimed.envelope.attempts).expect("attempts fits an i32"),
        stored,
        "column and envelope must not disagree — the worker reads the envelope"
    );
}

/// The control. Without this, "count every claim" would pass the test
/// above while silently burning an attempt on every job's first delivery,
/// cutting every configured `max_tries` by one.
#[tokio::test]
async fn a_first_claim_does_not_consume_an_attempt() {
    let db = fresh_db().await;
    let d = DatabaseQueueDriver::new(db.clone(), "jobs".into()).unwrap();
    d.push(env("ordinary")).await.unwrap();

    let claimed = d
        .pop(Duration::from_secs(60))
        .await
        .unwrap()
        .expect("the queued job");
    assert_eq!(claimed.envelope.attempts, 0);

    let row = db.query_one_raw(sea_orm::Statement::from_string(db.get_database_backend(),
    "SELECT attempts FROM jobs".to_string(),))
        .await
        .unwrap()
        .expect("row");
    let stored: i32 = row.try_get_by_index(0).unwrap();
    assert_eq!(stored, 0, "a first delivery is not a retry");
}

/// The reason this matters, driven to its end: a job that keeps killing
/// its worker eventually runs out of attempts, which is what lets the
/// worker dead-letter it instead of handing it to a fourth victim.
#[tokio::test]
async fn repeated_worker_loss_exhausts_max_tries() {
    let db = fresh_db().await;
    let d = DatabaseQueueDriver::new(db, "jobs".into()).unwrap();
    let e = env("poison");
    let max_tries = e.max_tries;
    d.push(e).await.unwrap();

    let mut last = 0;
    for _ in 0..max_tries {
        last = d
            .pop(Duration::from_secs(0))
            .await
            .unwrap()
            .expect("still claimable")
            .envelope
            .attempts;
    }

    assert_eq!(
        last,
        max_tries - 1,
        "after {max_tries} deliveries the envelope has counted {} lost workers, \
         which is what `worker.rs` compares against max_tries",
        max_tries - 1
    );
}
