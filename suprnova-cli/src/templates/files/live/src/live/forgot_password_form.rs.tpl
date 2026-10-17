//! `ForgotPasswordForm`: the form on `/forgot-password` that asks for a
//! reset link.
//!
//! `PasswordReset` mails single-use links to verified accounts only, and
//! the form answers an unknown or unverified address exactly like a known
//! one, so it never reveals whether an account exists. Tokens live in the
//! `auth_flow_tokens` table and expire after 15 minutes.

use suprnova::auth_flows::PasswordReset;
use suprnova::live::{ActionResult, ComponentError, LiveComponent, live};
use suprnova::{Validate, url};

use super::{FieldErrors, redirect};
use crate::flash::Toast;

/// Where the mailed link lands: the form that takes the new password.
const RESET_PATH: &str = "/reset-password";

/// The rule the address must pass.
#[derive(Validate)]
struct ForgotPasswordInput {
    #[validate(email(message = "Please enter a valid email address"))]
    email: String,
}

/// The reset-link form, rendered by `live/forgot_password_form.html`.
#[derive(LiveComponent)]
#[live(
    name = "{package_name}.forgot-password-form",
    view = "live/forgot_password_form.html"
)]
pub struct ForgotPasswordForm {
    /// The address to mail the link to.
    #[model]
    email: String,
    /// The message under the control when the address fails its rule.
    errors: FieldErrors,
}

#[live]
impl ForgotPasswordForm {
    /// Mails a reset link and returns to the form with the same toast
    /// whether or not the address is on file. `url::to` prefixes `APP_URL`,
    /// so the link works from any inbox.
    #[action]
    pub async fn send(&mut self) -> Result<ActionResult, ComponentError> {
        let input = ForgotPasswordInput {
            email: self.email.trim().to_owned(),
        };
        if let Err(errors) = input.validate() {
            self.errors = FieldErrors::from_rules(&errors);
            return Ok(ActionResult::render());
        }
        let sent = PasswordReset::send_link(&input.email, &url::to(RESET_PATH)).await;
        let toast = match sent {
            Ok(_) => Toast::info("If that address is registered, a reset link is on its way."),
            Err(error) => {
                tracing::error!(error = %error, "sending a password reset link failed");
                Toast::error("The reset link could not be sent. Try again later.")
            }
        };
        redirect("password.request", Some(toast))
    }
}
