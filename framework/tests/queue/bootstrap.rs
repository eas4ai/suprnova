//! `bootstrap_from_env` must always replace the registered driver - even when
//! the requested driver is `memory` or unknown. The earlier implementation
//! delegated those branches to `bootstrap_default`, which short-circuits if a
//! driver is already wired, pinning a long-running process to whatever booted
//! first (Redis/database/etc.) and silently ignoring later `QUEUE_DRIVER`
//! changes.

use async_trait::async_trait;
use serial_test::serial;
use std::sync::Arc;
use std::time::Duration;
use suprnova::FrameworkError;
use suprnova::queue::driver::{QueueDriver, Reservation, ReservationToken};
use suprnova::queue::envelope::Envelope;
use suprnova::queue::{Queue, bootstrap_from_env};

/// A driver that names itself "bogus" so the swap is observable; every
/// non-`name` method returns or no-ops in a harmless way.
struct BogusDriver;

#[async_trait]
impl QueueDriver for BogusDriver {
    async fn push(&self, _env: Envelope) -> Result<(), FrameworkError> {
        Ok(())
    }
    async fn pop(
        &self,
        _visibility_timeout: Duration,
    ) -> Result<Option<Reservation>, FrameworkError> {
        Ok(None)
    }
    async fn ack(&self, _token: &ReservationToken) -> Result<(), FrameworkError> {
        Ok(())
    }
    async fn nack(
        &self,
        _token: &ReservationToken,
        _requeue_delay: Duration,
    ) -> Result<(), FrameworkError> {
        Ok(())
    }
    fn name(&self) -> &'static str {
        "bogus"
    }
}

