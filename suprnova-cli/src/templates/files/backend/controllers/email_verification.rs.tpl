//! Email verification.
//!
//! Registration mails a verification link (see `auth::register`). This
//! controller renders the "check your inbox" notice, mails a fresh link on
//! request, and consumes the link. `EmailVerification::verify` is
//! actor-bound: the account that owns the token must be the one signed in,
//! so every route here sits in the authenticated group and a link opened
//! while signed out first goes through `/login`.
//!
//! Tokens live in the `auth_flow_tokens` table (see
//! `migrations::create_auth_flow_tokens_table`), are single-use, and expire
//! after 24 hours. The mail goes out through the `Mail` facade with the
//! `MAIL_*` settings in `.env`.

use suprnova::{
    Auth, FrameworkError, InertiaProps, MustVerifyEmail, Request, Response,
    auth_flows::EmailVerification, handler, inertia_response, redirect, url,
};

use crate::models::user::User;

/// Where the mailed link lands: the route that consumes the token.
const VERIFY_PATH: &str = "/verify-email/verify";

/// The absolute URL a verification mail points at. `url::to` prefixes
/// `APP_URL`, so the link works from any inbox rather than only on the host
/// that rendered the page.
pub fn verification_link() -> String {
    url::to(VERIFY_PATH)
}

#[derive(InertiaProps)]
pub struct VerifyEmailProps {
    /// The address the link went to, so the notice can name it.
    pub email: String,
}

/// The signed-in user as the typed model. The routes here run behind the
/// `auth` middleware, so `None` means the session lost its user between
/// the guard and the handler; refusing is the safe answer.
async fn current_user() -> Result<User, FrameworkError> {
    Auth::user_as::<User>()
        .await?
        .ok_or(FrameworkError::Unauthorized)
}

/// `GET /verify-email` - the notice. An already verified account has
/// nothing to do here and continues to the dashboard.
#[handler]
pub async fn notice(req: Request) -> Response {
    let user = current_user().await?;
    if user.is_email_verified() {
        return redirect!("/dashboard").into();
    }
    inertia_response!(
        &req,
        "auth/VerifyEmail",
        VerifyEmailProps { email: user.email }
    )
}

/// `POST /email/verification-notification` - mail a fresh link to the
/// signed-in user and return to the notice. A verified account gets no
/// mail: there is nothing left to prove.
#[handler]
pub async fn resend(_req: Request) -> Response {
    let user = current_user().await?;
    if !user.is_email_verified() {
        EmailVerification::send_link(&user, &verification_link()).await?;
    }
    redirect!("/verify-email").into()
}

/// `GET /verify-email/verify?token=...` - consume the mailed link.
///
/// The framework checks that the signed-in user owns the token, stamps it
/// used, and records `email_verified_at`. An invalid, expired, reused, or
/// foreign token is a `400`, which the Inertia error page renders.
#[handler]
pub async fn verify(req: Request) -> Response {
    let token = req.query_param("token").unwrap_or_default();
    EmailVerification::verify(&token).await?;
    redirect!("/dashboard").into()
}
