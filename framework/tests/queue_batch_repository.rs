//! `BatchRepository` is a contract, not an implementation, so the memory and
//! database backends are held to exactly the same behaviour here.
//!
//! That symmetry is the point. `MemoryBatchRepository` guards its counters
//! with a `HashSet` of settled job ids; `DatabaseBatchRepository` derives them
//! from rows keyed `(batch_id, job_id)`. Two different mechanisms answering one
//! set of assertions is what keeps a batch behaving the same after an operator
//! switches to a durable repository - and what would catch either drifting.

use chrono::Utc;
use sea_orm::{ConnectionTrait, Database, DatabaseConnection};
use std::sync::Arc;
use suprnova::queue::{
    Batch, BatchOptions, BatchRepository, DatabaseBatchRepository, MemoryBatchRepository,
};
use uuid::Uuid;

async fn fresh_db() -> DatabaseConnection {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    db.execute_unprepared(
        r"
        CREATE TABLE job_batches (
            id            TEXT PRIMARY KEY,
            name          TEXT NOT NULL,
            total_jobs    INTEGER NOT NULL,
            options_json  TEXT NOT NULL,
            created_at    INTEGER NOT NULL,
            cancelled_at  INTEGER NULL,
            finished_at   INTEGER NULL
        );
        CREATE TABLE job_batch_settlements (
            batch_id   TEXT NOT NULL,
            job_id     TEXT NOT NULL,
            failed     INTEGER NOT NULL,
            settled_at INTEGER NOT NULL,
            PRIMARY KEY (batch_id, job_id)
        );
    ",
    )
    .await
    .unwrap();
    db
}

fn fresh(name: &str, total: u64) -> Batch {
    Batch {
        id: Uuid::new_v4().to_string(),
        name: name.into(),
        total_jobs: total,
        pending_jobs: total,
        failed_jobs: 0,
        failed_job_ids: Vec::new(),
        options: BatchOptions::default(),
        created_at: Utc::now(),
        cancelled_at: None,
        finished_at: None,
    }
}

/// Both repositories, so every contract test below runs against each.
async fn backends() -> Vec<(&'static str, Arc<dyn BatchRepository>)> {
    vec![
        ("memory", Arc::new(MemoryBatchRepository::new())),
        (
            "database",
            Arc::new(DatabaseBatchRepository::new(fresh_db().await)),
        ),
    ]
}

// ---------------------------------------------------------------------------
// The settlement contract
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_settled_job_consumes_exactly_one_pending_slot() {
    for (label, repo) in backends().await {
        let b = fresh("X", 3);
        let id = b.id.clone();
        repo.store(b).await.unwrap();

        let u = repo
            .record_successful_job(&id, Uuid::new_v4())
            .await
            .unwrap();
        assert_eq!(u.pending_jobs, 2, "{label}");
        assert_eq!(u.failed_jobs, 0, "{label}");
    }
}

#[tokio::test]
async fn a_failed_job_moves_both_counters_and_is_listed() {
    for (label, repo) in backends().await {
        let b = fresh("X", 3);
        let id = b.id.clone();
        repo.store(b).await.unwrap();

        let job = Uuid::new_v4();
        let u = repo.record_failed_job(&id, job).await.unwrap();
        assert_eq!(u.pending_jobs, 2, "{label}");
        assert_eq!(u.failed_jobs, 1, "{label}");

        let snap = repo.find(&id).await.unwrap().unwrap();
        assert_eq!(snap.failed_job_ids, vec![job], "{label}");
    }
}

/// Queues are at-least-once. `pending_jobs` gates the batch callbacks, so a
/// double decrement fires `then`/`finally` while other jobs are still running.
#[tokio::test]
async fn a_redelivered_success_settles_the_job_only_once() {
    for (label, repo) in backends().await {
        let b = fresh("redelivery", 3);
        let id = b.id.clone();
        repo.store(b).await.unwrap();

        let job = Uuid::new_v4();
        let first = repo.record_successful_job(&id, job).await.unwrap();
        let second = repo.record_successful_job(&id, job).await.unwrap();

        assert_eq!(
            first.pending_jobs, 2,
            "{label}: the first settlement counts"
        );
        assert_eq!(
            second.pending_jobs, 2,
            "{label}: the same job settled twice must not decrement twice"
        );
    }
}

