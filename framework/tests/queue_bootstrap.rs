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
    Queue::set_driver(Arc::new(BogusDriver));
    assert_eq!(Queue::driver_name().unwrap(), "bogus");

    set_env("QUEUE_DRIVER", None);
    bootstrap_from_env().await.unwrap();
    assert_eq!(Queue::driver_name().unwrap(), "memory");
}

#[tokio::test]
#[serial]
async fn bootstrap_from_env_unknown_driver_resets_to_memory() {
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
#[tokio::test]
#[serial]
async fn the_database_driver_binds_a_failed_jobs_store() {
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