/// A minimal immediately-available envelope, for the boot paths that prove a
/// wired driver actually accepts a push rather than only reporting a name.
fn bootstrap_env() -> Envelope {
    Envelope {
        schema_version: suprnova::queue::CURRENT_SCHEMA_VERSION,
        id: uuid::Uuid::new_v4(),
        job_name: "queue-bootstrap-probe".into(),
        queue: None,
        payload: serde_json::json!({}),
        dispatched_at: chrono::Utc::now(),
        available_at: chrono::Utc::now(),
        attempts: 0,
        max_tries: 1,
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

/// SAFETY: env mutation is process-global; `#[serial]` keeps queue tests from
/// racing with each other.
fn set_env(key: &str, value: Option<&str>) {
    unsafe {
        match value {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }
}

#[tokio::test]
#[serial]
async fn bootstrap_from_env_memory_branch_replaces_an_existing_driver() {
    let _env = crate::env_lock::lock_env_async().await;
    Queue::set_driver(Arc::new(BogusDriver));
    assert_eq!(Queue::driver_name().unwrap(), "bogus");

    set_env("QUEUE_DRIVER", Some("memory"));
    bootstrap_from_env().await.unwrap();
    assert_eq!(
        Queue::driver_name().unwrap(),
        "memory",
        "QUEUE_DRIVER=memory must replace, not no-op"
    );
}

#[tokio::test]
#[serial]
async fn bootstrap_from_env_unset_falls_back_to_a_fresh_memory_driver() {
    let _env = crate::env_lock::lock_env_async().await;
    Queue::set_driver(Arc::new(BogusDriver));
    assert_eq!(Queue::driver_name().unwrap(), "bogus");

    set_env("QUEUE_DRIVER", None);
    bootstrap_from_env().await.unwrap();
    assert_eq!(Queue::driver_name().unwrap(), "memory");
}

#[tokio::test]
#[serial]
async fn bootstrap_from_env_unknown_driver_resets_to_memory() {
    let _env = crate::env_lock::lock_env_async().await;
    Queue::set_driver(Arc::new(BogusDriver));
    assert_eq!(Queue::driver_name().unwrap(), "bogus");

    set_env("QUEUE_DRIVER", Some("definitely-not-a-real-driver"));
    bootstrap_from_env().await.unwrap();
    assert_eq!(
        Queue::driver_name().unwrap(),
        "memory",
        "unknown QUEUE_DRIVER must fall back to a fresh memory driver, \
         not leave the prior driver in place"
    );

    // Cleanup so a later test running in this binary doesn't see the
    // synthetic unknown value lingering in env.
    set_env("QUEUE_DRIVER", None);
}

/// `sync` and `null` are documented drivers; the environment selects them
/// like any other.
#[tokio::test]
#[serial]
async fn bootstrap_from_env_installs_the_sync_and_null_drivers() {
    let _env = crate::env_lock::lock_env_async().await;
    for name in ["sync", "null"] {
        Queue::set_driver(Arc::new(BogusDriver));
        set_env("QUEUE_DRIVER", Some(name));
        let result = bootstrap_from_env().await;
        set_env("QUEUE_DRIVER", None);

        result.unwrap_or_else(|e| panic!("QUEUE_DRIVER={name} is a driver: {e}"));
        assert_eq!(Queue::driver_name().unwrap(), name);
    }
}

/// In production a name that is no driver stops the boot: an in-memory queue
/// taken by mistake loses every job at the next restart.
#[tokio::test]
#[serial]
async fn production_refuses_an_unknown_queue_driver() {
    let _env = crate::env_lock::lock_env_async().await;
    Queue::set_driver(Arc::new(BogusDriver));
    set_env("APP_ENV", Some("production"));
    set_env("QUEUE_DRIVER", Some("redsi"));
    let result = bootstrap_from_env().await;
    set_env("QUEUE_DRIVER", None);
    set_env("APP_ENV", None);

    let error = result.expect_err("a typo must not become an in-memory queue in production");
    let message = error.to_string();
    assert!(message.contains("QUEUE_DRIVER=`redsi`"), "{message}");
    assert!(
        message.contains("memory, sync, null, redis, database, failover"),
        "the message lists what is accepted: {message}"
    );
    assert_eq!(
        Queue::driver_name().unwrap(),
        "bogus",
        "a refused boot leaves the installed driver alone"
    );
}

/// `QUEUE_DRIVER=database` must bring its failed-jobs store with it.
///
/// The `failed_jobs` table is part of that driver's contract -
/// `queue:retry` reads it and `Queue::retry_failed` cannot work without
/// it - but `bootstrap_from_env` used to bind the driver and leave the
/// store unset. A database-backed queue therefore dead-lettered into
/// nothing unless the app wired one by hand, and nothing in the scaffold
/// or the docs prompted anyone to.
///
/// Found in the container harness: the dogfood app had the table, ran the
/// migration, and still recorded `failed_jobs = 0` when a poison job was
/// finally dead-lettered.
///
/// Runs alone in a child process (see `own_process`). The test leaves the
/// process-wide `DB` connection (an in-memory SQLite database with no
/// `failed_jobs` table) and the database failed-jobs store it binds in
/// place for the life of the process, so a later test in this binary that
/// dead-letters a job would write into a table that does not exist.
#[test]
fn the_database_driver_binds_a_failed_jobs_store() {
    crate::own_process::run_alone("bootstrap::the_database_driver_binds_a_failed_jobs_store_child");
}

#[tokio::test]
#[serial]
async fn the_database_driver_binds_a_failed_jobs_store_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let _env = crate::env_lock::lock_env_async().await;
    // The binding is what is under test, not the schema - `bootstrap_from_env`
    // only needs a live connection to hand the two stores.
    suprnova::DB::init_with(
        suprnova::DatabaseConfig::builder()
            .url("sqlite::memory:")
            .build(),
    )
    .await
    .expect("in-memory sqlite connects");

    set_env("QUEUE_DRIVER", Some("database"));
    let result = bootstrap_from_env().await;
    set_env("QUEUE_DRIVER", None);
    result.expect("database queue driver bootstraps");

    assert!(
        suprnova::Queue::failed_store().is_some(),
        "a database-backed queue must dead-letter somewhere durable; without this \
         binding every exhausted job vanishes with only a log line behind it"
    );
}

// ---------------------------------------------------------------------------
// QUEUE_DRIVER=failover
// ---------------------------------------------------------------------------

/// The env path has to build every inner connection and install the decorator,
/// not just parse the list.
#[tokio::test]
#[serial]
async fn failover_wires_a_driver_over_the_listed_connections() {
    let _env = crate::env_lock::lock_env_async().await;
    Queue::set_driver(Arc::new(BogusDriver));
    set_env("QUEUE_DRIVER", Some("failover"));
    set_env("QUEUE_FAILOVER_CONNECTIONS", Some("memory, memory"));
    let result = bootstrap_from_env().await;
    set_env("QUEUE_DRIVER", None);
    set_env("QUEUE_FAILOVER_CONNECTIONS", None);
    result.expect("failover bootstraps over two memory connections");

    assert_eq!(Queue::driver_name().unwrap(), "failover");

    // The whole point is that pushes reach a real backend, so prove one does
    // rather than stopping at the driver's name.
    let driver = Queue::driver().expect("driver");
    driver
        .push(bootstrap_env())
        .await
        .expect("push through the failover connection");
    assert!(
        driver
            .pop(Duration::from_secs(1))
            .await
            .expect("pop")
            .is_some(),
        "the primary connection accepted the push, so the primary must pop it back"
    );
}

#[tokio::test]
#[serial]
async fn failover_without_a_connection_list_is_a_boot_error() {
    let _env = crate::env_lock::lock_env_async().await;
    set_env("QUEUE_DRIVER", Some("failover"));
    set_env("QUEUE_FAILOVER_CONNECTIONS", None);
    let result = bootstrap_from_env().await;
    set_env("QUEUE_DRIVER", None);

    let err = result.expect_err("a failover connection with no list cannot boot");
    assert!(
        err.to_string().contains("QUEUE_FAILOVER_CONNECTIONS"),
        "the error must name the missing variable, got {err}"
    );
}

#[tokio::test]
#[serial]
async fn failover_with_a_blank_connection_list_is_a_boot_error() {
    let _env = crate::env_lock::lock_env_async().await;
    set_env("QUEUE_DRIVER", Some("failover"));
    set_env("QUEUE_FAILOVER_CONNECTIONS", Some("   "));
    let result = bootstrap_from_env().await;
    set_env("QUEUE_DRIVER", None);
    set_env("QUEUE_FAILOVER_CONNECTIONS", None);

    let err = result.expect_err("a blank list is a half-finished edit, not a queue");
    assert!(
        err.to_string().contains("QUEUE_FAILOVER_CONNECTIONS"),
        "the error must name the variable, got {err}"
    );
}

#[tokio::test]
#[serial]
async fn failover_rejects_a_nested_failover_connection() {
    let _env = crate::env_lock::lock_env_async().await;
    set_env("QUEUE_DRIVER", Some("failover"));
    set_env("QUEUE_FAILOVER_CONNECTIONS", Some("memory,failover"));
    let result = bootstrap_from_env().await;
    set_env("QUEUE_DRIVER", None);
    set_env("QUEUE_FAILOVER_CONNECTIONS", None);

    let err = result.expect_err("nesting must be rejected");
    assert!(
        err.to_string().contains("no nesting"),
        "the error must say why, got {err}"
    );
}

/// The warn-and-fall-back-to-memory behaviour belongs to `QUEUE_DRIVER` alone.
/// Inside a failover chain a typo would silently splice an ephemeral in-memory
/// connection into a durable list, so it has to be a boot error instead.
#[tokio::test]
#[serial]
async fn failover_rejects_an_unknown_inner_connection_instead_of_falling_back() {
    let _env = crate::env_lock::lock_env_async().await;
    set_env("QUEUE_DRIVER", Some("failover"));
    set_env("QUEUE_FAILOVER_CONNECTIONS", Some("memory,redsi"));
    let result = bootstrap_from_env().await;
    set_env("QUEUE_DRIVER", None);
    set_env("QUEUE_FAILOVER_CONNECTIONS", None);

    let err = result.expect_err("an unknown inner connection must not become memory");
    assert!(
        err.to_string().contains("redsi"),
        "the error must name the offending entry, got {err}"
    );
}

#[tokio::test]
#[serial]
async fn queue_connections_registers_one_connection_per_entry() {
    let _env = crate::env_lock::lock_env_async().await;
    suprnova::queue::testing::forget_connections();
    set_env("QUEUE_DRIVER", Some("memory"));
    set_env("QUEUE_CONNECTIONS", Some("null, memory"));
    let booted = bootstrap_from_env().await;
    set_env("QUEUE_CONNECTIONS", None);
    booted.unwrap();

    assert_eq!(Queue::connection_names().unwrap(), ["memory", "null"]);
    assert_eq!(Queue::connection("null").unwrap().name(), "null");

    // `memory` is also QUEUE_DRIVER, so the entry is the default connection
    // under its own name, not a second queue beside it.
    Queue::connection("memory")
        .unwrap()
        .push(bootstrap_env())
        .await
        .unwrap();
    assert_eq!(Queue::size().await.unwrap(), 1);
    suprnova::queue::testing::forget_connections();
}

#[tokio::test]
#[serial]
async fn queue_connections_refuses_a_name_that_is_no_driver() {
    let _env = crate::env_lock::lock_env_async().await;
    suprnova::queue::testing::forget_connections();
    set_env("QUEUE_DRIVER", Some("memory"));
    set_env("QUEUE_CONNECTIONS", Some("redsi"));
    let booted = bootstrap_from_env().await;
    set_env("QUEUE_CONNECTIONS", None);

    let err = booted.expect_err("a typo must not become a connection");
    assert!(
        err.to_string().contains("QUEUE_CONNECTIONS names `redsi`"),
        "{err}"
    );
    assert!(Queue::connection_names().unwrap().is_empty());
    suprnova::queue::testing::forget_connections();
}

#[tokio::test]
#[serial]
async fn without_queue_connections_no_connection_is_registered() {
    let _env = crate::env_lock::lock_env_async().await;
    suprnova::queue::testing::forget_connections();
    set_env("QUEUE_DRIVER", Some("memory"));
    set_env("QUEUE_CONNECTIONS", None);
    bootstrap_from_env().await.unwrap();

    assert!(Queue::connection_names().unwrap().is_empty());
}
