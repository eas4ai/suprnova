//! Sign-in and registration: the `auth/Login` and `auth/Register` pages and
//! the forms they post, as the Svelte Inertia kit has them.
//!
//! The OAuth server and agent auth send a guest to `/login?return_to=...`
//! (`src/agents.rs`), and the approval and claim pages send one there too
//! (`controllers::agents`). Either way the sign-in continues to the address
//! the session holds as intended, the page the user was going to, and to
//! the home page otherwise. Registration does the same, so a new user an
//! MCP client sent here comes back to approve it.
//!
//! An address is folded before the rules run, as `UserObserver` stores it:
//! trimmed and in lowercase. The provider looks a user up by the stored
//! address, so a sign-in typed in another case finds the same account.
//!
//! On an Inertia visit a field's failure comes back as a redirect to the
//! form with the errors flashed; a JSON client gets the `422` envelope.
//! Neither page declares an `errors` prop: the framework fills it from the
//! flashed errors, and a prop of the same name would replace them.

use std::sync::Arc;

use serde::Deserialize;
use suprnova::{
    Auth, Credentials, FormRequest, FrameworkError, Inertia, InertiaProps, LoginThrottle, Model,
    Redirect, Request, Response, Validate, ValidationErrors, attrs, handler, hashing,
    inertia_response,
};

use crate::middleware::authenticate::remember_return_to;
use crate::models::users::User;

/// Where a sign-in or a registration goes when no address is intended.
const HOME: &str = "/";

/// `email` as `UserObserver::creating` stores it.
fn fold(email: &str) -> String {
    email.trim().to_lowercase()
}

/// Continue to the intended address, or home. The intended address may be
/// a route a plugin serves, such as the authorization endpoint, which
/// leaves for another site: an Inertia visit goes there with a full page
/// load.
fn continue_on() -> Response {
    if Redirect::intended_url().is_some() {
        return Ok(Inertia::location(Redirect::intended(HOME)));
    }
    Redirect::to(HOME).into()
}

/// The `auth/Login` page's props: none, since the framework fills `errors`.
#[derive(InertiaProps)]
pub struct LoginProps {}

/// `GET /login` - the sign-in form. A sign-in link names where the user was
/// going in `return_to`, and the sign-in continues there.
#[handler]
pub async fn show_login(req: Request) -> Response {
    remember_return_to(&req);
    inertia_response!(&req, "auth/Login", LoginProps {})
}

/// The sign-in form.
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
        req.transform_input("email", |email| fold(&email));
        Ok(())
    }
}

/// The one answer for an unknown address and a wrong password, so the form
/// does not tell which addresses have accounts.
fn invalid_credentials() -> FrameworkError {
    let mut errs = ValidationErrors::new();
    errs.add("email", "These credentials do not match our records.");
    FrameworkError::Validation(errs)
}

/// `POST /login` - check the password through the user provider
/// `bootstrap::register` binds, start the session, and continue.
#[handler]
pub async fn login(form: LoginRequest) -> Response {
    match LoginThrottle::for_current_request(&form.email)?
        .attempt(
            &Credentials::password(&form.email, &form.password),
            form.remember,
        )
        .await?
    {
        Some(_user) => continue_on(),
        None => Err(invalid_credentials().into()),
    }
}

/// The `auth/Register` page's props: none, since the framework fills
/// `errors`.
#[derive(InertiaProps)]
pub struct RegisterProps {}

/// `GET /register` - the registration form.
#[handler]
pub async fn show_register(req: Request) -> Response {
    remember_return_to(&req);
    inertia_response!(&req, "auth/Register", RegisterProps {})
}

/// The registration form.
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
        req.transform_input("email", |email| fold(&email));
        Ok(())
    }

    /// The password and its confirmation match. Runs after the per-field
    /// rules pass, so each value is well formed before they are compared.
    fn after_validation(&self) -> Result<(), ValidationErrors> {
        if self.password != self.password_confirmation {
            let mut errs = ValidationErrors::new();
            errs.add("password_confirmation", "Passwords do not match.");
            return Err(errs);
        }
        Ok(())
    }
}

/// `POST /register` - create the account with the password hashed, sign it
/// in, and continue.
#[handler]
pub async fn register(form: RegisterRequest) -> Response {
    // A deleted account keeps its address, which the database's
    // one-account-per-address index still holds.
    let taken = User::query()
        .with_trashed()
        .filter("email", form.email.clone())
        .first()
        .await?
        .is_some();
    if taken {
        let mut errs = ValidationErrors::new();
        errs.add("email", "This email is already registered.");
        return Err(FrameworkError::Validation(errs).into());
    }
    let password = hashing::hash_async(&form.password).await?;
    let user = User::create(attrs! {
        name: form.name,
        email: form.email,
        password: password,
    })
    .await?;
    Auth::login(Arc::new(user), false).await?;
    continue_on()
}
