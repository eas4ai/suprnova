//! `ResetPasswordForm`: the form on `/reset-password` that takes the new
//! password.
//!
//! Completion rotates the password and revokes the account's other
//! sessions and remember-me tokens. A revocation failure is an error here,
//! because a changed password is not enough while an old credential still
//! works.

use suprnova::Validate;
use suprnova::auth_flows::PasswordReset;
use suprnova::live::{ActionResult, ComponentError, LiveComponent, live};

use super::{FieldErrors, failed, redirect};
use crate::flash::Toast;

/// The rule the new password must pass.
#[derive(Validate)]
struct ResetPasswordInput {
    #[validate(length(min = 8, message = "Password must be at least 8 characters"))]
    password: String,
}

/// The new-password form, rendered by `live/reset_password_form.html`.
#[derive(LiveComponent)]
#[live(
    name = "{package_name}.reset-password-form",
    view = "live/reset_password_form.html"
)]
pub struct ResetPasswordForm {
    /// The token from the mailed link. Public, so the seed every visitor
    /// renders carries it into the form's first request; it is in the
    /// link's own URL already.
    #[public]
    token: String,
    /// The new password. Transient, so it never enters a snapshot.
    #[model(transient)]
    password: String,
    /// The new password again, which must match.
    #[model(transient)]
    password_confirmation: String,
    /// The message under each control the last attempt failed.
    errors: FieldErrors,
}

#[live]
impl ResetPasswordForm {
    /// Takes the token from the link's `?token=` query.
    #[mount]
    pub fn mount(token: String) -> Self {
        Self {
            token,
            password: String::new(),
            password_confirmation: String::new(),
            errors: FieldErrors::default(),
        }
    }

    /// Rotates the password and sends the user to sign in with it.
    #[action]
    pub async fn reset(&mut self) -> Result<ActionResult, ComponentError> {
        let input = ResetPasswordInput {
            password: self.password.clone(),
        };
        if let Err(errors) = input.validate() {
            self.errors = FieldErrors::from_rules(&errors);
            return Ok(ActionResult::render());
        }
        if self.password != self.password_confirmation {
            self.errors = FieldErrors::one("password_confirmation", "Passwords do not match.");
            return Ok(ActionResult::render());
        }
        let completed = PasswordReset::complete_with_outcome(&self.token, &self.password).await;
        let outcome = match completed {
            Ok(outcome) => outcome,
            // An invalid, expired, or already used token is something the
            // user fixes by requesting another link, so it comes back as a
            // message on the form.
            Err(error) if error.status_code() == 400 => {
                self.errors = FieldErrors::one(
                    "token",
                    "This reset link is invalid or has expired. Request a new one.",
                );
                return Ok(ActionResult::render());
            }
            Err(error) => return Err(failed("resetting a password", error)),
        };
        // The password has changed. Refuse to call that done while another
        // session or a remember-me cookie could still act as the old one.
        outcome
            .sessions_revoked
            .map_err(|error| failed("revoking the other sessions", error))?;
        outcome
            .remember_tokens_revoked
            .map_err(|error| failed("revoking the remember-me tokens", error))?;
        redirect("login", Some(Toast::success("Your password was reset.")))
    }
}
