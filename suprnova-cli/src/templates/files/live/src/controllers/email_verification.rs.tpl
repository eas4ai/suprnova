//! Email verification pages.
//!
//! Registration mails a verification link (see `live::register_form`).
//! `/verify-email` mounts the notice, whose button mails a fresh link
//! (`live::verify_email`), and `/verify-email/verify` consumes the link.
//! `EmailVerification::verify` is actor-bound: the account that owns the
//! token must be the one signed in, so every route here sits in the
//! authenticated group and a link opened while signed out first goes
//! through `/login`.

use suprnova::auth_flows::EmailVerification;
use suprnova::live::{LiveDocument, MountFlags};
use suprnova::{
    Auth, FrameworkError, HttpResponse, MustVerifyEmail, Request, Response, handler, redirect, url,
};

use super::auth::no_parameters;
use crate::flash::Toast;
use crate::live::mounts;
use crate::models::user::User;
use crate::views::{self, Frame, Nav, VerifyEmailPage};

/// Where the mailed link lands: the route that consumes the token.
const VERIFY_PATH: &str = "/verify-email/verify";

/// The absolute URL a verification mail points at. `url::to` prefixes
/// `APP_URL`, so the link works from any inbox rather than only on the host
/// that rendered the page.
pub fn verification_link() -> String {
    url::to(VERIFY_PATH)
}

/// `GET /verify-email`: the notice. An already verified account has
/// nothing to do here and continues to the dashboard.
#[handler]
pub async fn notice(req: Request) -> Response {
    let user = Auth::user_as::<User>()
        .await?
        .ok_or(FrameworkError::Unauthorized)?;
    if user.is_email_verified() {
        return redirect!("/dashboard").into();
    }
    Ok(notice_page(&req).await?)
}

async fn notice_page(req: &Request) -> Result<HttpResponse, FrameworkError> {
    let mut document = LiveDocument::from_request(req)?;
    let notice = document
        .mount(
            &mounts()?.verify_email,
            no_parameters(),
            MountFlags::empty(),
        )
        .await?;
    let frame = Frame::live(&mut document, "Verify your email", Nav::None).await?;
    let page = VerifyEmailPage {
        frame,
        notice: notice.html().clone(),
    };
    views::render(document, "live/pages/verify-email.html", &page)
}

/// `GET /verify-email/verify?token=...`: consume the mailed link.
///
/// The framework checks that the signed-in user owns the token, stamps it
/// used, and records `email_verified_at`. An invalid, expired, reused, or
/// foreign token is a `400`, which the error page renders.
#[handler]
pub async fn verify(req: Request) -> Response {
    let token = req.query_param("token").unwrap_or_default();
    EmailVerification::verify(&token).await?;
    Toast::success("Your email address is verified.").flash()?;
    redirect!("/dashboard").into()
}
