//! Wave 6 T52 - pool liveness knobs, the reachable port of Laravel's
//! libpq `keepalives_*` DSN options (#61307).
//!
//! sqlx 0.9 exposes no TCP keepalive setting at any layer, so the
//! mechanism for "the network killed a connection the pool still thinks
//! is good" is pool recycling plus a pre-hand-out ping. These tests pin:
//!
//!   1. every knob is unset by default, so an existing deployment's pool
//!      is byte-for-byte what it was;
//!   2. each variable round-trips from the environment into the config;
//!   3. `0` on the two reaping knobs survives as `Some(0)` - the
//!      "never reap" spelling;
//!   4. a zero acquire timeout is rejected;
//!   5. builder setters win over the environment;
//!   6. a pool actually builds with every knob set.

use std::sync::Mutex;

use suprnova::database::DbConnection;
use suprnova::database::config::DatabaseConfig;

/// Every test in this file reads or writes the same process-wide
/// environment through `DatabaseConfig::from_env`.
static ENV_LOCK: Mutex<()> = Mutex::new(());

const KEYS: &[&str] = &[
    "DATABASE_URL",
    "DB_IDLE_TIMEOUT",
    "DB_MAX_LIFETIME",
    "DB_ACQUIRE_TIMEOUT",
    "DB_TEST_BEFORE_ACQUIRE",
    "DB_PING_AFTER_IDLE",
];

struct EnvSnapshot {
    keys: Vec<(&'static str, Option<String>)>,
}

impl EnvSnapshot {
    fn capture(keys: &[&'static str]) -> Self {
        Self {
            keys: keys.iter().map(|k| (*k, std::env::var(k).ok())).collect(),
        }
    }
}

impl Drop for EnvSnapshot {
    fn drop(&mut self) {
        for (k, v) in &self.keys {
            // SAFETY: ENV_LOCK serializes these tests within the suite,
            // and each integration test file is its own binary.
            unsafe {
                match v {
                    Some(val) => std::env::set_var(k, val),
                    None => std::env::remove_var(k),
                }
            }
        }
    }
}

fn set_env(key: &str, value: Option<&str>) {
    // SAFETY: ENV_LOCK held by the caller.
    unsafe {
        match value {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }
}

fn clear_liveness_env() {
    for k in KEYS.iter().skip(1) {
        set_env(k, None);
    }
}

#[test]
fn defaults_leave_the_pool_exactly_as_it_is_today() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _snap = EnvSnapshot::capture(KEYS);
    clear_liveness_env();

    let cfg = DatabaseConfig::from_env();
    assert_eq!(cfg.idle_timeout, None, "unset means sqlx's own default");
    assert_eq!(cfg.max_lifetime, None);
    assert_eq!(
        cfg.acquire_timeout, None,
        "unset means the pool keeps inheriting DB_CONNECT_TIMEOUT"
    );
    assert!(
        cfg.test_before_acquire,
        "pinging before hand-out is sqlx's default and must stay on"
    );
    assert_eq!(cfg.ping_after_idle, None);
}

#[test]
fn every_liveness_knob_round_trips_from_the_environment() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _snap = EnvSnapshot::capture(KEYS);
    clear_liveness_env();

    set_env("DB_IDLE_TIMEOUT", Some("120"));
    set_env("DB_MAX_LIFETIME", Some("900"));
    set_env("DB_ACQUIRE_TIMEOUT", Some("5"));
    set_env("DB_TEST_BEFORE_ACQUIRE", Some("false"));
    set_env("DB_PING_AFTER_IDLE", Some("30"));

    let cfg = DatabaseConfig::from_env();
    assert_eq!(cfg.idle_timeout, Some(120));
    assert_eq!(cfg.max_lifetime, Some(900));
    assert_eq!(cfg.acquire_timeout, Some(5));
    assert!(!cfg.test_before_acquire);
    assert_eq!(cfg.ping_after_idle, Some(30));
}

#[test]
fn zero_on_a_reaping_knob_is_kept_as_the_disable_signal() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _snap = EnvSnapshot::capture(KEYS);
    clear_liveness_env();

    set_env("DB_IDLE_TIMEOUT", Some("0"));
    set_env("DB_MAX_LIFETIME", Some("0"));

    let cfg = DatabaseConfig::from_env();
    assert_eq!(
        cfg.idle_timeout,
        Some(0),
        "0 must survive as an explicit 'never reap', not collapse to None"
    );
    assert_eq!(cfg.max_lifetime, Some(0));
    cfg.validate_pool()
        .expect("0 is a legal value for the reaping knobs");
}

#[test]
fn an_unparseable_value_falls_back_instead_of_failing_the_boot() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _snap = EnvSnapshot::capture(KEYS);
    clear_liveness_env();

    set_env("DB_IDLE_TIMEOUT", Some("ten minutes"));
    set_env("DB_TEST_BEFORE_ACQUIRE", Some("yes-please"));

    let cfg = DatabaseConfig::from_env();
    assert_eq!(cfg.idle_timeout, None, "unparseable is treated as unset");
    assert!(
        cfg.test_before_acquire,
        "unparseable falls back to the default, not to false"
    );
}

#[test]
fn validate_pool_rejects_a_zero_acquire_timeout() {
    let _guard = ENV_LOCK.lock().unwrap();
    let cfg = DatabaseConfig::builder()
        .url("sqlite::memory:")
        .acquire_timeout(0)
        .build();
    let err = cfg
        .validate_pool()
        .expect_err("a zero-second acquire timeout fails every checkout");
    assert!(
        err.to_string().contains("DB_ACQUIRE_TIMEOUT"),
        "the message must name the variable, got: {err}"
    );
}

#[test]
fn builder_setters_win_over_the_environment() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _snap = EnvSnapshot::capture(KEYS);
    clear_liveness_env();
    set_env("DB_IDLE_TIMEOUT", Some("120"));

    let cfg = DatabaseConfig::builder()
        .url("sqlite::memory:")
        .idle_timeout(45)
        .max_lifetime(600)
        .test_before_acquire(false)
        .ping_after_idle(15)
        .build();

    assert_eq!(cfg.idle_timeout, Some(45));
    assert_eq!(cfg.max_lifetime, Some(600));
    assert!(!cfg.test_before_acquire);
    assert_eq!(cfg.ping_after_idle, Some(15));
}

#[tokio::test]
async fn a_pool_builds_with_every_liveness_knob_set() {
    // The knobs are wired onto `ConnectOptions` in `connect_as`; this is
    // the test that proves the wiring matches SeaORM's real signatures
    // and that no combination stops the pool from coming up.
    let _guard = ENV_LOCK.lock().unwrap();
    let cfg = DatabaseConfig::builder()
        .url("sqlite::memory:")
        .idle_timeout(60)
        .max_lifetime(300)
        .acquire_timeout(5)
        .test_before_acquire(true)
        .ping_after_idle(10)
        .build();

    let conn = DbConnection::connect(&cfg)
        .await
        .expect("a pool with liveness knobs set must still connect");
    drop(conn);
}

#[tokio::test]
async fn a_pool_builds_with_reaping_disabled() {
    let _guard = ENV_LOCK.lock().unwrap();
    let cfg = DatabaseConfig::builder()
        .url("sqlite::memory:")
        .idle_timeout(0)
        .max_lifetime(0)
        .build();

    let conn = DbConnection::connect(&cfg)
        .await
        .expect("'never reap' must be a legal pool configuration");
    drop(conn);
}
