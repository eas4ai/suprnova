//! P2-02 - production must not boot on a per-process rate limiter.
//!
//! `rate_limit::bootstrap_from_env` defaulted to `InMemoryRateLimiter`
//! when `RATE_LIMIT_DRIVER` was unset, and fell back to it - with a
//! single `warn!` - for any value it did not recognise.
//!
//! Behind more than one replica that is not a rate limit. Each process
//! keeps its own bucket map, so a "5 attempts per 15 minutes"
//! password-reset throttle is really 5N, and every deploy resets all of
//! them to zero. Nothing about the running system reveals it: the
//! requests succeed, which is exactly what a working throttle looks like
//! from outside. The failure only becomes visible as a credential-stuffing
//! or enumeration incident.
//!
//! Same fail-closed shape as SEC-03's mail guard, and the same escape
//! hatch idiom, because an operator should learn this pattern once.
//!
//! `Environment::detect()` reads process-wide `APP_ENV` and these tests
//! mutate it alongside `RATE_LIMIT_DRIVER`, so - like
//! `mail_production_fail_closed.rs` - they live in their own test binary
//! and serialise against each other. The env-free half of the matrix
//! (`select_limiter_driver` with explicit arguments) is unit-tested in
//! `framework/src/rate_limit/mod.rs`.

use serial_test::serial;

/// Spelled out literally rather than imported: the name is the public
/// contract an operator types into a deployment config, so a rename must
/// break this test.
const OVERRIDE_ENV: &str = "RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION";

struct EnvGuard {
    app_env: Option<String>,
    driver: Option<String>,
    override_flag: Option<String>,
}

impl EnvGuard {
    /// # Safety
    /// Mutates process-global env. Safe here because every test in this
    /// file is `#[serial]`-locked and this binary contains no others.
    fn take() -> Self {
        let guard = Self {
            app_env: std::env::var("APP_ENV").ok(),
            driver: std::env::var("RATE_LIMIT_DRIVER").ok(),
            override_flag: std::env::var(OVERRIDE_ENV).ok(),
        };
        unsafe {
            std::env::remove_var("APP_ENV");
            std::env::remove_var("RATE_LIMIT_DRIVER");
            std::env::remove_var(OVERRIDE_ENV);
        }
        guard
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        // SAFETY: see `EnvGuard::take`.
        unsafe {
            for (name, value) in [
                ("APP_ENV", &self.app_env),
                ("RATE_LIMIT_DRIVER", &self.driver),
                (OVERRIDE_ENV, &self.override_flag),
            ] {
                match value {
                    Some(v) => std::env::set_var(name, v),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
}

/// # Safety
/// See [`EnvGuard::take`].
fn set(name: &str, value: &str) {
    unsafe {
        std::env::set_var(name, value);
    }
}

/// The shipping case: a deploy that set `APP_ENV=production` and never
/// thought about the limiter at all.
#[tokio::test]
#[serial(rate_limit_env)]
async fn production_boot_without_a_driver_fails_closed() {
    let _guard = EnvGuard::take();
    set("APP_ENV", "production");

    let err = suprnova::rate_limit::bootstrap_from_env()
        .await
        .expect_err("production with no RATE_LIMIT_DRIVER must refuse to boot");
    let msg = format!("{err}");

    assert!(
        msg.contains(OVERRIDE_ENV),
        "the refusal must name the override that unblocks it: {msg}"
    );
    assert!(
        msg.contains("RATE_LIMIT_DRIVER=redis"),
        "and the configuration that is actually correct: {msg}"
    );
}

#[tokio::test]
#[serial(rate_limit_env)]
async fn production_boot_on_the_memory_driver_fails_closed() {
    let _guard = EnvGuard::take();
    set("APP_ENV", "production");
    set("RATE_LIMIT_DRIVER", "memory");

    suprnova::rate_limit::bootstrap_from_env()
        .await
        .expect_err("an explicit `memory` in production must still refuse");
}

/// The case most likely to reach production, because it *looks*
/// configured. A capitalised driver name used to warn once at boot and
/// then quietly limit per-process.
#[tokio::test]
#[serial(rate_limit_env)]
async fn production_boot_on_an_unknown_driver_fails_instead_of_falling_back() {
    let _guard = EnvGuard::take();
    set("APP_ENV", "production");
    set("RATE_LIMIT_DRIVER", "Redis");

    let err = suprnova::rate_limit::bootstrap_from_env()
        .await
        .expect_err("an unrecognised driver falls back to memory, so it must refuse");

    assert!(
        format!("{err}").contains("Redis"),
        "the error must quote the operator's literal value - it is usually \
         the typo itself: {err}"
    );
}

/// Presence is not consent. Same discipline as the mail override.
#[tokio::test]
#[serial(rate_limit_env)]
async fn a_non_truthy_override_keeps_the_guard_armed() {
    for value in ["false", "0", "no", "maybe", ""] {
        let _guard = EnvGuard::take();
        set("APP_ENV", "production");
        set(OVERRIDE_ENV, value);

        let result = suprnova::rate_limit::bootstrap_from_env().await;
        assert!(
            result.is_err(),
            "{OVERRIDE_ENV}={value:?} must not count as consent"
        );
    }
}

/// The override is for a genuinely single-process deployment, where a
/// per-process quota *is* the global quota.
#[tokio::test]
#[serial(rate_limit_env)]
async fn the_override_lets_a_single_process_deployment_boot() {
    let _guard = EnvGuard::take();
    set("APP_ENV", "production");
    set(OVERRIDE_ENV, "true");

    suprnova::rate_limit::bootstrap_from_env()
        .await
        .expect("the override exists precisely to permit this");
}

/// Development must be untouched - a fresh `suprnova new` has no
/// `RATE_LIMIT_DRIVER` and must keep working with zero configuration.
#[tokio::test]
#[serial(rate_limit_env)]
async fn non_production_boot_is_unchanged() {
    for app_env in [None, Some("local"), Some("development"), Some("staging")] {
        let _guard = EnvGuard::take();
        if let Some(v) = app_env {
            set("APP_ENV", v);
        }

        suprnova::rate_limit::bootstrap_from_env()
            .await
            .unwrap_or_else(|e| panic!("APP_ENV={app_env:?} must boot unchanged: {e}"));
    }
}

/// Staging deliberately stays permissive, matching SEC-03's reasoning:
/// hard-failing a staging environment pushes teams to set the override
/// globally, which disarms it where it actually matters.
#[tokio::test]
#[serial(rate_limit_env)]
async fn staging_is_not_gated() {
    let _guard = EnvGuard::take();
    set("APP_ENV", "staging");
    set("RATE_LIMIT_DRIVER", "memory");

    suprnova::rate_limit::bootstrap_from_env()
        .await
        .expect("staging on the memory limiter is a normal, deliberate setup");
}
