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
//! existence.
//!
//! # Completion ordering
//!
//! Magnetar commits its transaction before framework notifications. The
//! verified-provider fallback consumes its framework token, rechecks mailbox
//! verification, rotates the provider password, and then attempts framework
//! session and remember revocation before notifications. Its revocation
//! outcomes remain explicit because a generic provider cannot join those
//! framework stores into the provider's password transaction.

use crate::auth::{AuthFlowUser, UserProvider, active_user_provider};
use crate::auth_flows::mail::{PasswordChangedMail, PasswordResetMail};
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
/// // Or, for a caller that wants to alert/retry on a revocation failure
/// // instead of relying on the `warn!` log line (SEC-02(d)):
/// let outcome = PasswordReset::complete_with_outcome(&token_from_query, &new_password).await?;
/// if outcome.sessions_revoked.is_err() || outcome.remember_tokens_revoked.is_err() {
///     // e.g. page an operator - the password rotated, but a stolen
///     // credential may still have a live session or remember-me cookie.
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
    /// Result of revoking every session row for `user_id` via
    /// [`crate::session::destroy_all_for_user`]. `Ok(n)` is the number
    /// of rows revoked - zero is a legitimate outcome (the user simply
    /// had no other active sessions), not a failure signal on its own.
    pub sessions_revoked: Result<u64, FrameworkError>,
    /// Result of revoking every remember-me token row for `user_id` via
    /// [`crate::auth::remember::revoke_all_for_user`]. Same `Ok(n)` /
    /// `Err` shape as [`Self::sessions_revoked`].
    pub remember_tokens_revoked: Result<u64, FrameworkError>,
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
    /// On the on-file path, fires
    /// [`crate::auth_flows::events::PasswordResetLinkSent`]. The dispatch is
    /// best-effort: a listener panic or transient dispatcher error is discarded
    /// (the token is already minted) and does not surface as an `Err`.
    ///
    /// Reads `APP_NAME` (defaults to `"Suprnova"`) and `MAIL_FROM` (required -
    /// errors if unset) from the process environment. Defaulting `MAIL_FROM` to
    /// a placeholder breaks DMARC/SPF in production, so the facade fails closed
    /// instead of silently sending from a domain the operator doesn't control.
    pub async fn send_link(email: &str, base_url: &str) -> Result<(), FrameworkError> {
        crate::magnetar_integration::abuse_limiter::check_auth_abuse(
            crate::magnetar_integration::abuse_limiter::AuthAbuseRoute::PasswordResetSend,
            email,
        )
        .await?;
        let from_address = crate::auth_flows::require_mail_from()?;
        let Some(engine) = crate::magnetar_integration::password_engine_if_installed()? else {
            return Self::send_provider_link(email, base_url, from_address).await;
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
        let mail = PasswordResetMail {
            to_address: to_address.clone(),
            user_name: None,
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
    /// password mutation succeeds. Notification failures never roll the
    /// mutation back.
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
    ///
    /// A session-revocation or remember-me-revocation failure does
    /// **not** surface as an `Err` here - see
    /// [`Self::complete_with_outcome`] for a sibling that reports those
    /// outcomes to the caller (SEC-02(d)) instead of only logging them.
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
    /// # Errors
    ///
    /// Identical to [`Self::complete`] - a revocation failure is
    /// reported through the returned [`PasswordResetOutcome`], not
    /// through this method's `Result`.
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

    async fn send_provider_link(
        email: &str,
        base_url: &str,
        from_address: String,
    ) -> Result<(), FrameworkError> {
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
        let to_address = user.email;
        let mail = PasswordResetMail {
            to_address: to_address.clone(),
            user_name: user.name,
            reset_link,
            app_name: crate::auth_flows::app_name(),
            from_address,
        };
        Mail::to(to_address.as_str()).send(mail).await?;
        let _ = crate::events::EventFacade::dispatch(
            crate::auth_flows::events::PasswordResetLinkSent {
                user_id: user.id,
                email: to_address,
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
        })
    }
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
