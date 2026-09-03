#![cfg(feature = "magnetar-oauth")]

use std::sync::Arc;

use suprnova::{Auth, FrameworkError, RateLimiterDriver, SlidingWindowConfig};

const FACTOR_SESSION_MISSING: &str = "Internal server error: Magnetar factor/session authentication subsystem was not initialized during application bootstrap; call init_magnetar(...), install_magnetar_engines(...), install_magnetar_engines_with_factor(...), or install_magnetar_oauth_engine_with_factor(...) during bootstrap";
const PASSWORD_MISSING: &str = "Internal server error: Magnetar password authentication subsystem was not initialized during application bootstrap; call init_magnetar(...) or install_magnetar_engines(...)/install_magnetar_engines_with_factor(...) during bootstrap";
const PASSKEY_MISSING: &str = "Internal server error: Magnetar passkey authentication subsystem was not initialized during application bootstrap; call init_magnetar(...) or install_magnetar_engines(...)/install_magnetar_engines_with_factor(...) during bootstrap";
const OAUTH_MISSING: &str = "Internal server error: Magnetar OAuth authentication subsystem was not initialized during application bootstrap; configure MagnetarConfig::oauth(...) before init_magnetar(...), or use init_magnetar_oauth_only(...), install_magnetar_oauth_engine(...), or install_magnetar_oauth_engine_with_factor(...)";

struct AllowingLimiter;

#[suprnova::async_trait]
impl RateLimiterDriver for AllowingLimiter {
    async fn try_acquire(
        &self,
        _key: &str,
        _config: &SlidingWindowConfig,
    ) -> Result<bool, FrameworkError> {
        Ok(true)
    }

    async fn retry_after(
        &self,
        _key: &str,
        _config: &SlidingWindowConfig,
    ) -> Result<Option<std::time::Duration>, FrameworkError> {
        Ok(None)
    }
}

#[tokio::test]
async fn missing_factor_session_engine_names_bootstrap_action() {
    let error = Auth::oauth("missing")
        .complete_outcome("code", "state")
        .await
        .expect_err("OAuth session completion requires the factor/session authority");

    assert_eq!(error.to_string(), FACTOR_SESSION_MISSING);
}

#[tokio::test]
async fn missing_password_engine_names_bootstrap_action() {
    suprnova::App::bind::<dyn RateLimiterDriver>(Arc::new(AllowingLimiter));
    let error = Auth::password()
        .register("missing@example.test", "correct-password")
        .await
        .expect_err("password registration requires the password engine");

    assert_eq!(error.to_string(), PASSWORD_MISSING);
}

#[tokio::test]
async fn missing_passkey_engine_names_bootstrap_action() {
    let slot = suprnova::session::new_session_slot_for_test();
    let error = suprnova::session::session_scope_for_test(slot, async {
        Auth::passkey()
            .begin_authentication("missing@example.test")
            .await
    })
    .await
    .expect_err("passkey authentication requires the passkey engine");

    assert_eq!(error.to_string(), PASSKEY_MISSING);
}

#[tokio::test]
async fn missing_oauth_engine_names_bootstrap_action() {
    let error = Auth::oauth("missing")
        .verify_oauth_identity("code", "state")
        .await
        .expect_err("OAuth verification requires the OAuth engine");

    assert_eq!(error.to_string(), OAUTH_MISSING);
}
