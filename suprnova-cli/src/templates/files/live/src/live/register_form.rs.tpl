//! `RegisterForm`: the registration form on `/register`.
//!
//! A public seed, so a visitor who is not signed in can use it. The action
//! checks the fields, creates the account, mails a verification link, signs
//! the new user in and sends the browser to the verification notice. A
//! visitor a sign-in link sent here, from a claim page for instance, goes
//! back there instead through `login.continue`, and that page asks an
//! unverified address to verify first. The address is not carried across
//! verification: its link may open in another browser.
//!
//! The address is folded before the rules run, so they check what is
//! stored (`crate::email_address`), and the duplicate check compares without
//! case through the database's `LOWER`.

use std::sync::Arc;

use suprnova::auth_flows::EmailVerification;
use suprnova::live::{ActionResult, ComponentError, LiveComponent, live};
use suprnova::{Auth, Redirect, Validate};

use super::{FieldErrors, failed, redirect};
use crate::controllers::email_verification::verification_link;
use crate::email_address;
use crate::flash::Toast;
use crate::models::user::User;

/// The rules each field must pass.
#[derive(Validate)]
struct RegisterInput {
    #[validate(length(min = 2, max = 255, message = "Name must be 2 to 255 characters"))]
    name: String,
    #[validate(
        email(message = "Please enter a valid email address"),
        length(max = 254, message = "Email must be at most 254 characters")
    )]
    email: String,
    #[validate(length(min = 8, message = "Password must be at least 8 characters"))]
    password: String,
}

/// The registration form, rendered by `live/register_form.html`.
#[derive(LiveComponent)]
#[live(name = "{package_name}.register-form", view = "live/register_form.html")]
pub struct RegisterForm {
    /// The display name.
    #[model]
    name: String,
    /// The email address the account signs in with.
    #[model]
    email: String,
    /// The password. Transient, so it never enters a snapshot.
    #[model(transient)]
    password: String,
    /// The password again, which must match.
    #[model(transient)]
    password_confirmation: String,
    /// The message under each control the last attempt failed.
    errors: FieldErrors,
}

#[live]
impl RegisterForm {
    /// Creates the account and signs the new user in, or shows why not.
    #[action]
    pub async fn register(&mut self) -> Result<ActionResult, ComponentError> {
        let input = RegisterInput {
            name: self.name.trim().to_owned(),
            email: email_address::normalize(&self.email),
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
        let taken = User::find_by_email(&input.email)
            .await
            .map_err(|error| failed("looking up an email address", error))?;
        if taken.is_some() {
            self.errors = FieldErrors::one("email", "This email is already registered.");
            return Ok(ActionResult::render());
        }

        let user = User::create(&input.name, &input.email, &input.password)
            .await
            .map_err(|error| failed("creating an account", error))?;
        // The account exists from here on. A verification mail that cannot
        // be sent (nothing listening on `MAIL_SMTP_HOST`, `MAIL_FROM` unset)
        // is logged and said in the toast; the user is still signed in and
        // asks the notice page for a new link once mail works.
        let sent = EmailVerification::send_link(&user, &verification_link()).await;
        Auth::login(Arc::new(user), false)
            .await
            .map_err(|error| failed("signing a new user in", error))?;
        let toast = match sent {
            Ok(_) => Toast::success(format!("Welcome, {}.", input.name)),
            Err(error) => {
                tracing::error!(error = %error, "sending a verification link failed");
                Toast::error(
                    "Your account is ready, but the verification email could not be sent. Ask for a new link below.",
                )
            }
        };
        let next = if Redirect::intended_url().is_some() {
            "login.continue"
        } else {
            "verification.notice"
        };
        redirect(next, Some(toast))
    }
}
