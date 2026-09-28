//! The pusher's `Context` travels with a queued job: the push snapshots it
//! into the envelope and the worker runs the job inside a scope restored
//! from that snapshot.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use suprnova::error::FrameworkError;
use suprnova::events::{EventFacade, Listener};
use suprnova::queue::events::JobProcessed;
use suprnova::queue::{
    Envelope, Job, MemoryQueueDriver, Queue, QueueDriver, SyncQueueDriver,
    worker::{WorkerConfig, register_job, run_worker},
};
use suprnova::{Context, ContextStore};
use tokio_util::sync::CancellationToken;

/// What one run of [`ReadsContext`] found.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Seen {
    label: u32,
    /// The visible `trace_id`.
    trace_id: Option<String>,
    /// The hidden `api_key`.
    api_key: Option<String>,
}

impl Seen {
    /// What a job sees when the pusher's context reached it.
    fn carried(label: u32) -> Self {
        Self {
            label,
            trace_id: Some("abc".to_owned()),
            api_key: Some("s3cret".to_owned()),
        }
    }

    /// What a job sees when no context reached it.
    fn nothing(label: u32) -> Self {
        Self {
            label,
            trace_id: None,
            api_key: None,
        }
    }
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

#[derive(Serialize, Deserialize, Clone)]
struct ReadsContext {
    label: u32,
}

#[async_trait]
impl Job for ReadsContext {
    fn job_name() -> &'static str {
        "queue_context::ReadsContext"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        SEEN.lock().unwrap().push(Seen {
            label: self.label,
            trace_id: Context::get::<String>("trace_id"),
            api_key: Context::hidden_get::<String>("api_key"),
        });
        // A job's own additions stay in the job's copy.
        Context::add("added_by_the_job", self.label);
        Ok(())
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

/// Run `body` the way a request handler runs: inside a context scope that
/// holds a visible trace id and a hidden key.
async fn in_a_request<F: std::future::Future>(body: F) -> F::Output {
    Context::scope(ContextStore::default(), async {
        Context::add("trace_id", "abc");
        Context::hidden_add("api_key", "s3cret");
        body.await
    })
    .await
}

#[tokio::test]
#[serial]
async fn a_job_reads_the_context_of_the_code_that_pushed_it() {
    SEEN.lock().unwrap().clear();
    register_job::<ReadsContext>();
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    in_a_request(async { Queue::push(ReadsContext { label: 1 }).await.unwrap() }).await;

    // The worker runs outside every scope, as a worker process does.
    run_worker(driver, worker_for(1), CancellationToken::new()).await;

    assert_eq!(*SEEN.lock().unwrap(), [Seen::carried(1)]);
}

#[tokio::test]
#[serial]
async fn a_push_with_no_context_writes_the_envelope_it_always_wrote() {
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    Queue::push(ReadsContext { label: 1 }).await.unwrap();

    let envelope = driver
        .pop(Duration::from_secs(5))
        .await
        .unwrap()
        .expect("the pushed job")
        .envelope;
    assert_eq!(envelope.context, None);
    let wire = envelope.to_json().unwrap();
    assert!(
        !wire.contains("\"context\""),
        "an absent context must stay off the wire: {wire}"
    );
}

#[tokio::test]
#[serial]
async fn an_envelope_written_before_context_travelled_still_decodes_and_runs() {
    SEEN.lock().unwrap().clear();
    register_job::<ReadsContext>();
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    Queue::push(ReadsContext { label: 7 }).await.unwrap();
    let written = driver
        .pop(Duration::from_secs(5))
        .await
        .unwrap()
        .expect("the pushed job")
        .envelope
        .to_json()
        .unwrap();

    let old = Envelope::from_json(&written).expect("no context key is still an envelope");
    assert_eq!(old.context, None);

    let fresh = Arc::new(MemoryQueueDriver::new());
    fresh.push(old).await.unwrap();
    run_worker(fresh, worker_for(1), CancellationToken::new()).await;

    assert_eq!(*SEEN.lock().unwrap(), [Seen::nothing(7)]);
}

#[tokio::test]
#[serial]
async fn a_job_run_inline_works_on_a_copy_of_the_callers_context() {
    SEEN.lock().unwrap().clear();
    register_job::<ReadsContext>();
    Queue::set_driver(Arc::new(SyncQueueDriver::new()));

    in_a_request(async {
        Queue::push(ReadsContext { label: 3 }).await.unwrap();

        assert!(
            !Context::has("added_by_the_job"),
            "a job run inline must not write the caller's context"
        );
        assert_eq!(Context::get::<String>("trace_id").as_deref(), Some("abc"));
    })
    .await;

    assert_eq!(*SEEN.lock().unwrap(), [Seen::carried(3)]);
}

#[tokio::test]
#[serial]
async fn every_link_of_a_chain_gets_the_context_of_the_dispatch() {
    SEEN.lock().unwrap().clear();
    register_job::<ReadsContext>();
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    in_a_request(async {
        Queue::chain()
            .add(ReadsContext { label: 1 })
            .unwrap()
            .add(ReadsContext { label: 2 })
            .unwrap()
            .add(ReadsContext { label: 3 })
            .unwrap()
            .dispatch()
            .await
            .unwrap();
    })
    .await;

    run_worker(driver, worker_for(3), CancellationToken::new()).await;

    let expected: Vec<Seen> = (1..=3).map(Seen::carried).collect();
    assert_eq!(*SEEN.lock().unwrap(), expected);
}

#[tokio::test]
#[serial]
async fn every_job_of_a_batch_gets_the_context_of_the_code_that_built_it() {
    SEEN.lock().unwrap().clear();
    register_job::<ReadsContext>();
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    in_a_request(async {
        Queue::batch()
            .name("context")
            .add(ReadsContext { label: 1 })
            .add(ReadsContext { label: 2 })
            .dispatch()
            .await
            .unwrap();
    })
    .await;

    run_worker(driver, worker_for(2), CancellationToken::new()).await;

    let mut seen = SEEN.lock().unwrap().clone();
    seen.sort();
    let expected: Vec<Seen> = (1..=2).map(Seen::carried).collect();
    assert_eq!(seen, expected);
}

/// What the `JobProcessed` listener found in the context after one job.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AfterTheJob {
    job_name: String,
    /// The visible `trace_id` the pusher added.
    trace_id: Option<String>,
    /// What the job itself added while it ran.
    added_by_the_job: Option<u32>,
}

static LISTENER_SAW: Mutex<Vec<AfterTheJob>> = Mutex::new(Vec::new());

struct ReadsContextAfterTheJob;

#[async_trait]
impl Listener<JobProcessed> for ReadsContextAfterTheJob {
    async fn handle(&self, event: &JobProcessed) -> Result<(), FrameworkError> {
        LISTENER_SAW.lock().unwrap().push(AfterTheJob {
            job_name: event.job.job_name.clone(),
            trace_id: Context::get::<String>("trace_id"),
            added_by_the_job: Context::get::<u32>("added_by_the_job"),
        });
        Ok(())
    }
}

#[tokio::test]
#[serial]
async fn the_events_around_a_job_run_in_the_jobs_context() {
    SEEN.lock().unwrap().clear();
    LISTENER_SAW.lock().unwrap().clear();
    register_job::<ReadsContext>();
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    EventFacade::listen::<JobProcessed, _>(Arc::new(ReadsContextAfterTheJob)).await;

    in_a_request(async { Queue::push(ReadsContext { label: 9 }).await.unwrap() }).await;
    run_worker(driver, worker_for(1), CancellationToken::new()).await;

    let ours: Vec<AfterTheJob> = LISTENER_SAW
        .lock()
        .unwrap()
        .iter()
        .filter(|saw| saw.job_name == "queue_context::ReadsContext")
        .cloned()
        .collect();
    assert_eq!(
        ours,
        [AfterTheJob {
            job_name: "queue_context::ReadsContext".to_owned(),
            trace_id: Some("abc".to_owned()),
            added_by_the_job: Some(9),
        }],
        "a listener reads what the job read and what the job added"
    );
}

#[tokio::test]
#[serial]
async fn the_hooks_run_on_a_push_and_on_the_worker() {
    SEEN.lock().unwrap().clear();
    register_job::<ReadsContext>();
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    Context::test_clear_hooks();
    // Both hooks are process-wide. They act only on a snapshot that carries
    // this test's marker, so a test that runs beside this one is untouched.
    Context::dehydrating(|snapshot| {
        if snapshot.data.contains_key("hook_test") {
            snapshot
                .data
                .insert("trace_id".into(), serde_json::json!("set by the hook"));
        }
    });
    Context::hydrated(|snapshot| {
        if snapshot.data.contains_key("hook_test") {
            Context::hidden_add("api_key", "restored by the hook");
        }
    });

    Context::scope(ContextStore::default(), async {
        Context::add("hook_test", true);
        Queue::push(ReadsContext { label: 5 }).await.unwrap();
    })
    .await;
    run_worker(driver, worker_for(1), CancellationToken::new()).await;
    Context::test_clear_hooks();

    assert_eq!(
        *SEEN.lock().unwrap(),
        [Seen {
            label: 5,
            trace_id: Some("set by the hook".to_owned()),
            api_key: Some("restored by the hook".to_owned()),
        }]
    );
}
