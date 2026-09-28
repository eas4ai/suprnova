//! Named queue connections: a connection name selects the driver a job is
//! pushed to, and the worker that drains it.
//!
//! The failure that matters is a push that lands on the wrong driver without
//! a word: work that a dedicated connection's workers never see, or work
//! that the default connection's workers were never meant to run. Every test
//! pins one direction of that, or the error that replaces it.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serial_test::serial;
use suprnova::events::{EventFacade, Listener, dispatched};
use suprnova::queue::events::{JobProcessed, JobQueued, WorkerQueuePaused, WorkerStarting};
use suprnova::queue::testing::forget_connections;
use suprnova::queue::worker::{WorkerConfig, register_job, run_worker, run_worker_on};
use suprnova::queue::{
    FailedJobStore, Job, MemoryFailedJobStore, MemoryQueueDriver, Queue, QueueDriver,
};
use suprnova::{EnvelopeOverrides, FrameworkError};
use tokio_util::sync::CancellationToken;

/// The label of every job that ran, in order.
static RAN: Mutex<Vec<String>> = Mutex::new(Vec::new());

macro_rules! job {
    ($name:ident, $wire:literal $(, connection = $c:literal)? $(, unique = $u:literal)?) => {
        #[derive(Serialize, Deserialize, Clone)]
        struct $name;

        #[suprnova::async_trait]
        impl Job for $name {
            fn job_name() -> &'static str {
                $wire
            }
            async fn handle(self) -> Result<(), FrameworkError> {
                RAN.lock().unwrap().push($wire.to_owned());
                Ok(())
            }
            $(fn connection() -> Option<&'static str> { Some($c) })?
            $(fn unique_id(&self) -> Option<String> { Some($u.to_owned()) })?
        }
    };
}

job!(PlainJob, "connections::PlainJob");
job!(
    DurableJob,
    "connections::DurableJob",
    connection = "durable"
);
job!(
    OtherDurableJob,
    "connections::OtherDurableJob",
    connection = "durable"
);
job!(StrayJob, "connections::StrayJob", connection = "nowhere");
job!(RoutedJob, "connections::RoutedJob");
job!(
    ForwardedJob,
    "connections::ForwardedJob",
    connection = "durable"
);
job!(
    UniqueDurableJob,
    "connections::UniqueDurableJob",
    connection = "durable",
    unique = "one"
);
job!(
    UniqueLaterJob,
    "connections::UniqueLaterJob",
    connection = "later",
    unique = "later-one"
);
job!(AliasJob, "connections::AliasJob", connection = "alias");

#[derive(Serialize, Deserialize, Clone)]
struct AfterCommitStrayJob;

#[suprnova::async_trait]
impl Job for AfterCommitStrayJob {
    fn job_name() -> &'static str {
        "connections::AfterCommitStrayJob"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
    fn connection() -> Option<&'static str> {
        Some("nowhere")
    }
    fn after_commit() -> bool {
        true
    }
}

fn cache_init() {
    if !suprnova::cache::Cache::is_initialized() {
        suprnova::App::bind::<dyn suprnova::cache::CacheStore>(Arc::new(
            suprnova::cache::InMemoryCache::new(),
        ));
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct FailingDurableJob;

#[suprnova::async_trait]
impl Job for FailingDurableJob {
    fn job_name() -> &'static str {
        "connections::FailingDurableJob"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Err(FrameworkError::internal("permanent failure"))
    }
    fn connection() -> Option<&'static str> {
        Some("durable")
    }
    fn max_tries() -> u32 {
        1
    }
}

/// A default connection and a connection named `durable`, each on its own
/// in-memory driver. The connections are process-wide, so they are removed
/// again when the test ends, passed or not: a connection left registered
/// would change what a connection name means for every later test.
struct TwoConnections {
    default: Arc<MemoryQueueDriver>,
    durable: Arc<MemoryQueueDriver>,
}

