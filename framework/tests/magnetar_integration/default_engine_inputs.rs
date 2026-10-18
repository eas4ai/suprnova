//! How the default Magnetar engine answers registration's rules and inputs
//! past the default schema's columns.
//!
//! Registration answers Magnetar's validation failures as the framework's
//! validation error: 422, with the field in `errors`. Sign-in keeps its
//! indistinguishable invalid-credentials answer. The `BruteForce` facade
//! counts and locks an address longer than 254 characters, which an
//! application's own user provider can hold.
//!
//! Each test installs the default Magnetar engine, which is process-wide,
//! so each runs in a process of its own.

use std::sync::Arc;

use async_trait::async_trait;
use sea_orm::Database;
use suprnova::auth_flows::BruteForce;
use suprnova::{
    Auth, Crypt, EncryptionKey, FrameworkError, MagnetarConfig, RateLimiterDriver,
    SlidingWindowConfig, init_magnetar,
};

struct AllowingLimiter;

#[async_trait]
impl RateLimiterDriver for AllowingLimiter {
    async fn try_acquire(&self, _: &str, _: &SlidingWindowConfig) -> Result<bool, FrameworkError> {
        Ok(true)
    }

    async fn retry_after(
        &self,
        _: &str,
        _: &SlidingWindowConfig,
    ) -> Result<Option<std::time::Duration>, FrameworkError> {
        Ok(None)
    }
}

async fn install_default_engine() {
    Crypt::init(EncryptionKey::generate());
    suprnova::App::bind::<dyn RateLimiterDriver>(Arc::new(AllowingLimiter));
    let connection = Database::connect("sqlite::memory:")
        .await
        .expect("connect SQLite");
    init_magnetar(MagnetarConfig::from_sea_orm(connection))
        .await
        .expect("install default Magnetar engine");
}

/// An address one character past RFC 5321's 254.
fn over_long_email() -> String {
    format!("{}@example.test", "a".repeat(242))
}

/// The fields a validation error names, or a description of what it was.
fn validation_fields(error: &FrameworkError) -> Vec<String> {
    match error {
        FrameworkError::Validation(errors) => {
            let mut fields = errors.errors.keys().cloned().collect::<Vec<_>>();
            fields.sort();
            fields
        }
        other => vec![format!("not a validation error: {other:?}")],
    }
}

#[test]
fn registration_refuses_an_over_long_email_as_a_validation_error() {
    crate::own_process::run_alone(
        "default_engine_inputs::registration_refuses_an_over_long_email_as_a_validation_error_child",
    );
}

#[tokio::test]
async fn registration_refuses_an_over_long_email_as_a_validation_error_child() {
    if !crate::own_process::is_child() {
        return;
    }
    install_default_engine().await;
    let error = Auth::password()
        .register(&over_long_email(), "correct-password")
        .await
        .expect_err("an over-long address is refused");
    assert_eq!(error.status_code(), 422);
    assert_eq!(validation_fields(&error), vec!["email".to_owned()]);
}

#[test]
fn registration_refuses_a_weak_password_as_a_validation_error() {
    crate::own_process::run_alone(
        "default_engine_inputs::registration_refuses_a_weak_password_as_a_validation_error_child",
    );
}

#[tokio::test]
async fn registration_refuses_a_weak_password_as_a_validation_error_child() {
    if !crate::own_process::is_child() {
        return;
    }
    install_default_engine().await;
    let error = Auth::password()
        .register("weak-password@example.test", "short")
        .await
        .expect_err("a weak password is refused");
    assert_eq!(error.status_code(), 422);
    assert_eq!(validation_fields(&error), vec!["password".to_owned()]);
}

#[test]
fn sign_in_keeps_the_invalid_credentials_answer() {
    crate::own_process::run_alone(
        "default_engine_inputs::sign_in_keeps_the_invalid_credentials_answer_child",
    );
}

#[tokio::test]
async fn sign_in_keeps_the_invalid_credentials_answer_child() {
    if !crate::own_process::is_child() {
        return;
    }
    install_default_engine().await;
    Auth::password()
        .register("signs-in@example.test", "correct-password")
        .await
        .expect("register an account")
        .created()
        .expect("registration creates a new account");
    let over_long = over_long_email();
    for (email, password) in [
        ("signs-in@example.test", "wrong-password"),
        ("nobody@example.test", "correct-password"),
        (over_long.as_str(), "correct-password"),
    ] {
        let error = Auth::password()
            .authenticate(email, password, None, None)
            .await
            .expect_err("the sign-in is refused");
        assert_eq!(error.status_code(), 401, "{email}");
        assert!(
            !matches!(error, FrameworkError::Validation(_)),
            "a sign-in failure says nothing about which part was wrong"
        );
    }
}

#[test]
fn brute_force_counts_and_locks_an_address_longer_than_254_characters() {
    crate::own_process::run_alone(
        "default_engine_inputs::brute_force_counts_and_locks_an_address_longer_than_254_characters_child",
    );
}

#[tokio::test]
async fn brute_force_counts_and_locks_an_address_longer_than_254_characters_child() {
    if !crate::own_process::is_child() {
        return;
    }
    install_default_engine().await;
    let email = format!("{}@example.com", "l".repeat(288));
    assert_eq!(email.chars().count(), 300);
    for _ in 0..5 {
        BruteForce::record_failed_attempt(&email, Some("203.0.113.7"))
            .await
            .expect("a failure for a long address is counted");
    }
    assert!(
        BruteForce::is_locked(&email).await.expect("lock status"),
        "five failures lock the long address"
    );
    assert!(BruteForce::unlock_account(&email).await.expect("unlock"));
    assert!(!BruteForce::is_locked(&email).await.expect("lock status"));
}
