//! PAR-186: `QUEUE_DRIVER=mongodb` selects a `QueueDriver` over a `jobs`
//! collection, with its failed-job store over `failed_jobs` and its batch
//! repository over `job_batches`.
//!
//! The unignored tests read the documents each store sends (rendered
//! without a server), the selection by the environment (in a child process
//! with the variables it needs) and the errors of a server that does not
//! answer. The `mongodb_` tests run each falsifier against the server at
//! `MONGODB_TEST_URL`: two workers racing for one job, a delayed job, a
//! nack with a delay, the sizes, the batch counters racing, and a failed
//! job listed by the failed-job store.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use serde::{Deserialize, Serialize};
use suprnova::bson::{Bson, Document, doc};
use suprnova::queue::worker::{WorkerConfig, register_job, run_worker};
use suprnova::queue::{
    BackoffSchedule, Batch, BatchOptions, BatchRepository, CURRENT_SCHEMA_VERSION, Envelope,
    FailedJobStore, Job, Queue, QueueDriver, ReservationToken,
};
use suprnova::testing::TestClock;
use suprnova::{
    FrameworkError, Mongo, MongoBatchRepository, MongoFailedJobStore, MongoQueueDriver,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::store_support::{drop_collections, run_alone_with_drivers, server, unique, unreachable};
use crate::support::UNREACHABLE_URI;

fn is_child() -> bool {
    crate::own_process::is_child()
}

/// A fixed moment a quarter of a second past a whole second, so a test sees
/// which way a stored time rounds.
fn moment() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-10T12:00:00.250Z")
        .expect("valid time")
        .with_timezone(&Utc)
}

fn bson_time(at: DateTime<Utc>) -> suprnova::bson::DateTime {
    suprnova::bson::DateTime::from_millis(at.timestamp_millis())
}

fn seconds(n: i64) -> chrono::Duration {
    chrono::Duration::seconds(n)
}

fn envelope(name: &str, queue: Option<&str>, available_at: DateTime<Utc>) -> Envelope {
    Envelope {
        schema_version: CURRENT_SCHEMA_VERSION,
        id: Uuid::new_v4(),
        job_name: name.to_owned(),
        queue: queue.map(str::to_owned),
        message_group: None,
        deduplication_id: None,
        payload: serde_json::json!({ "report": 7 }),
        dispatched_at: available_at,
        available_at,
        attempts: 0,
        max_tries: 3,
        backoff: BackoffSchedule::default(),
        timeout_secs: None,
        fail_on_timeout: false,
        idempotency_key: None,
        unique_lock_owner: None,
        debounce_id: None,
        debounce_owner: None,
        batch_id: None,
        chain_remaining: Vec::new(),
        context: None,
    }
}

fn names_mongodb(error: &FrameworkError) -> bool {
    error.to_string().contains("MongoDB")
}

// --- Selection by the environment ---------------------------------------------

#[test]
fn queue_driver_mongodb_selects_the_driver_its_failed_job_store_and_its_batches() {
    run_alone_with_drivers(
        "queue::queue_driver_mongodb_selects_the_driver_its_failed_job_store_and_its_batches_child",
        &[
            ("QUEUE_DRIVER", "mongodb"),
            ("MONGODB_URI", UNREACHABLE_URI),
            ("MONGODB_DATABASE", "suprnova_queue"),
        ],
    );
}

#[tokio::test]
async fn queue_driver_mongodb_selects_the_driver_its_failed_job_store_and_its_batches_child() {
    if !is_child() {
        return;
    }
    Mongo::bootstrap()
        .await
        .expect("the boot registers the connection");
    suprnova::queue::bootstrap_from_env()
        .await
        .expect("QUEUE_DRIVER=mongodb boots without a server");
    assert_eq!(Queue::driver_name().expect("a driver"), "mongodb");

    // The stores are the MongoDB ones: with no server, they fail as MongoDB.
    let failed = Queue::failed_store().expect("the driver brings its failed-job store");
    let error = failed.count().await.expect_err("no server answers");
    assert!(names_mongodb(&error), "{error}");
    let batches = Queue::batch_repository().expect("and its batch repository");
    let error = batches.find("absent").await.expect_err("no server answers");
    assert!(names_mongodb(&error), "{error}");
}

