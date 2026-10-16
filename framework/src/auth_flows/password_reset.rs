//! Magnetar-first password-reset facade with a verified-provider fallback.
//!
//! When a [`MagnetarPasswordAuthEngine`](crate::magnetar_integration::engine::MagnetarPasswordAuthEngine)
//! is installed, reset issuance and completion use Magnetar's atomic
//! first-email-proof, auth-epoch, credential-cleanup, and revocation policy.
//! Without an engine, an explicitly reset-capable [`UserProvider`] may reset
//! already verified accounts through the framework's `auth_flow_tokens` table.
//! Unverified provider accounts receive no link: only Magnetar can safely make
//! password reset their atomic first mailbox proof.
//!
//! Mail and lifecycle events remain framework-owned in both modes.
//!
//! # Anti-enumeration semantics
//!
//! [`PasswordReset::send_link`] returns `Ok(())` for an unknown email. No token,
//! mail, or event is created, but the return shape does not reveal account
//! existence. Every answer, for a known address and an unknown one, a refusal
//! and an error alike, is held until `PASSWORD_RESET_TIMEBOX_MS` (200 ms by
//! default) has passed since the call began, as Laravel's broker answers
//! inside its `Timebox`, so the time an answer takes does not reveal account
//! existence either.
//!
//! # Completion ordering
//!
//! Magnetar commits its transaction before framework notifications. The
//! verified-provider fallback consumes its framework token, rechecks mailbox
//! verification, rotates the provider password, and then attempts framework
//! session and remember revocation before notifications. Its revocation
//! outcomes remain explicit because a generic provider cannot join those
//! framework stores into the provider's password transaction. When either
//! revocation fails it queues [`RevokeUserCredentials`] before the
//! notifications, so a queue worker retries both until both stores confirm.
//! When that push fails too, nothing would retry them, so the completion
//! returns an error and sends no notification.

use crate::auth::{AuthFlowUser, UserProvider, active_user_provider};
use crate::auth_flows::mail::{PasswordChangedMail, PasswordResetMail};
use crate::auth_flows::revoke_user_credentials::RevokeUserCredentials;
use crate::auth_flows::token_store::{TokenPurpose, TokenStore};
use crate::error::FrameworkError;
use crate::mail::Mail;
use secrecy::{ExposeSecret, SecretString};
use std::sync::Arc;

/// Password-reset facade for Magnetar and verified provider-backed users.
///
/// An installed Magnetar engine is authoritative. Otherwise the active
/// provider must opt in through [`UserProvider::supports_password_reset`] and
/// return only verified users from
/// [`UserProvider::retrieve_verified_user_for_password_reset`].
///
/// # Example
///
/// ```rust,no_run
/// use suprnova::auth_flows::PasswordReset;
///
/// # async fn ex() -> Result<(), Box<dyn std::error::Error>> {
/// # let token_from_query = String::new();
/// # let new_password = String::new();
/// // From the "forgot password" form (anti-enumeration: an unknown address
/// // silently sends nothing):
/// PasswordReset::send_link("alice@example.com", "https://example.com/reset").await?;
///
/// // From the click-through handler, after the user enters a new password:
/// let user_id = PasswordReset::complete(&token_from_query, &new_password).await?;
///
/// // Or, for a caller that reports a revocation failure instead of
/// // relying on the `warn!` log line (SEC-02(d)):
/// let outcome = PasswordReset::complete_with_outcome(&token_from_query, &new_password).await?;
/// if outcome.sessions_revoked.is_err() || outcome.remember_tokens_revoked.is_err() {
///     // The password rotated, but a stolen credential may still have a
///     // live session or remember-me cookie until the queued
///     // `RevokeUserCredentials` retry succeeds. A retry that could not be
///     // queued is the error the `?` above returned.
/// }
/// # Ok(()) }
/// ```
pub struct PasswordReset;