impl TwoConnections {
    fn install() -> Self {
        forget_connections();
        let default = Arc::new(MemoryQueueDriver::new());
        let durable = Arc::new(MemoryQueueDriver::new());
        Queue::set_driver(default.clone());
        Queue::register_connection("durable", durable.clone());
        Self { default, durable }
    }

    async fn sizes(&self) -> (u64, u64) {
        (
            self.default.size().await.unwrap(),
            self.durable.size().await.unwrap(),
        )
    }
}

impl Drop for TwoConnections {
    fn drop(&mut self) {
        forget_connections();
    }
}

fn worker_for(jobs: u64) -> WorkerConfig {
    WorkerConfig {
        visibility_timeout: Duration::from_secs(5),
        poll_interval: Duration::from_millis(5),
        max_jobs: Some(jobs),
        queues: Vec::new(),
    }
}

#[tokio::test]
#[serial]
async fn a_job_is_pushed_to_the_connection_it_names() {
    let connections = TwoConnections::install();

    Queue::push(DurableJob).await.unwrap();
    assert_eq!(connections.sizes().await, (0, 1));

    Queue::push(PlainJob).await.unwrap();
    assert_eq!(
        connections.sizes().await,
        (1, 1),
        "a job that names no connection stays on the default one"
    );
}