#[test]
fn queue_driver_mongodb_without_a_connection_is_an_error_naming_mongodb_uri() {
    run_alone_with_drivers(
        "queue::queue_driver_mongodb_without_a_connection_is_an_error_naming_mongodb_uri_child",
        &[("QUEUE_DRIVER", "mongodb")],
    );
}

#[tokio::test]
async fn queue_driver_mongodb_without_a_connection_is_an_error_naming_mongodb_uri_child() {
    if !is_child() {
        return;
    }
    let error = suprnova::queue::bootstrap_from_env()
        .await
        .expect_err("no MongoDB connection to run on");
    let message = error.to_string();
    assert!(message.contains("MONGODB_URI"), "{message}");
    assert!(message.contains("mongodb"), "{message}");
}

// --- The documents the driver sends ----------------------------------------------

#[tokio::test]
async fn a_push_inserts_the_envelope_with_its_queue_and_availability() {
    let driver = MongoQueueDriver::new(&unreachable().await);
    let now = moment();

    let due = envelope("Report", None, now);
    let document = driver.rendered_push(&due, now).expect("rendered");
    assert_eq!(document.get_str("queue").unwrap(), "default");
    assert_eq!(document.get_i64("available_at").unwrap(), now.timestamp());
    assert_eq!(document.get_i64("created_at").unwrap(), now.timestamp());
    assert_eq!(document.get_i64("attempts").unwrap(), 0);
    assert_eq!(document.get("reserved_at"), Some(&Bson::Null));
    assert_eq!(document.get("token"), Some(&Bson::Null));
    assert_eq!(document.get("reserved_until"), Some(&Bson::Null));
    let payload = document.get_str("payload").unwrap();
    assert!(payload.starts_with("{\"schema_version\":"), "{payload}");
    assert!(payload.contains("\"displayName\":\"Report\""), "{payload}");
    assert!(payload.contains(&due.id.to_string()), "{payload}");

    // A delayed job rounds its time up, so it is never handed out early.
    let delayed = envelope("Report", Some("reports"), now + seconds(30));
    let document = driver.rendered_push(&delayed, now).expect("rendered");
    assert_eq!(document.get_str("queue").unwrap(), "reports");
    assert_eq!(
        document.get_i64("available_at").unwrap(),
        now.timestamp() + 31,
        "12:00:30.250 is stored as 12:00:31"
    );
}

#[tokio::test]
async fn a_pop_reserves_one_job_with_one_find_one_and_update() {
    let driver = MongoQueueDriver::new(&unreachable().await);
    let now = moment();
    let token = Uuid::new_v4();
    let rendered = driver.rendered_pop(
        &["high".to_owned(), "default".to_owned()],
        Duration::from_secs(90),
        now,
        token,
    );

    let filter = rendered.get_document("filter").expect("a filter");
    assert_eq!(
        filter.get_document("queue").unwrap(),
        &doc! { "$in": ["high", "default"] }
    );
    assert_eq!(
        filter.get_document("available_at").unwrap(),
        &doc! { "$lte": now.timestamp() }
    );
    assert_eq!(
        filter.get_array("$or").unwrap(),
        &vec![
            Bson::Document(doc! { "token": Bson::Null }),
            Bson::Document(doc! { "reserved_until": { "$lte": bson_time(now) } }),
        ],
        "free: never reserved, or its reservation has lapsed"
    );
    assert!(
        filter.contains_key("payload"),
        "only the jobs this driver wrote: {filter}"
    );

    let update = rendered.get_document("update").expect("an update");
    assert_eq!(
        update.get_document("$inc").unwrap(),
        &doc! { "attempts": 1_i64 }
    );
    let set = update.get_document("$set").unwrap();
    assert_eq!(set.get_str("token").unwrap(), token.to_string());
    assert_eq!(set.get_i64("reserved_at").unwrap(), now.timestamp());
    assert_eq!(
        *set.get_datetime("reserved_until").unwrap(),
        bson_time(now + seconds(90))
    );
    assert_eq!(
        rendered.get_document("sort").unwrap(),
        &doc! { "available_at": 1, "_id": 1 }
    );

    let any = driver.rendered_pop(&[], Duration::from_secs(90), now, token);
    assert!(
        !any.get_document("filter").unwrap().contains_key("queue"),
        "no filter means any queue"
    );
}

