#![cfg(all(feature = "queue-sqs", feature = "testing"))]

//! The SQS queue driver (PAR-018, PAR-019, PAR-020), driven against an
//! in-process server that speaks the AWS JSON 1.0 protocol of SQS.
//!
//! The fake keeps its own clock in whole seconds, which a test moves with
//! [`fake::FakeSqs::advance`], so delays and visibility timeouts are
//! observable without sleeping. The driver reads the framework clock, which
//! the delay tests freeze and move with `TestClock`. The fake refuses a
//! request whose Signature Version 4 signature it cannot reproduce, so every
//! test also checks the signing.

use serial_test::serial;
use std::time::Duration;
use suprnova::Storage;
use suprnova::filesystem::testing::StorageFakeGuard;
use suprnova::queue::driver::QueueDriver;
use suprnova::queue::envelope::Envelope;
use suprnova::queue::{Queue, bootstrap_from_env};
use suprnova::testing::TestClock;
use suprnova::{SqsConfig, SqsCredentials, SqsOverflow, SqsQueueDriver};

use crate::env_lock::lock_env_async;
use crate::env_snapshot::{EnvSnapshot, set_env};

use fake::FakeSqs;

const PREFIX: &str = "https://sqs.us-east-1.amazonaws.com/123456789012";

/// Every variable these tests set, restored when the snapshot drops.
const VARIABLES: &[&str] = &[
    "APP_ENV",
    "QUEUE_DRIVER",
    "QUEUE_CONNECTIONS",
    "QUEUE_FAILOVER_CONNECTIONS",
    "SQS_PREFIX",
    "SQS_QUEUE",
    "SQS_SUFFIX",
    "SQS_ENDPOINT",
    "AWS_DEFAULT_REGION",
    "AWS_REGION",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "AWS_PROFILE",
    "AWS_CONFIG_FILE",
    "AWS_SHARED_CREDENTIALS_FILE",
    "AWS_EC2_METADATA_DISABLED",
    "SQS_WAIT_TIME_SECONDS",
    "SQS_OVERFLOW_ENABLED",
    "SQS_OVERFLOW_ALWAYS",
    "SQS_OVERFLOW_DISK",
    "SQS_OVERFLOW_DELETE_AFTER_PROCESSING",
    "SQS_OVERFLOW_FLUSH_ON_CLEAR",
    "FILESYSTEM_DISK",
];

fn url(name: &str) -> String {
    format!("{PREFIX}/{name}")
}

/// Point the driver at `fake` with static keys, `SQS_QUEUE=default`, and
/// nothing else set.
fn configure(fake: &FakeSqs) {
    for name in VARIABLES {
        set_env(name, None);
    }
    set_env("AWS_EC2_METADATA_DISABLED", Some("true"));
    set_env("SQS_PREFIX", Some(PREFIX));
    set_env("SQS_QUEUE", Some("default"));
    set_env("SQS_ENDPOINT", Some(&fake.endpoint));
    set_env("AWS_DEFAULT_REGION", Some("us-east-1"));
    set_env("AWS_ACCESS_KEY_ID", Some("AKIDSUPRNOVATEST"));
    set_env("AWS_SECRET_ACCESS_KEY", Some(fake::SECRET));
}

