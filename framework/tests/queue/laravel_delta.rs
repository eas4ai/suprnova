use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use suprnova::queue::worker::{WorkerConfig, register_job};
use suprnova::{
    Application, BackoffSchedule, Envelope, EnvelopeOverrides, FrameworkError, Job,
    MemoryQueueDriver, Queue, QueueDriver, Reservation, ReservationToken, WorkerControls,
    assert_not_pushed, assert_pushed_on_queue, async_trait, run_worker_with_controls,
};
use tokio_util::sync::CancellationToken;

static RUNS: AtomicU32 = AtomicU32::new(0);

#[derive(Serialize, Deserialize)]
struct Work {
    id: u32,
    fail: bool,
    hang: bool,
}

#[async_trait]
impl Job for Work {
    fn job_name() -> &'static str {
        "delta-worker"
    }
    fn backoff() -> BackoffSchedule {
        BackoffSchedule::Fixed { secs: 0 }
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        RUNS.fetch_add(1, Ordering::SeqCst);
        if self.hang {
            std::future::pending::<()>().await;
        }
        if self.fail {
            Err(FrameworkError::internal("job failed"))
        } else {
            Ok(())
        }
    }
}

fn work(id: u32) -> Work {
    Work {
        id,
        fail: false,
        hang: false,
    }
}

async fn setup(jobs: u32) -> Arc<MemoryQueueDriver> {
    RUNS.store(0, Ordering::SeqCst);
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    register_job::<Work>();
    for id in 0..jobs {
        Queue::push(work(id)).await.expect("push");
    }
    driver
}

async fn run(driver: Arc<dyn QueueDriver>, controls: WorkerControls) -> i32 {
    tokio::time::timeout(
        Duration::from_secs(5),
        run_worker_with_controls(
            driver,
            WorkerConfig::default(),
            controls,
            CancellationToken::new(),
        ),
    )
    .await
    .expect("worker exits")
    .expect("worker succeeds")
}

#[tokio::test]
#[serial]
async fn once_settles_one_job_and_leaves_the_second() {
    let driver = setup(2).await;
    assert_eq!(
        run(
            driver.clone(),
            WorkerControls {
                once: true,
                ..Default::default()
            }
        )
        .await,
        0
    );
    assert_eq!(RUNS.load(Ordering::SeqCst), 1);
    assert_eq!(driver.size(None).await.expect("size"), 1);
}

