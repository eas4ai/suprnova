//! Authentication pages.
//!
//! `/login` and `/register` are Live documents: each mounts its form, a
//! public seed, whose action signs the user in or creates the account
//! (`live::login_form`, `live::register_form`). Signing out is an ordinary
//! form post, so it works before any script runs.
//!
//! A sign-in link from the OAuth server or agent auth names where the user
//! was going in `return_to`, which `/login` keeps as the session's intended
//! address. A Live action redirects only to a named route, so the forms
//! send a user with an intended address to `/login/continue`, which takes
//! them there.

use suprnova::live::{CanonicalValue, LiveDocument, MountFlags};
use suprnova::{
    Auth, FrameworkError, HttpResponse, Redirect, Request, Response, handler, redirect,
};

use crate::flash::Toast;
use crate::live::mounts;
use crate::views::{self, Frame, LoginPage, Nav, RegisterPage};

/// No mount parameters: the forms start empty.
pub(crate) fn no_parameters() -> CanonicalValue {
    CanonicalValue::Object(Default::default())
}

/// `GET /login`: the sign-in form.
#[handler]
pub async fn login(req: Request) -> Response {
    crate::middleware::authenticate::remember_return_to(&req);
    Ok(login_page(&req).await?)
}

async fn login_page(req: &Request) -> Result<HttpResponse, FrameworkError> {
    let mut document = LiveDocument::from_request(req)?;
    let form = document
        .mount(&mounts()?.login, no_parameters(), MountFlags::empty())
        .await?;
    let frame = Frame::live(&mut document, "Sign in", Nav::None).await?;
    let page = LoginPage {
        frame,
        form: form.html().clone(),
    };
    views::render(document, "live/pages/login.html", &page)
}

/// `GET /register`: the registration form.
#[handler]
pub async fn register(req: Request) -> Response {
    Ok(register_page(&req).await?)
}

async fn register_page(req: &Request) -> Result<HttpResponse, FrameworkError> {
    let mut document = LiveDocument::from_request(req)?;
    let form = document
        .mount(&mounts()?.register, no_parameters(), MountFlags::empty())
        .await?;
    let frame = Frame::live(&mut document, "Register", Nav::None).await?;
    let page = RegisterPage {
        frame,
        form: form.html().clone(),
    };
    views::render(document, "live/pages/register.html", &page)
}

/// `GET /login/continue`: where the sign-in and registration forms send a
/// user whose session holds an intended address, such as the claim page an
/// agent sent them to. The address was checked when it was stored.
#[handler]
pub async fn continue_after_sign_in(_req: Request) -> Response {
    Redirect::intended("/dashboard").into()
}

/// `POST /logout`: ends the session and returns to the welcome page.
#[handler]
pub async fn logout(_req: Request) -> Response {
    Auth::logout().await?;
    Toast::info("Signed out.").flash()?;
    redirect!("/").into()
}
