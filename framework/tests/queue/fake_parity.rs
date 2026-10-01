//! The queue fake's `except`, raw pushes and `assert_pushed_without_chain`,
//! matching Laravel's `QueueFake::except`, `pushRaw` / `pushedRaw` /
//! `rawPushes` and `assertPushedWithoutChain`.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use suprnova::App;
use suprnova::cache::{CacheStore, InMemoryCache};
use suprnova::queue::testing::{
    assert_batched, assert_chained, assert_nothing_chained, assert_pushed_without_chain,
    forget_connections, pushed, pushed_raw, raw_pushes,
};
use suprnova::queue::worker::{WorkerConfig, register_job, run_worker};
use suprnova::queue::{
    CURRENT_SCHEMA_VERSION, FailedJobStore, MemoryFailedJobStore, MemoryQueueDriver, QueueDriver,
    SyncQueueDriver,
};
use suprnova::{ChainLink, EnvelopeOverrides, FrameworkError, Job, Queue, async_trait};
use tokio_util::sync::CancellationToken;

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Greet {
    name: String,
}

#[async_trait]
impl Job for Greet {
    fn job_name() -> &'static str {
        "Greet"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Farewell {
    name: String,
}

#[async_trait]
impl Job for Farewell {
    fn job_name() -> &'static str {
        "Farewell"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Note {
    text: String,
}

#[async_trait]
impl Job for Note {
    fn job_name() -> &'static str {
        "Note"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Ticket {
    id: u32,
}

#[async_trait]
impl Job for Ticket {
    fn job_name() -> &'static str {
        "Ticket"
    }
    fn unique_id(&self) -> Option<String> {
        Some(self.id.to_string())
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

static COUNTED_RUNS: AtomicUsize = AtomicUsize::new(0);

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Counted;

#[async_trait]
impl Job for Counted {
    fn job_name() -> &'static str {
        "FakeParityCounted"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        COUNTED_RUNS.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// Install a real driver and hand it back, so a test can see what reached
/// the real queue and what did not.
fn real_driver() -> Arc<MemoryQueueDriver> {
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    driver
}

/// The job names of everything on `driver`, in the order the driver hands
/// them out. Each envelope is acknowledged, so the driver ends up empty.
async fn drain(driver: &MemoryQueueDriver) -> Vec<String> {
    let mut names = Vec::new();
    while let Some(reserved) = driver.pop(Duration::from_millis(50)).await.unwrap() {
        names.push(reserved.envelope.job_name.clone());
        driver.ack(&reserved.token).await.unwrap();
    }
    names
}

/// The wire form of an envelope for `job`, which is what a raw push takes.
fn raw_payload<J: Job>(job: J) -> String {
    ChainLink::from_job(job)
        .unwrap()
        .to_envelope()
        .to_json()
        .unwrap()
}

// ---- except --------------------------------------------------------------

#[tokio::test]
#[serial]
async fn fake_except_sends_the_named_job_to_the_real_queue_and_records_the_rest() {
    let driver = real_driver();
    let _guard = Queue::fake_except(&["Farewell"]);

    Queue::push(Greet { name: "Ada".into() }).await.unwrap();
    Queue::push(Farewell {
        name: "Grace".into(),
    })
    .await
    .unwrap();

    assert_eq!(
        drain(&driver).await,
        ["Farewell"],
        "the excepted job reaches the real queue, and only it does"
    );
    let greeted: Vec<String> = pushed::<Greet>().into_iter().map(|job| job.name).collect();
    assert_eq!(greeted, ["Ada"], "a job except does not name is recorded");
    assert!(
        pushed::<Farewell>().is_empty(),
        "the excepted job is dispatched, not recorded"
    );
}

#[tokio::test]
#[serial]
async fn except_on_the_guard_adds_to_the_jobs_already_excepted() {
    let driver = real_driver();
    let _guard = Queue::fake().except(&["Greet"]).except(&["Farewell"]);

    Queue::push(Greet { name: "Ada".into() }).await.unwrap();
    Queue::push(Farewell {
        name: "Grace".into(),
    })
    .await
    .unwrap();
    Queue::push(Note {
        text: "kept".into(),
    })
    .await
    .unwrap();

    let mut reached = drain(&driver).await;
    reached.sort();
    assert_eq!(reached, ["Farewell", "Greet"]);
    assert!(pushed::<Greet>().is_empty());
    assert!(pushed::<Farewell>().is_empty());
    let notes: Vec<String> = pushed::<Note>().into_iter().map(|job| job.text).collect();
    assert_eq!(
        notes,
        ["kept"],
        "a job neither call names is still recorded"
    );
}

#[tokio::test]
#[serial]
async fn except_holds_on_every_push_entry_point() {
    // `push_unique` takes its dedupe lock in the cache on the real path.
    App::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));
    let driver = real_driver();
    let _guard = Queue::fake_except(&["Farewell", "Ticket"]);

    Queue::push_later(Farewell { name: "a".into() }, Utc::now())
        .await
        .unwrap();
    Queue::later(Duration::ZERO, Greet { name: "a".into() })
        .await
        .unwrap();
    Queue::push_with(
        Farewell { name: "b".into() },
        EnvelopeOverrides {
            queue: Some("vip".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    Queue::push_with(Greet { name: "b".into() }, EnvelopeOverrides::default())
        .await
        .unwrap();
    Queue::bulk(vec![
        Farewell { name: "c".into() },
        Farewell { name: "d".into() },
    ])
    .await
    .unwrap();
    Queue::bulk(vec![Greet { name: "c".into() }]).await.unwrap();
    assert!(Queue::push_unique(Ticket { id: 7 }).await.unwrap());

    let mut reached = drain(&driver).await;
    reached.sort();
    assert_eq!(
        reached,
        ["Farewell", "Farewell", "Farewell", "Farewell", "Ticket"],
        "every entry point sends an excepted job to the real queue"
    );
    let mut greeted: Vec<String> = pushed::<Greet>().into_iter().map(|job| job.name).collect();
    greeted.sort();
    assert_eq!(greeted, ["a", "b", "c"], "and records every other job");
    assert!(pushed::<Farewell>().is_empty());
    assert!(pushed::<Ticket>().is_empty());
}

#[tokio::test]
#[serial]
async fn an_excepted_job_fails_where_the_real_queue_fails() {
    let _driver = real_driver();
    Queue::register_connection("reports", Arc::new(MemoryQueueDriver::new()));
    let _guard = Queue::fake_except(&["Farewell"]);
    let nowhere = EnvelopeOverrides {
        connection: Some("nowhere".into()),
        ..Default::default()
    };

    // A recorded job never resolves its connection.
    let recorded = Queue::push_with(Greet { name: "Ada".into() }, nowhere.clone()).await;
    // The excepted one takes the real path, which refuses a connection
    // nobody registered.
    let refused = Queue::push_with(
        Farewell {
            name: "Grace".into(),
        },
        nowhere,
    )
    .await;
    forget_connections();

    recorded.expect("a recorded push resolves no connection");
    let err = refused.expect_err("the real path refuses an unregistered connection");
    assert!(
        err.to_string().contains("nowhere"),
        "the error names the connection: {err}"
    );
    assert!(pushed::<Farewell>().is_empty());
}

#[tokio::test]
#[serial]
async fn a_batch_under_except_dispatches_the_excepted_jobs_and_records_the_rest() {
    let driver = real_driver();
    let _guard = Queue::fake_except(&["Farewell"]);

    let id = Queue::batch()
        .name("mixed")
        .add(Greet { name: "Ada".into() })
        .add(Farewell {
            name: "Grace".into(),
        })
        .add(Greet { name: "Lin".into() })
        .dispatch()
        .await
        .unwrap();

    let reserved = driver
        .pop(Duration::from_secs(5))
        .await
        .unwrap()
        .expect("the excepted batch job is on the real queue");
    assert_eq!(reserved.envelope.job_name, "Farewell");
    assert_eq!(reserved.envelope.batch_id.as_deref(), Some(id.as_str()));
    assert_eq!(driver.size().await.unwrap(), 1, "only that one job");

    assert_batched(|batch| batch.id == id && batch.jobs.len() == 3);
    let greeted: Vec<String> = pushed::<Greet>().into_iter().map(|job| job.name).collect();
    assert_eq!(greeted, ["Ada", "Lin"]);
    assert!(pushed::<Farewell>().is_empty());
}

#[tokio::test]
#[serial]
async fn a_chain_under_except_sends_each_link_where_except_sends_it() {
    register_job::<Farewell>();
    register_job::<Counted>();
    let driver = real_driver();
    let _guard = Queue::fake_except(&["Farewell", "FakeParityCounted"]);
    let before = COUNTED_RUNS.load(Ordering::SeqCst);

    // The head is excepted, so the chain goes to the real queue.
    Queue::chain()
        .add(Farewell {
            name: "Grace".into(),
        })
        .unwrap()
        .add(Counted)
        .unwrap()
        .add(Greet { name: "Ada".into() })
        .unwrap()
        .dispatch()
        .await
        .unwrap();
    assert_nothing_chained();
    assert!(pushed::<Farewell>().is_empty());

    // The worker dispatches each later link through the fake, as Laravel
    // dispatches the next job of a chain through the queue facade: the
    // excepted link runs, and the one `except` does not name is recorded.
    let cfg = WorkerConfig {
        visibility_timeout: Duration::from_secs(5),
        poll_interval: Duration::from_millis(5),
        max_jobs: Some(2),
        queues: Vec::new(),
    };
    tokio::time::timeout(
        Duration::from_secs(10),
        run_worker(driver.clone(), cfg, CancellationToken::new()),
    )
    .await
    .expect("the worker runs the head and the excepted link");
    assert_eq!(
        COUNTED_RUNS.load(Ordering::SeqCst),
        before + 1,
        "the excepted later link runs on the real queue"
    );
    let greeted: Vec<String> = pushed::<Greet>().into_iter().map(|job| job.name).collect();
    assert_eq!(
        greeted,
        ["Ada"],
        "the later link except does not name is recorded"
    );
    assert_eq!(
        driver.size().await.unwrap(),
        0,
        "and it does not reach the real queue"
    );

    // The head is faked, so the chain is recorded.
    Queue::chain()
        .add(Greet { name: "Lin".into() })
        .unwrap()
        .add(Farewell { name: "Mae".into() })
        .unwrap()
        .dispatch()
        .await
        .unwrap();
    assert_chained(&["Greet", "Farewell"]);
    assert_eq!(driver.size().await.unwrap(), 0, "nothing more reached it");
}

#[tokio::test]
#[serial]
async fn a_retry_under_except_dispatches_the_excepted_job_and_records_the_rest() {
    let source = real_driver();
    Queue::push(Greet { name: "Ada".into() }).await.unwrap();
    Queue::push(Farewell {
        name: "Grace".into(),
    })
    .await
    .unwrap();
    let store = Arc::new(MemoryFailedJobStore::new());
    Queue::set_failed_store(store.clone());
    for _ in 0..2 {
        let reserved = source
            .pop(Duration::from_secs(5))
            .await
            .unwrap()
            .expect("the pushed job is reservable");
        store
            .log("memory", "default", &reserved.envelope, "boom")
            .await
            .unwrap();
    }

    let driver = real_driver();
    let _guard = Queue::fake_except(&["Farewell"]);
    assert_eq!(Queue::retry_all_failed(None).await.unwrap(), 2);

    assert_eq!(drain(&driver).await, ["Farewell"]);
    let greeted: Vec<String> = pushed::<Greet>().into_iter().map(|job| job.name).collect();
    assert_eq!(greeted, ["Ada"]);
    assert!(pushed::<Farewell>().is_empty());
    assert_eq!(store.count().await.unwrap(), 0);
}

// ---- raw pushes ----------------------------------------------------------

#[tokio::test]
#[serial]
async fn a_raw_push_is_recorded_and_read_back() {
    let driver = real_driver();
    let _guard = Queue::fake();
    let payload = raw_payload(Greet { name: "Ada".into() });

    Queue::push_raw(&payload, Some("imports")).await.unwrap();
    Queue::push_raw(&payload, None).await.unwrap();

    assert_eq!(driver.size().await.unwrap(), 0, "the fake writes no driver");
    let raws = raw_pushes();
    assert_eq!(raws.len(), 2, "every raw push is recorded, in order");
    assert_eq!(raws[0].payload, payload, "the payload is kept verbatim");
    assert_eq!(raws[0].queue.as_deref(), Some("imports"));
    assert_eq!(raws[1].queue, None);
    let env = raws[0].envelope().expect("the payload decodes");
    assert_eq!(env.job_name, "Greet");
    assert_eq!(env.payload["name"], "Ada");
    assert!(
        pushed::<Greet>().is_empty(),
        "a raw push is not a typed push"
    );
}

#[tokio::test]
async fn pushed_raw_returns_the_raw_pushes_a_predicate_accepts() {
    let _guard = Queue::fake();
    Queue::push_raw(&raw_payload(Greet { name: "Ada".into() }), Some("imports"))
        .await
        .unwrap();
    Queue::push_raw(
        &raw_payload(Farewell {
            name: "Grace".into(),
        }),
        Some("exports"),
    )
    .await
    .unwrap();
    Queue::push_raw(&raw_payload(Greet { name: "Lin".into() }), Some("exports"))
        .await
        .unwrap();

    assert_eq!(
        pushed_raw(|raw| raw.queue.as_deref() == Some("exports")).len(),
        2
    );
    let farewells = pushed_raw(|raw| raw.envelope().is_ok_and(|env| env.job_name == "Farewell"));
    assert_eq!(farewells.len(), 1);
    assert_eq!(farewells[0].queue.as_deref(), Some("exports"));
    assert!(pushed_raw(|raw| raw.queue.is_none()).is_empty());
    assert_eq!(pushed_raw(|_| true).len(), raw_pushes().len());
}

#[tokio::test]
async fn raw_pushes_is_empty_when_nothing_was_pushed_raw() {
    let _guard = Queue::fake();
    Queue::push(Greet { name: "Ada".into() }).await.unwrap();
    assert!(raw_pushes().is_empty());
    assert!(pushed_raw(|_| true).is_empty());
}

#[tokio::test]
#[serial]
async fn a_raw_push_is_recorded_even_for_a_job_except_names() {
    let driver = real_driver();
    let _guard = Queue::fake_except(&["Greet"]);

    Queue::push_raw(&raw_payload(Greet { name: "Ada".into() }), None)
        .await
        .unwrap();

    assert_eq!(
        raw_pushes().len(),
        1,
        "a raw push is a payload, not a job type"
    );
    assert_eq!(driver.size().await.unwrap(), 0);
}

#[tokio::test]
#[serial]
async fn a_raw_push_reaches_the_driver_without_the_fake() {
    let driver = real_driver();
    let env = ChainLink::from_job(Greet { name: "Ada".into() })
        .unwrap()
        .to_envelope();

    Queue::push_raw(&env.to_json().unwrap(), Some("imports"))
        .await
        .unwrap();

    let reserved = driver
        .pop(Duration::from_secs(5))
        .await
        .unwrap()
        .expect("the raw payload is on the queue");
    assert_eq!(
        reserved.envelope.id, env.id,
        "the envelope is pushed as given"
    );
    assert_eq!(reserved.envelope.job_name, "Greet");
    assert_eq!(reserved.envelope.payload, env.payload);
    assert_eq!(
        reserved.envelope.queue.as_deref(),
        Some("imports"),
        "the queue argument names the queue"
    );
}

#[tokio::test]
#[serial]
async fn a_raw_push_without_a_queue_keeps_the_envelope_queue() {
    let driver = real_driver();
    let mut env = ChainLink::from_job(Greet { name: "Ada".into() })
        .unwrap()
        .to_envelope();
    env.queue = Some("billing".into());

    Queue::push_raw(&env.to_json().unwrap(), None)
        .await
        .unwrap();

    let reserved = driver
        .pop(Duration::from_secs(5))
        .await
        .unwrap()
        .expect("the raw payload is on the queue");
    assert_eq!(reserved.envelope.queue.as_deref(), Some("billing"));
}

#[tokio::test]
#[serial]
async fn a_raw_push_runs_like_any_envelope() {
    register_job::<Counted>();
    Queue::set_driver(Arc::new(SyncQueueDriver::new()));
    let before = COUNTED_RUNS.load(Ordering::SeqCst);

    Queue::push_raw(&raw_payload(Counted), None).await.unwrap();

    assert_eq!(
        COUNTED_RUNS.load(Ordering::SeqCst),
        before + 1,
        "the handler registered for the payload's job name ran"
    );
}

#[tokio::test]
async fn a_raw_push_that_is_not_an_envelope_is_refused() {
    let _guard = Queue::fake();

    assert!(
        Queue::push_raw(r#"{"job":"SendInvoice","data":{}}"#, None)
            .await
            .is_err(),
        "a payload that is not an envelope is refused"
    );
    assert!(Queue::push_raw("not json", None).await.is_err());
    let mut newer = ChainLink::from_job(Greet { name: "Ada".into() })
        .unwrap()
        .to_envelope();
    newer.schema_version = CURRENT_SCHEMA_VERSION + 1;
    assert!(
        Queue::push_raw(&newer.to_json().unwrap(), None)
            .await
            .is_err(),
        "an envelope from a newer schema is refused"
    );
    assert!(raw_pushes().is_empty(), "a refused payload is not recorded");
}

// ---- assert_pushed_without_chain -----------------------------------------

#[tokio::test]
async fn assert_pushed_without_chain_passes_for_a_job_pushed_alone() {
    let _guard = Queue::fake();
    Queue::push(Greet { name: "Ada".into() }).await.unwrap();
    assert_pushed_without_chain::<Greet>();
}

#[tokio::test]
async fn assert_pushed_without_chain_passes_for_a_chain_of_one_job() {
    let _guard = Queue::fake();
    Queue::chain()
        .add(Greet { name: "Ada".into() })
        .unwrap()
        .dispatch()
        .await
        .unwrap();
    assert_pushed_without_chain::<Greet>();
}

#[tokio::test]
async fn assert_pushed_without_chain_passes_when_one_push_has_no_chain() {
    let _guard = Queue::fake();
    Queue::chain()
        .add(Greet { name: "Ada".into() })
        .unwrap()
        .add(Farewell {
            name: "Grace".into(),
        })
        .unwrap()
        .dispatch()
        .await
        .unwrap();
    Queue::push(Greet { name: "Lin".into() }).await.unwrap();
    assert_pushed_without_chain::<Greet>();
}

#[tokio::test]
#[should_panic(expected = "without a chain")]
async fn assert_pushed_without_chain_fails_for_a_job_pushed_with_a_chain() {
    let _guard = Queue::fake();
    Queue::chain()
        .add(Greet { name: "Ada".into() })
        .unwrap()
        .add(Farewell {
            name: "Grace".into(),
        })
        .unwrap()
        .dispatch()
        .await
        .unwrap();
    assert_pushed_without_chain::<Greet>();
}

#[tokio::test]
#[should_panic(expected = "expected at least one pushed Greet")]
async fn assert_pushed_without_chain_fails_when_the_job_was_not_pushed() {
    let _guard = Queue::fake();
    Queue::push(Farewell {
        name: "Grace".into(),
    })
    .await
    .unwrap();
    assert_pushed_without_chain::<Greet>();
}