#[tokio::test]
async fn nack_and_release_return_the_job_with_the_given_delay() {
    let driver = MongoQueueDriver::new(&unreachable().await);
    let now = moment();
    let token = ReservationToken(Uuid::new_v4());

    let nack = driver
        .rendered_requeue(&token, Duration::from_secs(30), true, now)
        .expect("rendered");
    assert_eq!(
        nack.get_document("filter").unwrap(),
        &doc! { "token": token.0.to_string() }
    );
    let update = nack.get_document("update").unwrap();
    let set = update.get_document("$set").unwrap();
    assert_eq!(set.get_i64("available_at").unwrap(), now.timestamp() + 31);
    assert_eq!(set.get("token"), Some(&Bson::Null));
    assert_eq!(set.get("reserved_at"), Some(&Bson::Null));
    assert_eq!(set.get("reserved_until"), Some(&Bson::Null));
    assert!(
        !update.contains_key("$inc"),
        "a nack keeps the attempt the pop counted"
    );

    let release = driver
        .rendered_requeue(&token, Duration::ZERO, false, now)
        .expect("rendered");
    let update = release.get_document("update").unwrap();
    assert_eq!(
        update.get_document("$inc").unwrap(),
        &doc! { "attempts": -1_i64 },
        "a release gives the attempt back"
    );
    assert_eq!(
        update
            .get_document("$set")
            .unwrap()
            .get_i64("available_at")
            .unwrap(),
        now.timestamp(),
        "no delay: available at once"
    );
}

#[test]
fn a_batch_settlement_moves_its_counters_in_one_update() {
    let job = Uuid::new_v4();
    let success = MongoBatchRepository::rendered_settlement("batch-1", job, false);
    assert_eq!(
        success.get_document("filter").unwrap(),
        &doc! {
            "_id": "batch-1",
            "settled_job_ids": { "$ne": job.to_string() },
            "pending_jobs": { "$gt": 0_i64 },
        }
    );
    assert_eq!(
        success.get_document("update").unwrap(),
        &doc! {
            "$inc": { "pending_jobs": -1_i64 },
            "$push": { "settled_job_ids": job.to_string() },
        }
    );

    let failure = MongoBatchRepository::rendered_settlement("batch-1", job, true);
    assert_eq!(
        failure.get_document("update").unwrap(),
        &doc! {
            "$inc": { "pending_jobs": -1_i64, "failed_jobs": 1_i64 },
            "$push": { "settled_job_ids": job.to_string() },
            "$addToSet": { "failed_job_ids": job.to_string() },
        }
    );

    let now = moment();
    let claim = MongoBatchRepository::rendered_claim("batch-1", now);
    assert_eq!(
        claim.get_document("filter").unwrap(),
        &doc! { "_id": "batch-1", "pending_jobs": 0_i64, "finished_at": Bson::Null }
    );
    assert_eq!(
        claim.get_document("update").unwrap(),
        &doc! { "$set": { "finished_at": bson_time(now) } }
    );
}

// --- Errors -------------------------------------------------------------------------