#[tokio::test]
async fn a_redelivered_failure_counts_once_in_both_counters() {
    for (label, repo) in backends().await {
        let b = fresh("redelivery-fail", 3);
        let id = b.id.clone();
        repo.store(b).await.unwrap();

        let job = Uuid::new_v4();
        repo.record_failed_job(&id, job).await.unwrap();
        let second = repo.record_failed_job(&id, job).await.unwrap();

        assert_eq!(second.failed_jobs, 1, "{label}");
        assert_eq!(second.pending_jobs, 2, "{label}");

        let snap = repo.find(&id).await.unwrap().unwrap();
        assert_eq!(
            snap.failed_job_ids,
            vec![job],
            "{label}: the id list stays deduplicated too"
        );
    }
}

/// A job that succeeded and is then redelivered and fails must not retroactively
/// fail the batch - its pending slot is already spent.
#[tokio::test]
async fn a_job_that_settles_both_ways_consumes_one_slot() {
    for (label, repo) in backends().await {
        let b = fresh("mixed", 2);
        let id = b.id.clone();
        repo.store(b).await.unwrap();

        let job = Uuid::new_v4();
        repo.record_successful_job(&id, job).await.unwrap();
        let after = repo.record_failed_job(&id, job).await.unwrap();

        assert_eq!(after.pending_jobs, 1, "{label}");
        assert_eq!(
            after.failed_jobs, 0,
            "{label}: the first settlement is the one that counts"
        );
    }
}

/// The control: the guard must key on the job id, not suppress every repeat
/// call, or a batch would never finish.
#[tokio::test]
async fn distinct_jobs_each_settle_normally() {
    for (label, repo) in backends().await {
        let b = fresh("distinct", 3);
        let id = b.id.clone();
        repo.store(b).await.unwrap();

        repo.record_successful_job(&id, Uuid::new_v4())
            .await
            .unwrap();
        repo.record_successful_job(&id, Uuid::new_v4())
            .await
            .unwrap();
        let third = repo
            .record_successful_job(&id, Uuid::new_v4())
            .await
            .unwrap();

        assert_eq!(
            third.pending_jobs, 0,
            "{label}: three distinct jobs settle it"
        );
    }
}

#[tokio::test]
async fn cancel_and_finish_are_observable() {
    for (label, repo) in backends().await {
        let b = fresh("X", 3);
        let id = b.id.clone();
        repo.store(b).await.unwrap();

        assert!(!repo.is_cancelled(&id).await.unwrap(), "{label}");
        repo.cancel(&id).await.unwrap();
        assert!(repo.is_cancelled(&id).await.unwrap(), "{label}");

        repo.mark_finished(&id).await.unwrap();
        let snap = repo.find(&id).await.unwrap().unwrap();
        assert!(snap.finished_at.is_some(), "{label}");
        assert!(snap.cancelled(), "{label}");
    }
}

#[tokio::test]
async fn growing_a_batch_raises_both_total_and_pending() {
    for (label, repo) in backends().await {
        let b = fresh("growing", 2);
        let id = b.id.clone();
        repo.store(b).await.unwrap();

        let u = repo.increment_total_jobs(&id, 3).await.unwrap();
        assert_eq!(u.pending_jobs, 5, "{label}");

        let snap = repo.find(&id).await.unwrap().unwrap();
        assert_eq!(snap.total_jobs, 5, "{label}");
    }
}

#[tokio::test]
async fn settling_against_an_unknown_batch_is_an_error() {
    for (label, repo) in backends().await {
        let missing = Uuid::new_v4().to_string();
        assert!(
            repo.record_successful_job(&missing, Uuid::new_v4())
                .await
                .is_err(),
            "{label}: a settlement for a batch that does not exist must not \
             silently succeed"
        );
        assert!(
            repo.increment_total_jobs(&missing, 1).await.is_err(),
            "{label}"
        );
        assert!(repo.find(&missing).await.unwrap().is_none(), "{label}");
    }
}

#[tokio::test]
async fn delete_removes_the_batch_and_reports_whether_it_existed() {
    for (label, repo) in backends().await {
        let b = fresh("doomed", 2);
        let id = b.id.clone();
        repo.store(b).await.unwrap();
        repo.record_successful_job(&id, Uuid::new_v4())
            .await
            .unwrap();

        assert!(repo.delete(&id).await.unwrap(), "{label}");
        assert!(repo.find(&id).await.unwrap().is_none(), "{label}");
        assert!(
            !repo.delete(&id).await.unwrap(),
            "{label}: deleting twice reports the second as a no-op"
        );
    }
}

