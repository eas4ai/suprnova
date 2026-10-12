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
/// queue it when a revocation after the password change fails. Queue it
/// yourself after any other credential change whose revocation failed:
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
/// after the credentials are gone succeeds and revokes nothing. A retry
/// that overlaps another run, or a run `queue:retry` starts again from
/// `failed_jobs`, is safe for the same reason.
///
/// The framework registers the job with the worker, as it registers
/// [`SendMailJob`](crate::mail::SendMailJob), so `queue:work` runs it with no
/// [`register_job`](crate::queue::worker::register_job) call.
///
/// # Retry policy
///
/// - **24 tries.** A refused revocation is almost always an outage of the
///   database or the session backend. The first retry waits about 10
///   seconds and each later one twice as long, up to one hour, so the job
///   covers a short blip quickly and keeps trying through an outage of
///   about half a day without loading a store that is already struggling.
///   After the last try the job lands in `failed_jobs`, where an operator
///   sees it and `queue:retry` can run it again.
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

    fn max_tries() -> u32 {
        24
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
