//! Queue routing — `Queue::route`, `Job::queue`, and their precedence.
//!
//! Routing decides which worker pool drains which job, so the failure that
//! matters is a *silent* one: a job that looks routed but lands on the default
//! queue anyway (work never picked up by the dedicated pool), or a job that
//! picks up a route it was never given (work stolen from the default pool).
//! Every test below pins one of those two directions.
//!
//! Each test uses its own job type. Routes are keyed by `Job::job_name()` in a
//! process-global registry, so distinct types keep tests hermetic under
//! parallel execution without needing to clear shared state.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serial_test::serial;
use suprnova::FrameworkError;
use suprnova::queue::driver::QueueDriver;
use suprnova::queue::memory::MemoryQueueDriver;
use suprnova::queue::{Job, Queue};

/// Push `job` through the facade and return the envelope the driver received.
///
/// Goes through `Queue::push` rather than building an envelope directly so the
/// test exercises the real resolution path, not a reimplementation of it.
async fn pushed_envelope<J: Job>(job: J) -> suprnova::queue::Envelope {
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    Queue::push(job).await.expect("push should succeed");
    driver
        .pop(Duration::from_secs(60))
        .await
        .expect("pop should succeed")
        .expect("an envelope should be queued")
        .envelope
}

macro_rules! job {
    ($name:ident, $wire:literal $(, queue = $q:literal)? $(, connection = $c:literal)?) => {
        #[derive(Serialize, Deserialize)]
        struct $name;

        #[suprnova::async_trait]
        impl Job for $name {
            fn job_name() -> &'static str {
                $wire
            }
            async fn handle(self) -> Result<(), FrameworkError> {
                Ok(())
            }
            $(fn queue() -> Option<&'static str> { Some($q) })?
            $(fn connection() -> Option<&'static str> { Some($c) })?
        }
    };
}

job!(PlainJob, "routing::PlainJob");
job!(SelfQueuedJob, "routing::SelfQueuedJob", queue = "reports");
job!(OverriddenJob, "routing::OverriddenJob", queue = "reports");
job!(
    ConnectionOnlyJob,
    "routing::ConnectionOnlyJob",
    queue = "reports"
);
job!(ReRoutedJob, "routing::ReRoutedJob");
job!(SerializedJob, "routing::SerializedJob");

#[tokio::test]
#[serial]
async fn unrouted_job_uses_the_driver_default_queue() {
    let env = pushed_envelope(PlainJob).await;
    assert_eq!(
        env.queue, None,
        "a job with no route and no declared queue must stay on the default"
    );
}

#[tokio::test]
#[serial]
async fn job_declared_queue_is_used_when_nothing_is_registered() {
    let env = pushed_envelope(SelfQueuedJob).await;
    assert_eq!(env.queue.as_deref(), Some("reports"));
}

#[tokio::test]
#[serial]
async fn registered_route_overrides_the_jobs_own_queue() {
    Queue::route::<OverriddenJob>(None, Some("urgent"));
    let env = pushed_envelope(OverriddenJob).await;
    assert_eq!(
        env.queue.as_deref(),
        Some("urgent"),
        "an operator's route must win over the job's own opinion"
    );
}

#[tokio::test]
#[serial]
async fn routing_only_the_connection_leaves_the_declared_queue_intact() {
    Queue::route::<ConnectionOnlyJob>(Some("redis"), None);
    let env = pushed_envelope(ConnectionOnlyJob).await;
    assert_eq!(
        env.queue.as_deref(),
        Some("reports"),
        "a None queue in a route defers to the job rather than clearing it"
    );
    let route = Queue::route_for::<ConnectionOnlyJob>().expect("route should be registered");
    assert_eq!(route.connection.as_deref(), Some("redis"));
    assert_eq!(route.queue, None);
}

#[tokio::test]
#[serial]
async fn reregistering_a_job_replaces_the_previous_route() {
    Queue::route::<ReRoutedJob>(None, Some("first"));
    Queue::route::<ReRoutedJob>(None, Some("second"));
    let env = pushed_envelope(ReRoutedJob).await;
    assert_eq!(env.queue.as_deref(), Some("second"));
}

#[test]
fn unrouted_envelopes_stay_byte_identical_on_the_wire() {
    // The queue key is skipped when absent, which is what lets a routed and an
    // unrouted deployment share a queue backend during a rolling upgrade.
    // Byte-for-byte the payload frozen in `queue_envelope.rs`, so this test
    // fails if that wire format ever drifts out from under routing.
    let unrouted = serde_json::json!({
        "schema_version": 2,
        "id": "550e8400-e29b-41d4-a716-446655440000",
        "job_name": "Frozen",
        "payload": {"k": "v"},
        "dispatched_at": "2026-05-16T12:34:56Z",
        "available_at": "2026-05-16T12:34:56Z",
        "attempts": 0,
        "max_tries": 3,
        "backoff": {"kind": "exponential", "base_secs": 2, "cap_secs": 300, "jitter_ratio": 0.25},
        "timeout_secs": null,
        "fail_on_timeout": false,
        "idempotency_key": null,
        "batch_id": null,
        "chain_remaining": []
    });

    let env: suprnova::queue::Envelope =
        serde_json::from_value(unrouted.clone()).expect("legacy envelope must still deserialize");
    assert_eq!(env.queue, None);

    let reserialized = serde_json::to_value(&env).expect("serialize");
    assert_eq!(
        reserialized, unrouted,
        "an unrouted envelope must not gain a queue key, or old workers see a changed wire format"
    );
}

/// The whole point of routing: a worker dedicated to one queue must take only
/// that queue's work, and must not consume anyone else's.
#[tokio::test]
async fn a_filtered_worker_takes_only_its_own_queue() {
    use suprnova::queue::BackoffSchedule;
    use suprnova::queue::envelope::{CURRENT_SCHEMA_VERSION, Envelope};

    fn env(name: &str, queue: Option<&str>) -> Envelope {
        Envelope {
            schema_version: CURRENT_SCHEMA_VERSION,
            id: uuid::Uuid::new_v4(),
            job_name: name.into(),
            queue: queue.map(str::to_owned),
            payload: serde_json::json!({}),
            dispatched_at: chrono::Utc::now(),
            available_at: chrono::Utc::now(),
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

    let d = MemoryQueueDriver::new();
    d.push(env("a", Some("billing"))).await.unwrap();
    d.push(env("b", Some("reports"))).await.unwrap();
    d.push(env("c", None)).await.unwrap();

    let billing = vec!["billing".to_string()];
    let got = d.pop_from(Duration::from_secs(60), &billing).await.unwrap();
    assert_eq!(got.expect("billing job").envelope.job_name, "a");

    // Nothing else on `billing` — the reports and unrouted jobs are untouched.
    assert!(
        d.pop_from(Duration::from_secs(60), &billing)
            .await
            .unwrap()
            .is_none(),
        "a billing worker must not consume other queues' jobs"
    );

    // An unrouted job belongs to `default` and is drained by a default worker.
    let default = vec!["default".to_string()];
    let got = d.pop_from(Duration::from_secs(60), &default).await.unwrap();
    assert_eq!(
        got.expect("unrouted job").envelope.job_name,
        "c",
        "a job with no route must be reachable as `default`, not stranded"
    );

    // And an unfiltered worker still drains whatever remains.
    let got = d.pop(Duration::from_secs(60)).await.unwrap();
    assert_eq!(got.expect("remaining job").envelope.job_name, "b");
}

/// A driver that cannot filter must say so loudly rather than draining
/// everything, which would look identical to a working setup.
#[tokio::test]
async fn a_driver_without_filtering_rejects_a_queue_filter() {
    let d = suprnova::queue::NullQueueDriver;
    let filtered = d
        .pop_from(Duration::from_secs(1), &["billing".to_string()])
        .await;
    assert!(
        filtered.is_err(),
        "an unsupported filter must not silently pass"
    );

    // ...but an unfiltered pop is unaffected.
    assert!(d.pop_from(Duration::from_secs(1), &[]).await.is_ok());
}

#[tokio::test]
#[serial]
async fn routed_envelope_carries_the_queue_on_the_wire() {
    Queue::route::<SerializedJob>(None, Some("billing"));
    let env = pushed_envelope(SerializedJob).await;
    let json = serde_json::to_value(&env).expect("serialize");
    assert_eq!(
        json.get("queue").and_then(|v| v.as_str()),
        Some("billing"),
        "a routed job must carry its queue so the driver can honor it"
    );
}
