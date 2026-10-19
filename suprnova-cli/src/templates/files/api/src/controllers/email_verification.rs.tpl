//! Email verification: a link mailed to the account's address, which
//! verifies it for the user signed in where it opens.
//!
//! Registration mails the link to a new account, and
//! `POST /api/email/verification-notification` mails a new one to the
//! signed-in user. The link opens `/email/verify` in a browser, usually a
//! mail client's, so it carries no bearer token: a guest signs in at the
//! kit's sign-in page and comes back, and the page then verifies the
//! address of that user. A verified address is what agent auth binds an
//! agent's claim to.
//!
//! The link verifies only for the account's own signed-in user. Anyone can
//! register another person's address with a password of their own; a link
//! that verified whoever opened it would let that person's click verify the
//! registrant's account. Verifying changes no credential and starts no
//! session.

use suprnova::{
    Auth, EmailVerificationIssue, EmailVerificationLink, EmailVerificationMail,
    EmailVerificationOutcome, FrameworkError, HttpResponse, Mail, Redirect, Request, Response,
    handler, url,
};

use crate::agents::{RETURN_TO, SIGN_IN_PAGE, VERIFY_EMAIL_PAGE};

// ============================================================================
// POST /api/email/verification-notification
// ============================================================================

/// Mail the signed-in user a new link: `202`, or `204` when the address is
/// already verified.
#[handler]
pub async fn notification(_req: Request) -> Response {
    match Auth::email_verification().issue().await? {
        EmailVerificationIssue::Issued(link) => {
            mail_link(&link).await?;
            Ok(HttpResponse::json(serde_json::json!({
                "message": "Verification link sent."
            }))
            .status(202))
        }
        EmailVerificationIssue::AlreadyVerified => Ok(HttpResponse::new().status(204)),
        EmailVerificationIssue::SignIn => Err(FrameworkError::Unauthorized.into()),
        _ => Err(FrameworkError::internal("unexpected email verification outcome").into()),
    }
}

/// Mail `link` to its address, from `MAIL_FROM`, as the framework's
/// verification mail.
pub async fn mail_link(link: &EmailVerificationLink) -> Result<(), FrameworkError> {
    let from_address = std::env::var("MAIL_FROM").map_err(|_| {
        FrameworkError::internal("set MAIL_FROM: the email verification mail is sent from it")
    })?;
    let mail = EmailVerificationMail {
        to_address: link.email.clone(),
        user_name: None,
        verification_link: url::to(&format!("{VERIFY_EMAIL_PAGE}?token={}", link.token)),
        app_name: std::env::var("APP_NAME").unwrap_or_else(|_| "Suprnova".to_owned()),
        from_address,
    };
    Mail::to(link.email.as_str()).send(mail).await
}

// ============================================================================
// GET /email/verify?token=...
// ============================================================================

/// The page the link opens: a guest goes to sign in and comes back; a
/// signed-in user's address is verified when the link is theirs.
#[handler]
pub async fn verify(req: Request) -> Response {
    // A token is 64 hexadecimal characters. Anything else is no token, and
    // the page never sends it on.
    let token = req
        .query_param("token")
        .filter(|token| !token.is_empty() && token.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .unwrap_or_default();
    if token.is_empty() {
        return page(404, UNKNOWN);
    }
    match Auth::email_verification().verify(&token).await? {
        EmailVerificationOutcome::Verified => page(200, VERIFIED),
        EmailVerificationOutcome::AlreadyVerified => page(200, ALREADY_VERIFIED),
        EmailVerificationOutcome::OtherAccount => page(403, OTHER_ACCOUNT),
        EmailVerificationOutcome::Expired => page(410, EXPIRED),
        EmailVerificationOutcome::Unknown => page(404, UNKNOWN),
        EmailVerificationOutcome::SignIn => Redirect::to(SIGN_IN_PAGE)
            .query(RETURN_TO, format!("{VERIFY_EMAIL_PAGE}?token={token}"))
            .into(),
        _ => Err(FrameworkError::internal("unexpected email verification outcome").into()),
    }
}

const VERIFIED: &str = "Your email address is verified.";
const ALREADY_VERIFIED: &str = "Your email address was already verified.";
const OTHER_ACCOUNT: &str =
    "This link is for another account. Sign in to the account it was sent to, then open it again.";
const EXPIRED: &str = "This link expired. Ask for a new one.";
const UNKNOWN: &str = "This link is not valid, or it was already used. Ask for a new one.";

/// The page, with `message` as its one paragraph. Every message is fixed
/// text.
fn page(status: u16, message: &'static str) -> Response {
    let html = [PAGE_HEAD, message, PAGE_TAIL].concat();
    Ok(HttpResponse::html(html).status(status))
}

/// The page, up to its message.
const PAGE_HEAD: &str = concat!(
    "<!doctype html>\n",
    "<html lang=\"en\">\n",
    "<head>\n",
    "<meta charset=\"utf-8\">\n",
    "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n",
    "<title>Email verification</title>\n",
    "</head>\n",
    "<body>\n",
    "<main>\n",
    "<h1>Email verification</h1>\n",
    "<p role=\"status\">",
);

/// The rest of the page.
const PAGE_TAIL: &str = concat!("</p>\n", "</main>\n", "</body>\n", "</html>\n");
