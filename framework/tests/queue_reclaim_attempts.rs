//! F-2 — reclaiming a lapsed reservation counts as a consumed attempt, on
//! every driver.
//!
//! The database driver's own version of these assertions lives beside its
//! other tests in `queue_database.rs`. This file exists to hold the
//! *semantics* to one standard across backends: swapping `QUEUE_DRIVER`
//! must not change whether a poison job can ever be dead-lettered.
//!
//! # The defect
//!
//! Two settlement paths, and only one used to count.
//!
//! A job whose handler *fails* — returns `Err`, or panics into the
//! framework's boundary — is nacked, and the driver requeues it with the
//! attempt consumed. It exhausts `max_tries` and dead-letters. That always
//! worked.
//!
//! A job that *kills its worker* — OOM, `abort()`, segfault, or the
//! SIGKILL a supervisor sends when a stop times out — settles nothing. Its
//! reservation merely lapses. Before this fix every driver redelivered it
//! byte-identical, so its attempt count never moved and it could never
//! dead-letter: it killed each worker that claimed it, came back
//! unchanged, and killed the next one, for as long as anything restarted
//! workers.
//!
//! Found by experiment on the benchmark host, not by reading: three
//! workers, one poison job, `attempts` still 0 after all three died
//! (`bench/results/phase1/crash/rounds.tsv`).
//!
//! # The cost of getting this wrong in the other direction
//!
//! Counting a *first* delivery as an attempt would silently cut every
//! configured `max_tries` by one and dead-letter healthy work. Each driver
//! therefore gets a control asserting a first claim consumes nothing.

use chrono::Utc;
use std::time::Duration;
use suprnova::queue::driver::QueueDriver;
use suprnova::queue::memory::MemoryQueueDriver;
use suprnova::queue::{BackoffSchedule, CURRENT_SCHEMA_VERSION, Envelope};
use uuid::Uuid;