/// Outcome of a [`PasswordReset::complete_with_outcome`] call.
///
/// By the time this value exists the password rotation has succeeded. Magnetar
/// returns committed revocation counts. The verified-provider fallback returns
/// the actual `Result` from each framework revocation store so callers can
/// alert or retry when a stolen session or remember credential may remain.
///
/// [`PasswordReset::complete`] keeps the historical convenience shape and
/// returns only the user id; callers that require revocation evidence use
/// `complete_with_outcome`.
#[derive(Debug)]
pub struct PasswordResetOutcome {
    /// The id of the user whose password was rotated.
    pub user_id: String,
    /// Result of revoking every session in which the default guard is
    /// signed in as `user_id`, via [`crate::session::destroy_all_for_user`].
    /// `Ok(n)` is the number of sessions revoked - zero is a legitimate
    /// outcome (the user simply had no other active sessions), not a
    /// failure signal on its own.
    pub sessions_revoked: Result<u64, FrameworkError>,
    /// Result of revoking every remember-me token row for `user_id` via
    /// [`crate::auth::remember::revoke_all_for_user`]. Same `Ok(n)` /
    /// `Err` shape as [`Self::sessions_revoked`].
    pub remember_tokens_revoked: Result<u64, FrameworkError>,
    /// What the reset did about a failed revocation:
    /// [`RevocationRetry::Queued`] when it queued [`RevokeUserCredentials`]
    /// for `user_id` before returning, [`RevocationRetry::NotNeeded`] when
    /// both revocations succeeded. A reset whose retry could not be queued
    /// returns an error instead of an outcome, so no outcome reports one.
    pub revocation_retry: RevocationRetry,
}

/// Whether a password reset queued the retry of a failed revocation.
///
/// Read from [`PasswordResetOutcome::revocation_retry`]. The retry is a
/// [`RevokeUserCredentials`] job for the user, which a queue worker runs
/// until both stores confirm the revocations. There is no variant for a
/// retry that could not be queued: nothing would retry the revocations
/// then, so [`PasswordReset::complete_with_outcome`] returns an error in
/// place of the outcome.
#[derive(Debug)]
pub enum RevocationRetry {
    /// Nothing to retry: both revocations succeeded, or Magnetar committed
    /// them in the reset's own transaction.
    NotNeeded,
    /// A revocation failed and the retry was queued.
    Queued,
}

