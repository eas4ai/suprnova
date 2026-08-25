//! Queue routing - `Queue::route`, `Job::queue`, and their precedence.
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
job!(
    ChainedDeclaredJob,
    "routing::ChainedDeclaredJob",
    queue = "reports"
);
job!(
    ChainedRoutedJob,
    "routing::ChainedRoutedJob",
    queue = "reports"
);

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
            unique_lock_owner: None,
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

    // Nothing else on `billing` - the reports and unrouted jobs are untouched.
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

/// Chains store their jobs type-erased, which is exactly where the "job" tier
/// of the resolution order can silently vanish: the head of this chain would
/// land on the default queue while a direct `Queue::push` of the same job
/// lands on `reports`.
#[tokio::test]
#[serial]
async fn a_chained_job_keeps_its_declared_queue() {
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    Queue::chain()
        .add(ChainedDeclaredJob)
        .expect("add link")
        .dispatch()
        .await
        .expect("dispatch chain");
    let env = driver
        .pop(Duration::from_secs(60))
        .await
        .expect("pop should succeed")
        .expect("chain head should be queued")
        .envelope;
    assert_eq!(
        env.queue.as_deref(),
        Some("reports"),
        "a chained job must keep its declared queue, same as a direct push"
    );
}

/// And the operator's route must still outrank the captured declaration, so
/// chains follow the same route → job → default order as everything else.
#[tokio::test]
#[serial]
async fn a_route_overrides_a_chained_jobs_declared_queue() {
    Queue::route::<ChainedRoutedJob>(None, Some("urgent"));
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    Queue::chain()
        .add(ChainedRoutedJob)
        .expect("add link")
        .dispatch()
        .await
        .expect("dispatch chain");
    let env = driver
        .pop(Duration::from_secs(60))
        .await
        .expect("pop should succeed")
        .expect("chain head should be queued")
        .envelope;
    assert_eq!(env.queue.as_deref(), Some("urgent"));
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

// ============================================================================
// Queue::forward - name-keyed redirects (Laravel #61188)
// ============================================================================

job!(ForwardedJob, "routing::ForwardedJob", queue = "fwd_src");
job!(
    ForwardRoutedJob,
    "routing::ForwardRoutedJob",
    queue = "fwd_declared"
);
job!(
    ForwardScopedJob,
    "routing::ForwardScopedJob",
    queue = "fwd_scoped_src"
);
job!(ForwardDefaultJob, "routing::ForwardDefaultJob");
job!(
    ForwardChainedJob,
    "routing::ForwardChainedJob",
    queue = "fwd_chain_src"
);

#[tokio::test]
#[serial]
async fn a_forward_moves_the_push_to_the_destination_queue() {
    Queue::forward("fwd_src", "fwd_dest");
    let env = pushed_envelope(ForwardedJob).await;
    assert_eq!(
        env.queue.as_deref(),
        Some("fwd_dest"),
        "a forwarded queue name must reach the driver rewritten"
    );
    let fwd = Queue::forward_for("fwd_src").expect("forward should be registered");
    assert_eq!(fwd.queue.as_deref(), Some("fwd_dest"));
    assert_eq!(fwd.connection, None);
}

#[tokio::test]
#[serial]
async fn a_route_resolves_first_and_the_forward_rewrites_its_result() {
    // The job declares `fwd_declared`; the operator routes it to `fwd_routed`;
    // `fwd_routed` is then forwarded to `fwd_final`. Only the last hop applies
    // to the route's output - forwards are a single lookup, never a chain.
    Queue::route::<ForwardRoutedJob>(None, Some("fwd_routed"));
    Queue::forward("fwd_routed", "fwd_final");
    Queue::forward("fwd_declared", "fwd_never");
    let env = pushed_envelope(ForwardRoutedJob).await;
    assert_eq!(
        env.queue.as_deref(),
        Some("fwd_final"),
        "the forward applies to what routing resolved, not to the job's own declaration"
    );
}

#[tokio::test]
#[serial]
async fn a_forward_scoped_to_another_connection_is_inert() {
    Queue::forward_on("fwd_scoped_src", "fwd_scoped_dest", "some-other-connection");
    let env = pushed_envelope(ForwardScopedJob).await;
    assert_eq!(
        env.queue.as_deref(),
        Some("fwd_scoped_src"),
        "a forward gated on a connection this push is not on must not fire"
    );
}

#[tokio::test]
#[serial]
async fn forwarding_the_default_queue_catches_jobs_that_named_none() {
    Queue::forward("default", "fwd_from_default");
    let env = pushed_envelope(ForwardDefaultJob).await;
    assert_eq!(
        env.queue.as_deref(),
        Some("fwd_from_default"),
        "an envelope with no queue means `default`, which a forward on `default` must catch"
    );

    // `default` is the one source name a test cannot make unique, and there is
    // no un-forward. Re-registering it onto itself is the documented no-op
    // form, so the rest of this binary sees `default` passing through again.
    Queue::forward("default", "default");
    let env = pushed_envelope(ForwardDefaultJob).await;
    assert_eq!(
        env.queue, None,
        "a forward onto a queue's own name is the identity, so an envelope that \
         named no queue must still put no queue on the wire"
    );
}

/// A chain builds its envelopes through `ChainLink`, not through
/// `build_envelope`, so the forward has to be applied there too. Without it a
/// chained job would be pushed to the source queue while every worker started
/// on that source queue is claiming the destination - the exact stranding this
/// feature exists to prevent.
#[tokio::test]
#[serial]
async fn a_chained_job_follows_the_forward_too() {
    Queue::forward("fwd_chain_src", "fwd_chain_dest");
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    Queue::chain()
        .add(ForwardChainedJob)
        .expect("add link")
        .dispatch()
        .await
        .expect("dispatch chain");
    let env = driver
        .pop(Duration::from_secs(60))
        .await
        .expect("pop should succeed")
        .expect("chain head should be queued")
        .envelope;
    assert_eq!(
        env.queue.as_deref(),
        Some("fwd_chain_dest"),
        "a chained job must follow the forward, or the chain strands on a queue \
         no worker claims"
    );
}

/// The half that makes forwarding usable rather than a way to strand work: a
/// worker told to drain the source queue must follow the forward, or the
/// destination accumulates jobs nobody claims.
#[tokio::test]
#[serial]
async fn a_worker_started_on_the_source_queue_drains_the_destination() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use suprnova::App;
    use suprnova::cache::{Cache, CacheStore, InMemoryCache};
    use suprnova::queue::worker::{WorkerConfig, register_job, run_worker};
    use tokio_util::sync::CancellationToken;

    #[derive(Serialize, Deserialize)]
    struct ForwardDrainJob;
    static DRAIN_RUNS: AtomicU32 = AtomicU32::new(0);

    #[suprnova::async_trait]
    impl Job for ForwardDrainJob {
        fn job_name() -> &'static str {
            "routing::ForwardDrainJob"
        }
        fn queue() -> Option<&'static str> {
            Some("fwd_drain_src")
        }
        async fn handle(self) -> Result<(), FrameworkError> {
            DRAIN_RUNS.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    if !Cache::is_initialized() {
        App::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));
    }
    DRAIN_RUNS.store(0, Ordering::SeqCst);
    register_job::<ForwardDrainJob>();

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    Queue::forward("fwd_drain_src", "fwd_drain_dest");
    Queue::push(ForwardDrainJob).await.expect("push");

    // The envelope now carries `fwd_drain_dest`, so a worker that honoured its
    // `--queue` list literally would never see it.
    let handle = tokio::spawn(run_worker(
        driver.clone(),
        WorkerConfig {
            visibility_timeout: Duration::from_secs(30),
            poll_interval: Duration::from_millis(5),
            max_jobs: None,
            queues: vec!["fwd_drain_src".to_string()],
        },
        CancellationToken::new(),
    ));
    for _ in 0..300 {
        if DRAIN_RUNS.load(Ordering::SeqCst) > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    handle.abort();
    assert_eq!(
        DRAIN_RUNS.load(Ordering::SeqCst),
        1,
        "a forward must move the worker's claim too, or the destination queue \
         accumulates work nobody drains"
    );
}