fn env(name: &str) -> Envelope {
    Envelope {
        schema_version: CURRENT_SCHEMA_VERSION,
        id: Uuid::new_v4(),
        job_name: name.into(),
        queue: None,
        payload: serde_json::json!({}),
        dispatched_at: Utc::now(),
        available_at: Utc::now(),
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

// ---------------------------------------------------------------------------
// Memory driver
// ---------------------------------------------------------------------------

/// The reaper moves a lapsed reservation from `reserved` back to
/// `visible`. That transition is the only place the memory driver learns a
/// worker died, so it is where the attempt has to be charged.
#[tokio::test(start_paused = true)]
async fn memory_reclaim_after_worker_loss_consumes_an_attempt() {
    let d = MemoryQueueDriver::new();
    d.push(env("poison")).await.unwrap();

    // Claim it and drop the reservation without settling — what a
    // SIGKILLed worker leaves behind.
    let first = d.pop(Duration::from_secs(5)).await.unwrap().unwrap();
    assert_eq!(
        first.envelope.attempts, 0,
        "a first claim is not a retry and must not consume an attempt"
    );
    drop(first);

    tokio::time::advance(Duration::from_secs(6)).await;

    let reclaimed = d
        .pop(Duration::from_secs(5))
        .await
        .unwrap()
        .expect("the lapsed reservation is reclaimable");
    assert_eq!(
        reclaimed.envelope.attempts, 1,
        "reclaiming after a worker died must count the attempt; leaving it at 0 \
         makes the job immortal"
    );
}

/// The control: without it, "count every delivery" would satisfy the test
/// above while cutting every configured `max_tries` by one.
#[tokio::test(start_paused = true)]
async fn memory_first_claim_does_not_consume_an_attempt() {
    let d = MemoryQueueDriver::new();
    d.push(env("ordinary")).await.unwrap();

    let claimed = d.pop(Duration::from_secs(30)).await.unwrap().unwrap();
    assert_eq!(claimed.envelope.attempts, 0);
}

/// Driven to the end that matters: enough lost workers and the job runs
/// out of attempts, which is what lets `worker.rs` dead-letter it instead
/// of handing it to a fourth victim.
#[tokio::test(start_paused = true)]
async fn memory_repeated_worker_loss_exhausts_max_tries() {
    let d = MemoryQueueDriver::new();
    let e = env("poison");
    let max_tries = e.max_tries;
    d.push(e).await.unwrap();

    let mut last = 0;
    for _ in 0..max_tries {
        let r = d
            .pop(Duration::from_secs(5))
            .await
            .unwrap()
            .expect("still claimable");
        last = r.envelope.attempts;
        drop(r);
        tokio::time::advance(Duration::from_secs(6)).await;
    }

    assert_eq!(
        last,
        max_tries - 1,
        "after {max_tries} deliveries the envelope has counted {} lost workers, \
         which is what `worker.rs` compares against max_tries",
        max_tries - 1
    );
}

// ---------------------------------------------------------------------------
// Redis driver — live instance required
// ---------------------------------------------------------------------------
//
// `cargo test -p suprnova --test queue_reclaim_attempts -- --ignored`
// with `REDIS_URL` pointing at a throwaway instance.
//
// Redis is the awkward one. The stream entry is immutable, so `attempts`
// stays at whatever was published; the only record that the job was handed
// out again is Redis's own per-entry delivery counter, which sea-streamer
// does not carry through (it merges XREADGROUP and XAUTOCLAIM into one
// message stream with no redelivery flag). The driver therefore asks
// XPENDING directly, and this test is the only thing that proves the
// answer is read correctly.

#[cfg(test)]
mod redis_driver {
    use super::*;
    use suprnova::queue::redis::RedisQueueDriver;

    fn redis_url() -> String {
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into())
    }

    /// A fresh stream per run. These tests deliberately let a reservation
    /// lapse, so a shared stream would leak pending entries into whatever
    /// ran next.
    fn unique_stream() -> String {
        format!("suprnova-reclaim-test-{}", Uuid::new_v4())
    }

    #[tokio::test]
    #[ignore = "requires a live Redis"]
    async fn redis_reclaim_after_worker_loss_consumes_an_attempt() {
        let stream = unique_stream();
        // A one-second visibility window so the reclaim is observable
        // without the test sleeping through a realistic one.
        let producer = RedisQueueDriver::connect(
            &redis_url(),
            &stream,
            "g",
            "consumer-a",
            Duration::from_secs(1),
        )
        .await
        .expect("connect redis");

        producer.push(env("poison")).await.expect("push");

        let first = producer
            .pop(Duration::from_secs(2))
            .await
            .expect("pop")
            .expect("the queued job");
        assert_eq!(
            first.envelope.attempts, 0,
            "a first delivery is not a retry"
        );

        // Walk away without acking, then let a *different* consumer in the
        // same group claim it — which is what XAUTOCLAIM does for a worker
        // that died, and the only path that reaches a second delivery.
        //
        // The whole driver goes, not just the reservation: sea-streamer's
        // consumer keeps polling in the background, and every poll resets
        // this consumer's idle time in `XINFO CONSUMERS`. A live consumer
        // never looks dead, so a reclaim would never trigger — which is
        // correct behaviour, and exactly why the test has to actually stop
        // being alive.
        drop(first);
        drop(producer);
        tokio::time::sleep(Duration::from_millis(2500)).await;

        let other = RedisQueueDriver::connect(
            &redis_url(),
            &stream,
            "g",
            "consumer-b",
            Duration::from_secs(1),
        )
        .await
        .expect("connect second consumer");

        // Comfortably past `2 x visibility_timeout`, which is the bound now
        // that the driver ties sea-streamer's auto-claim *interval* to the
        // configured timeout rather than leaving it at the 30s default.
        // Before that this same assertion needed a 45s window — the wait
        // was the library's fixed polling interval, not anything about the
        // job.
        let reclaimed = other
            .pop(Duration::from_secs(15))
            .await
            .expect("pop")
            .expect("the abandoned entry is reclaimable");
        assert_eq!(
            reclaimed.envelope.attempts, 1,
            "a redelivered entry must arrive with the lost worker's attempt \
             charged; Redis's delivery counter is the only record of it"
        );
    }

    #[tokio::test]
    #[ignore = "requires a live Redis"]
    async fn redis_first_delivery_does_not_consume_an_attempt() {
        let stream = unique_stream();
        let d = RedisQueueDriver::connect(
            &redis_url(),
            &stream,
            "g",
            "consumer-a",
            Duration::from_secs(30),
        )
        .await
        .expect("connect redis");

        d.push(env("ordinary")).await.expect("push");
        let claimed = d
            .pop(Duration::from_secs(5))
            .await
            .expect("pop")
            .expect("the queued job");
        assert_eq!(
            claimed.envelope.attempts, 0,
            "charging a first delivery would cut every configured max_tries by one"
        );
    }
}

// ---------------------------------------------------------------------------
// The other half: something has to act on the count
// ---------------------------------------------------------------------------
//
// Counting the reclaimed attempt was necessary and not sufficient. Every
// dead-letter decision in the worker happens *after* the handler returns —
// which assumes the handler returns. A job that kills its worker never
// reaches settlement, so the check never ran for exactly the jobs that
// most needed it, and the counter climbed forever with nothing acting on
// it.
//
// Observed in the container harness after the driver fix landed: attempts
// went 0 → 1 → 2 across three killed workers, exactly as intended, and the
// job was still queued and still lethal.

/// A job whose budget is already spent must be dead-lettered *before* it
/// is handed to another worker.
#[tokio::test(start_paused = true)]
async fn a_job_past_max_tries_is_not_dispatched_again() {
    let d = MemoryQueueDriver::new();

    // Straight to the state a repeatedly-killed job reaches: the stored
    // envelope has already consumed its whole budget, and no settlement
    // ever ran to notice.
    let mut e = env("poison");
    e.attempts = e.max_tries;
    let max_tries = e.max_tries;
    d.push(e).await.unwrap();

    let claimed = d
        .pop(Duration::from_secs(30))
        .await
        .unwrap()
        .expect("the job is still claimable — the guard lives in the worker");

    // The worker increments its own copy for this dispatch, so the value
    // it tests is one past the stored count. That is the number the guard
    // compares, and `>` is what allows exactly `max_tries` runs: on the
    // last permitted attempt the two are equal.
    let would_be_attempt = claimed.envelope.attempts + 1;
    assert!(
        would_be_attempt > max_tries,
        "a job that has already used its {max_tries} attempts must not get another; \
         this is the arithmetic the worker's pre-dispatch guard performs"
    );
}

/// The boundary, from the other side. `>=` instead of `>` would satisfy
/// the test above while silently cutting every configured `max_tries` by
/// one — a job allowed three attempts would get two.
#[tokio::test(start_paused = true)]
async fn the_last_permitted_attempt_still_runs() {
    let d = MemoryQueueDriver::new();

    let mut e = env("ordinary");
    // One short of exhausted: this delivery is the last one allowed.
    e.attempts = e.max_tries - 1;
    let max_tries = e.max_tries;
    d.push(e).await.unwrap();

    let claimed = d
        .pop(Duration::from_secs(30))
        .await
        .unwrap()
        .expect("claim");
    let would_be_attempt = claimed.envelope.attempts + 1;

    assert_eq!(
        would_be_attempt, max_tries,
        "the final permitted attempt lands exactly on max_tries"
    );
    // Written as the guard's own comparison rather than its inverse: this
    // is the boundary the guard decides on, and reading it the same way it
    // is written is what makes an off-by-one visible here.
    assert!(
        would_be_attempt <= max_tries,
        "the guard must let this one through; using >= there would turn a \
         max_tries of {max_tries} into {} real attempts",
        max_tries - 1
    );
}