impl PasswordReset {
    /// Send a password-reset link by email - the anti-enumeration entry point.
    ///
    /// Uses the installed Magnetar engine when present. Otherwise the active
    /// [`UserProvider`] must explicitly support password reset and return an
    /// already verified user. Unknown and unverified provider addresses are
    /// indistinguishable no-ops: no token, mail, or event is created.
    ///
    /// The reset URL has the shape `{base_url}?token={plaintext_token}`. Both
    /// engines use a 15-minute single-use token.
    ///
    /// On the provider path the link goes out through
    /// [`UserProvider::send_password_reset_notification`], which
    /// `EloquentUserProvider` answers by loading the model and calling its
    /// [`CanResetPassword::send_password_reset_notification`](crate::CanResetPassword::send_password_reset_notification),
    /// as Laravel's broker calls `sendPasswordResetNotification` on the user.
    /// The Magnetar path holds an address, not a model, and sends the
    /// framework's [`PasswordResetMail`].
    ///
    /// Every answer is held until `PASSWORD_RESET_TIMEBOX_MS` milliseconds
    /// (200 by default, Laravel's `auth.timebox_duration`) have passed since
    /// the call began: a known and an unknown address, the abuse refusal and
    /// every error. An unknown address therefore answers no sooner than a
    /// known one. `0` turns the hold off.
    ///
    /// On the on-file path, fires
    /// [`crate::auth_flows::events::PasswordResetLinkSent`]. The dispatch is
    /// best-effort: a listener panic or transient dispatcher error is discarded
    /// (the token is already minted) and does not surface as an `Err`.
    ///
    /// Reads `APP_NAME` (defaults to `"Suprnova"`) and `MAIL_FROM` (required -
    /// errors if unset) from the process environment. Defaulting `MAIL_FROM` to
    /// a placeholder breaks DMARC/SPF in production, so the facade fails closed
    /// instead of silently sending from a domain the operator doesn't control.
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError::param`] at once, before any lookup or mail,
    /// when `PASSWORD_RESET_TIMEBOX_MS` is not a whole number of milliseconds
    /// or is too large to add to the current time.
    pub async fn send_link(email: &str, base_url: &str) -> Result<(), FrameworkError> {
        let started = tokio::time::Instant::now();
        let timebox = crate::auth_flows::password_reset_timebox()?;
        let deadline = started.checked_add(timebox).ok_or_else(|| {
            FrameworkError::param(format!(
                "{}={} ms runs past the time the clock can hold",
                crate::auth_flows::PASSWORD_RESET_TIMEBOX_ENV,
                timebox.as_millis()
            ))
        })?;
        let answer = Self::send_link_now(email, base_url).await;
        // Laravel's `Timebox::call` sleeps out the remainder whatever the
        // callback returned or threw; this does the same for every branch.
        tokio::time::sleep_until(deadline).await;
        answer
    }

    /// The body of [`Self::send_link`], answered as soon as it is known.
    async fn send_link_now(email: &str, base_url: &str) -> Result<(), FrameworkError> {
        crate::magnetar_integration::abuse_limiter::check_auth_abuse(
            crate::magnetar_integration::abuse_limiter::AuthAbuseRoute::PasswordResetSend,
            email,
        )
        .await?;
        let from_address = crate::auth_flows::require_mail_from()?;
        let Some(engine) = crate::magnetar_integration::password_engine_if_installed()? else {
            return Self::send_provider_link(email, base_url).await;
        };
        let Some(issued) = engine
            .issue_password_reset(email)
            .await
            .map_err(map_magnetar_reset_service_error)?
        else {
            return Ok(());
        };
        let url =
            crate::auth_flows::append_token_query(base_url, issued.token.plaintext.expose_secret());
        let to_address = issued.email;
        // Greet the reader by name, as the verification and password-changed
        // mails do. Magnetar never calls an app's own reset notification, so
        // this is the only place the name can come from. A failed lookup only
        // costs the name, never the reset.
        let user_name = match engine.user_by_id(&issued.user_id).await {
            Ok(Some(user)) => user.name,
            Ok(None) => {
                tracing::warn!(
                    "password reset mail greets user {} without a name: no such user",
                    issued.user_id
                );
                None
            }
            Err(error) => {
                tracing::warn!(
                    "password reset mail greets user {} without a name: {error}",
                    issued.user_id
                );
                None
            }
        };
        let mail = PasswordResetMail {
            to_address: to_address.clone(),
            user_name,
            reset_link: url,
            app_name: crate::auth_flows::app_name(),
            from_address,
        };
        Mail::to(to_address.as_str()).send(mail).await?;
        let _ = crate::events::EventFacade::dispatch(
            crate::auth_flows::events::PasswordResetLinkSent {
                user_id: issued.user_id,
                email: to_address,
            },
        )
        .await;
        Ok(())
    }

    /// Check whether `token` is a live, unused reset token without consuming it.
    ///
    /// Useful for landing pages that want to confirm the token before rendering
    /// the new-password form, so a refresh does not burn the token.
    pub async fn check(token: &str) -> Result<bool, FrameworkError> {
        if let Some(engine) = crate::magnetar_integration::password_engine_if_installed()? {
            return engine
                .check_password_reset(SecretString::from(token.to_owned()))
                .await
                .map_err(map_magnetar_reset_service_error);
        }
        TokenStore::check(token, TokenPurpose::PasswordReset).await
    }

    /// Consume `token` (single-use) and rotate the user's password to
    /// `new_password`, returning the user's id.
    ///
    /// With Magnetar installed, completion applies its atomic first-proof,
    /// auth-epoch, credential-cleanup, and revocation transaction. Without
    /// Magnetar, completion is available only for an explicitly reset-capable
    /// provider and a still-verified user: it consumes the framework token,
    /// hashes and persists the password, then attempts framework session and
    /// remember revocation.
    ///
    /// Both modes dispatch [`PasswordChangedMail`] and
    /// [`crate::auth_flows::events::PasswordResetCompleted`] only after the
    /// password mutation succeeds, and on the provider path only after the
    /// retry of a failed revocation is queued. Notification failures never
    /// roll the mutation back.
    ///
    /// Reads `APP_NAME` (defaults to `"Suprnova"`) and `MAIL_FROM` (required for
    /// the notification - a missing `MAIL_FROM` only skips the best-effort
    /// notification; the password change itself still commits).
    ///
    /// # Errors
    ///
    /// - [`crate::FrameworkError::bad_request`] (400) when `new_password` is
    ///   empty/whitespace, or when the token is invalid, already consumed, or
    ///   expired.
    /// - A provider capability/configuration error when no Magnetar engine is
    ///   installed.
    /// - Whatever a reset-capable provider returns while rechecking verification
    ///   or persisting the password.
    /// - [`crate::FrameworkError::internal`] (500) when a session or
    ///   remember-me revocation failed and queuing [`RevokeUserCredentials`]
    ///   failed too. The password has changed by then, and nothing would
    ///   retry the revocations, so the reset must not answer as completed.
    ///   The message names the user, each failed revocation with its error,
    ///   the push's error, and the job to queue once the queue works. The
    ///   error is logged at error level with the user's id, and no
    ///   [`PasswordChangedMail`] or `PasswordResetCompleted` goes out.
    ///
    /// A session-revocation or remember-me-revocation failure whose retry
    /// was queued does **not** surface as an `Err` here - see
    /// [`Self::complete_with_outcome`] for a sibling that reports those
    /// outcomes to the caller (SEC-02(d)) instead of only logging them.
    /// The queued [`RevokeUserCredentials`] retries the revocations
    /// although this caller never sees the outcome.
    pub async fn complete(token: &str, new_password: &str) -> Result<String, FrameworkError> {
        Self::complete_with_outcome(token, new_password)
            .await
            .map(|outcome| outcome.user_id)
    }

    /// Same rotation as [`Self::complete`], but returns a
    /// [`PasswordResetOutcome`] carrying the session- and remember-me-
    /// revocation results instead of discarding them into `tracing`
    /// alone.
    ///
    /// See [`Self::complete`] for the full side-effect ordering and
    /// error semantics - they are identical here; this method differs
    /// only in what it returns on success. Both revocation steps are
    /// still logged exactly as [`Self::complete`] logs them (`info!` on
    /// a nonzero revoked count, `warn!` on failure), so existing
    /// log-based monitoring is unaffected by which entry point a
    /// caller uses.
    ///
    /// When either revocation fails, the completion pushes a
    /// [`RevokeUserCredentials`] job for the user with
    /// [`Queue::push`](crate::queue::Queue::push) before it returns, and
    /// [`PasswordResetOutcome::revocation_retry`] is
    /// [`RevocationRetry::Queued`]. A worker running `queue:work` retries
    /// both revocations until both stores confirm them. When the push
    /// fails too, nothing would retry them, so the completion returns an
    /// error instead of the outcome, although the password has changed.
    ///
    /// # Errors
    ///
    /// Identical to [`Self::complete`]. A revocation failure whose retry
    /// was queued is reported through the returned
    /// [`PasswordResetOutcome`], not through this method's `Result`; one
    /// whose retry could not be queued is an `Err`.
    pub async fn complete_with_outcome(
        token: &str,
        new_password: &str,
    ) -> Result<PasswordResetOutcome, FrameworkError> {
        if new_password.trim().is_empty() {
            return Err(FrameworkError::bad_request(
                "new_password must not be empty",
            ));
        }
        let Some(engine) = crate::magnetar_integration::password_engine_if_installed()? else {
            return Self::complete_with_provider(token, new_password).await;
        };
        let commit = engine
            .complete_password_reset(
                SecretString::from(token.to_owned()),
                SecretString::from(new_password.to_owned()),
            )
            .await
            .map_err(map_magnetar_reset_completion_error)?;
        let id = commit.user_id.clone();

        match engine
            .user_by_id(&id)
            .await
            .map_err(map_magnetar_reset_service_error)
        {
            Ok(Some(user)) => match crate::auth_flows::require_mail_from() {
                Ok(from_address) => {
                    let to_address = user.email;
                    let mail = PasswordChangedMail {
                        to_address: to_address.clone(),
                        user_name: user.name,
                        app_name: crate::auth_flows::app_name(),
                        from_address,
                    };
                    if let Err(error) = Mail::to(to_address.as_str()).send(mail).await {
                        tracing::warn!(
                            "password-changed security notification failed for user {id}: {error}"
                        );
                    }
                }
                Err(error) => tracing::warn!(
                    "password-changed security notification skipped for user {id}: {error}"
                ),
            },
            Ok(None) => tracing::warn!(
                "password-changed security notification skipped: user {id} not found after reset"
            ),
            Err(error) => tracing::warn!(
                "password-changed security notification skipped for user {id}: lookup failed: {error}"
            ),
        }
        let _ = crate::events::EventFacade::dispatch(
            crate::auth_flows::events::PasswordResetCompleted {
                user_id: id.clone(),
            },
        )
        .await;
        Ok(PasswordResetOutcome {
            user_id: id,
            sessions_revoked: Ok(commit.revoked_sessions),
            remember_tokens_revoked: Ok(commit.remember_rows_revoked),
            revocation_retry: RevocationRetry::NotNeeded,
        })
    }

    fn provider_for_password_reset() -> Result<Arc<dyn UserProvider>, FrameworkError> {
        let provider = active_user_provider()?;
        if !provider.supports_password_reset() {
            return Err(FrameworkError::internal(
                "the active user provider does not support password reset",
            ));
        }
        Ok(provider)
    }

    /// Mint a framework reset token for the verified user `email` names and
    /// send the link through the provider's
    /// [`UserProvider::send_password_reset_notification`], which holds the
    /// model when the provider has one.
    async fn send_provider_link(email: &str, base_url: &str) -> Result<(), FrameworkError> {
        let provider = Self::provider_for_password_reset()?;
        let Some(user) = provider
            .retrieve_verified_user_for_password_reset(email)
            .await?
        else {
            return Ok(());
        };
        let token = TokenStore::issue(
            &user.id,
            TokenPurpose::PasswordReset,
            TokenPurpose::PasswordReset.default_ttl(),
        )
        .await?;
        let reset_link = crate::auth_flows::append_token_query(base_url, &token);
        provider
            .send_password_reset_notification(&user.id, &reset_link)
            .await?;
        let _ = crate::events::EventFacade::dispatch(
            crate::auth_flows::events::PasswordResetLinkSent {
                user_id: user.id,
                email: user.email,
            },
        )
        .await;
        Ok(())
    }

    async fn complete_with_provider(
        token: &str,
        new_password: &str,
    ) -> Result<PasswordResetOutcome, FrameworkError> {
        let provider = Self::provider_for_password_reset()?;
        let id = TokenStore::consume(token, TokenPurpose::PasswordReset)
            .await?
            .ok_or_else(|| FrameworkError::bad_request("invalid or expired reset token"))?;
        if !provider.is_email_verified(&id).await? {
            return Err(FrameworkError::bad_request(
                "invalid or expired reset token",
            ));
        }

        let password_hash = crate::hashing::hash(new_password)?;
        provider.set_password(&id, &password_hash).await?;

        let sessions_revoked = match crate::session::destroy_all_for_user(&id).await {
            Ok(count) => {
                if count > 0 {
                    tracing::info!(
                        "revoked {count} session row(s) for user {id} after password reset"
                    );
                }
                Ok(count)
            }
            Err(error) => {
                tracing::warn!(
                    "session revocation failed for user {id} after password reset: {error}"
                );
                Err(error)
            }
        };
        let remember_tokens_revoked = match crate::auth::remember::revoke_all_for_user(&id).await {
            Ok(count) => {
                if count > 0 {
                    tracing::info!(
                        "revoked {count} remember-me row(s) for user {id} after password reset"
                    );
                }
                Ok(count)
            }
            Err(error) => {
                tracing::warn!(
                    "remember-me revocation failed for user {id} after password reset: {error}"
                );
                Err(error)
            }
        };
        // Before the notifications: a reset whose retry could not be queued
        // returns its error here and sends neither.
        let revocation_retry =
            queue_revocation_retry(&id, &sessions_revoked, &remember_tokens_revoked).await?;

        match provider.flow_user_by_id(&id).await {
            Ok(Some(AuthFlowUser { email, name, .. })) => {
                match crate::auth_flows::require_mail_from() {
                    Ok(from_address) => {
                        let mail = PasswordChangedMail {
                            to_address: email.clone(),
                            user_name: name,
                            app_name: crate::auth_flows::app_name(),
                            from_address,
                        };
                        if let Err(error) = Mail::to(email.as_str()).send(mail).await {
                            tracing::warn!(
                                "password-changed security notification failed for user {id}: {error}"
                            );
                        }
                    }
                    Err(error) => tracing::warn!(
                        "password-changed security notification skipped for user {id}: {error}"
                    ),
                }
            }
            Ok(None) => tracing::warn!(
                "password-changed security notification skipped: user {id} not found after reset"
            ),
            Err(error) => tracing::warn!(
                "password-changed security notification skipped for user {id}: lookup failed: {error}"
            ),
        }
        let _ = crate::events::EventFacade::dispatch(
            crate::auth_flows::events::PasswordResetCompleted {
                user_id: id.clone(),
            },
        )
        .await;
        Ok(PasswordResetOutcome {
            user_id: id,
            sessions_revoked,
            remember_tokens_revoked,
            revocation_retry,
        })
    }
}

/// Queue [`RevokeUserCredentials`] for `user_id` when either revocation
/// failed, and say whether it was needed.
///
/// # Errors
///
/// [`FrameworkError::internal`] when a revocation failed and the push failed
/// too. Nothing would retry the revocations then, so the reset must not
/// answer as completed. The password has already changed, so the error says
/// so, and it is logged at error level with the user's id: an operator has
/// to queue the job once the queue works.
async fn queue_revocation_retry(
    user_id: &str,
    sessions_revoked: &Result<u64, FrameworkError>,
    remember_tokens_revoked: &Result<u64, FrameworkError>,
) -> Result<RevocationRetry, FrameworkError> {
    if sessions_revoked.is_ok() && remember_tokens_revoked.is_ok() {
        return Ok(RevocationRetry::NotNeeded);
    }
    let push_error = match crate::queue::Queue::push(RevokeUserCredentials::new(user_id)).await {
        Ok(()) => {
            tracing::info!(
                "queued a retry of the revocations for user {user_id} after password reset"
            );
            return Ok(RevocationRetry::Queued);
        }
        Err(error) => error,
    };
    let failed_revocations = [
        ("sessions", sessions_revoked),
        ("remember-me tokens", remember_tokens_revoked),
    ]
    .into_iter()
    .filter_map(|(credentials, revoked)| {
        revoked
            .as_ref()
            .err()
            .map(|error| format!("revoking the {credentials} failed: {error}"))
    })
    .collect::<Vec<_>>()
    .join("; ");
    let message = format!(
        "the password of user {user_id} changed, but {failed_revocations}; and the retry of the \
         revocations could not be queued: {push_error}. No password-changed mail or \
         PasswordResetCompleted event was sent for this reset. Queue \
         RevokeUserCredentials::new(\"{user_id}\") once the queue works"
    );
    tracing::error!(user_id, "password reset: {message}");
    Err(FrameworkError::internal(message))
}

fn map_magnetar_reset_service_error(error: magnetar::Error) -> FrameworkError {
    FrameworkError::internal(format!("Magnetar password reset: {error}"))
}

fn map_magnetar_reset_completion_error(error: magnetar::Error) -> FrameworkError {
    match error {
        magnetar::Error::InvalidInput { .. }
        | magnetar::Error::NotFound { .. }
        | magnetar::Error::Conflict { .. } => {
            FrameworkError::bad_request("invalid or expired reset token")
        }
        other => map_magnetar_reset_service_error(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The provider-backed paths (`send_link` / `complete`) need a real
    // `UserProvider` + DB and are covered by the integration test in
    // `framework/tests/password_reset.rs`. The one branch that needs no setup is
    // the empty-password guard in `complete`: it returns `bad_request` before
    // touching the token store or the provider, so it can be exercised here.
    #[tokio::test]
    async fn complete_rejects_empty_password_before_touching_the_store() {
        assert!(
            PasswordReset::complete("any-token", "   ").await.is_err(),
            "an empty/whitespace password must be rejected up front"
        );
        assert!(
            PasswordReset::complete("any-token", "").await.is_err(),
            "an empty password must be rejected up front"
        );
    }
}