#[tokio::test]
#[serial]
async fn empty_worker_stops_immediately_with_either_control() {
    for controls in [
        WorkerControls {
            once: true,
            ..Default::default()
        },
        WorkerControls {
            stop_when_empty: true,
            ..Default::default()
        },
    ] {
        assert_eq!(run(setup(0).await, controls).await, 0);
        assert_eq!(RUNS.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
#[serial]
async fn stop_when_empty_drains_every_available_job() {
    let driver = setup(2).await;
    assert_eq!(
        run(
            driver.clone(),
            WorkerControls {
                stop_when_empty: true,
                ..Default::default()
            }
        )
        .await,
        0
    );
    assert_eq!(RUNS.load(Ordering::SeqCst), 2);
    assert_eq!(driver.size(None).await.expect("size"), 0);
}

#[tokio::test]
#[serial]
async fn resident_memory_limit_returns_twelve_after_settlement() {
    let driver = setup(2).await;
    assert_eq!(
        run(
            driver.clone(),
            WorkerControls {
                memory: Some(1),
                once: true,
                ..Default::default()
            }
        )
        .await,
        12
    );
    assert_eq!(RUNS.load(Ordering::SeqCst), 1);
    assert_eq!(driver.size(None).await.expect("size"), 1);
}

#[tokio::test]
#[serial]
async fn zero_memory_limit_disables_the_limit() {
    assert_eq!(
        run(
            setup(2).await,
            WorkerControls {
                memory: Some(0),
                stop_when_empty: true,
                ..Default::default()
            }
        )
        .await,
        0
    );
    assert_eq!(RUNS.load(Ordering::SeqCst), 2);
}

struct CountingEmpty {
    polls: std::sync::Mutex<Vec<tokio::time::Instant>>,
    fail: bool,
}

#[async_trait]
impl QueueDriver for CountingEmpty {
    async fn push(&self, _: Envelope) -> Result<(), FrameworkError> {
        Ok(())
    }
    async fn pop(&self, _: Duration) -> Result<Option<Reservation>, FrameworkError> {
        self.polls
            .lock()
            .expect("polls")
            .push(tokio::time::Instant::now());
        if self.fail {
            Err(FrameworkError::internal("queue unreachable"))
        } else {
            Ok(None)
        }
    }
    async fn ack(&self, _: &ReservationToken) -> Result<(), FrameworkError> {
        Ok(())
    }
    async fn nack(&self, _: &ReservationToken, _: Duration) -> Result<(), FrameworkError> {
        Ok(())
    }
    fn name(&self) -> &'static str {
        "delta-counting-empty"
    }
}

#[tokio::test(start_paused = true)]
#[serial]
async fn sleep_three_seconds_spaces_empty_polls_and_cancels_promptly() {
    let driver = Arc::new(CountingEmpty {
        polls: Default::default(),
        fail: false,
    });
    let cancel = CancellationToken::new();
    let worker = tokio::spawn(run_worker_with_controls(
        driver.clone(),
        WorkerConfig::default(),
        WorkerControls {
            sleep: Some(3),
            ..Default::default()
        },
        cancel.clone(),
    ));
    tokio::task::yield_now().await;
    assert_eq!(driver.polls.lock().expect("polls").len(), 1);
    tokio::time::advance(Duration::from_millis(2999)).await;
    tokio::task::yield_now().await;
    assert_eq!(driver.polls.lock().expect("polls").len(), 1);
    tokio::time::advance(Duration::from_millis(1)).await;
    tokio::task::yield_now().await;
    let polls = driver.polls.lock().expect("polls").clone();
    assert_eq!(polls.len(), 2);
    assert_eq!(polls[1] - polls[0], Duration::from_secs(3));
    cancel.cancel();
    assert_eq!(worker.await.expect("join").expect("worker"), 0);
}

#[tokio::test]
#[serial]
async fn once_reports_a_failed_pop_instead_of_hanging_or_claiming_empty() {
    let driver = Arc::new(CountingEmpty {
        polls: Default::default(),
        fail: true,
    });
    let result = run_worker_with_controls(
        driver,
        WorkerConfig::default(),
        WorkerControls {
            once: true,
            ..Default::default()
        },
        CancellationToken::new(),
    )
    .await;
    assert!(
        result
            .expect_err("pop failure")
            .to_string()
            .contains("queue unreachable")
    );
}

#[tokio::test]
#[serial]
async fn tries_caps_failures_and_timeout_settles_a_stalled_job() {
    let driver = setup(0).await;
    Queue::push(Work {
        id: 1,
        fail: true,
        hang: false,
    })
    .await
    .expect("push");
    assert_eq!(
        run(
            driver.clone(),
            WorkerControls {
                tries: Some(2),
                stop_when_empty: true,
                ..Default::default()
            }
        )
        .await,
        0
    );
    assert_eq!(RUNS.load(Ordering::SeqCst), 2);
    assert_eq!(driver.size(None).await.expect("failed job removed"), 0);
    Queue::push(Work {
        id: 2,
        fail: false,
        hang: true,
    })
    .await
    .expect("push");
    assert_eq!(
        run(
            driver.clone(),
            WorkerControls {
                tries: Some(1),
                timeout: Some(1),
                once: true,
                ..Default::default()
            }
        )
        .await,
        0
    );
    assert_eq!(driver.size(None).await.expect("size"), 0);
}

#[test]
#[serial]
fn cli_accepts_controls_alongside_existing_worker_options() {
    suprnova::boot::load_env().expect("pre-runtime configuration");
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(async {
            RUNS.store(0, Ordering::SeqCst);
            Application::new()
                .bootstrap(|| async {
                    register_job::<Work>();
                    Queue::try_register_connection("delta-cli", Arc::new(MemoryQueueDriver::new()))
                        .expect("connection");
                    for id in [1, 2] {
                        Queue::push_with(
                            work(id),
                            EnvelopeOverrides {
                                connection: Some("delta-cli".into()),
                                ..Default::default()
                            },
                        )
                        .await
                        .expect("push");
                    }
                })
                .run_with_args([
                    "app",
                    "queue:work",
                    "--once",
                    "--stop-when-empty",
                    "--sleep=3",
                    "--tries=2",
                    "--timeout=1",
                    "--memory=0",
                    "--poll=20",
                    "--max-jobs=9",
                    "--visibility-timeout=60",
                    "--queue=default",
                    "--connection=delta-cli",
                ])
                .await
                .expect("CLI controls");
            assert_eq!(RUNS.load(Ordering::SeqCst), 1);
        });
}

#[test]
#[serial]
fn cli_propagates_the_memory_exit_status() {
    suprnova::boot::load_env().expect("pre-runtime configuration");
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(async {
            let error = Application::new()
                .bootstrap(|| async {
                    register_job::<Work>();
                    Queue::try_register_connection(
                        "delta-cli-memory",
                        Arc::new(MemoryQueueDriver::new()),
                    )
                    .expect("connection");
                    Queue::push_with(
                        work(1),
                        EnvelopeOverrides {
                            connection: Some("delta-cli-memory".into()),
                            ..Default::default()
                        },
                    )
                    .await
                    .expect("push");
                })
                .run_with_args([
                    "app",
                    "queue:work",
                    "--memory=1",
                    "--once",
                    "--connection=delta-cli-memory",
                ])
                .await
                .expect_err("memory exit");
            assert!(error.to_string().contains("status 12"), "{error}");
        });
}

#[tokio::test]
async fn cli_rejects_negative_and_malformed_worker_controls() {
    for option in ["--sleep=-1", "--tries=bad", "--timeout=-1", "--memory=bad"] {
        assert!(
            Application::new()
                .run_with_args(["app", "queue:work", option])
                .await
                .is_err(),
            "{option}"
        );
    }
}

#[tokio::test]
#[serial]
async fn queue_fake_filters_absence_and_queue_against_the_same_job() {
    let _fake = Queue::fake();
    assert_not_pushed::<Work>(|job| job.id == 7);
    Queue::push_with(
        work(8),
        EnvelopeOverrides {
            queue: Some("emails".into()),
            ..Default::default()
        },
    )
    .await
    .expect("push eight");
    Queue::push_with(
        work(7),
        EnvelopeOverrides {
            queue: Some("other".into()),
            ..Default::default()
        },
    )
    .await
    .expect("push seven");
    assert_pushed_on_queue::<Work>("emails", |job| job.id == 8);
    assert_not_pushed::<Work>(|job| job.id == 9);
    assert!(std::panic::catch_unwind(|| assert_not_pushed::<Work>(|job| job.id == 7)).is_err());
    assert!(
        std::panic::catch_unwind(|| assert_pushed_on_queue::<Work>("emails", |job| job.id == 7))
            .is_err()
    );
    assert!(
        std::panic::catch_unwind(|| assert_pushed_on_queue::<Work>("missing", |_| true)).is_err()
    );
}

#[tokio::test]
#[serial]
async fn queue_fake_queue_filter_accepts_the_jobs_declared_queue() {
    #[derive(Serialize, Deserialize)]
    struct Email {
        id: u32,
    }
    #[async_trait]
    impl Job for Email {
        fn job_name() -> &'static str {
            "delta-email"
        }
        fn queue() -> Option<&'static str> {
            Some("emails")
        }
        async fn handle(self) -> Result<(), FrameworkError> {
            Ok(())
        }
    }
    let _fake = Queue::fake();
    Queue::push(Email { id: 7 }).await.expect("email");
    assert_pushed_on_queue::<Email>("emails", |job| job.id == 7);
}
