//! `LoginForm`: the sign-in form on `/login`.
//!
//! A public seed, so a visitor who is not signed in can use it. The action
//! checks the fields, asks the registered user provider through
//! `Auth::attempt`, and on success sends the browser on with a toast: to the
//! session's intended address through `login.continue`, or else to the
//! dashboard. A failure stays on the form, with the message under its
//! control. The address is folded before the rules run, as registration
//! stores it (`crate::email_address`).

use suprnova::live::{ActionResult, ComponentError, LiveComponent, live};
use suprnova::{Credentials, FrameworkError, LoginThrottle, Redirect, Validate};

use super::{FieldErrors, failed, redirect};
use crate::email_address;
use crate::flash::Toast;

/// Laravel's starter kits allow five failed sign-ins per email and address,
/// then a sixty-second wait. Both numbers are this application's to tune.
const LOGIN_ATTEMPTS: u32 = 5;
const LOGIN_WINDOW: std::time::Duration = std::time::Duration::from_secs(60);

/// The rules the form checks before it asks for the account.
#[derive(Validate)]
struct LoginInput {
    #[validate(
        email(message = "Please enter a valid email address"),
        length(max = 254, message = "Email must be at most 254 characters")
    )]
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
    /// Signs the user in and continues, or shows why not.
    #[action]
    pub async fn login(&mut self) -> Result<ActionResult, ComponentError> {
        let input = LoginInput {
            email: email_address::normalize(&self.email),
            password: self.password.clone(),
        };
        if let Err(errors) = input.validate() {
            self.errors = FieldErrors::from_rules(&errors);
            return Ok(ActionResult::render());
        }
        // `Auth::attempt` verifies the password through the registered user
        // provider, logs the user into the session on success, and issues a
        // remember-me token when asked.
        let throttle = LoginThrottle::for_current_request(&input.email, LOGIN_ATTEMPTS, LOGIN_WINDOW)
            .map_err(|error| failed("signing in", error))?;
        let user = match throttle
            .attempt(
                &Credentials::password(&input.email, &input.password),
                self.remember,
            )
            .await
        {
            Ok(user) => user,
            Err(FrameworkError::Validation(errors)) => {
                self.errors = FieldErrors::one("email", &errors.messages_for("email").join(" "));
                return Ok(ActionResult::render());
            }
            Err(error) => return Err(failed("signing in", error)),
        };
        if user.is_none() {
            self.errors = FieldErrors::one("email", "These credentials do not match our records.");
            return Ok(ActionResult::render());
        }
        let next = if Redirect::intended_url().is_some() {
            "login.continue"
        } else {
            "dashboard"
        };
        redirect(next, Some(Toast::success("Signed in.")))
    }
}
