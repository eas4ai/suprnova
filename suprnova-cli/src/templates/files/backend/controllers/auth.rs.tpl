//! Authentication controller.
//!
//! Renders the login/register Inertia pages on GET and validates and
//! persists credentials on POST. A login continues to `/dashboard`; a
//! registration mails a verification link and continues to
//! `/verify-email` (see `email_verification`). Either continues instead to
//! the address the session holds as intended (`Redirect::intended`): the
//! page a guarded route or a sign-in link's `return_to` sent the user from,
//! such as the claim page an agent sent them to. Each action that succeeds
//! flashes a toast (`crate::props::flash::Toast`) the page it lands on
//! shows once. Each form folds the address it takes before its rules run
//! (`crate::email_address`), so the rules check what is stored, and
//! registration's duplicate check compares without case through the
//! database's `LOWER`. Form bodies are extracted via `FormRequest`. On an Inertia visit a per-field failure
//! comes back as a `303` to the form page with the errors flashed, so
//! `useForm().errors` fills in; a plain REST client still gets the `422`
//! `{ message, errors }` envelope.
//!
//! Neither page declares an `errors` prop. The framework seeds `errors`
//! on every Inertia page from the session-flashed validation bag, and an
//! explicit prop of the same name replaces that seed - `errors: None`
//! serialised as `errors: null` and hid every flashed message. Leave the
//! key to the framework.

use std::sync::Arc;

use serde::Deserialize;
use suprnova::{
    Auth, Credentials, FormRequest, FrameworkError, Inertia, InertiaProps, LoginThrottle, Redirect,
    Request, Response, Validate, ValidationErrors, auth_flows::EmailVerification, handler,
    inertia_response, redirect,
};

use crate::email_address;
use crate::models::user::User;
use crate::props::flash::Toast;

// ============================================================================
// Login
// ============================================================================

#[derive(InertiaProps)]
pub struct LoginProps {}

#[handler]
pub async fn show_login(req: Request) -> Response {
    // A sign-in link from the OAuth server or agent auth names where the
    // user was going; the login continues there.
    crate::middleware::authenticate::remember_return_to(&req);
    inertia_response!(&req, "auth/Login", LoginProps {})
}

#[derive(Deserialize, Validate)]
pub struct LoginRequest {
    #[validate(
        email(message = "Please enter a valid email address"),
        length(max = 254, message = "Email must be at most 254 characters")
    )]
    pub email: String,
    #[validate(length(min = 1, message = "Password is required"))]
    pub password: String,
    #[serde(default)]
    pub remember: bool,
}

impl FormRequest for LoginRequest {
    /// Fold the address before the rules run, so they check what is looked
    /// up.
    fn prepare_for_validation(req: &mut Request) -> Result<(), FrameworkError> {
        req.transform_input("email", |email| email_address::normalize(&email));
        Ok(())
    }
}

/// Build a `FrameworkError::Validation` that pins the failure to the
/// `email` field, mirroring how the bundled validators surface errors.
fn invalid_credentials() -> FrameworkError {
    let mut errs = ValidationErrors::new();
    errs.add("email", "These credentials do not match our records.");
    FrameworkError::Validation(errs)
}

#[handler]
pub async fn login(form: LoginRequest) -> Response {
    // `Auth::attempt` verifies the password through the registered user
    // provider, logs the user into the session on success, and issues a
    // remember-me token when requested - all via the named-guard system
    // wired in bootstrap.rs.
    match LoginThrottle::for_current_request(&form.email)?
        .attempt(
            &Credentials::password(&form.email, &form.password),
            form.remember,
        )
        .await?
    {
        Some(_user) => {
            Inertia::flash("toast", Toast::success("Signed in."))?;
            // The intended address may be a route a plugin serves, such as
            // the authorization endpoint, which can end at another site. An
            // Inertia visit leaves for it with a full page load.
            if Redirect::intended_url().is_some() {
                return Ok(Inertia::location(Redirect::intended("/dashboard")));
            }
            redirect!("/dashboard").into()
        }
        None => Err(invalid_credentials().into()),
    }
}

// ============================================================================
// Registration
// ============================================================================

#[derive(InertiaProps)]
pub struct RegisterProps {}

#[handler]
pub async fn show_register(req: Request) -> Response {
    inertia_response!(&req, "auth/Register", RegisterProps {})
}

#[derive(Deserialize, Validate)]
pub struct RegisterRequest {
    #[validate(length(min = 2, max = 255, message = "Name must be 2 to 255 characters"))]
    pub name: String,
    #[validate(
        email(message = "Please enter a valid email address"),
        length(max = 254, message = "Email must be at most 254 characters")
    )]
    pub email: String,
    #[validate(length(min = 8, message = "Password must be at least 8 characters"))]
    pub password: String,
    pub password_confirmation: String,
}

impl FormRequest for RegisterRequest {
    /// Fold the address before the rules run, so they check what is stored.
    fn prepare_for_validation(req: &mut Request) -> Result<(), FrameworkError> {
        req.transform_input("email", |email| email_address::normalize(&email));
        Ok(())
    }

    /// Cross-field check: confirm the password and its confirmation
    /// match. Runs after the per-field rules pass, so we know each
    /// individual value is well-formed before comparing them.
    fn after_validation(&self) -> Result<(), ValidationErrors> {
        if self.password != self.password_confirmation {
            let mut errs = ValidationErrors::new();
            errs.add("password_confirmation", "Passwords do not match.");
            return Err(errs);
        }
        Ok(())
    }
}

#[handler]
pub async fn register(form: RegisterRequest) -> Response {
    if User::find_by_email(&form.email).await?.is_some() {
        let mut errs = ValidationErrors::new();
        errs.add("email", "This email is already registered.");
        return Err(FrameworkError::Validation(errs).into());
    }

    let user = User::create(&form.name, &form.email, &form.password).await?;
    // Mail the verification link. This runs before the login below, so a
    // delivery failure (nothing listening on `MAIL_SMTP_HOST`, `MAIL_FROM`
    // unset) is reported as the error it is rather than after a session
    // the user never sees. The account exists by then: the user signs in
    // with the password they chose and asks `/verify-email` for a new link.
    EmailVerification::send_link(&user, &super::email_verification::verification_link()).await?;
    // Log the freshly-created user into the session (fires the Login event).
    Auth::login(Arc::new(user), false).await?;

    Inertia::flash("toast", Toast::success(format!("Welcome, {}.", form.name)))?;
    // A visitor a sign-in link sent here, from a claim page for instance,
    // goes back there, and that page asks an unverified address to verify
    // first. The address is not carried across verification: its link may
    // open in another browser.
    if Redirect::intended_url().is_some() {
        return Ok(Inertia::location(Redirect::intended("/verify-email")));
    }
    redirect!("/verify-email").into()
}

// ============================================================================
// Logout
// ============================================================================

#[handler]
pub async fn logout(_req: Request) -> Response {
    Auth::logout().await?;
    Inertia::flash("toast", Toast::info("Signed out."))?;
    redirect!("/").into()
}
