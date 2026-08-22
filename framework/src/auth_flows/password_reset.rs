//! `PasswordReset` — provider-backed password-reset facade.
//!
//! Mints, checks, and consumes reset tokens through the installed
//! [`MagnetarPasswordAuthEngine`](crate::magnetar_integration::engine::MagnetarPasswordAuthEngine),
//! which owns the application token and credential stores, and dispatches the
//! reset and changed emails through Suprnova's [`crate::Mail`] facade.
//!
//! [`PasswordReset::send_link`] dispatches [`crate::auth_flows::PasswordResetMail`]
//! and fires [`crate::auth_flows::events::PasswordResetLinkSent`].
//! [`PasswordReset::complete`] rotates the password, revokes every session and
//! remember-me token for the user, dispatches
//! [`crate::auth_flows::PasswordChangedMail`] as a fire-and-forget security
//! notification, and fires
//! [`crate::auth_flows::events::PasswordResetCompleted`].
//!
//! # No global auth instance
//!
//! Tokens live in the framework's own `auth_flow_tokens` table, not in any
//! particular auth backend, and the user lookup goes through whichever
//! [`UserProvider`](crate::auth::UserProvider) the app registered (the same one
//! [`Auth::user`](crate::auth::Auth::user) resolves against). There is no
//! global-instance initialization step and no provider-specific coupling — a
//! `send_link` / `complete` work purely by email and token.
//!
//! # Anti-enumeration semantics
//!
//! [`PasswordReset::send_link`] is anti-enumeration: callers cannot distinguish
//! "email exists" from "email does not exist" through the return type or
//! through whether mail was dispatched. When the email is absent **no token is
//! minted and no mail is sent**, and the absence is **not** leaked through an
//! `Err` — the method still returns `Ok(())`. The
//! [`crate::auth_flows::events::PasswordResetLinkSent`] event is likewise not
//! fired for an absent email, so a listener that counts events cannot
//! distinguish absent addresses.
//!
//! # Failure semantics on `complete()`
//!
//! The token is consumed (the single-use stamp) and the provider's
//! `set_password` both happen before sessions are revoked, before the
//! security-notification email is dispatched, and before the
//! [`crate::auth_flows::events::PasswordResetCompleted`] event fires. A
//! revocation failure, a mail-transport failure, or a listener panic therefore
//! cannot un-reset the password. [`PasswordReset::complete`] logs those
//! failures via tracing and discards them (and discards the event-dispatch
//! error) — a side-effect on a notification path must never roll back a
//! successful reset. [`PasswordReset::complete_with_outcome`] runs the exact
//! same steps but returns a [`PasswordResetOutcome`] so a caller that needs
//! to alert or retry on a revocation failure doesn't have to scrape logs for
//! it (SEC-02(d)).

use crate::auth_flows::mail::{PasswordChangedMail, PasswordResetMail};
use crate::error::FrameworkError;
use crate::mail::Mail;
use secrecy::{ExposeSecret, SecretString};

/// Facade for password-reset token operations.
///
/// All methods operate over the framework's `auth_flow_tokens` table and the
/// application's configured [`UserProvider`](crate::auth::UserProvider) — no
/// global auth instance to initialise first. Mail goes out through the
/// [`crate::Mail`] facade.
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
///     // e.g. page an operator — the password rotated, but a stolen
///     // credential may still have a live session or remember-me cookie.
/// }
/// # Ok(()) }
/// ```
pub struct PasswordReset;

/// Outcome of a [`PasswordReset::complete_with_outcome`] call.
///
/// By the time this value exists the password rotation itself has
/// already succeeded — a rotation failure returns `Err` from
/// `complete_with_outcome` before any `PasswordResetOutcome` is
/// constructed. These fields report whether the two FOLLOW-UP
/// revocation steps also succeeded, so a caller that cares can alert or
/// retry instead of relying solely on the `warn!` log lines
/// [`PasswordReset::complete`] leaves as its only trace of a failure.
///
/// # Security — SEC-02(d)
///
/// [`PasswordReset::complete`] discarded both revocation outcomes
/// (success or failure) into `tracing` only, and only logged the
/// success case when the revoked count was greater than zero — so a
/// revocation that silently no-op'd (e.g. the SEC-02(b) container-
/// binding bug) was completely invisible to the caller and to a `n > 0`
/// log-scraping alert alike. `complete_with_outcome` surfaces both
/// outcomes directly; `complete` still logs exactly as before and
/// simply discards the detail for callers that don't need it.
#[derive(Debug)]
pub struct PasswordResetOutcome {
    /// The id of the user whose password was rotated.
    pub user_id: String,
    /// Result of revoking every session row for `user_id` via
    /// [`crate::session::destroy_all_for_user`]. `Ok(n)` is the number
    /// of rows revoked — zero is a legitimate outcome (the user simply
    /// had no other active sessions), not a failure signal on its own.
    pub sessions_revoked: Result<u64, FrameworkError>,
    /// Result of revoking every remember-me token row for `user_id` via
    /// [`crate::auth::remember::revoke_all_for_user`]. Same `Ok(n)` /
    /// `Err` shape as [`Self::sessions_revoked`].
    pub remember_tokens_revoked: Result<u64, FrameworkError>,
}

