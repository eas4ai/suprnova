//! Password reset.
//!
//! `PasswordReset` issues single-use links to verified accounts only, and
//! answers an unknown or unverified address exactly like a known one, so
//! the form never reveals whether an account exists. Completion rotates the
//! password and revokes the account's other sessions and remember-me
//! tokens; a revocation failure is an error here, because a changed
//! password is not enough while an old credential still works.
//!
//! Tokens live in the `auth_flow_tokens` table (see
//! `migrations::create_auth_flow_tokens_table`) and expire after 15
//! minutes. The mail goes out through the `Mail` facade with the `MAIL_*`
//! settings in `.env`.

use serde::Deserialize;
use suprnova::{
    FormRequest, FrameworkError, InertiaProps, Request, Response, Validate, ValidationErrors,
    auth_flows::PasswordReset, handler, inertia_response, redirect, url,
};

/// Where the mailed link lands: the form that takes the new password.
const RESET_PATH: &str = "/reset-password";

#[derive(InertiaProps)]
pub struct ForgotPasswordProps {}

#[derive(InertiaProps)]
pub struct ResetPasswordProps {
    /// The token from the mailed link, which the form sends back.
    pub token: String,
}

#[derive(Deserialize, Validate)]
pub struct ForgotPasswordRequest {
    #[validate(email(message = "Please enter a valid email address"))]
    pub email: String,
}

impl FormRequest for ForgotPasswordRequest {}

#[derive(Deserialize, Validate)]
pub struct ResetPasswordRequest {
    pub token: String,
    #[validate(length(min = 8, message = "Password must be at least 8 characters"))]
    pub password: String,
    pub password_confirmation: String,
}

impl FormRequest for ResetPasswordRequest {
    /// Cross-field check: confirm the password and its confirmation
    /// match. Runs after the per-field rules pass.
    fn after_validation(&self) -> Result<(), ValidationErrors> {
        if self.password != self.password_confirmation {
            let mut errs = ValidationErrors::new();
            errs.add("password_confirmation", "Passwords do not match.");
            return Err(errs);
        }
        Ok(())
    }
}

/// `GET /forgot-password` - the form that asks for the address.
#[handler]
pub async fn forgot(req: Request) -> Response {
    inertia_response!(&req, "auth/ForgotPassword", ForgotPasswordProps {})
}

/// `POST /forgot-password` - mail a reset link and return to the form.
///
/// The response is the same whether or not the address is on file (or
/// verified); only a verified account receives mail. `url::to` prefixes
/// `APP_URL`, so the link works from any inbox.
#[handler]
pub async fn send_link(form: ForgotPasswordRequest) -> Response {
    PasswordReset::send_link(&form.email, &url::to(RESET_PATH)).await?;
    redirect!("/forgot-password").into()
}

/// `GET /reset-password?token=...` - the form that takes the new password.
#[handler]
pub async fn reset_form(req: Request) -> Response {
    inertia_response!(
        &req,
        "auth/ResetPassword",
        ResetPasswordProps {
            token: req.query_param("token").unwrap_or_default(),
        }
    )
}

/// `POST /reset-password` - rotate the password and send the user to
/// sign in with it.
#[handler]
pub async fn reset(form: ResetPasswordRequest) -> Response {
    let outcome = PasswordReset::complete_with_outcome(&form.token, &form.password)
        .await
        .map_err(|error| {
            // An invalid, expired, or already used token is something the
            // user fixes by requesting another link, so it comes back as
            // a field error on the form rather than as a `400` page.
            // Anything else is still the server error it was.
            if error.status_code() == 400 {
                let mut errs = ValidationErrors::new();
                errs.add(
                    "token",
                    "This reset link is invalid or has expired. Request a new one.",
                );
                FrameworkError::Validation(errs)
            } else {
                error
            }
        })?;
    // The password has changed. Refuse to call that done while another
    // session or a remember-me cookie could still act as the old one.
    outcome.sessions_revoked?;
    outcome.remember_tokens_revoked?;
    redirect!("/login").into()
}