#[tokio::test]
async fn a_server_that_does_not_answer_fails_each_store_as_mongodb() {
    let connection = unreachable().await;
    let driver = MongoQueueDriver::new(&connection);
    let error = driver
        .push(envelope("Report", None, suprnova::clock::now()))
        .await
        .expect_err("no server answers");
    assert!(names_mongodb(&error), "{error}");
    let error = driver
        .pop(Duration::from_secs(5))
        .await
        .expect_err("no server answers");
    assert!(names_mongodb(&error), "{error}");

    let failed = MongoFailedJobStore::new(&connection);
    let error = failed
        .log(
            "mongodb",
            "default",
            &envelope("Report", None, suprnova::clock::now()),
            "boom",
        )
        .await
        .expect_err("no server answers");
    assert!(names_mongodb(&error), "{error}");

    let batches = MongoBatchRepository::new(&connection);
    let error = batches
        .record_successful_job("batch-1", Uuid::new_v4())
        .await
        .expect_err("no server answers");
    assert!(names_mongodb(&error), "{error}");
}

#[tokio::test]
async fn a_collection_name_mongodb_refuses_is_an_error_naming_it() {
    let connection = unreachable().await;
    for bad in ["", "jobs$", "system.jobs", "jo\0bs"] {
        let error = MongoQueueDriver::with_collection(&connection, bad)
            .err()
            .unwrap_or_else(|| panic!("{bad:?} is refused"));
        assert!(error.to_string().contains("collection"), "{error}");
        assert!(MongoFailedJobStore::with_collection(&connection, bad).is_err());
        assert!(MongoBatchRepository::with_collection(&connection, bad).is_err());
    }
    let driver = MongoQueueDriver::with_collection(&connection, "queued_jobs").expect("valid");
    assert_eq!(driver.collection().name(), "queued_jobs");
    assert_eq!(
        MongoQueueDriver::new(&connection).collection().name(),
        "jobs"
    );
}

