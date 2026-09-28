//! Adapter between Magnetar's abuse contract and Suprnova rate-limit drivers.
//!
//! The route helpers in this module are deliberately used before any account
//! lookup or token issuance. They use opaque, purpose-scoped keys so a shared
//! Redis limiter never stores a raw mailbox, token, or provider value.

use std::sync::Arc;
use std::time::Duration;

use crate::container::App;
use crate::error::FrameworkError;
use crate::rate_limit::{RateLimiterDriver, SlidingWindowConfig};

/// The authentication start route whose budget is being consumed.
#[derive(Clone, Copy)]
pub(crate) enum AuthAbuseRoute {
    MagicLinkSend,
    PasswordResetSend,
    EmailVerificationSend,
    EmailVerificationResend,
    PasswordRegister,
}

impl AuthAbuseRoute {
    fn purpose(self) -> &'static str {
        match self {
            Self::MagicLinkSend => "magic-link-send",
            Self::PasswordResetSend => "password-reset-send",
            Self::EmailVerificationSend => "email-verification-send",
            Self::EmailVerificationResend => "email-verification-resend",
            Self::PasswordRegister => "password-register",
        }
    }

    fn policy(self) -> SlidingWindowConfig {
        let (max_requests, window) = match self {
            Self::MagicLinkSend
            | Self::PasswordResetSend
            | Self::EmailVerificationSend
            | Self::EmailVerificationResend => (3, Duration::from_secs(60 * 60)),
            Self::PasswordRegister => (10, Duration::from_secs(60 * 60)),
        };
        SlidingWindowConfig {
            max_requests,
            window,
        }
    }

    fn key(self, identity: &str) -> String {
        use sha2::{Digest, Sha256};

        let normalized = identity.trim().to_lowercase();
        let digest = Sha256::digest(normalized.as_bytes());
        use std::fmt::Write as _;

        let mut opaque_identity = String::with_capacity(32);
        for byte in &digest[..16] {
            let _ = write!(&mut opaque_identity, "{byte:02x}");
        }
        format!("auth:{}:{opaque_identity}", self.purpose())
    }
}

enum DriverPermit {
    Allowed,
    Rejected(Duration),
}

async fn acquire_from_driver(
    driver: &dyn RateLimiterDriver,
    key: &str,
    config: &SlidingWindowConfig,
) -> Result<DriverPermit, ()> {
    match driver.try_acquire(key, config).await {
        Ok(true) => Ok(DriverPermit::Allowed),
        Ok(false) => {
            let retry_after = driver
                .retry_after(key, config)
                .await
                .map_err(|_| ())?
                .unwrap_or(config.window)
                .max(Duration::from_millis(1));
            Ok(DriverPermit::Rejected(retry_after))
        }
        Err(_) => Err(()),
    }
}

/// Consume a fail-closed budget before an authentication send/start operation.
pub(crate) async fn check_auth_abuse(
    route: AuthAbuseRoute,
    identity: &str,
) -> Result<(), FrameworkError> {
    let limiter = FrameworkAbuseLimiter::from_app().map_err(|_| unavailable())?;
    let config = route.policy();
    let key = route.key(identity);
    let policy = magnetar::abuse::AbusePolicy {
        max_requests: config.max_requests,
        window: config.window,
    };

    match magnetar::abuse::AbuseLimiter::acquire(&limiter, &key, policy)
        .await
        .map_err(|_| unavailable())?
    {
        magnetar::abuse::Permit::Allowed { .. } => Ok(()),
        magnetar::abuse::Permit::Rejected { .. } => Err(FrameworkError::Domain {
            message: "too many requests".to_owned(),
            status_code: 429,
        }),
    }
}

fn unavailable() -> FrameworkError {
    FrameworkError::Domain {
        message: "authentication service temporarily unavailable".to_owned(),
        status_code: 503,
    }
}

/// Magnetar's [`magnetar::abuse::AbuseLimiter`] backed by the framework's
/// configured rate-limit driver.
///
/// The implementation turns any driver failure into
/// [`magnetar::Error::DependencyUnavailable`], preserving Magnetar's
/// fail-closed contract rather than treating an unavailable Redis backend as
/// permission to continue.
pub struct FrameworkAbuseLimiter {
    driver: Arc<dyn RateLimiterDriver>,
}

impl FrameworkAbuseLimiter {
    /// Wrap an already configured framework rate-limit driver.
    pub fn new(driver: Arc<dyn RateLimiterDriver>) -> Self {
        Self { driver }
    }

    fn from_app() -> Result<Self, FrameworkError> {
        Ok(Self::new(App::resolve_make::<dyn RateLimiterDriver>()?))
    }

    fn backend_unavailable() -> magnetar::Error {
        magnetar::Error::DependencyUnavailable {
            dependency: "suprnova rate limiter".to_owned(),
            message: "the rate-limit backend could not make an abuse decision".to_owned(),
        }
    }
}

#[async_trait::async_trait]
impl magnetar::abuse::AbuseLimiter for FrameworkAbuseLimiter {
    async fn acquire(
        &self,
        key: &str,
        policy: magnetar::abuse::AbusePolicy,
    ) -> magnetar::Result<magnetar::abuse::Permit> {
        policy.validate()?;
        let config = SlidingWindowConfig {
            max_requests: policy.max_requests,
            window: policy.window,
        };

        match acquire_from_driver(self.driver.as_ref(), key, &config).await {
            Ok(DriverPermit::Allowed) => Ok(magnetar::abuse::Permit::Allowed { retry_after: None }),
            Ok(DriverPermit::Rejected(retry_after)) => {
                Ok(magnetar::abuse::Permit::Rejected { retry_after })
            }
            Err(()) => Err(Self::backend_unavailable()),
        }
    }
}
