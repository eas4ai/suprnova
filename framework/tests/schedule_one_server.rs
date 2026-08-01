//! F-3 — a scheduled task runs once per tick across replicas, not once
//! per replica.
//!
//! # The defect
//!
//! Nothing elected a leader for a due tick. Each `schedule:work` process
//! evaluated `is_due()` independently and ran. The per-process
//! `AtomicI64` minute-dedup inside the scheduler stops *one* process
//! running a tick twice; it says nothing about the other replicas, because
//! it is an atomic in this process's memory.
//!
//! Measured on the benchmark host with three replicas over four minutes —
//! `runs=3, instances=3` on every single minute, no variance
//! (`bench/results/phase1/scheduler/per-minute.txt`). A nightly billing
//! job on three replicas bills every customer three times.
//!
//! # Why this is not `without_overlapping`
//!
//! `without_overlapping` already takes a cross-process `Cache::lock`, so
//! it looks like the answer and is not. Its lock is keyed on the task and
//! held for the task's *duration*: a fast task acquires and releases
//! before a second replica even looks, so every replica still runs. The
//! control test at the bottom of this file drives exactly that, because
//! "why do both of these exist" is the question anyone reading the
//! scheduler will ask.
//!
//! `on_one_server` keys on the task *and the tick*, and holds the lock
//! past the handler so a late replica loses. That difference is the whole
//! feature.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use suprnova::testing::{TestContainer, TestContainerGuard};
use suprnova::{CacheStore, InMemoryCache, Schedule};

/// Two `Schedule` instances against one shared cache is the in-process
/// stand-in for two replicas: separate schedulers, separate per-process
/// dedup state, one coordination backend. It reproduces the measured
/// failure without needing three containers.
///
/// The guard must outlive the test body — dropping it restores the
/// previous container, and these tests run in parallel in one binary.
#[must_use]
fn install_shared_cache() -> TestContainerGuard {
    let guard = TestContainer::fake();
    TestContainer::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));
    guard
}

/// A distinct task name per test. The election lock key is derived from
/// the name, and these tests run in one binary in parallel — a shared name
/// would make one test's claim decide another's outcome.
fn unique_name(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4())
}

