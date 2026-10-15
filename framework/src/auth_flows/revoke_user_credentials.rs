//! The queued retry of a password reset's revocations.
//!
//! A password reset rotates the password first and then revokes the user's
//! other sessions and remember-me tokens. When a store refuses one of those
//! revocations, the reset token is already spent and the password already
//! changed, so the request cannot simply be tried again. This job makes the
//! revocation durable instead: the queue runs it until both stores confirm.

use crate::error::FrameworkError;
use crate::queue::{BackoffSchedule, Job};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Revoke every session in which the default guard is signed in as a user,
/// and every remember-me token of that user.
///
/// [`PasswordReset::complete`](crate::auth_flows::PasswordReset::complete)
/// and [`PasswordReset::complete_with_outcome`](crate::auth_flows::PasswordReset::complete_with_outcome)
/// queue it when a revocation after the password change fails, and return
/// an error naming this job when they cannot. Queue it yourself then, once
/// the queue works, and after any other credential change whose revocation
/// failed:
///
/// ```rust,no_run
/// use suprnova::Queue;
/// use suprnova::auth_flows::RevokeUserCredentials;
///
/// # async fn ex() -> Result<(), suprnova::FrameworkError> {
/// Queue::push(RevokeUserCredentials::new("42")).await?;
/// # Ok(()) }
/// ```
///
/// Each run revokes through
/// [`session::destroy_all_for_user`](crate::session::destroy_all_for_user)
/// and [`auth::remember::revoke_all_for_user`](crate::auth::remember::revoke_all_for_user),
/// both on every run, so a store that refuses does not keep the other
/// store's credentials alive. The run returns an error while either store
/// refuses, which makes the queue retry it, and succeeds once both confirm.
///
/// The job is idempotent: both revocations delete what is there, so a run
/// after the credentials are gone succeeds and revokes nothing. Running it
/// again is always safe: a retry that overlaps another run, a run
/// `queue:retry` starts again from `failed_jobs`, or a second push of it.
///
/// The framework registers the job with the worker, as it registers
/// [`SendMailJob`](crate::mail::SendMailJob), so `queue:work` runs it with no
/// [`register_job`](crate::queue::worker::register_job) call.
///
/// # Retry policy
///
/// - **No bound on the tries.** The job retries until both stores confirm:
///   [`Job::max_tries`] is zero, which the queue reads as unlimited. A
///   refused revocation is almost always an outage of the database or the
///   session backend, and while it lasts a stolen credential the store
///   still holds is not revoked, so the job does not give up. The first
///   retry waits about 10 seconds and each later one twice as long, up to
///   one hour between tries, so the job covers a short blip quickly and
///   keeps trying through an outage of any length without loading a store
///   that is already struggling.
/// - **`failed_jobs` sees it only when something else ends the retries:**
///   a worker started with `--tries`, whose budget replaces the job's own,
///   or an error the application named with
///   [`Exceptions::dont_retry`](crate::Exceptions::dont_retry) or
///   [`Exceptions::dont_retry_when`](crate::Exceptions::dont_retry_when).
///   `queue:retry` runs it again from there.
/// - **A 60-second timeout per try.** A store connection that hangs would
///   otherwise hold a worker for good. A timeout counts as a failed try and
///   is retried.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevokeUserCredentials {
    /// The id of the user whose credentials are revoked, as
    /// [`Authenticatable::get_auth_identifier`](crate::auth::Authenticatable::get_auth_identifier)
    /// returns it.
    pub user_id: String,
}

impl RevokeUserCredentials {
    /// The job that revokes the credentials of `user_id`.
    pub fn new(user_id: impl Into<String>) -> Self {
        Self {
            user_id: user_id.into(),
        }
    }
}

#[async_trait]
impl Job for RevokeUserCredentials {
    fn job_name() -> &'static str {
        "Suprnova::RevokeUserCredentials"
    }

    /// Unlimited: retried until both stores confirm.
    fn max_tries() -> u32 {
        0
    }

    fn backoff() -> BackoffSchedule {
        BackoffSchedule::Exponential {
            base_secs: 10,
            cap_secs: 3600,
            jitter_ratio: 0.25,
        }
    }

    fn timeout() -> Option<Duration> {
        Some(Duration::from_secs(60))
    }

    async fn handle(self) -> Result<(), FrameworkError> {
        let user_id = self.user_id;
        let sessions = crate::session::destroy_all_for_user(&user_id).await;
        let remember_tokens = crate::auth::remember::revoke_all_for_user(&user_id).await;
        match (sessions, remember_tokens) {
            (Ok(sessions), Ok(remember_tokens)) => {
                if sessions > 0 || remember_tokens > 0 {
                    tracing::info!(
                        user_id,
                        sessions,
                        remember_tokens,
                        "the queued revocation retry revoked the user's credentials"
                    );
                }
                Ok(())
            }
            (Err(error), Ok(_)) => {
                tracing::warn!(
                    user_id,
                    error = %error,
                    "the queued revocation retry could not revoke the user's sessions"
                );
                Err(error)
            }
            (Ok(_), Err(error)) => {
                tracing::warn!(
                    user_id,
                    error = %error,
                    "the queued revocation retry could not revoke the user's remember-me tokens"
                );
                Err(error)
            }
            (Err(sessions), Err(remember_tokens)) => {
                tracing::warn!(
                    user_id,
                    sessions = %sessions,
                    remember_tokens = %remember_tokens,
                    "the queued revocation retry could not revoke the user's sessions \
                     or remember-me tokens"
                );
                Err(FrameworkError::internal(format!(
                    "revoking the sessions failed: {sessions}; \
                     revoking the remember-me tokens failed: {remember_tokens}"
                )))
            }
        }
    }
}