job!(
    ForwardOverrideJob,
    "routing::ForwardOverrideJob",
    queue = "fwd_ovr_src"
);

/// A per-push connection override outranks routing, so it has to move the gate
/// a connection-scoped forward is evaluated against. Miss that and the push
/// stays on the source queue while a worker on the same connection is already
/// claiming the destination - a forward applied to one half of the pair.
#[tokio::test]
#[serial]
async fn a_per_push_connection_override_moves_the_gate_of_a_scoped_forward() {
    use suprnova::queue::EnvelopeOverrides;

    Queue::forward_on("fwd_ovr_src", "fwd_ovr_dest", "fwd-ovr-conn");

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    Queue::push_with(
        ForwardOverrideJob,
        EnvelopeOverrides {
            connection: Some("fwd-ovr-conn".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("push should succeed");
    let env = driver
        .pop(Duration::from_secs(60))
        .await
        .expect("pop should succeed")
        .expect("an envelope should be queued")
        .envelope;
    assert_eq!(
        env.queue.as_deref(),
        Some("fwd_ovr_dest"),
        "a push that declared it is on the gated connection must follow the forward"
    );

    // And the other direction: without the override the push is not on that
    // connection, so the same forward stays inert.
    let env = pushed_envelope(ForwardOverrideJob).await;
    assert_eq!(
        env.queue.as_deref(),
        Some("fwd_ovr_src"),
        "the gate must still hold for a push that never named the connection"
    );
}