// --- Against a server --------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_two_workers_racing_for_one_job_never_both_receive_it() {
    let connection = server().await;
    let name = unique("par186_race");
    let first = Arc::new(MongoQueueDriver::with_collection(&connection, &name).unwrap());
    // A second pool, as a second worker process has.
    let second = Arc::new(MongoQueueDriver::with_collection(&server().await, &name).unwrap());

    for round in 0..25 {
        first
            .push(envelope("Race", None, suprnova::clock::now()))
            .await
            .expect("push");
        let (a, b) = (Arc::clone(&first), Arc::clone(&second));
        let left = tokio::spawn(async move { a.pop(Duration::from_secs(60)).await });
        let right = tokio::spawn(async move { b.pop(Duration::from_secs(60)).await });
        let reserved: Vec<_> = [left.await.unwrap(), right.await.unwrap()]
            .into_iter()
            .filter_map(|result| result.expect("pop"))
            .collect();
        assert_eq!(reserved.len(), 1, "round {round}: one worker, not two");
        first.ack(&reserved[0].token).await.expect("ack");
    }
    assert_eq!(first.size(None).await.unwrap(), 0);
    drop_collections(&connection, &[&name]).await;
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_a_delayed_job_is_not_reserved_before_its_time() {
    let connection = server().await;
    let name = unique("par186_delay");
    let driver = MongoQueueDriver::with_collection(&connection, &name).unwrap();
    let clock = TestClock::freeze();
    let now = clock.now();

    driver
        .push(envelope("Later", None, now + seconds(10)))
        .await
        .expect("push");
    assert!(driver.pop(Duration::from_secs(60)).await.unwrap().is_none());
    assert_eq!(driver.delayed_size(None).await.unwrap(), 1);
    assert_eq!(driver.pending_size(None).await.unwrap(), 0);

    clock.advance(seconds(9));
    assert!(
        driver.pop(Duration::from_secs(60)).await.unwrap().is_none(),
        "nine seconds in, a ten-second delay still holds"
    );
    clock.advance(seconds(2));
    let reserved = driver
        .pop(Duration::from_secs(60))
        .await
        .unwrap()
        .expect("due now");
    assert_eq!(reserved.envelope.job_name, "Later");
    assert_eq!(reserved.envelope.attempts, 0);
    drop_collections(&connection, &[&name]).await;
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_nack_with_a_delay_does_not_make_the_job_available_at_once() {
    let connection = server().await;
    let name = unique("par186_nack");
    let driver = MongoQueueDriver::with_collection(&connection, &name).unwrap();
    let clock = TestClock::freeze();
    let visibility = Duration::from_secs(60);

    driver
        .push(envelope("Flaky", None, clock.now()))
        .await
        .expect("push");
    let first = driver.pop(visibility).await.unwrap().expect("due");
    assert_eq!(first.envelope.attempts, 0);
    driver
        .nack(&first.token, Duration::from_secs(30))
        .await
        .expect("nack");
    assert!(
        driver.pop(visibility).await.unwrap().is_none(),
        "the delay holds the job back"
    );
    assert_eq!(driver.delayed_size(None).await.unwrap(), 1);
    assert_eq!(driver.reserved_size(None).await.unwrap(), 0);

    clock.advance(seconds(31));
    let second = driver.pop(visibility).await.unwrap().expect("due again");
    assert_eq!(second.envelope.attempts, 1, "the nack spent an attempt");

    driver
        .nack(&second.token, Duration::ZERO)
        .await
        .expect("nack");
    let third = driver.pop(visibility).await.unwrap().expect("at once");
    assert_eq!(third.envelope.attempts, 2);

    driver
        .release(&third.token, &third.envelope, Duration::ZERO)
        .await
        .expect("release");
    let fourth = driver.pop(visibility).await.unwrap().expect("released");
    assert_eq!(fourth.envelope.attempts, 2, "a release spends no attempt");

    driver.ack(&fourth.token).await.expect("ack");
    driver
        .ack(&fourth.token)
        .await
        .expect("a second ack is a no-op");
    driver
        .nack(&fourth.token, Duration::ZERO)
        .await
        .expect("a nack of a settled job is a no-op");
    assert_eq!(driver.size(None).await.unwrap(), 0);
    drop_collections(&connection, &[&name]).await;
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_a_lapsed_reservation_is_reclaimed_and_counts_the_attempt() {
    let connection = server().await;
    let name = unique("par186_lapse");
    let driver = MongoQueueDriver::with_collection(&connection, &name).unwrap();
    let clock = TestClock::freeze();

    driver
        .push(envelope("Crashes", None, clock.now()))
        .await
        .expect("push");
    let first = driver
        .pop(Duration::from_secs(5))
        .await
        .unwrap()
        .expect("due");
    assert!(driver.pop(Duration::from_secs(5)).await.unwrap().is_none());

    clock.advance(seconds(6));
    let second = driver
        .pop(Duration::from_secs(5))
        .await
        .unwrap()
        .expect("the reservation lapsed");
    assert_ne!(second.token, first.token);
    assert_eq!(
        second.envelope.attempts, 1,
        "a worker that died spent its attempt"
    );

    driver
        .ack(&first.token)
        .await
        .expect("the first worker's ack");
    assert_eq!(
        driver.size(None).await.unwrap(),
        1,
        "a stale token settles nothing"
    );
    driver.ack(&second.token).await.expect("ack");
    assert_eq!(driver.size(None).await.unwrap(), 0);
    drop_collections(&connection, &[&name]).await;
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_pop_from_honours_queue_names() {
    let connection = server().await;
    let name = unique("par186_queues");
    let driver = MongoQueueDriver::with_collection(&connection, &name).unwrap();
    let now = suprnova::clock::now();
    for queue in [Some("high"), Some("low"), None] {
        driver
            .push(envelope("Routed", queue, now))
            .await
            .expect("push");
    }
    let visibility = Duration::from_secs(60);
    let low = driver
        .pop_from(visibility, &["low".to_owned()])
        .await
        .unwrap()
        .expect("a low job");
    assert_eq!(low.envelope.queue.as_deref(), Some("low"));
    let default = driver
        .pop_from(visibility, &["default".to_owned()])
        .await
        .unwrap()
        .expect("the unrouted job is on default");
    assert_eq!(default.envelope.queue, None);
    assert!(
        driver
            .pop_from(visibility, &["missing".to_owned()])
            .await
            .unwrap()
            .is_none()
    );
    let high = driver.pop(visibility).await.unwrap().expect("any queue");
    assert_eq!(high.envelope.queue.as_deref(), Some("high"));
    drop_collections(&connection, &[&name]).await;
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_size_agrees_with_the_documents_in_the_collection() {
    let connection = server().await;
    let name = unique("par186_size");
    let driver = MongoQueueDriver::with_collection(&connection, &name).unwrap();
    let collection = connection.collection::<Document>(&name);
    let now = suprnova::clock::now();

    driver
        .push(envelope("A", Some("a"), now))
        .await
        .expect("push");
    driver.push(envelope("B", None, now)).await.expect("push");
    driver
        .push(envelope("C", None, now + seconds(3600)))
        .await
        .expect("push");
    let reserved = driver
        .pop_from(Duration::from_secs(60), &["a".to_owned()])
        .await
        .unwrap()
        .expect("the a job");

    let counted = collection.count_documents(doc! {}).await.unwrap();
    assert_eq!(driver.size(None).await.unwrap(), counted);
    assert_eq!(counted, 3);
    assert_eq!(
        driver.size(Some("a")).await.unwrap(),
        collection
            .count_documents(doc! { "queue": "a" })
            .await
            .unwrap()
    );
    assert_eq!(driver.pending_size(None).await.unwrap(), 1);
    assert_eq!(driver.delayed_size(None).await.unwrap(), 1);
    assert_eq!(driver.reserved_size(None).await.unwrap(), 1);
    assert_eq!(driver.reserved_size(Some("default")).await.unwrap(), 0);

    let pending = driver.pending_jobs(None).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].name, "B");
    let delayed = driver.delayed_jobs(None).await.unwrap();
    assert_eq!(delayed.len(), 1);
    assert_eq!(delayed[0].name, "C");
    let held = driver.reserved_jobs(None).await.unwrap();
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].id, Some(reserved.envelope.id));

    driver
        .bulk_push(vec![
            envelope("D", Some("a"), now),
            envelope("E", Some("a"), now),
        ])
        .await
        .expect("bulk push");
    assert_eq!(driver.size(None).await.unwrap(), 5);
    assert_eq!(driver.clear(Some("a")).await.unwrap(), 3);
    assert_eq!(driver.clear(None).await.unwrap(), 2);
    assert_eq!(collection.count_documents(doc! {}).await.unwrap(), 0);
    drop_collections(&connection, &[&name]).await;
}

