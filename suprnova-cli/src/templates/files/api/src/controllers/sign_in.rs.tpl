//! The sign-in page the framework's approval and claim pages send a guest
//! to. A person approves an MCP client (`/oauth/approve`) or claims an
//! agent (`/agent/claim`) in a browser session, so this application, which
//! otherwise serves JSON, signs a browser in here. A client of the JSON API
//! signs in through `POST /api/auth/login` and keeps its bearer token.
//!
//! A guest reaches `/login` with the page to come back to in `return_to`,
//! signs in with their email and password, and goes back there. Only a
//! path on this application is kept, so a sign-in link cannot send anyone
//! to another site. A user with no page to go back to goes to the
//! connected-agents page. The paths come from `src/agents.rs`, which gives
//! the same ones to the OAuth server and agent auth.
//!
//! The page is fixed markup. The one value in it is the session's CSRF
//! token, escaped. Both routes carry `ConsentPageHeaders`, so the page is
//! never framed, cached or sent as a referrer, and the form posts through
//! the route's CSRF middleware. An account with a second factor is told
//! that this page does not offer one, and stays signed out.

use serde::Deserialize;
use suprnova::{
    Auth, FormRequest, FrameworkError, HttpResponse, Redirect, Request, Response, SignInOutcome,
    Validate, handler, url,
};

use crate::agents::{CONNECTED_PAGE, RETURN_TO};
use crate::email_address;

/// The answer to a wrong address or password. A form that is not valid gets
/// it too, so the page says nothing about which addresses have accounts.
const INVALID: &str = "These credentials do not match our records.";

/// The answer to an account that signs in with a second factor.
const SECOND_FACTOR: &str =
    "This account signs in with a second factor, which this page does not offer.";

/// The page, up to the notice above its form.
const PAGE_HEAD: &str = concat!(
    "<!doctype html>\n",
    "<html lang=\"en\">\n",
    "<head>\n",
    "<meta charset=\"utf-8\">\n",
    "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n",
    "<title>Sign in</title>\n",
    "</head>\n",
    "<body>\n",
    "<main>\n",
    "<h1>Sign in</h1>\n",
    "<p>Sign in to answer an app or an agent that asks to act for you.</p>\n",
);

/// The form, up to its CSRF token. It names no `action`, so it posts to the
/// page's own address, `return_to` included.
const FORM_HEAD: &str = concat!(
    "<form method=\"post\">\n",
    "<input type=\"hidden\" name=\"_token\" value=\"",
);

/// The rest of the page.
const PAGE_TAIL: &str = concat!(
    "\">\n",
    "<p><label for=\"email\">Email</label>\n",
    "<input id=\"email\" name=\"email\" type=\"email\" autocomplete=\"username\" ",
    "maxlength=\"254\" required></p>\n",
    "<p><label for=\"password\">Password</label>\n",
    "<input id=\"password\" name=\"password\" type=\"password\" ",
    "autocomplete=\"current-password\" required></p>\n",
    "<button type=\"submit\">Sign in</button>\n",
    "</form>\n",
    "</main>\n",
    "</body>\n",
    "</html>\n",
);

/// `GET /login`: the sign-in page, or, for a user already signed in, the
/// page `return_to` names.
#[handler]
pub async fn show(req: Request) -> Response {
    // `same_origin_path` refuses an address a browser would read as another
    // host, such as `//evil.example`.
    let return_to = req
        .query_param(RETURN_TO)
        .and_then(|candidate| url::same_origin_path(&candidate));
    if Auth::check() {
        return Redirect::to(return_to.unwrap_or_else(|| CONNECTED_PAGE.to_owned())).into();
    }
    // The sign-in that follows continues there (`Redirect::intended`).
    if let Some(path) = return_to {
        Redirect::set_intended_url(path);
    }
    page(200, None)
}

/// The sign-in form.
#[derive(Deserialize, Validate)]
pub struct SignInRequest {
    #[serde(default)]
    #[validate(email, length(max = 254))]
    pub email: String,
    #[serde(default)]
    #[validate(length(min = 1))]
    pub password: String,
}

impl FormRequest for SignInRequest {
    /// Fold the address before the rules run, so they check what is looked
    /// up.
    fn prepare_for_validation(req: &mut Request) -> Result<(), FrameworkError> {
        req.transform_input("email", |email| email_address::normalize(&email));
        Ok(())
    }
}

/// `POST /login`: sign in through Magnetar, which binds this browser's
/// session, and continue to the page the sign-in link named.
#[handler]
pub async fn store(req: Request) -> Response {
    let user_agent = req.user_agent().map(str::to_owned);
    let ip_address = req.ip();
    let form = match req.validate::<SignInRequest>().await {
        Ok(form) => form,
        Err(FrameworkError::Validation(_) | FrameworkError::ValidationRedirect { .. }) => {
            return page(422, Some(INVALID));
        }
        Err(error) => return Err(error.into()),
    };
    match Auth::password()
        .authenticate_outcome(&form.email, &form.password, user_agent, ip_address)
        .await
    {
        Ok(SignInOutcome::Authenticated { .. }) => Redirect::intended(CONNECTED_PAGE).into(),
        // Magnetar binds no session until the second factor is proved.
        Ok(SignInOutcome::FactorRequired { .. }) => page(422, Some(SECOND_FACTOR)),
        // A sign-in step this page does not know: refuse rather than guess.
        Ok(_) => Err(
            FrameworkError::internal("the sign-in needs a step this page does not offer").into(),
        ),
        // A wrong address or password.
        Err(error) if error.status_code() == 401 => page(422, Some(INVALID)),
        Err(error) => Err(error.into()),
    }
}

/// The page with `status`, and `notice` above the form when there is one.
fn page(status: u16, notice: Option<&'static str>) -> Response {
    let token = suprnova::csrf_token().ok_or_else(|| {
        FrameworkError::internal("the sign-in page needs the session middleware (bootstrap.rs)")
    })?;
    let (open, notice, close) = match notice {
        Some(notice) => ("<p role=\"alert\">", notice, "</p>\n"),
        None => ("", "", ""),
    };
    let html = [
        PAGE_HEAD,
        open,
        notice,
        close,
        FORM_HEAD,
        &escape(&token),
        PAGE_TAIL,
    ]
    .concat();
    Ok(HttpResponse::html(html).status(status))
}

/// `text` escaped for a double-quoted attribute value.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
