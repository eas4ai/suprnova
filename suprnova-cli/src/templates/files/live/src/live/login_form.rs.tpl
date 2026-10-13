//! `LoginForm`: the sign-in form on `/login`.
//!
//! A public seed, so a visitor who is not signed in can use it. The action
//! checks the fields, asks the registered user provider through
//! `Auth::attempt`, and on success sends the browser to the dashboard with a
//! toast. A failure stays on the form, with the message under its control.

use suprnova::live::{ActionResult, ComponentError, LiveComponent, live};
use suprnova::{Auth, Credentials, Validate};

use super::{FieldErrors, failed, redirect};
use crate::flash::Toast;

/// The rules the form checks before it asks for the account.
#[derive(Validate)]
struct LoginInput {
    #[validate(email(message = "Please enter a valid email address"))]
    email: String,
    #[validate(length(min = 1, message = "Password is required"))]
    password: String,
}

/// The sign-in form, rendered by `live/login_form.html`.
#[derive(LiveComponent)]
#[live(name = "{package_name}.login-form", view = "live/login_form.html")]
pub struct LoginForm {
    /// The email address, bound to the email input.
    #[model]
    email: String,
    /// The password. Transient, so it never enters a snapshot.
    #[model(transient)]
    password: String,
    /// Whether to keep the user signed in after the browser closes.
    #[model]
    remember: bool,
    /// The message under each control the last attempt failed.
    errors: FieldErrors,
}

#[live]
impl LoginForm {
    /// Signs the user in and continues to the dashboard, or shows why not.
    #[action]
    pub async fn login(&mut self) -> Result<ActionResult, ComponentError> {
        let input = LoginInput {
            email: self.email.trim().to_owned(),
            password: self.password.clone(),
        };
        if let Err(errors) = input.validate() {
            self.errors = FieldErrors::from_rules(&errors);
            return Ok(ActionResult::render());
        }
        // `Auth::attempt` verifies the password through the registered user
        // provider, logs the user into the session on success, and issues a
        // remember-me token when asked.
        let user = Auth::attempt(
            &Credentials::password(&input.email, &input.password),
            self.remember,
        )
        .await
        .map_err(|error| failed("signing in", error))?;
        if user.is_none() {
            self.errors = FieldErrors::one("email", "These credentials do not match our records.");
            return Ok(ActionResult::render());
        }
        redirect("dashboard", Some(Toast::success("Signed in.")))
    }
}