/// The bug, stated as the sequence that produced it: two schedulers, one
/// due tick, and the task must run once.
#[tokio::test]
async fn two_replicas_run_a_single_server_task_once_per_tick() {
    let _cache = install_shared_cache();

    let runs = Arc::new(AtomicUsize::new(0));
    let name = unique_name("one-server");

    let mut replica_a = Schedule::new();
    let counter = Arc::clone(&runs);
    let task = replica_a
        .call(move || {
            let counter = Arc::clone(&counter);
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .name(&name)
        .every_minute()
        .on_one_server();
    replica_a.add(task);

    let mut replica_b = Schedule::new();
    let counter = Arc::clone(&runs);
    let task = replica_b
        .call(move || {
            let counter = Arc::clone(&counter);
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .name(&name)
        .every_minute()
        .on_one_server();
    replica_b.add(task);

    // Both replicas evaluate the same tick, the way three containers did.
    for (_name, result) in replica_a.run_due_tasks().await {
        result.expect("replica A must not error");
    }
    for (_name, result) in replica_b.run_due_tasks().await {
        result.expect("replica B must not error");
    }

    assert_eq!(
        runs.load(Ordering::SeqCst),
        1,
        "one due tick must produce one execution across replicas; 2 means every \
         replica ran it, which is the measured defect"
    );
}

/// The control that keeps the fix honest in the other direction: an
/// election that never lets anybody through would pass the test above.
#[tokio::test]
async fn a_single_replica_still_runs_the_task() {
    let _cache = install_shared_cache();

    let runs = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&runs);

    let mut schedule = Schedule::new();
    let task = schedule
        .call(move || {
            let counter = Arc::clone(&counter);
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .name(&unique_name("solo"))
        .every_minute()
        .on_one_server();
    schedule.add(task);

    for (_name, result) in schedule.run_due_tasks().await {
        result.expect("the only replica must run the task");
    }

    assert_eq!(
        runs.load(Ordering::SeqCst),
        1,
        "the winner of an uncontested election still has to run"
    );
}

/// Why both APIs exist, asserted rather than described.
///
/// `without_overlapping` holds its lock only for the task body, so a fast
/// task releases before the next replica looks and both run. If this ever
/// starts passing with a count of 1, `without_overlapping` has silently
/// become `on_one_server` and one of them should be deleted.
#[tokio::test]
async fn without_overlapping_alone_does_not_elect_one_replica() {
    let _cache = install_shared_cache();

    let runs = Arc::new(AtomicUsize::new(0));
    let name = unique_name("overlap-only");

    let mut replica_a = Schedule::new();
    let counter = Arc::clone(&runs);
    let task = replica_a
        .call(move || {
            let counter = Arc::clone(&counter);
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .name(&name)
        .every_minute()
        .without_overlapping();
    replica_a.add(task);

    let mut replica_b = Schedule::new();
    let counter = Arc::clone(&runs);
    let task = replica_b
        .call(move || {
            let counter = Arc::clone(&counter);
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .name(&name)
        .every_minute()
        .without_overlapping();
    replica_b.add(task);

    for (_name, result) in replica_a.run_due_tasks().await {
        result.expect("replica A");
    }
    for (_name, result) in replica_b.run_due_tasks().await {
        result.expect("replica B");
    }

    assert_eq!(
        runs.load(Ordering::SeqCst),
        2,
        "without_overlapping releases its lock when the handler returns, so a \
         second replica finds it free and runs the same tick — this is the gap \
         on_one_server exists to close, not a bug in this test"
    );
}

/// The lock must not outlive its tick. A TTL longer than the interval
/// would make the *next* due run find the lock still held and skip it —
/// turning "runs on one server" into "runs once, ever".
#[tokio::test]
async fn a_later_tick_is_a_separate_election() {
    let _cache = install_shared_cache();

    let name = unique_name("next-tick");
    let runs = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&runs);

    let mut schedule = Schedule::new();
    let task = schedule
        .call(move || {
            let counter = Arc::clone(&counter);
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .name(&name)
        // A one-second window so the test does not sit through a minute.
        .every_minute()
        .on_one_server_for(Duration::from_secs(1));
    schedule.add(task);

    for (_name, result) in schedule.run_due_tasks().await {
        result.expect("first tick");
    }
    assert_eq!(runs.load(Ordering::SeqCst), 1);

    // A fresh scheduler stands in for the next tick's process: the
    // per-process minute dedup would otherwise suppress the second run for
    // reasons unrelated to the election, and this test is about the lock.
    tokio::time::sleep(Duration::from_millis(1200)).await;

    let counter = Arc::clone(&runs);
    let mut later = Schedule::new();
    let task = later
        .call(move || {
            let counter = Arc::clone(&counter);
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .name(&name)
        .every_minute()
        .on_one_server_for(Duration::from_secs(1));
    later.add(task);

    for (_name, result) in later.run_due_tasks().await {
        result.expect("second tick");
    }

    assert_eq!(
        runs.load(Ordering::SeqCst),
        2,
        "once the lock expires the next tick must be electable again; a lock \
         that outlives its tick turns this feature into a one-shot"
    );
}

// ---------------------------------------------------------------------------
// The production boot guard
// ---------------------------------------------------------------------------
//
// The election is a `Cache::lock`. Under `CACHE_DRIVER=memory` that lock
// lives in one process's heap, so every replica wins its own election and
// every replica runs the task — the exact outcome `on_one_server` was
// called to prevent, with nothing in the logs to say so.
//
// Same shape as the in-memory rate limiter, same answer: fail the boot in
// production, offer an acknowledgement for the operator who really does
// run one scheduler, stay quiet everywhere else.
//
// These drive `validate_single_server_locking` directly rather than
// through `APP_ENV`/`CACHE_DRIVER` env writes, which would race every
// other test in this binary.

/// A schedule with no single-server tasks has nothing to guard, whatever
/// the cache is. Without this, adding the check would break every existing
/// production deployment that never asked for the feature.
#[test]
fn a_schedule_without_single_server_tasks_is_never_blocked() {
    let mut schedule = Schedule::new();
    let task = schedule
        .call(|| async { Ok(()) })
        .name(&unique_name("ordinary"))
        .every_minute();
    schedule.add(task);

    assert!(
        schedule.validate_single_server_locking().is_ok(),
        "a task that never asked for single-server execution cannot be \
         affected by how the cache is configured"
    );
}

/// Outside production the memory driver is the useful default for a
/// single-process dev loop, so the guard must not fire there — it warns
/// instead.
#[test]
fn outside_production_a_memory_cache_is_allowed() {
    let mut schedule = Schedule::new();
    let task = schedule
        .call(|| async { Ok(()) })
        .name(&unique_name("dev-loop"))
        .every_minute()
        .on_one_server();
    schedule.add(task);

    // The suite runs with APP_ENV unset or `testing`; neither is
    // production, so this is the non-production branch.
    assert!(
        schedule.validate_single_server_locking().is_ok(),
        "blocking dev loops would make the feature unusable locally"
    );
}