#[tokio::test]
#[serial]
async fn a_route_and_a_per_push_override_each_select_the_connection() {
    let connections = TwoConnections::install();

    Queue::route::<RoutedJob>(Some("durable"), None);
    Queue::push(RoutedJob).await.unwrap();
    assert_eq!(connections.sizes().await, (0, 1), "the route selects");

    Queue::push_with(
        PlainJob,
        EnvelopeOverrides {
            connection: Some("durable".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(connections.sizes().await, (0, 2), "the override selects");

    // The override outranks the job's own connection, in both directions.
    Queue::push_with(
        DurableJob,
        EnvelopeOverrides {
            connection: Some(Queue::connection_name()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(connections.sizes().await, (1, 2));
}

#[tokio::test]
#[serial]
async fn the_other_push_paths_select_the_connection_too() {
    let connections = TwoConnections::install();
    cache_init();

    Queue::bulk(vec![DurableJob, DurableJob]).await.unwrap();
    assert_eq!(connections.sizes().await, (0, 2), "bulk");

    assert!(Queue::push_unique(UniqueDurableJob).await.unwrap());
    assert_eq!(connections.sizes().await, (0, 3), "push_unique");

    Queue::later(Duration::from_secs(60), DurableJob)
        .await
        .unwrap();
    assert_eq!(connections.sizes().await, (0, 4), "later");
}

#[tokio::test]
#[serial]
async fn a_name_that_selects_no_connection_is_refused_and_nothing_is_pushed() {
    let connections = TwoConnections::install();

    let error = Queue::push(StrayJob)
        .await
        .expect_err("`nowhere` is not a connection");

    let message = error.to_string();
    assert!(
        message.contains("queue connection `nowhere` is not registered"),
        "{message}"
    );
    assert!(
        message.contains("durable"),
        "the error names what is registered: {message}"
    );
    assert_eq!(connections.sizes().await, (0, 0));
    assert!(Queue::connection("nowhere").is_err());
}

#[tokio::test]
#[serial]
async fn with_no_connection_registered_a_name_is_only_a_label() {
    forget_connections();
    let default = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(default.clone());
    let _events = EventFacade::fake();

    // An application with one driver that names connections on its jobs,
    // as it could before a name selected a driver. Nothing changes for it.
    Queue::push(StrayJob).await.unwrap();

    assert_eq!(default.size().await.unwrap(), 1);
    let queued = dispatched::<JobQueued>(|_| true);
    assert_eq!(queued.len(), 1);
    assert_eq!(
        queued[0].connection, "nowhere",
        "the events still report the name the job declared"
    );
    assert!(Queue::connection_names().unwrap().is_empty());
}

#[tokio::test]
#[serial]
async fn a_worker_on_a_connection_drains_that_connection_and_carries_its_name() {
    RAN.lock().unwrap().clear();
    register_job::<PlainJob>();
    register_job::<DurableJob>();
    let connections = TwoConnections::install();
    Queue::push(PlainJob).await.unwrap();
    Queue::push(DurableJob).await.unwrap();

    let _events = EventFacade::fake();
    run_worker_on("durable", worker_for(1), CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(*RAN.lock().unwrap(), ["connections::DurableJob"]);
    assert_eq!(
        connections.sizes().await,
        (1, 0),
        "the default connection's job is not this worker's to run"
    );
    let started = dispatched::<WorkerStarting>(|_| true);
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].connection, "durable");
    let processed = dispatched::<JobProcessed>(|_| true);
    assert_eq!(processed.len(), 1);
    assert_eq!(processed[0].job.connection, "durable");
}

#[tokio::test]
#[serial]
async fn a_worker_on_the_default_connection_by_name_drains_the_default_driver() {
    RAN.lock().unwrap().clear();
    register_job::<PlainJob>();
    let connections = TwoConnections::install();
    Queue::push(PlainJob).await.unwrap();
    Queue::push(DurableJob).await.unwrap();

    run_worker_on(
        &Queue::connection_name(),
        worker_for(1),
        CancellationToken::new(),
    )
    .await
    .unwrap();

    assert_eq!(*RAN.lock().unwrap(), ["connections::PlainJob"]);
    assert_eq!(connections.sizes().await, (0, 1));
}

#[tokio::test]
#[serial]
async fn a_worker_on_a_name_that_is_no_connection_does_not_start() {
    let _connections = TwoConnections::install();

    let error = run_worker_on("nowhere", worker_for(1), CancellationToken::new())
        .await
        .expect_err("there is nothing to drain");

    assert!(error.to_string().contains("`nowhere` is not registered"));
}

#[tokio::test]
#[serial]
async fn the_jobs_of_a_batch_each_go_to_their_own_connection() {
    let connections = TwoConnections::install();

    Queue::batch()
        .name("mixed")
        .add(PlainJob)
        .add(DurableJob)
        .add(DurableJob)
        .dispatch()
        .await
        .unwrap();

    assert_eq!(connections.sizes().await, (1, 2));
}

#[tokio::test]
#[serial]
async fn a_batch_with_a_job_for_no_connection_is_refused_whole() {
    let connections = TwoConnections::install();

    let error = Queue::batch()
        .name("stray")
        .add(PlainJob)
        .add(StrayJob)
        .dispatch()
        .await
        .expect_err("one job names no connection");

    assert!(error.to_string().contains("`nowhere` is not registered"));
    assert_eq!(
        connections.sizes().await,
        (0, 0),
        "the job before the stray one must not have been pushed"
    );
}

#[tokio::test]
#[serial]
async fn a_chain_runs_on_the_connection_of_its_first_job() {
    RAN.lock().unwrap().clear();
    register_job::<DurableJob>();
    register_job::<OtherDurableJob>();
    let connections = TwoConnections::install();

    Queue::chain()
        .add(DurableJob)
        .unwrap()
        .add(OtherDurableJob)
        .unwrap()
        .dispatch()
        .await
        .unwrap();
    assert_eq!(
        connections.sizes().await,
        (0, 1),
        "the head is on `durable`"
    );

    run_worker_on("durable", worker_for(2), CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(
        *RAN.lock().unwrap(),
        ["connections::DurableJob", "connections::OtherDurableJob"]
    );
    assert_eq!(connections.sizes().await, (0, 0));
}

#[tokio::test]
#[serial]
async fn a_chain_whose_links_name_different_connections_is_refused() {
    let connections = TwoConnections::install();

    let error = Queue::chain()
        .add(DurableJob)
        .unwrap()
        .add(PlainJob)
        .unwrap()
        .dispatch()
        .await
        .expect_err("the second link is for the default connection");

    let message = error.to_string();
    assert!(message.contains("connections::PlainJob"), "{message}");
    assert!(
        message.contains("A chain runs on one connection"),
        "{message}"
    );
    assert_eq!(connections.sizes().await, (0, 0), "nothing was pushed");
}

#[tokio::test]
#[serial]
async fn a_forward_scoped_to_a_connection_moves_the_push_and_the_claim_on_it() {
    RAN.lock().unwrap().clear();
    register_job::<ForwardedJob>();
    let connections = TwoConnections::install();
    Queue::forward_on("conn_fwd_src", "conn_fwd_dest", "durable");

    // A push to `durable` is forwarded...
    Queue::push_with(
        ForwardedJob,
        EnvelopeOverrides {
            queue: Some("conn_fwd_src".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    // ...and a push to the default connection is not.
    Queue::push_with(
        PlainJob,
        EnvelopeOverrides {
            queue: Some("conn_fwd_src".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let on_default = connections
        .default
        .pop(Duration::from_secs(5))
        .await
        .unwrap()
        .expect("the default connection's job")
        .envelope;
    assert_eq!(on_default.queue.as_deref(), Some("conn_fwd_src"));

    // A worker started on the source queue of `durable` claims the
    // destination, so the forwarded job is not stranded.
    let cfg = WorkerConfig {
        queues: vec!["conn_fwd_src".into()],
        ..worker_for(1)
    };
    run_worker_on("durable", cfg, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(*RAN.lock().unwrap(), ["connections::ForwardedJob"]);
}

#[tokio::test]
#[serial]
async fn a_failed_job_is_retried_on_the_connection_it_failed_on() {
    register_job::<FailingDurableJob>();
    let connections = TwoConnections::install();
    let store = Arc::new(MemoryFailedJobStore::new());
    Queue::set_failed_store(store.clone());
    Queue::push(FailingDurableJob).await.unwrap();

    run_worker_on("durable", worker_for(1), CancellationToken::new())
        .await
        .unwrap();
    let failed = store.all().await.unwrap();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].connection, "durable");
    assert_eq!(connections.sizes().await, (0, 0));

    assert!(Queue::retry_failed(failed[0].id).await.unwrap());

    assert_eq!(
        connections.sizes().await,
        (0, 1),
        "a retry on the default connection would never be run by a `durable` worker"
    );
}

#[tokio::test]
#[serial]
async fn run_worker_with_an_explicit_driver_keeps_the_default_label() {
    RAN.lock().unwrap().clear();
    register_job::<PlainJob>();
    let connections = TwoConnections::install();
    Queue::push(PlainJob).await.unwrap();

    let _events = EventFacade::fake();
    run_worker(
        connections.default.clone(),
        worker_for(1),
        CancellationToken::new(),
    )
    .await;

    let started = dispatched::<WorkerStarting>(|_| true);
    assert_eq!(started[0].connection, Queue::connection_name());
    assert_eq!(*RAN.lock().unwrap(), ["connections::PlainJob"]);
}

// Async only because the in-memory driver is built inside a runtime.
#[tokio::test]
#[serial]
async fn a_connection_needs_a_name() {
    forget_connections();
    let error = Queue::try_register_connection("  ", Arc::new(MemoryQueueDriver::new()))
        .expect_err("a blank name is the default connection's, not a connection");
    assert!(error.to_string().contains("needs a name"));
    assert!(Queue::connection_names().unwrap().is_empty());

    Queue::register_connection("b", Arc::new(MemoryQueueDriver::new()));
    Queue::register_connection("a", Arc::new(MemoryQueueDriver::new()));
    assert_eq!(Queue::connection_names().unwrap(), ["a", "b"]);
    forget_connections();
}

// --- One driver has one label -----------------------------------------------

#[tokio::test]
#[serial]
async fn a_second_name_for_the_default_connection_is_the_default_connection() {
    RAN.lock().unwrap().clear();
    register_job::<AliasJob>();
    register_job::<PlainJob>();
    cache_init();
    let connections = TwoConnections::install();
    // What `QUEUE_CONNECTIONS` does when it lists the driver `QUEUE_DRIVER`
    // selects: the default's own driver, under a name of its own.
    Queue::register_connection("alias", connections.default.clone());
    let default_name = Queue::connection_name();

    // Both names reach the one queue.
    Queue::push(AliasJob).await.unwrap();
    assert_eq!(connections.sizes().await, (1, 0));

    // A forward scoped to either name moves the pushes of both.
    Queue::forward_on("alias_fwd_src", "alias_fwd_dest", "alias");
    Queue::push_with(
        PlainJob,
        EnvelopeOverrides {
            queue: Some("alias_fwd_src".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    // ...and the claim of a worker started on the default connection, so
    // the forwarded job is not stranded.
    let cfg = WorkerConfig {
        queues: vec!["alias_fwd_src".into()],
        ..worker_for(1)
    };
    run_worker(connections.default.clone(), cfg, CancellationToken::new()).await;
    assert_eq!(*RAN.lock().unwrap(), ["connections::PlainJob"]);

    // A pause set under one name is the pause a worker under the other reads.
    Queue::pause("alias", "alias_paused").await.unwrap();
    assert!(
        Queue::is_paused(&default_name, "alias_paused")
            .await
            .unwrap()
    );
    Queue::resume(&default_name, "alias_paused").await.unwrap();
    assert!(!Queue::is_paused("alias", "alias_paused").await.unwrap());

    // And a chain over both names is a chain on one connection.
    Queue::chain()
        .add(AliasJob)
        .unwrap()
        .add(PlainJob)
        .unwrap()
        .dispatch()
        .await
        .expect("both links are on the default connection");
}

#[tokio::test]
#[serial]
async fn the_defaults_own_name_always_means_the_default_driver() {
    let connections = TwoConnections::install();
    let shadow = Arc::new(MemoryQueueDriver::new());
    Queue::register_connection(&Queue::connection_name(), shadow.clone());

    Queue::push(PlainJob).await.unwrap();

    assert_eq!(connections.sizes().await, (1, 0));
    assert_eq!(
        shadow.size().await.unwrap(),
        0,
        "a connection registered under the default's name must not shadow it: \
         Queue::driver() and Queue::size() would then read another queue than pushes write"
    );
}

// --- A name that is no connection fails early and leaves nothing behind -----

#[tokio::test]
#[serial]
async fn a_deferred_push_to_no_connection_fails_before_the_commit() {
    let connections = TwoConnections::install();
    let _db = suprnova::testing::TestDatabase::sqlite_memory()
        .await
        .expect("sqlite");

    let committed: Result<(), FrameworkError> = suprnova::DB::transaction(|_tx| {
        Box::pin(async {
            let refused = Queue::push(AfterCommitStrayJob)
                .await
                .expect_err("the caller must learn it while it can still roll back");
            assert!(refused.to_string().contains("`nowhere` is not registered"));
            Ok::<(), FrameworkError>(())
        })
    })
    .await;

    committed.expect("nothing was deferred, so the commit has nothing to fail on");
    assert_eq!(connections.sizes().await, (0, 0));
}

#[tokio::test]
#[serial]
async fn a_unique_push_to_no_connection_gives_its_lock_back() {
    let _connections = TwoConnections::install();
    cache_init();

    Queue::push_unique(UniqueLaterJob)
        .await
        .expect_err("`later` is not a connection yet");

    // Had the refused push kept its lock, this one would be a duplicate.
    let later = Arc::new(MemoryQueueDriver::new());
    Queue::register_connection("later", later.clone());
    assert!(
        Queue::push_unique(UniqueLaterJob).await.unwrap(),
        "the refused push must not block the next one"
    );
    assert_eq!(later.size().await.unwrap(), 1);
}

#[tokio::test]
#[serial]
async fn retrying_every_failed_job_returns_each_to_its_connection_and_keeps_the_stray() {
    register_job::<FailingDurableJob>();
    let connections = TwoConnections::install();
    let store = Arc::new(MemoryFailedJobStore::new());
    Queue::set_failed_store(store.clone());

    // Fail one real job on `durable`, then file its envelope under the
    // default connection and under a connection that no longer exists.
    Queue::push(FailingDurableJob).await.unwrap();
    run_worker_on("durable", worker_for(1), CancellationToken::new())
        .await
        .unwrap();
    let failed = store.all().await.unwrap();
    assert_eq!(failed.len(), 1);
    let envelope = suprnova::queue::Envelope::from_json(&failed[0].envelope_json).unwrap();
    store
        .log(&Queue::connection_name(), "default", &envelope, "boom")
        .await
        .unwrap();
    let stray = store
        .log("removed", "default", &envelope, "boom")
        .await
        .unwrap();

    let retried = Queue::retry_all_failed(None).await.unwrap();

    assert_eq!(retried, 2, "the stray record is not counted");
    assert_eq!(connections.sizes().await, (1, 1));
    assert_eq!(
        store.ids().await.unwrap(),
        [stray],
        "a record whose connection is gone is kept, not lost and not a reason to stop"
    );
}

// --- Pause ------------------------------------------------------------------

/// Stops the worker of the test that is running, the moment the worker
/// reports that it found its queue paused.
static STOP_ON_PAUSE: Mutex<Option<CancellationToken>> = Mutex::new(None);

struct StopOnPause;

#[suprnova::async_trait]
impl Listener<WorkerQueuePaused> for StopOnPause {
    async fn handle(&self, event: &WorkerQueuePaused) -> Result<(), FrameworkError> {
        if event.connection == "durable"
            && let Some(token) = STOP_ON_PAUSE.lock().unwrap().as_ref()
        {
            token.cancel();
        }
        Ok(())
    }
}

#[tokio::test]
#[serial]
async fn a_pause_on_one_connection_stops_its_worker_and_no_other() {
    RAN.lock().unwrap().clear();
    register_job::<PlainJob>();
    register_job::<DurableJob>();
    cache_init();
    let connections = TwoConnections::install();
    Queue::resume_all().await.unwrap();
    Queue::pause("durable", "conn_paused").await.unwrap();

    let on_queue = || EnvelopeOverrides {
        queue: Some("conn_paused".into()),
        ..Default::default()
    };
    Queue::push_with(DurableJob, on_queue()).await.unwrap();
    Queue::push_with(PlainJob, on_queue()).await.unwrap();
    let cfg = || WorkerConfig {
        queues: vec!["conn_paused".into()],
        ..worker_for(1)
    };

    // The same queue name on the default connection is not paused.
    run_worker_on(&Queue::connection_name(), cfg(), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(*RAN.lock().unwrap(), ["connections::PlainJob"]);

    // The worker on `durable` finds its queue paused and claims nothing.
    let stop = CancellationToken::new();
    *STOP_ON_PAUSE.lock().unwrap() = Some(stop.clone());
    EventFacade::listen::<WorkerQueuePaused, _>(Arc::new(StopOnPause)).await;
    run_worker_on("durable", cfg(), stop).await.unwrap();
    *STOP_ON_PAUSE.lock().unwrap() = None;

    assert_eq!(*RAN.lock().unwrap(), ["connections::PlainJob"]);
    assert_eq!(connections.sizes().await, (0, 1));
    Queue::resume("durable", "conn_paused").await.unwrap();
}

// --- The fake refuses what production refuses -------------------------------

#[tokio::test]
#[serial]
async fn the_fake_refuses_a_chain_over_two_connections_too() {
    let _connections = TwoConnections::install();
    let _fake = Queue::fake();

    let error = Queue::chain()
        .add(DurableJob)
        .unwrap()
        .add(PlainJob)
        .unwrap()
        .dispatch()
        .await
        .expect_err("a test must not pass for a chain production refuses");

    assert!(error.to_string().contains("A chain runs on one connection"));
    suprnova::queue::testing::assert_nothing_chained();
}