fn envelope(queue: Option<&str>) -> Envelope {
    let now = suprnova::clock::now();
    Envelope {
        schema_version: suprnova::queue::CURRENT_SCHEMA_VERSION,
        id: uuid::Uuid::new_v4(),
        job_name: "sqs-probe".into(),
        queue: queue.map(str::to_owned),
        payload: serde_json::json!({ "order": 42 }),
        dispatched_at: now,
        available_at: now,
        attempts: 0,
        max_tries: 5,
        backoff: suprnova::queue::BackoffSchedule::default(),
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

/// An envelope whose payload alone is 2 MiB, over SQS's 1 MiB limit.
fn large_envelope() -> Envelope {
    let mut env = envelope(None);
    env.payload = serde_json::json!({ "blob": "x".repeat(2 * 1024 * 1024) });
    env
}

const VISIBILITY: Duration = Duration::from_secs(30);

fn driver() -> SqsQueueDriver {
    SqsQueueDriver::from_env().expect("the SQS driver builds from the environment")
}

/// Hold the env lock, snapshot the variables, start a fake with the named
/// queues and configure the driver against it.
macro_rules! setup {
    ($($queue:expr),* $(,)?) => {{
        let env = lock_env_async().await;
        let restore = EnvSnapshot::capture(VARIABLES);
        let fake = FakeSqs::start(&[$(&url($queue)),*]).await;
        configure(&fake);
        (env, restore, fake)
    }};
}

// PAR-018: the driver.

#[tokio::test]
async fn a_pushed_job_is_received_and_an_acknowledged_job_is_gone() {
    let (_env, _restore, fake) = setup!("default");
    let driver = driver();
    let sent = envelope(None);

    driver.push(sent.clone()).await.unwrap();
    assert_eq!(
        fake.messages(&url("default")).len(),
        1,
        "the push reached SQS_QUEUE"
    );

    let reservation = driver
        .pop(VISIBILITY)
        .await
        .unwrap()
        .expect("the pushed job is received");
    assert_eq!(reservation.envelope.id, sent.id);
    assert_eq!(reservation.envelope.payload, sent.payload);
    assert_eq!(reservation.envelope.attempts, 0);

    driver.ack(&reservation.token).await.unwrap();
    fake.advance(60);
    assert!(
        driver.pop(VISIBILITY).await.unwrap().is_none(),
        "an acknowledged job is not received again"
    );
    assert!(
        fake.messages(&url("default")).is_empty(),
        "the acknowledgement deleted it"
    );
}

#[tokio::test]
async fn a_pop_hides_the_job_for_the_visibility_timeout() {
    let (_env, _restore, fake) = setup!("default");
    let driver = driver();
    driver.push(envelope(None)).await.unwrap();

    let first = driver.pop(VISIBILITY).await.unwrap().expect("received");
    let receive = fake
        .last("AmazonSQS.ReceiveMessage")
        .expect("a receive was sent");
    assert_eq!(
        receive["VisibilityTimeout"], 30,
        "the worker's visibility timeout"
    );
    assert_eq!(receive["MaxNumberOfMessages"], 1, "one message per pop");
    assert_eq!(
        receive["WaitTimeSeconds"], 1,
        "a one-second long poll by default"
    );

    fake.advance(29);
    assert!(
        driver.pop(VISIBILITY).await.unwrap().is_none(),
        "hidden for 30 seconds"
    );
    fake.advance(1);
    let again = driver
        .pop(VISIBILITY)
        .await
        .unwrap()
        .expect("visible again");
    assert_eq!(again.envelope.id, first.envelope.id);
    assert_eq!(
        again.envelope.attempts, 1,
        "an expired reservation counts an attempt"
    );
}

#[tokio::test]
async fn a_job_names_its_queue_and_a_worker_without_a_list_receives_from_sqs_queue() {
    let (_env, _restore, fake) = setup!("default", "emails");
    let driver = driver();

    driver.push(envelope(Some("emails"))).await.unwrap();
    assert_eq!(
        fake.messages(&url("emails")).len(),
        1,
        "the job went to its queue"
    );
    assert!(fake.messages(&url("default")).is_empty());

    assert!(
        driver.pop(VISIBILITY).await.unwrap().is_none(),
        "a worker with no --queue receives from SQS_QUEUE only"
    );
    let reservation = driver
        .pop_from(VISIBILITY, &["emails".to_owned()])
        .await
        .unwrap()
        .expect("a worker with --queue=emails receives it");
    assert_eq!(reservation.envelope.queue.as_deref(), Some("emails"));
}

#[tokio::test]
async fn a_worker_receives_from_the_queues_it_names_in_order() {
    let (_env, _restore, fake) = setup!("default", "emails", "billing");
    let driver = driver();
    let default_job = envelope(None);
    let emails_job = envelope(Some("emails"));
    let billing_job = envelope(Some("billing"));
    driver.push(default_job.clone()).await.unwrap();
    driver.push(emails_job.clone()).await.unwrap();
    driver.push(billing_job.clone()).await.unwrap();

    let list = ["billing".to_owned(), "emails".to_owned()];
    let first = driver.pop_from(VISIBILITY, &list).await.unwrap().unwrap();
    assert_eq!(first.envelope.id, billing_job.id, "billing is named first");
    let second = driver.pop_from(VISIBILITY, &list).await.unwrap().unwrap();
    assert_eq!(second.envelope.id, emails_job.id);
    assert!(
        driver.pop_from(VISIBILITY, &list).await.unwrap().is_none(),
        "a worker with --queue=billing,emails never receives from SQS_QUEUE"
    );
    assert_eq!(fake.messages(&url("default")).len(), 1);
    assert!(
        fake.urls_received_from()
            .iter()
            .all(|u| u != &url("default")),
        "no receive was sent to SQS_QUEUE"
    );
}

#[tokio::test]
async fn a_job_delayed_twenty_minutes_is_not_received_before_its_time() {
    let (_env, _restore, fake) = setup!("default");
    let clock = TestClock::freeze();
    let driver = driver();
    let mut delayed = envelope(None);
    delayed.available_at = suprnova::clock::now() + chrono::Duration::minutes(20);

    driver.push(delayed.clone()).await.unwrap();
    for (_, delay) in fake.sends() {
        assert!(
            delay.unwrap_or(0) <= 900,
            "SQS refuses DelaySeconds over 900"
        );
    }

    // Fifteen minutes on, SQS shows the message, but its time has not come.
    fake.advance(900);
    clock.advance(chrono::Duration::minutes(15));
    assert!(driver.pop(VISIBILITY).await.unwrap().is_none());

    fake.advance(299);
    clock.advance(chrono::Duration::seconds(299));
    assert!(
        driver.pop(VISIBILITY).await.unwrap().is_none(),
        "one second early"
    );

    fake.advance(1);
    clock.advance(chrono::Duration::seconds(1));
    let reservation = driver
        .pop(VISIBILITY)
        .await
        .unwrap()
        .expect("received once twenty minutes have passed");
    assert_eq!(reservation.envelope.id, delayed.id);
    assert_eq!(
        reservation.envelope.attempts, 0,
        "waiting out a delay is not an attempt"
    );
}

#[tokio::test]
async fn a_nack_counts_one_attempt_after_the_requeue_delay() {
    let (_env, _restore, fake) = setup!("default");
    let driver = driver();
    driver.push(envelope(None)).await.unwrap();

    let first = driver.pop(VISIBILITY).await.unwrap().unwrap();
    assert_eq!(first.envelope.attempts, 0);
    driver
        .nack(&first.token, Duration::from_secs(45))
        .await
        .unwrap();

    fake.advance(44);
    assert!(
        driver.pop(VISIBILITY).await.unwrap().is_none(),
        "the requeue delay holds"
    );
    fake.advance(1);
    let second = driver
        .pop(VISIBILITY)
        .await
        .unwrap()
        .expect("back after 45 seconds");
    assert_eq!(second.envelope.id, first.envelope.id);
    assert_eq!(second.envelope.attempts, 1, "a nack counts one attempt");
}

#[tokio::test]
async fn a_release_returns_the_job_with_the_same_attempts() {
    let (_env, _restore, fake) = setup!("default");
    let clock = TestClock::freeze();
    let driver = driver();
    driver.push(envelope(None)).await.unwrap();

    let first = driver.pop(VISIBILITY).await.unwrap().unwrap();
    driver.nack(&first.token, Duration::ZERO).await.unwrap();
    let second = driver.pop(VISIBILITY).await.unwrap().unwrap();
    assert_eq!(second.envelope.attempts, 1);

    // The worker bumps attempts on its own copy before it runs the job.
    let mut running = second.envelope.clone();
    running.attempts += 1;
    driver
        .release(&second.token, &running, Duration::from_secs(10))
        .await
        .unwrap();

    fake.advance(9);
    clock.advance(chrono::Duration::seconds(9));
    assert!(
        driver.pop(VISIBILITY).await.unwrap().is_none(),
        "the release delay holds"
    );
    fake.advance(1);
    clock.advance(chrono::Duration::seconds(1));
    let third = driver
        .pop(VISIBILITY)
        .await
        .unwrap()
        .expect("back after 10 seconds");
    assert_eq!(third.envelope.id, first.envelope.id);
    assert_eq!(third.envelope.attempts, 1, "a release counts no attempt");
    assert_eq!(
        fake.messages(&url("default")).len(),
        1,
        "one copy of the job"
    );
}

#[tokio::test]
async fn size_reports_the_counts_sqs_keeps_and_clear_purges_the_queue() {
    let (_env, _restore, fake) = setup!("default");
    let driver = driver();
    driver.push(envelope(None)).await.unwrap();
    driver.push(envelope(None)).await.unwrap();
    let mut later = envelope(None);
    later.available_at = suprnova::clock::now() + chrono::Duration::seconds(120);
    driver.push(later).await.unwrap();
    let _held = driver.pop(VISIBILITY).await.unwrap().unwrap();

    assert_eq!(driver.size().await.unwrap(), 3);
    assert_eq!(driver.pending_size().await.unwrap(), 1);
    assert_eq!(driver.reserved_size().await.unwrap(), 1);
    assert_eq!(driver.delayed_size().await.unwrap(), 1);

    assert_eq!(
        driver.clear().await.unwrap(),
        3,
        "clear returns the count it held"
    );
    assert!(
        fake.messages(&url("default")).is_empty(),
        "clear purged the queue"
    );
}

#[tokio::test]
async fn bulk_push_sends_each_queue_in_batches_of_ten_in_order() {
    let (_env, _restore, fake) = setup!("default", "emails");
    let driver = driver();
    let mut jobs: Vec<Envelope> = (0..12).map(|_| envelope(None)).collect();
    jobs.insert(5, envelope(Some("emails")));
    let order: Vec<uuid::Uuid> = jobs
        .iter()
        .filter(|job| job.queue.is_none())
        .map(|job| job.id)
        .collect();

    driver.bulk_push(jobs).await.unwrap();

    let batches: Vec<(String, usize)> = fake
        .requests()
        .into_iter()
        .filter(|request| request.target == "AmazonSQS.SendMessageBatch")
        .map(|request| {
            (
                request.body["QueueUrl"].as_str().unwrap().to_owned(),
                request.body["Entries"].as_array().unwrap().len(),
            )
        })
        .collect();
    assert_eq!(
        batches,
        [
            (url("default"), 10),
            (url("default"), 2),
            (url("emails"), 1)
        ],
        "ten to a batch, one queue to a request"
    );
    let mut received = Vec::new();
    while let Some(reservation) = driver.pop(VISIBILITY).await.unwrap() {
        received.push(reservation.envelope.id);
    }
    assert_eq!(received, order, "every job arrives, in the order given");
}

// PAR-019: configuration and boot.

#[tokio::test]
async fn the_queue_url_is_built_from_prefix_queue_and_suffix() {
    let (_env, _restore, fake) = setup!("jobs-prod", "emails-prod", "other");
    set_env("SQS_QUEUE", Some("jobs"));
    set_env("SQS_SUFFIX", Some("-prod"));
    let driver = driver();

    driver.push(envelope(None)).await.unwrap();
    driver.push(envelope(Some("emails"))).await.unwrap();
    driver.push(envelope(Some("emails-prod"))).await.unwrap();
    driver.push(envelope(Some(&url("other")))).await.unwrap();

    let urls: Vec<String> = fake.sends().into_iter().map(|(url, _)| url).collect();
    assert_eq!(
        urls,
        [
            format!("{PREFIX}/jobs-prod"),
            format!("{PREFIX}/emails-prod"),
            format!("{PREFIX}/emails-prod"),
            format!("{PREFIX}/other"),
        ],
        "prefix, name and suffix, the suffix once, and a URL as it is"
    );
}

#[tokio::test]
async fn requests_are_signed_for_sqs_in_the_region() {
    let (_env, _restore, fake) = setup!("default");
    set_env("AWS_DEFAULT_REGION", Some("eu-west-2"));
    set_env("AWS_SESSION_TOKEN", Some("suprnova-session-token"));
    let driver = driver();
    driver.push(envelope(None)).await.unwrap();

    let request = fake.requests().pop().expect("a request reached the fake");
    let authorization = request.authorization.expect("the request is signed");
    assert!(
        authorization.starts_with("AWS4-HMAC-SHA256 Credential=AKIDSUPRNOVATEST/"),
        "Signature Version 4 with the access key: {authorization}"
    );
    assert!(
        authorization.contains("/eu-west-2/sqs/aws4_request"),
        "scoped to the region and sqs: {authorization}"
    );
    assert!(authorization.contains("Signature="), "{authorization}");
    assert_eq!(
        request.security_token.as_deref(),
        Some("suprnova-session-token")
    );
}

#[tokio::test]
#[serial]
async fn boot_fails_without_a_region() {
    let (_env, _restore, fake) = setup!("default");
    let _ = fake;
    set_env("QUEUE_DRIVER", Some("sqs"));
    set_env("AWS_DEFAULT_REGION", None);

    let error = bootstrap_from_env()
        .await
        .expect_err("no region is a boot error");
    assert!(error.to_string().contains("AWS_DEFAULT_REGION"), "{error}");
}

#[tokio::test]
#[serial]
async fn the_region_falls_back_to_aws_region() {
    let (_env, _restore, fake) = setup!("default");
    set_env("AWS_DEFAULT_REGION", None);
    set_env("AWS_REGION", Some("ap-south-1"));
    let driver = driver();
    driver.push(envelope(None)).await.unwrap();

    let authorization = fake.requests().pop().unwrap().authorization.unwrap();
    assert!(
        authorization.contains("/ap-south-1/sqs/aws4_request"),
        "{authorization}"
    );
}

#[tokio::test]
#[serial]
async fn boot_fails_with_a_plain_queue_name_and_no_prefix() {
    let (_env, _restore, _fake) = setup!("default");
    set_env("QUEUE_DRIVER", Some("sqs"));
    set_env("SQS_PREFIX", None);

    let error = bootstrap_from_env()
        .await
        .expect_err("no prefix is a boot error");
    assert!(error.to_string().contains("SQS_PREFIX"), "{error}");
}

#[tokio::test]
#[serial]
async fn a_queue_url_needs_no_prefix() {
    let (_env, _restore, fake) = setup!("default");
    set_env("SQS_PREFIX", None);
    set_env("SQS_QUEUE", Some(&url("default")));
    let driver = driver();
    driver.push(envelope(None)).await.unwrap();
    assert_eq!(fake.messages(&url("default")).len(), 1);
}

#[tokio::test]
#[serial]
async fn boot_fails_for_a_fifo_queue() {
    let (_env, _restore, _fake) = setup!("default");
    set_env("QUEUE_DRIVER", Some("sqs"));
    set_env("SQS_QUEUE", Some("jobs.fifo"));

    let error = bootstrap_from_env()
        .await
        .expect_err("a FIFO queue is a boot error");
    let text = error.to_string();
    assert!(text.contains("FIFO"), "{text}");
    assert!(text.contains("SQS_QUEUE"), "{text}");
}

#[tokio::test]
#[serial]
async fn queue_driver_sqs_registers_the_sqs_driver() {
    let (_env, _restore, _fake) = setup!("default");
    set_env("QUEUE_DRIVER", Some("sqs"));

    bootstrap_from_env().await.expect("QUEUE_DRIVER=sqs boots");
    assert_eq!(Queue::driver_name().unwrap(), "sqs");
}

#[tokio::test]
#[serial]
async fn queue_connections_and_failover_accept_sqs() {
    let (_env, _restore, _fake) = setup!("default");
    set_env("QUEUE_DRIVER", Some("memory"));
    set_env("QUEUE_CONNECTIONS", Some("sqs"));
    bootstrap_from_env()
        .await
        .expect("QUEUE_CONNECTIONS=sqs boots");
    assert_eq!(Queue::connection("sqs").unwrap().name(), "sqs");

    set_env("QUEUE_CONNECTIONS", None);
    set_env("QUEUE_DRIVER", Some("failover"));
    set_env("QUEUE_FAILOVER_CONNECTIONS", Some("sqs,memory"));
    bootstrap_from_env()
        .await
        .expect("an sqs failover connection boots");
}

// PAR-020: overflow of large payloads to a disk.

/// Overflow on, onto a memory disk named `overflow`.
fn overflow_on() -> StorageFakeGuard {
    let guard = Storage::fake();
    Storage::register_memory("overflow");
    set_env("SQS_OVERFLOW_ENABLED", Some("true"));
    set_env("SQS_OVERFLOW_DISK", Some("overflow"));
    guard
}

async fn stored_payloads() -> usize {
    Storage::disk("overflow")
        .unwrap()
        .list_with("/")
        .recursive(true)
        .await
        .unwrap()
        .into_iter()
        .filter(|entry| entry.metadata().is_file())
        .count()
}

#[tokio::test]
async fn with_overflow_a_large_job_is_stored_on_the_disk_and_sent_as_a_pointer() {
    let (_env, _restore, fake) = setup!("default");
    let _storage = overflow_on();
    let driver = driver();
    let sent = large_envelope();

    driver.push(sent.clone()).await.unwrap();
    let message = fake.messages(&url("default")).pop().unwrap();
    assert!(message.body.len() < 1024, "SQS got a pointer, not the job");
    assert!(message.body.contains("@pointer"), "{}", message.body);
    assert_eq!(stored_payloads().await, 1);

    let reservation = driver.pop(VISIBILITY).await.unwrap().unwrap();
    assert_eq!(reservation.envelope.id, sent.id);
    assert_eq!(
        reservation.envelope.payload, sent.payload,
        "the pop returns the job"
    );

    driver.ack(&reservation.token).await.unwrap();
    assert_eq!(
        stored_payloads().await,
        0,
        "the acknowledgement deleted the payload"
    );
}

#[tokio::test]
async fn delete_after_processing_off_keeps_the_stored_payload() {
    let (_env, _restore, _fake) = setup!("default");
    let _storage = overflow_on();
    set_env("SQS_OVERFLOW_DELETE_AFTER_PROCESSING", Some("false"));
    let driver = driver();

    driver.push(large_envelope()).await.unwrap();
    let reservation = driver.pop(VISIBILITY).await.unwrap().unwrap();
    driver.ack(&reservation.token).await.unwrap();
    assert_eq!(stored_payloads().await, 1);
}

#[tokio::test]
async fn overflow_always_stores_every_job() {
    let (_env, _restore, fake) = setup!("default");
    let _storage = overflow_on();
    set_env("SQS_OVERFLOW_ALWAYS", Some("true"));
    let driver = driver();
    let sent = envelope(None);

    driver.push(sent.clone()).await.unwrap();
    assert!(
        fake.messages(&url("default"))
            .pop()
            .unwrap()
            .body
            .contains("@pointer")
    );
    assert_eq!(stored_payloads().await, 1);
    let reservation = driver.pop(VISIBILITY).await.unwrap().unwrap();
    assert_eq!(reservation.envelope.payload, sent.payload);
}

#[tokio::test]
async fn flush_on_clear_deletes_the_stored_payloads() {
    let (_env, _restore, _fake) = setup!("default");
    let _storage = overflow_on();
    set_env("SQS_OVERFLOW_FLUSH_ON_CLEAR", Some("true"));
    let driver = driver();

    driver.push(large_envelope()).await.unwrap();
    driver.push(large_envelope()).await.unwrap();
    assert_eq!(stored_payloads().await, 2);
    driver.clear().await.unwrap();
    assert_eq!(
        stored_payloads().await,
        0,
        "clear deleted the stored payloads"
    );
}

#[tokio::test]
async fn clear_keeps_the_stored_payloads_without_flush_on_clear() {
    let (_env, _restore, _fake) = setup!("default");
    let _storage = overflow_on();
    let driver = driver();

    driver.push(large_envelope()).await.unwrap();
    driver.clear().await.unwrap();
    assert_eq!(stored_payloads().await, 1);
}

#[tokio::test]
async fn without_overflow_a_push_over_the_limit_fails_naming_the_limit() {
    let (_env, _restore, fake) = setup!("default");
    let driver = driver();

    let error = driver
        .push(large_envelope())
        .await
        .expect_err("a job over 1 MiB is refused without overflow");
    let text = error.to_string();
    assert!(text.contains("1 MiB"), "{text}");
    assert!(text.contains("SQS_OVERFLOW_ENABLED"), "{text}");
    assert!(fake.messages(&url("default")).is_empty());
}

#[tokio::test]
#[serial]
async fn boot_fails_with_overflow_on_and_no_disk_under_the_name() {
    let (_env, _restore, _fake) = setup!("default");
    let _storage = Storage::fake();
    set_env("QUEUE_DRIVER", Some("sqs"));
    set_env("SQS_OVERFLOW_ENABLED", Some("true"));
    set_env("SQS_OVERFLOW_DISK", Some("payloads"));

    let error = bootstrap_from_env()
        .await
        .expect_err("overflow onto a disk that is not registered");
    assert!(error.to_string().contains("payloads"), "{error}");
}

#[tokio::test]
#[serial]
async fn overflow_uses_the_default_disk_when_none_is_named() {
    let (_env, _restore, fake) = setup!("default");
    let _storage = Storage::fake();
    Storage::register_memory("overflow");
    Storage::set_default_disk("overflow");
    set_env("SQS_OVERFLOW_ENABLED", Some("true"));
    let driver = driver();

    driver.push(large_envelope()).await.unwrap();
    assert!(
        fake.messages(&url("default"))
            .pop()
            .unwrap()
            .body
            .contains("@pointer")
    );
    assert_eq!(stored_payloads().await, 1);
}

// Review fixes: settlement past SQS's limits, the request layer, and the
// overflow payloads a copy leaves behind.

#[tokio::test]
async fn a_nack_longer_than_the_twelve_hours_left_holds_and_counts_one_attempt() {
    let (_env, _restore, fake) = setup!("default");
    let clock = TestClock::freeze();
    let driver = driver();
    driver.push(envelope(None)).await.unwrap();
    let first = driver.pop(VISIBILITY).await.unwrap().unwrap();

    // The job ran for two minutes, so less than 12 hours of hiding is left.
    fake.advance(120);
    clock.advance(chrono::Duration::seconds(120));
    let twelve_hours = 12 * 3600;
    driver
        .nack(&first.token, Duration::from_secs(twelve_hours))
        .await
        .expect("a nack of 12 hours is accepted");

    let mut waited = 0;
    while waited + 900 < twelve_hours {
        fake.advance(900);
        clock.advance(chrono::Duration::seconds(900));
        waited += 900;
        assert!(
            driver.pop(VISIBILITY).await.unwrap().is_none(),
            "back {waited} seconds into a 12-hour requeue delay"
        );
    }
    fake.advance(twelve_hours - waited);
    clock.advance(chrono::Duration::seconds((twelve_hours - waited) as i64));
    let again = driver
        .pop(VISIBILITY)
        .await
        .unwrap()
        .expect("back after 12 hours");
    assert_eq!(again.envelope.id, first.envelope.id);
    assert_eq!(again.envelope.attempts, 1, "the nack counted one attempt");
}

#[tokio::test]
async fn a_settlement_after_the_reservation_expired_keeps_the_overflow_payload() {
    let (_env, _restore, fake) = setup!("default");
    let _storage = overflow_on();
    let clock = TestClock::freeze();
    let first_worker = driver();
    let second_worker = driver();
    let sent = large_envelope();
    first_worker.push(sent.clone()).await.unwrap();
    let first = first_worker.pop(VISIBILITY).await.unwrap().unwrap();

    // The first worker outlives its reservation, and SQS hands the job on.
    fake.advance(31);
    clock.advance(chrono::Duration::seconds(31));
    let second = second_worker
        .pop(VISIBILITY)
        .await
        .unwrap()
        .expect("redelivered");
    first_worker.ack(&first.token).await.unwrap();
    assert_eq!(
        stored_payloads().await,
        1,
        "a late ack must not delete the payload the message still points at"
    );

    second_worker
        .nack(&second.token, Duration::ZERO)
        .await
        .unwrap();
    let third = second_worker
        .pop(VISIBILITY)
        .await
        .unwrap()
        .expect("the job is still there");
    assert_eq!(third.envelope.payload, sent.payload, "and still readable");
}

#[tokio::test]
async fn a_refused_send_deletes_its_payload_and_an_unanswered_one_keeps_it() {
    let (_env, _restore, fake) = setup!("default");
    let _storage = overflow_on();
    let driver = driver();

    fake.script(
        "AmazonSQS.SendMessage",
        400,
        r#"{"__type":"com.amazonaws.sqs#InvalidParameterValue","message":"refused"}"#,
    );
    driver
        .push(large_envelope())
        .await
        .expect_err("SQS refused the send");
    assert_eq!(
        stored_payloads().await,
        0,
        "no message points at the payload"
    );

    fake.script("AmazonSQS.SendMessage", 200, "<html>not sqs</html>");
    let error = driver
        .push(large_envelope())
        .await
        .expect_err("a reply that is not SQS's fails the push");
    assert!(error.to_string().contains("SQS_ENDPOINT"), "{error}");
    assert_eq!(
        stored_payloads().await,
        1,
        "whatever answered may have taken the message, so its payload stays"
    );
}

#[tokio::test]
async fn a_delay_sent_on_keeps_one_payload_with_delete_after_processing_off() {
    let (_env, _restore, fake) = setup!("default");
    let _storage = overflow_on();
    set_env("SQS_OVERFLOW_DELETE_AFTER_PROCESSING", Some("false"));
    let clock = TestClock::freeze();
    let driver = driver();
    let mut delayed = large_envelope();
    delayed.available_at = suprnova::clock::now() + chrono::Duration::minutes(40);
    driver.push(delayed).await.unwrap();

    for _ in 0..2 {
        fake.advance(900);
        clock.advance(chrono::Duration::minutes(15));
        assert!(driver.pop(VISIBILITY).await.unwrap().is_none());
    }
    assert_eq!(
        stored_payloads().await,
        1,
        "each copy sent on replaces the payload of the one it supersedes"
    );
}

#[tokio::test]
async fn a_throttled_request_is_tried_again() {
    let (_env, _restore, fake) = setup!("default");
    let driver = driver();
    fake.script(
        "AmazonSQS.SendMessage",
        400,
        r#"{"__type":"com.amazonaws.sqs#RequestThrottled","message":"slow down"}"#,
    );

    driver
        .push(envelope(None))
        .await
        .expect("the second try succeeds");
    assert_eq!(fake.sends().len(), 2, "one throttled try, one that landed");
    assert_eq!(fake.messages(&url("default")).len(), 1, "one message");
}

#[tokio::test]
async fn a_service_that_keeps_failing_fails_the_push_after_three_tries() {
    let (_env, _restore, fake) = setup!("default");
    let driver = driver();
    for _ in 0..3 {
        fake.script(
            "AmazonSQS.SendMessage",
            503,
            r#"{"__type":"com.amazonaws.sqs#ServiceUnavailable","message":"down"}"#,
        );
    }

    let error = driver.push(envelope(None)).await.expect_err("three faults");
    assert!(error.to_string().contains("ServiceUnavailable"), "{error}");
    assert_eq!(fake.sends().len(), 3);
}

#[tokio::test]
async fn a_size_with_no_counts_in_the_reply_is_an_error() {
    let (_env, _restore, fake) = setup!("default");
    let driver = driver();
    fake.script("AmazonSQS.GetQueueAttributes", 200, "{}");

    assert!(
        driver.size().await.is_err(),
        "a reply without the counts is not an empty queue"
    );
}

#[tokio::test]
async fn a_receive_waits_for_the_configured_seconds() {
    let (_env, _restore, fake) = setup!("default");
    set_env("SQS_WAIT_TIME_SECONDS", Some("5"));
    let driver = driver();

    assert!(driver.pop(VISIBILITY).await.unwrap().is_none());
    let receive = fake.last("AmazonSQS.ReceiveMessage").unwrap();
    assert_eq!(receive["WaitTimeSeconds"], 5);
}

#[tokio::test]
async fn a_driver_built_from_a_config_reaches_its_queue() {
    let (_env, _restore, fake) = setup!("billing");
    for name in VARIABLES {
        set_env(name, None);
    }
    set_env("AWS_EC2_METADATA_DISABLED", Some("true"));
    let mut config = SqsConfig::new("us-east-1", "billing");
    config.prefix = Some(PREFIX.to_owned());
    config.endpoint = Some(fake.endpoint.clone());
    config.credentials = Some(SqsCredentials {
        access_key_id: "AKIDSUPRNOVATEST".into(),
        secret_access_key: fake::SECRET.into(),
        session_token: None,
    });
    assert!(SqsOverflow::default().delete_after_processing);
    let driver = SqsQueueDriver::new(config).expect("a driver from code");

    driver.push(envelope(None)).await.unwrap();
    assert_eq!(fake.messages(&url("billing")).len(), 1);
}

#[tokio::test]
async fn call_sends_any_sqs_action() {
    let (_env, _restore, fake) = setup!("default");
    let driver = driver();
    driver.push(envelope(None)).await.unwrap();

    let reply = driver
        .call(
            "GetQueueAttributes",
            serde_json::json!({
                "QueueUrl": url("default"),
                "AttributeNames": ["ApproximateNumberOfMessages"],
            }),
        )
        .await
        .expect("a signed request to any action");
    assert_eq!(reply["Attributes"]["ApproximateNumberOfMessages"], "1");

    let error = driver
        .call(
            "NoSuchAction",
            serde_json::json!({ "QueueUrl": url("default") }),
        )
        .await
        .expect_err("SQS refuses an action it does not know");
    assert!(error.to_string().contains("InvalidAction"), "{error}");
    assert_eq!(fake.messages(&url("default")).len(), 1);
}

#[tokio::test]
async fn a_request_signed_with_another_secret_is_refused() {
    let (_env, _restore, fake) = setup!("default");
    set_env("AWS_SECRET_ACCESS_KEY", Some("not-the-secret"));
    let driver = driver();

    let error = driver
        .push(envelope(None))
        .await
        .expect_err("the fake checks every signature");
    assert!(
        error.to_string().contains("SignatureDoesNotMatch"),
        "{error}"
    );
    assert!(fake.messages(&url("default")).is_empty());
}

// A live endpoint, for checking the wire format against a real
// SQS-compatible server. Run it with the two queues created:
//
//     SQS_LIVE_ENDPOINT=http://127.0.0.1:9324 \
//     SQS_LIVE_PREFIX=http://localhost:9324/000000000000 \
//     cargo nextest run -p suprnova --test queue --run-ignored only \
//         -E 'test(/^sqs::live_/)'
//
// with `suprnova-live` and `suprnova-emails` under the prefix.

#[tokio::test]
#[ignore = "needs an SQS-compatible endpoint: set SQS_LIVE_ENDPOINT and SQS_LIVE_PREFIX"]
async fn live_round_trip_against_an_sqs_endpoint() {
    let _env = lock_env_async().await;
    let _restore = EnvSnapshot::capture(VARIABLES);
    let endpoint = std::env::var("SQS_LIVE_ENDPOINT").expect("SQS_LIVE_ENDPOINT");
    let prefix = std::env::var("SQS_LIVE_PREFIX").expect("SQS_LIVE_PREFIX");
    for name in VARIABLES {
        set_env(name, None);
    }
    set_env("AWS_EC2_METADATA_DISABLED", Some("true"));
    set_env("SQS_ENDPOINT", Some(&endpoint));
    set_env("SQS_PREFIX", Some(&prefix));
    set_env("SQS_QUEUE", Some("suprnova-live"));
    set_env("AWS_DEFAULT_REGION", Some("us-east-1"));
    set_env("AWS_ACCESS_KEY_ID", Some("AKIDSUPRNOVALIVE"));
    set_env("AWS_SECRET_ACCESS_KEY", Some("suprnova-live-secret"));
    let driver = driver();
    driver.clear().await.expect("PurgeQueue");

    let sent = envelope(None);
    driver.push(sent.clone()).await.expect("SendMessage");
    let first = driver
        .pop(VISIBILITY)
        .await
        .expect("ReceiveMessage")
        .expect("received");
    assert_eq!(first.envelope.id, sent.id);
    assert_eq!(first.envelope.attempts, 0);
    driver
        .nack(&first.token, Duration::ZERO)
        .await
        .expect("ChangeMessageVisibility");
    let second = driver
        .pop(VISIBILITY)
        .await
        .unwrap()
        .expect("back after the nack");
    assert_eq!(
        second.envelope.attempts, 1,
        "ApproximateReceiveCount counts the nack"
    );
    driver
        .release(&second.token, &second.envelope, Duration::from_secs(1))
        .await
        .expect("release");
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let third = driver
        .pop(VISIBILITY)
        .await
        .unwrap()
        .expect("back after the release");
    assert_eq!(third.envelope.attempts, 1, "a release counts no attempt");
    driver.ack(&third.token).await.expect("DeleteMessage");
    assert!(driver.pop(VISIBILITY).await.unwrap().is_none());

    driver
        .push(envelope(Some("suprnova-emails")))
        .await
        .unwrap();
    let routed = driver
        .pop_from(VISIBILITY, &["suprnova-emails".to_owned()])
        .await
        .unwrap()
        .expect("received from the queue it names");
    driver.ack(&routed.token).await.unwrap();

    let mut later = envelope(None);
    later.available_at = suprnova::clock::now() + chrono::Duration::seconds(2);
    driver.push(later.clone()).await.unwrap();
    driver.push(envelope(None)).await.unwrap();
    assert_eq!(driver.size().await.expect("GetQueueAttributes"), 2);
    assert_eq!(driver.delayed_size().await.unwrap(), 1);
    assert_eq!(driver.clear().await.unwrap(), 2);
    assert_eq!(driver.size().await.unwrap(), 0);

    driver
        .bulk_push((0..12).map(|_| envelope(None)).collect())
        .await
        .expect("SendMessageBatch");
    assert_eq!(driver.size().await.unwrap(), 12);
    assert_eq!(driver.clear().await.unwrap(), 12);

    driver.push(later.clone()).await.unwrap();
    assert!(
        driver.pop(VISIBILITY).await.unwrap().is_none(),
        "the delay holds"
    );
    tokio::time::sleep(Duration::from_millis(3500)).await;
    let delayed = driver
        .pop(VISIBILITY)
        .await
        .unwrap()
        .expect("received after the delay");
    assert_eq!(delayed.envelope.id, later.id);
    driver.ack(&delayed.token).await.unwrap();
}

/// A server for the AWS JSON 1.0 protocol of SQS: the eight actions the
/// driver sends, against standard queues created up front. It checks the
/// Signature Version 4 signature of every request against [`fake::SECRET`],
/// and holds a message hidden for at most 12 hours from its receive, as SQS
/// does. A delete on a receipt handle that is not the latest one succeeds
/// and deletes nothing, which AWS documents SQS may do.
mod fake {
    use hmac::digest::KeyInit;
    use hmac::{Hmac, Mac};
    use http_body_util::{BodyExt, Full};
    use hyper::body::{Bytes, Incoming};
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    use std::collections::{HashMap, VecDeque};
    use std::convert::Infallible;
    use std::sync::{Arc, Mutex};

    /// The secret key every test signs with.
    pub const SECRET: &str = "suprnova-test-secret";

    /// SQS's limit on one message body.
    const MAX_BODY: usize = 1024 * 1024;

    /// The longest SQS hides a message, counted from its receive.
    const MAX_VISIBILITY: u64 = 43_200;

    #[derive(Clone, Debug)]
    pub struct Message {
        pub id: String,
        pub body: String,
        pub visible_at: u64,
        pub receive_count: u32,
        received_at: u64,
        receipts: Vec<String>,
    }

    #[derive(Clone, Debug)]
    pub struct Request {
        pub target: String,
        pub authorization: Option<String>,
        pub security_token: Option<String>,
        pub body: Value,
    }

    #[derive(Default)]
    struct State {
        now: u64,
        next: u64,
        queues: HashMap<String, Vec<Message>>,
        requests: Vec<Request>,
        /// Canned replies, by target, answered before the queue is touched.
        script: HashMap<String, VecDeque<(u16, String)>>,
    }

    #[derive(Clone)]
    pub struct FakeSqs {
        state: Arc<Mutex<State>>,
        pub endpoint: String,
    }

    impl FakeSqs {
        pub async fn start(queues: &[&str]) -> Self {
            let state = Arc::new(Mutex::new(State::default()));
            for queue in queues {
                state
                    .lock()
                    .unwrap()
                    .queues
                    .insert((*queue).to_owned(), Vec::new());
            }
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}", listener.local_addr().unwrap());
            let shared = Arc::clone(&state);
            tokio::spawn(async move {
                loop {
                    let Ok((stream, _)) = listener.accept().await else {
                        return;
                    };
                    let state = Arc::clone(&shared);
                    tokio::spawn(async move {
                        let service = service_fn(move |request: hyper::Request<Incoming>| {
                            let state = Arc::clone(&state);
                            async move { Ok::<_, Infallible>(serve(&state, request).await) }
                        });
                        let _ = hyper::server::conn::http1::Builder::new()
                            .serve_connection(TokioIo::new(stream), service)
                            .await;
                    });
                }
            });
            Self { state, endpoint }
        }

        /// Move the fake's clock on by `seconds`.
        pub fn advance(&self, seconds: u64) {
            self.state.lock().unwrap().now += seconds;
        }

        /// Answer the next request for `target` with `status` and `body`.
        pub fn script(&self, target: &str, status: u16, body: &str) {
            self.state
                .lock()
                .unwrap()
                .script
                .entry(target.to_owned())
                .or_default()
                .push_back((status, body.to_owned()));
        }

        /// The messages a queue holds, visible or not.
        pub fn messages(&self, queue_url: &str) -> Vec<Message> {
            self.state
                .lock()
                .unwrap()
                .queues
                .get(queue_url)
                .cloned()
                .unwrap_or_default()
        }

        pub fn requests(&self) -> Vec<Request> {
            self.state.lock().unwrap().requests.clone()
        }

        /// The body of the last request for `target`.
        pub fn last(&self, target: &str) -> Option<Value> {
            self.requests()
                .into_iter()
                .rev()
                .find(|request| request.target == target)
                .map(|request| request.body)
        }

        /// Each `SendMessage`, as its queue URL and its `DelaySeconds`.
        pub fn sends(&self) -> Vec<(String, Option<u64>)> {
            self.requests()
                .into_iter()
                .filter(|request| request.target == "AmazonSQS.SendMessage")
                .map(|request| {
                    (
                        request.body["QueueUrl"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned(),
                        request.body["DelaySeconds"].as_u64(),
                    )
                })
                .collect()
        }

        /// The queue URL of each `ReceiveMessage`.
        pub fn urls_received_from(&self) -> Vec<String> {
            self.requests()
                .into_iter()
                .filter(|request| request.target == "AmazonSQS.ReceiveMessage")
                .map(|request| {
                    request.body["QueueUrl"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned()
                })
                .collect()
        }
    }

    async fn serve(
        state: &Mutex<State>,
        request: hyper::Request<Incoming>,
    ) -> hyper::Response<Full<Bytes>> {
        let method = request.method().as_str().to_owned();
        let path = request.uri().path().to_owned();
        let query = request.uri().query().unwrap_or_default().to_owned();
        let headers: Vec<(String, String)> = request
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_owned(),
                    value.to_str().unwrap_or_default().to_owned(),
                )
            })
            .collect();
        let header = |name: &str| {
            headers
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        };
        let target = header("x-amz-target").unwrap_or_default();
        let content_type = header("content-type").unwrap_or_default();
        let bytes = request.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        let signature = verify_signature(&method, &path, &query, &headers, &bytes, SECRET);

        let mut state = state.lock().unwrap();
        state.requests.push(Request {
            target: target.clone(),
            authorization: header("authorization"),
            security_token: header("x-amz-security-token"),
            body: body.clone(),
        });
        let scripted = state.script.get_mut(&target).and_then(VecDeque::pop_front);
        let (status, reply) = if let Some((status, reply)) = scripted {
            (status, reply)
        } else if let Err(reason) = signature {
            let (_, reply) = error("SignatureDoesNotMatch", &reason);
            (403, reply.to_string())
        } else if content_type != "application/x-amz-json-1.0" {
            let (status, reply) = error(
                "InvalidParameterValue",
                "expected application/x-amz-json-1.0",
            );
            (status, reply.to_string())
        } else {
            let (status, reply) = act(&mut state, &target, &body);
            (status, reply.to_string())
        };
        hyper::Response::builder()
            .status(status)
            .header("content-type", "application/x-amz-json-1.0")
            .body(Full::new(Bytes::from(reply)))
            .unwrap()
    }

    fn hmac(key: &[u8], data: &str) -> Vec<u8> {
        let mut mac = Hmac::<Sha256>::new_from_slice(key).unwrap();
        mac.update(data.as_bytes());
        mac.finalize().into_bytes().to_vec()
    }

    /// Recompute the Signature Version 4 signature of a request from
    /// `secret` and compare it with the one it carries.
    pub fn verify_signature(
        method: &str,
        path: &str,
        query: &str,
        headers: &[(String, String)],
        body: &[u8],
        secret: &str,
    ) -> Result<(), String> {
        let header = |name: &str| {
            headers
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
        };
        let authorization = header("authorization").ok_or("no Authorization header")?;
        let fields = authorization
            .strip_prefix("AWS4-HMAC-SHA256 ")
            .ok_or("not AWS4-HMAC-SHA256")?;
        let field = |name: &str| {
            fields
                .split(',')
                .map(str::trim)
                .find_map(|part| part.strip_prefix(&format!("{name}=")))
                .ok_or(format!("no {name} in Authorization"))
        };
        let credential = field("Credential")?;
        let signed_headers = field("SignedHeaders")?;
        let signature = field("Signature")?;
        let scope = credential
            .split_once('/')
            .map(|(_, scope)| scope)
            .ok_or("no scope")?;
        let mut parts = scope.split('/');
        let (date, region, service) = (
            parts.next().ok_or("no date")?,
            parts.next().ok_or("no region")?,
            parts.next().ok_or("no service")?,
        );
        let signed: Vec<&str> = signed_headers.split(';').collect();
        for required in ["host", "x-amz-date", "x-amz-content-sha256", "x-amz-target"] {
            if !signed.contains(&required) {
                return Err(format!("{required} is not signed"));
            }
        }
        let payload_hash = header("x-amz-content-sha256").ok_or("no x-amz-content-sha256")?;
        if payload_hash != hex::encode(Sha256::digest(body)) {
            return Err("x-amz-content-sha256 is not the hash of the body".into());
        }
        let mut canonical_headers = String::new();
        for name in &signed {
            let value = header(name).ok_or(format!("signed header {name} is missing"))?;
            let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
            canonical_headers.push_str(&format!("{name}:{value}\n"));
        }
        let canonical = format!(
            "{method}\n{path}\n{query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
        );
        let amz_date = header("x-amz-date").ok_or("no x-amz-date")?;
        let to_sign = format!(
            "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
            hex::encode(Sha256::digest(canonical.as_bytes()))
        );
        let key = hmac(format!("AWS4{secret}").as_bytes(), date);
        let key = hmac(&key, region);
        let key = hmac(&key, service);
        let key = hmac(&key, "aws4_request");
        let expected = hex::encode(hmac(&key, &to_sign));
        if expected == signature {
            Ok(())
        } else {
            Err("the signature does not match".into())
        }
    }

    fn error(code: &str, message: &str) -> (u16, Value) {
        (
            400,
            json!({ "__type": format!("com.amazonaws.sqs#{code}"), "message": message }),
        )
    }

    fn act(state: &mut State, target: &str, body: &Value) -> (u16, Value) {
        let url = body["QueueUrl"].as_str().unwrap_or_default().to_owned();
        if !state.queues.contains_key(&url) {
            return error("QueueDoesNotExist", "The specified queue does not exist.");
        }
        let now = state.now;
        let send = |state: &mut State, text: &str, delay: u64| {
            state.next += 1;
            let id = format!("message-{}", state.next);
            state.queues.get_mut(&url).unwrap().push(Message {
                id: id.clone(),
                body: text.to_owned(),
                visible_at: now + delay,
                receive_count: 0,
                received_at: 0,
                receipts: Vec::new(),
            });
            id
        };
        match target {
            "AmazonSQS.SendMessage" => {
                let Some(text) = body["MessageBody"].as_str() else {
                    return error("MissingParameter", "MessageBody");
                };
                if text.len() > MAX_BODY {
                    return error("InvalidParameterValue", "message too long");
                }
                let delay = body["DelaySeconds"].as_u64().unwrap_or(0);
                if delay > 900 {
                    return error("InvalidParameterValue", "DelaySeconds over 900");
                }
                let id = send(state, text, delay);
                (200, json!({ "MessageId": id, "MD5OfMessageBody": "" }))
            }
            "AmazonSQS.SendMessageBatch" => {
                let entries = body["Entries"].as_array().cloned().unwrap_or_default();
                let total: usize = entries
                    .iter()
                    .map(|entry| entry["MessageBody"].as_str().map_or(0, str::len))
                    .sum();
                if entries.is_empty() || entries.len() > 10 {
                    return error("TooManyEntriesInBatchRequest", "1 to 10 entries");
                }
                if total > MAX_BODY {
                    return error("BatchRequestTooLong", "batch over 1 MiB");
                }
                let mut successful = Vec::new();
                for entry in entries {
                    let delay = entry["DelaySeconds"].as_u64().unwrap_or(0);
                    if delay > 900 {
                        return error("InvalidParameterValue", "DelaySeconds over 900");
                    }
                    let text = entry["MessageBody"].as_str().unwrap_or_default();
                    let id = send(state, text, delay);
                    successful.push(json!({ "Id": entry["Id"], "MessageId": id }));
                }
                (200, json!({ "Successful": successful, "Failed": [] }))
            }
            "AmazonSQS.ReceiveMessage" => {
                let visibility = body["VisibilityTimeout"].as_u64().unwrap_or(30);
                state.next += 1;
                let receipt = format!("receipt-{}", state.next);
                let queue = state.queues.get_mut(&url).unwrap();
                let Some(message) = queue.iter_mut().find(|m| m.visible_at <= now) else {
                    return (200, json!({}));
                };
                message.receive_count += 1;
                message.received_at = now;
                message.visible_at = now + visibility;
                message.receipts.push(receipt.clone());
                (
                    200,
                    json!({ "Messages": [{
                        "MessageId": message.id,
                        "ReceiptHandle": receipt,
                        "Body": message.body,
                        "Attributes": {
                            "ApproximateReceiveCount": message.receive_count.to_string()
                        }
                    }] }),
                )
            }
            "AmazonSQS.DeleteMessage" => {
                let receipt = body["ReceiptHandle"].as_str().unwrap_or_default();
                let queue = state.queues.get_mut(&url).unwrap();
                queue.retain(|m| m.receipts.last().is_none_or(|latest| latest != receipt));
                (200, json!({}))
            }
            "AmazonSQS.ChangeMessageVisibility" => {
                let receipt = body["ReceiptHandle"].as_str().unwrap_or_default();
                let timeout = body["VisibilityTimeout"].as_u64().unwrap_or(0);
                let queue = state.queues.get_mut(&url).unwrap();
                let Some(message) = queue
                    .iter_mut()
                    .find(|m| m.receipts.last().is_some_and(|r| r == receipt))
                else {
                    return error("ReceiptHandleIsInvalid", "unknown receipt handle");
                };
                if now + timeout > message.received_at + MAX_VISIBILITY {
                    return error(
                        "InvalidParameterValue",
                        &format!(
                            "Value {timeout} for parameter VisibilityTimeout is invalid. Reason: \
                             Total VisibilityTimeout for the message is beyond the limit [43200 \
                             seconds]"
                        ),
                    );
                }
                message.visible_at = now + timeout;
                (200, json!({}))
            }
            "AmazonSQS.GetQueueAttributes" => {
                let queue = &state.queues[&url];
                let visible = queue.iter().filter(|m| m.visible_at <= now).count();
                let in_flight = queue
                    .iter()
                    .filter(|m| m.visible_at > now && m.receive_count > 0)
                    .count();
                let delayed = queue
                    .iter()
                    .filter(|m| m.visible_at > now && m.receive_count == 0)
                    .count();
                (
                    200,
                    json!({ "Attributes": {
                        "ApproximateNumberOfMessages": visible.to_string(),
                        "ApproximateNumberOfMessagesNotVisible": in_flight.to_string(),
                        "ApproximateNumberOfMessagesDelayed": delayed.to_string(),
                    } }),
                )
            }
            "AmazonSQS.PurgeQueue" => {
                state.queues.get_mut(&url).unwrap().clear();
                (200, json!({}))
            }
            other => error("InvalidAction", &format!("unknown action {other}")),
        }
    }
}
