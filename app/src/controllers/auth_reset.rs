//! Phase 11 dogfood - password reset.
//!
//! Two handlers:
//!
//! - `POST /auth/password/request` - body is form-urlencoded
//!   `email=...`. Calls `PasswordReset::send_link` (anti-enumeration:
//!   always returns 200, dispatches mail only when the email is on
//!   file - the facade itself enforces this).
//! - `POST /auth/password/reset` - body is form-urlencoded
//!   `token=...&new_password=...`. Calls
//!   `PasswordReset::complete_with_outcome` and redirects to `/?reset=ok`
//!   only when the password changed and the user's other sessions and
//!   remember-me tokens were revoked. When a revocation fails, the
//!   framework has queued `RevokeUserCredentials` for the user by the time
//!   the outcome comes back, and the handler logs that before it answers
//!   the error. When the retry could not be queued, the framework returns
//!   an error in place of the outcome, which the handler answers as well.
//!
//! Both endpoints are public - they consume tokens minted out-of-
//! band, so they don't sit behind `SessionAuthMiddleware`. Token
//! validity is the auth check.

use serde::Deserialize;
use suprnova::auth_flows::{PasswordReset, RevocationRetry};
use suprnova::{FrameworkError, HttpResponse, Request, Response};

/// Body for `POST /auth/password/request`.
#[derive(Deserialize)]
pub struct RequestResetForm {
    pub email: String,
}

/// Body for `POST /auth/password/reset`.
#[derive(Deserialize)]
pub struct CompleteResetForm {
    pub token: String,
    pub new_password: String,
}

/// `POST /auth/password/request` - start a reset.
///
/// The `PasswordReset::send_link` facade is already anti-enumeration:
/// it returns `Ok(())` regardless of whether the email maps to a
/// real user, and only dispatches a `PasswordResetMail` when it
/// does. We don't need to add a second guard - surfacing the
/// facade's outcome directly is the right thing here.
pub async fn request_reset(req: Request) -> Response {
    request_reset_inner(req).await.map_err(HttpResponse::from)
}

async fn request_reset_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    let form: RequestResetForm = req.form().await?;

    let base = format!(
        "{}/auth/password/reset",
        std::env::var("APP_URL").unwrap_or_else(|_| "http://localhost:8080".into())
    );
    PasswordReset::send_link(&form.email, &base).await?;

    Ok(HttpResponse::text(
        "If this email is on file, a password reset link has been sent.",
    ))
}

/// The message of the error a failed revocation answers with, for the user
/// whose password changed. The framework's error path logs it, and a debug
/// build shows it as `debug_message`; a production `500` body stays
/// generic.
const REVOCATION_FAILED: &str = "Your password was changed, but signing you out of your \
    other devices failed. Sign in again on every device, and contact support.";

/// `POST /auth/password/reset` - consume the token, apply the new
/// password, and revoke the user's other sessions and remember-me tokens.
///
/// The redirect to `/?reset=ok` means all three happened. A changed
/// password is not enough while a stolen session or remember-me cookie
/// still works, so a revocation that fails is logged at error level with
/// the user's id and answered as a `500` through the framework's error
/// path. The password stays changed. The framework queued
/// `RevokeUserCredentials` for the user before the outcome came back, so a
/// queue worker retries both revocations until both stores confirm them;
/// the handler logs the queued retry with the user's id. When that push
/// failed, `complete_with_outcome` returns the framework's error instead,
/// already logged at error level with the user's id, and the `?` answers
/// it with the same `500`.
///
/// On success the facade also dispatches a `PasswordChangedMail`
/// security notification and fires `PasswordResetCompleted`. Both
/// are fire-and-forget - a transport / listener failure does not
/// roll back the password rotation (matching the facade's documented
/// failure semantics).
pub async fn complete_reset(req: Request) -> Response {
    complete_reset_inner(req).await.map_err(HttpResponse::from)
}

async fn complete_reset_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    let form: CompleteResetForm = req.form().await?;

    if form.new_password.trim().is_empty() {
        return Err(FrameworkError::bad_request(
            "new_password must not be empty",
        ));
    }

    let outcome = PasswordReset::complete_with_outcome(&form.token, &form.new_password).await?;
    // Both are checked, so a failure of each one is logged.
    let sessions_failed = revocation_failed(
        &outcome.user_id,
        "other sessions",
        &outcome.sessions_revoked,
    );
    let remember_failed = revocation_failed(
        &outcome.user_id,
        "remember-me tokens",
        &outcome.remember_tokens_revoked,
    );
    if sessions_failed || remember_failed {
        report_revocation_retry(&outcome.user_id, &outcome.revocation_retry);
        return Err(FrameworkError::internal(REVOCATION_FAILED));
    }

    // 302 → /?reset=ok . Built directly so the inner function's
    // `Result<HttpResponse, ...>` shape composes - see the matching
    // comment in `auth_verify::verify_inner` for the rationale.
    Ok(HttpResponse::new()
        .status(302)
        .header("Location", "/?reset=ok"))
}

/// Log `revoked` at error level with the user's id when it failed, and say
/// whether it did.
fn revocation_failed(
    user_id: &str,
    credentials: &str,
    revoked: &Result<u64, FrameworkError>,
) -> bool {
    match revoked {
        Ok(_) => false,
        Err(error) => {
            tracing::error!(
                user_id,
                error = %error,
                "password reset: the password changed, but revoking the user's {credentials} failed"
            );
            true
        }
    }
}

/// Log, with the user's id, the retry the framework queued for a failed
/// revocation. A retry it could not queue never reaches here: it is the
/// error `complete_with_outcome` returned.
fn report_revocation_retry(user_id: &str, retry: &RevocationRetry) {
    match retry {
        RevocationRetry::Queued => tracing::warn!(
            user_id,
            "password reset: queued a retry of the failed revocations"
        ),
        // The framework reports `NotNeeded` only when both revocations
        // succeeded, and this is called only when one failed.
        RevocationRetry::NotNeeded => {}
    }
}