#[tokio::test]
async fn progress_reflects_settled_jobs() {
    for (label, repo) in backends().await {
        let b = fresh("progress", 4);
        let id = b.id.clone();
        repo.store(b).await.unwrap();
        repo.record_successful_job(&id, Uuid::new_v4())
            .await
            .unwrap();
        repo.record_successful_job(&id, Uuid::new_v4())
            .await
            .unwrap();
        repo.record_successful_job(&id, Uuid::new_v4())
            .await
            .unwrap();

        let snap = repo.find(&id).await.unwrap().unwrap();
        assert_eq!(snap.progress(), 75, "{label}");
        assert_eq!(snap.processed_jobs(), 3, "{label}");
        assert!(!snap.finished(), "{label}");
    }
}

// ---------------------------------------------------------------------------
// What only the durable repository can be asked
// ---------------------------------------------------------------------------

/// The reason this backend exists: batch accounting has to survive the process
/// that created it. A `MemoryBatchRepository` loses every in-flight batch on
/// restart, which strands `pending_jobs` forever and means the callbacks never
/// fire.
#[tokio::test]
async fn batch_state_survives_a_new_repository_over_the_same_database() {
    let db = fresh_db().await;
    let id = {
        let repo = DatabaseBatchRepository::new(db.clone());
        let b = fresh("durable", 3);
        let id = b.id.clone();
        repo.store(b).await.unwrap();
        repo.record_successful_job(&id, Uuid::new_v4())
            .await
            .unwrap();
        repo.record_failed_job(&id, Uuid::new_v4()).await.unwrap();
        id
    };

    // A fresh repository - standing in for the worker that came back after a
    // restart - sees exactly what the previous process recorded.
    let after = DatabaseBatchRepository::new(db);
    let snap = after.find(&id).await.unwrap().expect("the batch persisted");
    assert_eq!(snap.total_jobs, 3);
    assert_eq!(snap.pending_jobs, 1);
    assert_eq!(snap.failed_jobs, 1);
    assert_eq!(snap.failed_job_ids.len(), 1);
    assert_eq!(snap.name, "durable");
}

/// The idempotency guard is the `(batch_id, job_id)` primary key rather than
/// in-process bookkeeping, so it holds across processes too - the case a
/// `HashSet` cannot cover at all.
#[tokio::test]
async fn the_settlement_guard_holds_across_repository_instances() {
    let db = fresh_db().await;
    let first = DatabaseBatchRepository::new(db.clone());
    let b = fresh("cross-process", 2);
    let id = b.id.clone();
    first.store(b).await.unwrap();

    let job = Uuid::new_v4();
    first.record_successful_job(&id, job).await.unwrap();

    // A different worker process settles the same redelivered job.
    let second = DatabaseBatchRepository::new(db);
    let counts = second.record_successful_job(&id, job).await.unwrap();

    assert_eq!(
        counts.pending_jobs, 1,
        "one job settled twice by two processes still consumes one slot"
    );
}

/// Reusing a batch id after a delete must not inherit the previous batch's
/// settlements - it would start life already finished.
#[tokio::test]
async fn deleting_a_batch_takes_its_settlement_rows_with_it() {
    let db = fresh_db().await;
    let repo = DatabaseBatchRepository::new(db);
    let b = fresh("reused", 2);
    let id = b.id.clone();
    repo.store(b).await.unwrap();
    repo.record_successful_job(&id, Uuid::new_v4())
        .await
        .unwrap();
    repo.delete(&id).await.unwrap();

    let mut again = fresh("reused-again", 2);
    again.id = id.clone();
    repo.store(again).await.unwrap();

    let snap = repo.find(&id).await.unwrap().unwrap();
    assert_eq!(
        snap.pending_jobs, 2,
        "the new batch starts with all its jobs outstanding"
    );
}

/// Table names are interpolated into every statement, so they get the same
/// identifier validation the queue driver's table gets.
#[tokio::test]
async fn table_names_are_validated_as_identifiers() {
    let db = fresh_db().await;
    assert!(
        DatabaseBatchRepository::with_tables(
            db.clone(),
            "job_batches; DROP TABLE users".into(),
            "job_batch_settlements".into(),
        )
        .is_err(),
        "a hostile batches table name is rejected at construction"
    );
    assert!(
        DatabaseBatchRepository::with_tables(
            db,
            "job_batches".into(),
            "settlements WHERE 1=1".into(),
        )
        .is_err(),
        "and so is a hostile settlements table name"
    );
}
