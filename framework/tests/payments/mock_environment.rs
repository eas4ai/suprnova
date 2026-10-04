//! The mock provider accepts every webhook unsigned, so it must refuse to
//! verify anywhere but a development environment. The effective environment
//! is the registered `AppConfig` when the application registers one in code,
//! and `APP_ENV` otherwise; the guard has to agree with both, or a forged
//! `POST /webhooks/payments/mock` hydrates billing mirror rows on a server
//! that boots as production.
//!
//! Each test registers an `AppConfig`, which the config repository keeps for
//! the life of the process, so each runs alone in a child (see `own_process`).

use http::HeaderMap;
use suprnova::config::{AppConfig, Config, Environment};
use suprnova::payments::{MockPaymentProvider, PaymentError, WebhookContext, WebhookHandler};

use crate::env_snapshot::{EnvSnapshot, set_env};

fn register_app_config(environment: Environment) {
    Config::register(
        AppConfig::builder()
            .name("payments-mock-environment-test")
            .environment(environment)
            .url("http://localhost:0")
            .build(),
    );
}

fn verify_unsigned() -> Result<(), PaymentError> {
    let headers = HeaderMap::new();
    let ctx = WebhookContext {
        body: br#"{"id":"evt_forged","type":"payment.succeeded"}"#,
        headers: &headers,
        remote_addr: None,
    };
    MockPaymentProvider::new().verify(&ctx)
}

#[test]
fn a_registered_production_config_refuses_with_app_env_unset() {
    crate::own_process::run_alone(
        "mock_environment::a_registered_production_config_refuses_with_app_env_unset_child",
    );
}

#[test]
fn a_registered_production_config_refuses_with_app_env_unset_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let _env = crate::env_lock::lock_env();
    let _restore = EnvSnapshot::capture(&["APP_ENV"]);
    // Unset APP_ENV detects as Local, the permissive answer the old guard read.
    set_env("APP_ENV", None);
    register_app_config(Environment::Production);
    assert!(Config::is_production(), "the server boots as production");

    let err = verify_unsigned()
        .expect_err("the mock must refuse when the effective environment is production");
    assert!(
        matches!(err, PaymentError::WebhookSignature(_)),
        "expected WebhookSignature, got {err:?}"
    );
}

#[test]
fn a_registered_staging_config_refuses_with_app_env_local() {
    crate::own_process::run_alone(
        "mock_environment::a_registered_staging_config_refuses_with_app_env_local_child",
    );
}

#[test]
fn a_registered_staging_config_refuses_with_app_env_local_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let _env = crate::env_lock::lock_env();
    let _restore = EnvSnapshot::capture(&["APP_ENV"]);
    set_env("APP_ENV", Some("local"));
    register_app_config(Environment::Staging);

    let err = verify_unsigned()
        .expect_err("the mock must refuse when the effective environment is staging");
    assert!(
        matches!(err, PaymentError::WebhookSignature(_)),
        "expected WebhookSignature, got {err:?}"
    );
}

/// The stricter half: a development `AppConfig` registered in code does not
/// open the mock on a host whose `APP_ENV` says production.
#[test]
fn a_registered_local_config_still_refuses_with_app_env_production() {
    crate::own_process::run_alone(
        "mock_environment::a_registered_local_config_still_refuses_with_app_env_production_child",
    );
}

#[test]
fn a_registered_local_config_still_refuses_with_app_env_production_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let _env = crate::env_lock::lock_env();
    let _restore = EnvSnapshot::capture(&["APP_ENV"]);
    set_env("APP_ENV", Some("production"));
    register_app_config(Environment::Local);

    let err = verify_unsigned().expect_err("the mock must refuse when APP_ENV is production");
    assert!(
        matches!(err, PaymentError::WebhookSignature(_)),
        "expected WebhookSignature, got {err:?}"
    );
}

#[test]
fn a_registered_development_config_accepts_with_app_env_unset() {
    crate::own_process::run_alone(
        "mock_environment::a_registered_development_config_accepts_with_app_env_unset_child",
    );
}

#[test]
fn a_registered_development_config_accepts_with_app_env_unset_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let _env = crate::env_lock::lock_env();
    let _restore = EnvSnapshot::capture(&["APP_ENV"]);
    set_env("APP_ENV", None);
    register_app_config(Environment::Development);

    verify_unsigned().expect("the mock stays usable in a development environment");
}
