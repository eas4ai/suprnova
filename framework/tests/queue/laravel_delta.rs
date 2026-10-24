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
    // An unset APP_ENV is production, whose rate-limit and mail checks stop
    // the worker's boot; this process names itself a test one.
    let _env = crate::env_lock::lock_env();
    let _app_env = crate::env_snapshot::EnvSnapshot::capture(&["APP_ENV"]);
    crate::env_snapshot::set_env("APP_ENV", Some("testing"));
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
    // An unset APP_ENV is production, whose rate-limit and mail checks stop
    // the worker's boot; this process names itself a test one.
    let _env = crate::env_lock::lock_env();
    let _app_env = crate::env_snapshot::EnvSnapshot::capture(&["APP_ENV"]);
    crate::env_snapshot::set_env("APP_ENV", Some("testing"));
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

// PAR-017: a driver the application installs in its bootstrap is the
// default connection; the boot's `bootstrap_from_env` keeps it.

/// A free loopback port: bind `:0`, read the port back, release it. The
/// server binds it a moment later; the window is small and local.
async fn free_port() -> u16 {
    let probe = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a probe listener");
    probe.local_addr().expect("the probe's address").port()
}

/// Sets `pairs` for the life of the guard and restores them after. The
/// caller holds the env lock.
fn set_vars(pairs: &[(&'static str, Option<&str>)]) -> crate::env_snapshot::EnvSnapshot {
    let keys: Vec<&'static str> = pairs.iter().map(|(key, _)| *key).collect();
    let snapshot = crate::env_snapshot::EnvSnapshot::capture(&keys);
    for (key, value) in pairs {
        crate::env_snapshot::set_env(key, *value);
    }
    snapshot
}

/// The manual's failover pattern: the bootstrap installs a driver with
/// `Queue::set_driver`, and the server boots after it. The server's boot
/// used to replace that driver with the one `QUEUE_DRIVER` names.
///
/// Runs alone in a child process (see `own_process`): the server boot binds
/// the process-wide cache, queue, rate limiter and mail transport.
#[test]
fn a_driver_set_in_the_bootstrap_survives_the_server_boot() {
    crate::own_process::run_alone(
        "laravel_delta::a_driver_set_in_the_bootstrap_survives_the_server_boot_child",
    );
}

#[tokio::test]
#[serial]
async fn a_driver_set_in_the_bootstrap_survives_the_server_boot_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let _lock = crate::env_lock::lock_env_async().await;
    // The server needs APP_KEY outside development, and an unset APP_ENV is
    // production; the child names a test process. `sync` is what the
    // environment would install in place of the bootstrap's driver.
    let _vars = set_vars(&[
        ("APP_ENV", Some("testing")),
        ("QUEUE_DRIVER", Some("sync")),
        ("QUEUE_CONNECTIONS", None),
    ]);

    // The application's bootstrap.
    let installed: Arc<dyn QueueDriver> = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(Arc::clone(&installed));

    let port = free_port().await;
    let server = suprnova::Server::from_config(suprnova::Router::new())
        .expect("the server builds in a test process")
        .host("127.0.0.1")
        .port(port);
    tokio::spawn(async move {
        // The runtime ends with the test and aborts the server; nothing
        // needs a graceful shutdown here.
        let _ = server.run().await;
    });
    // The server boots its drivers before it binds, so a connection that is
    // accepted means the queue boot has run.
    let mut accepted = false;
    for _ in 0..250 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            accepted = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(accepted, "the server did not start accepting connections");

    let driver = Queue::driver().expect("a default driver");
    assert!(
        Arc::ptr_eq(&driver, &installed),
        "the server's boot replaced the bootstrap's driver with `{}`",
        driver.name()
    );
}

/// With the bootstrap's driver as the default connection, the boot still
/// registers the connections `QUEUE_CONNECTIONS` lists beside it, as it
/// did before; only `QUEUE_DRIVER` is not read.
#[test]
fn queue_connections_register_beside_the_bootstraps_driver() {
    crate::own_process::run_alone(
        "laravel_delta::queue_connections_register_beside_the_bootstraps_driver_child",
    );
}

#[tokio::test]
#[serial]
async fn queue_connections_register_beside_the_bootstraps_driver_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let _lock = crate::env_lock::lock_env_async().await;
    let _vars = set_vars(&[
        ("APP_ENV", Some("testing")),
        ("QUEUE_DRIVER", Some("sync")),
        ("QUEUE_CONNECTIONS", Some("null, sync")),
    ]);
    let installed: Arc<dyn QueueDriver> = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(Arc::clone(&installed));

    suprnova::queue::bootstrap_from_env()
        .await
        .expect("the boot keeps the driver and registers the connections");

    assert!(Arc::ptr_eq(
        &Queue::driver().expect("a default driver"),
        &installed
    ));
    assert_eq!(
        Queue::connection_names().expect("the connection names"),
        ["null", "sync"]
    );
    // `sync` names no default connection here, so it is a connection of its
    // own and not a second name for the bootstrap's driver.
    let sync = Queue::connection("sync").expect("the sync connection");
    assert_eq!(sync.name(), "sync");
    assert!(!Arc::ptr_eq(&sync, &installed));
}