impl PasswordReset {
    /// Send a password-reset link by email — the anti-enumeration entry point.
    ///
    /// Looks the user up through the active
    /// [`UserProvider`](crate::auth::UserProvider) and only mints + sends a
    /// token when an account is on file. An unknown email is a silent no-op: no
    /// token is issued, no mail is dispatched, no
    /// [`crate::auth_flows::events::PasswordResetLinkSent`] event fires, and the
    /// method still returns `Ok(())` so a caller (and a network observer) cannot
    /// distinguish "no such account" from "link sent."
    ///
    /// The reset URL has the shape `{base_url}?token={plaintext_token}` (a
    /// trailing slash on `base_url` is trimmed first; an existing query string
    /// gets `&` instead of `?`). The token uses
    /// [`MagnetarPasswordAuthEngine::issue_password_reset`](crate::magnetar_integration::engine::MagnetarPasswordAuthEngine::issue_password_reset)'s
    /// 15-minute TTL.
    ///
    /// On the on-file path, fires
    /// [`crate::auth_flows::events::PasswordResetLinkSent`]. The dispatch is
    /// best-effort: a listener panic or transient dispatcher error is discarded
    /// (the token is already minted) and does not surface as an `Err`.
    ///
    /// Reads `APP_NAME` (defaults to `"Suprnova"`) and `MAIL_FROM` (required —
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
        let engine = crate::magnetar_integration::password_engine()?;
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
        crate::magnetar_integration::password_engine()?
            .check_password_reset(SecretString::from(token.to_owned()))
            .await
            .map_err(map_magnetar_reset_service_error)
    }

    /// Consume `token` (single-use) and rotate the user's password to
    /// `new_password`, returning the user's id.
    ///
    /// Side effects, in order:
    ///
    /// 1. The token is consumed (single-use; a second `complete` on the same
    ///    token returns an error) and the new password is hashed with
    ///    [`crate::hashing::hash`] and stored through the active
    ///    [`UserProvider`](crate::auth::UserProvider). The provider stores the
    ///    value verbatim, so the facade hashes before handing it over.
    /// 2. Every session row and every remember-me row for the user is revoked.
    ///    A stolen session must not outlive the credential it depended on. Both
    ///    are best-effort: failures log via `tracing` but do **not** roll back
    ///    the committed password change.
    /// 3. A [`PasswordChangedMail`] security notification is dispatched through
    ///    the [`Mail`] facade, addressed via the provider's
    ///    [`flow_user_by_id`](crate::auth::UserProvider::flow_user_by_id). If
    ///    the user vanished or the send fails, the failure is logged and the
    ///    method proceeds — the password is already rotated.
    /// 4. A [`crate::auth_flows::events::PasswordResetCompleted`] event is fired.
    ///    A dispatcher error is discarded (the dispatcher logs listener errors
    ///    via its own tracing instrumentation).
    ///
    /// Reads `APP_NAME` (defaults to `"Suprnova"`) and `MAIL_FROM` (required for
    /// the notification — a missing `MAIL_FROM` only skips the best-effort
    /// notification; the password change itself still commits).
    ///
    /// # Errors
    ///
    /// - [`crate::FrameworkError::bad_request`] (400) when `new_password` is
    ///   empty/whitespace, or when the token is invalid, already consumed, or
    ///   expired.
    /// - Whatever the provider returns from `set_password` when the storage
    ///   layer fails.
    /// - The "no provider configured" error from the active-user-provider
    ///   resolver when no `UserProvider` is registered.
    ///
    /// A session-revocation or remember-me-revocation failure does
    /// **not** surface as an `Err` here — see
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
    /// error semantics — they are identical here; this method differs
    /// only in what it returns on success. Both revocation steps are
    /// still logged exactly as [`Self::complete`] logs them (`info!` on
    /// a nonzero revoked count, `warn!` on failure), so existing
    /// log-based monitoring is unaffected by which entry point a
    /// caller uses.
    ///
    /// # Errors
    ///
    /// Identical to [`Self::complete`] — a revocation failure is
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
        let engine = crate::magnetar_integration::password_engine()?;
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
