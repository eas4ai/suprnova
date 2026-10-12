//! Framework abuse-limiter integration tests for Magnetar auth routes.

use std::any::Any;

use chrono::{DateTime, Utc};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use suprnova::auth::{Authenticatable, must_verify_email::MustVerifyEmail};

use suprnova::rate_limit::{RateLimiterDriver, SlidingWindowConfig};
use suprnova::testing::TestContainer;
use suprnova::{Auth, EmailVerification, FrameworkError, PasswordReset, async_trait};

#[derive(Default)]
struct CountingLimiter {
    calls: Mutex<Vec<(String, u32, Duration)>>,
    fail: bool,
}

impl CountingLimiter {
    fn failing() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            fail: true,
        }
    }

    fn calls(&self) -> Vec<(String, u32, Duration)> {
        self.calls.lock().expect("counting limiter lock").clone()
    }

    fn keys(&self) -> Vec<String> {
        self.calls
            .lock()
            .expect("counting limiter lock")
            .iter()
            .map(|(key, _, _)| key.clone())
            .collect()
    }
}

#[async_trait]
impl RateLimiterDriver for CountingLimiter {
    async fn try_acquire(
        &self,
        key: &str,
        _config: &SlidingWindowConfig,
    ) -> Result<bool, FrameworkError> {
        self.calls.lock().expect("counting limiter lock").push((
            key.to_owned(),
            _config.max_requests,
            _config.window,
        ));
        if self.fail {
            return Err(FrameworkError::internal("test limiter unavailable"));
        }
        Ok(true)
    }

    async fn retry_after(
        &self,
        _key: &str,
        _config: &SlidingWindowConfig,
    ) -> Result<Option<Duration>, FrameworkError> {
        Ok(None)
    }
}

struct VerificationUser {
    email: String,
}

impl Authenticatable for VerificationUser {
    fn get_auth_identifier(&self) -> String {
        self.email.clone()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

impl MustVerifyEmail for VerificationUser {
    fn email(&self) -> &str {
        &self.email
    }

    fn email_verified_at(&self) -> Option<DateTime<Utc>> {
        None
    }

    fn set_email_verified_at(&mut self, _value: Option<DateTime<Utc>>) {}
}
#[tokio::test]
async fn auth_start_routes_consult_the_limiter_for_present_and_absent_identities() {
    TestContainer::scope(async {
        let limiter = Arc::new(CountingLimiter::default());
        TestContainer::bind::<dyn RateLimiterDriver>(limiter.clone());

        let present = "present@example.test";
        let absent = "absent@example.test";

        let _ = Auth::magic_link()
            .send(present, "https://example.test/magic")
            .await;
        let _ = Auth::magic_link()
            .send(absent, "https://example.test/magic")
            .await;
        let _ = PasswordReset::send_link(present, "https://example.test/reset").await;
        let _ = PasswordReset::send_link(absent, "https://example.test/reset").await;
        let _ = EmailVerification::resend(present, "https://example.test/verify").await;
        let present_verification_user = VerificationUser {
            email: present.to_owned(),
        };
        let absent_verification_user = VerificationUser {
            email: absent.to_owned(),
        };
        let _ =
            EmailVerification::send_link(&present_verification_user, "https://example.test/verify")
                .await;
        let _ =
            EmailVerification::send_link(&absent_verification_user, "https://example.test/verify")
                .await;
        let _ = EmailVerification::resend(absent, "https://example.test/verify").await;
        let _ = Auth::password().register(present, "password").await;
        let _ = Auth::password().register(absent, "password").await;

        let keys = limiter.keys();
        assert_eq!(
            keys.len(),
            10,
            "every framework-owned auth start route must consume a permit"
        );
        for (purpose, max_requests, window) in [
            ("magic-link-send", 3, Duration::from_secs(60 * 60)),
            ("password-reset-send", 3, Duration::from_secs(60 * 60)),
            ("email-verification-send", 3, Duration::from_secs(60 * 60)),
            ("email-verification-resend", 3, Duration::from_secs(60 * 60)),
            ("password-register", 10, Duration::from_secs(60 * 60)),
        ] {
            assert_eq!(
                limiter
                    .calls()
                    .into_iter()
                    .filter(|(key, max, duration)| {
                        key.starts_with(&format!("auth:{purpose}:"))
                            && *max == max_requests
                            && *duration == window
                    })
                    .count(),
                2,
                "{purpose} must consult its configured policy for both identities"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn auth_start_routes_fail_closed_when_the_limiter_backend_is_unavailable() {
    TestContainer::scope(async {
        let limiter = Arc::new(CountingLimiter::failing());
        TestContainer::bind::<dyn RateLimiterDriver>(limiter.clone());

        let error = Auth::magic_link()
            .send("present@example.test", "https://example.test/magic")
            .await
            .expect_err("limiter backend failure must reject the route");

        assert!(
            matches!(
                error,
                FrameworkError::Domain {
                    status_code: 503,
                    ..
                }
            ),
            "a limiter backend failure must surface as a fail-closed 503"
        );
        assert_eq!(
            limiter.keys().len(),
            1,
            "the route must consult the limiter once"
        );
    })
    .await;
}