/// A driver the environment installed is replaced when the boot runs again
/// with another `QUEUE_DRIVER`, as before: only the application's driver is
/// kept. `testing::forget_driver` takes the slot back from a driver the
/// application installed.
#[test]
fn a_driver_the_environment_installed_is_replaced_by_the_next_boot() {
    crate::own_process::run_alone(
        "laravel_delta::a_driver_the_environment_installed_is_replaced_by_the_next_boot_child",
    );
}

#[tokio::test]
#[serial]
async fn a_driver_the_environment_installed_is_replaced_by_the_next_boot_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let _lock = crate::env_lock::lock_env_async().await;
    let _vars = set_vars(&[
        ("APP_ENV", Some("testing")),
        ("QUEUE_DRIVER", Some("sync")),
        ("QUEUE_CONNECTIONS", None),
    ]);
    Queue::set_driver(Arc::new(MemoryQueueDriver::new()));
    suprnova::queue::testing::forget_driver();
    assert!(Queue::driver().is_err(), "forget_driver empties the slot");

    suprnova::queue::bootstrap_from_env().await.expect("sync");
    assert_eq!(Queue::driver_name().expect("a driver"), "sync");
    crate::env_snapshot::set_env("QUEUE_DRIVER", Some("null"));
    suprnova::queue::bootstrap_from_env().await.expect("null");
    assert_eq!(Queue::driver_name().expect("a driver"), "null");
}

/// The boot that keeps the bootstrap's driver still fails on a
/// `QUEUE_CONNECTIONS` entry that names no driver, and leaves the default
/// connection as the bootstrap set it.
#[test]
fn a_bad_queue_connections_entry_fails_the_boot_beside_the_bootstraps_driver() {
    crate::own_process::run_alone(
        "laravel_delta::a_bad_queue_connections_entry_fails_the_boot_beside_the_bootstraps_driver_child",
    );
}

#[tokio::test]
#[serial]
async fn a_bad_queue_connections_entry_fails_the_boot_beside_the_bootstraps_driver_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let _lock = crate::env_lock::lock_env_async().await;
    let _vars = set_vars(&[
        ("APP_ENV", Some("testing")),
        ("QUEUE_DRIVER", Some("sync")),
        ("QUEUE_CONNECTIONS", Some("redsi")),
    ]);
    let installed: Arc<dyn QueueDriver> = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(Arc::clone(&installed));

    let error = suprnova::queue::bootstrap_from_env()
        .await
        .expect_err("a connection that names no driver stops the boot");
    assert!(
        error
            .to_string()
            .contains("QUEUE_CONNECTIONS names `redsi`"),
        "{error}"
    );
    assert!(Arc::ptr_eq(
        &Queue::driver().expect("a default driver"),
        &installed
    ));
}