fn batch(id: &str, jobs: u64) -> Batch {
    Batch {
        id: id.to_owned(),
        name: "imports".to_owned(),
        total_jobs: jobs,
        pending_jobs: jobs,
        failed_jobs: 0,
        failed_job_ids: Vec::new(),
        options: BatchOptions {
            then_callbacks: vec!["notify".to_owned()],
            ..BatchOptions::default()
        },
        created_at: suprnova::clock::now(),
        cancelled_at: None,
        finished_at: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_three_racing_successes_finish_a_batch_and_claim_then_once() {
    let connection = server().await;
    let name = unique("par186_batches");
    let repository = Arc::new(MongoBatchRepository::with_collection(&connection, &name).unwrap());
    let id = Uuid::new_v4().to_string();
    repository.store(batch(&id, 3)).await.expect("store");
    assert!(
        repository.store(batch(&id, 3)).await.is_err(),
        "ids are unique"
    );

    let jobs = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
    let settlements: Vec<_> = jobs
        .iter()
        .map(|job| {
            let (repository, id, job) = (Arc::clone(&repository), id.clone(), *job);
            tokio::spawn(async move { repository.record_successful_job(&id, job).await })
        })
        .collect();
    for settlement in settlements {
        settlement.await.unwrap().expect("settled");
    }
    let found = repository.find(&id).await.unwrap().expect("stored");
    assert_eq!(found.pending_jobs, 0);
    assert_eq!(found.failed_jobs, 0);
    assert_eq!(found.total_jobs, 3);
    assert_eq!(found.options.then_callbacks, vec!["notify".to_owned()]);

    let again = repository
        .record_successful_job(&id, jobs[0])
        .await
        .expect("a redelivered success");
    assert_eq!(again.pending_jobs, 0, "a redelivery settles nothing twice");

    let claims: Vec<_> = (0..3)
        .map(|_| {
            let (repository, id) = (Arc::clone(&repository), id.clone());
            tokio::spawn(async move { repository.claim_terminal_callbacks(&id).await })
        })
        .collect();
    let mut won = 0;
    for claim in claims {
        if claim.await.unwrap().expect("claim").is_some() {
            won += 1;
        }
    }
    assert_eq!(won, 1, "the then callbacks run once");
    assert!(
        repository
            .find(&id)
            .await
            .unwrap()
            .unwrap()
            .finished_at
            .is_some()
    );
    assert!(
        repository.increment_total_jobs(&id, 1).await.is_err(),
        "a finished batch takes no more jobs"
    );
    assert!(repository.delete(&id).await.unwrap());
    assert!(repository.find(&id).await.unwrap().is_none());
    drop_collections(&connection, &[&name]).await;
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_a_failed_batch_job_counts_once_and_lists_its_id() {
    let connection = server().await;
    let name = unique("par186_batch_failures");
    let repository = MongoBatchRepository::with_collection(&connection, &name).unwrap();
    let id = Uuid::new_v4().to_string();
    repository.store(batch(&id, 2)).await.expect("store");

    let grown = repository.increment_total_jobs(&id, 1).await.expect("grow");
    assert_eq!(grown.pending_jobs, 3);

    let job = Uuid::new_v4();
    let counts = repository
        .record_failed_job(&id, job)
        .await
        .expect("failed");
    assert_eq!((counts.pending_jobs, counts.failed_jobs), (2, 1));
    let counts = repository
        .record_failed_job(&id, job)
        .await
        .expect("redelivered");
    assert_eq!((counts.pending_jobs, counts.failed_jobs), (2, 1));
    let found = repository.find(&id).await.unwrap().unwrap();
    assert_eq!(found.failed_job_ids, vec![job]);
    assert!(
        repository.claim_terminal_callbacks(&id).await.is_err(),
        "pending jobs remain"
    );

    assert!(!repository.is_cancelled(&id).await.unwrap());
    repository.cancel(&id).await.expect("cancel");
    assert!(repository.is_cancelled(&id).await.unwrap());
    assert!(
        repository
            .record_successful_job("absent", Uuid::new_v4())
            .await
            .is_err(),
        "a batch that does not exist is an error"
    );
    drop_collections(&connection, &[&name]).await;
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_the_failed_job_store_lists_finds_forgets_and_flushes() {
    let connection = server().await;
    let name = unique("par186_failed");
    let store = MongoFailedJobStore::with_collection(&connection, &name).unwrap();
    let clock = TestClock::freeze();

    let first = store
        .log(
            "mongodb",
            "default",
            &envelope("First", None, clock.now()),
            "boom",
        )
        .await
        .expect("log");
    clock.advance(seconds(5));
    let second = store
        .log(
            "mongodb",
            "reports",
            &envelope("Second", Some("reports"), clock.now()),
            "bang",
        )
        .await
        .expect("log");

    let all = store.all().await.unwrap();
    assert_eq!(
        all.iter().map(|record| record.id).collect::<Vec<_>>(),
        vec![second, first],
        "newest first"
    );
    assert_eq!(all[0].job_name, "Second");
    assert_eq!(all[0].connection, "mongodb");
    assert_eq!(all[0].queue, "reports");
    assert_eq!(all[0].exception, "bang");
    assert_eq!(store.ids().await.unwrap(), vec![second, first]);
    assert_eq!(store.count().await.unwrap(), 2);
    let found = store.find(first).await.unwrap().expect("found");
    assert!(Envelope::from_json(&found.envelope_json).is_ok());
    assert!(store.find(Uuid::new_v4()).await.unwrap().is_none());

    assert!(store.forget(first).await.unwrap());
    assert!(!store.forget(first).await.unwrap());
    assert_eq!(
        store.flush(Some(clock.now() - seconds(60))).await.unwrap(),
        0
    );
    assert_eq!(
        store.flush(Some(clock.now() + seconds(1))).await.unwrap(),
        1
    );
    assert_eq!(store.count().await.unwrap(), 0);
    drop_collections(&connection, &[&name]).await;
}

#[derive(Serialize, Deserialize, Clone)]
struct DoomedJob;

#[async_trait]
impl Job for DoomedJob {
    fn job_name() -> &'static str {
        "mongodb_queue::DoomedJob"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Err(FrameworkError::internal("the job always fails"))
    }
    fn max_tries() -> u32 {
        1
    }
}

#[test]
#[ignore = "needs MONGODB_TEST_URL"]
fn mongodb_a_job_that_fails_its_last_try_is_listed_by_the_failed_job_store() {
    crate::own_process::run_alone(
        "queue::mongodb_a_job_that_fails_its_last_try_is_listed_by_the_failed_job_store_child",
    );
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_a_job_that_fails_its_last_try_is_listed_by_the_failed_job_store_child() {
    if !is_child() {
        return;
    }
    let connection = server().await;
    let (jobs, failed) = (unique("par186_worker_jobs"), unique("par186_worker_failed"));
    let driver = Arc::new(MongoQueueDriver::with_collection(&connection, &jobs).unwrap());
    let store = Arc::new(MongoFailedJobStore::with_collection(&connection, &failed).unwrap());
    register_job::<DoomedJob>();
    Queue::set_driver(driver.clone());
    Queue::set_failed_store(store.clone());

    Queue::push(DoomedJob).await.expect("push");
    let config = WorkerConfig {
        visibility_timeout: Duration::from_secs(30),
        poll_interval: Duration::from_millis(5),
        max_jobs: Some(1),
        queues: Vec::new(),
    };
    run_worker(driver.clone(), config, CancellationToken::new())
        .await
        .expect("the worker runs the job");

    let records = store.all().await.unwrap();
    assert_eq!(records.len(), 1, "the dead letter is listed");
    assert_eq!(records[0].job_name, DoomedJob::job_name());
    assert!(records[0].exception.contains("the job always fails"));
    assert_eq!(driver.size(None).await.unwrap(), 0, "and left the queue");
    let stored: Vec<Document> = connection
        .collection::<Document>(&failed)
        .find(doc! {})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(stored.len(), 1);
    assert!(
        stored[0]
            .get_str("connection")
            .unwrap()
            .starts_with("suprnova:")
    );

    assert!(Queue::retry_failed(records[0].id).await.expect("retry"));
    assert_eq!(store.count().await.unwrap(), 0);
    assert_eq!(driver.size(None).await.unwrap(), 1, "back on the queue");
    drop_collections(&connection, &[&jobs, &failed]).await;
}
