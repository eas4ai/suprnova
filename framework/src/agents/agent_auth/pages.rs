//! The framework's default claim page (AGENT-009, AGENT-017): plain
//! server-rendered HTML with no script, style or asset, for an application
//! without a kit.
//!
//! The page requires the signed-in user, refuses to be framed, is never
//! cached and sends no referrer, since its address carries the attempt
//! token. Its form posts the code with the session's CSRF token, and the
//! route that takes it runs the CSRF middleware itself. Every value goes
//! through the shared [`Html`] escaper; text an agent or a provider chose
//! is isolated as well. The provider's name is the trust list's, and
//! nothing on the page comes from the ID-JAG.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::agents::consent::{
    AgentClaims, ClaimConfirmation, ClaimLookup, ClaimRefusal, PendingClaim,
};
use crate::agents::page::{Html, csrf_token, guard_errors, guarded, notice, sign_in_redirect};
use crate::csrf::CsrfMiddleware;
use crate::error::FrameworkError;
use crate::http::{HttpResponse, Request, Response};
use crate::routing::Router;

/// The largest form the page's action reads, the bound of every agent auth
/// route.
const MAX_FORM_BYTES: usize = 16 * 1024;

/// What the claim page needs.
pub(super) struct ClaimPage {
    pub(super) engine: Arc<magnetar::agent_auth::AgentAuth>,
    /// The page's path, relative to the application's public root.
    pub(super) path: String,
    pub(super) sign_in_url: String,
    pub(super) return_to_parameter: String,
    /// The connected-agents page the success page links, when one is served.
    pub(super) connected_path: Option<String>,
}

/// Serve the claim page at its path: the page on `GET`, its form's action
/// on `POST`.
pub(super) fn mount_claim(router: Router, page: Arc<ClaimPage>) -> Result<Router, FrameworkError> {
    let path = page.path.clone();
    let show = Arc::clone(&page);
    let router: Router = router
        .try_get(&path, move |request: Request| {
            let page = Arc::clone(&show);
            async move { guard_errors(claim(page, request).await) }
        })?
        .into();
    Ok(router
        .try_post(&path, move |request: Request| {
            let page = Arc::clone(&page);
            async move { guard_errors(confirm(page, request).await) }
        })?
        .middleware(CsrfMiddleware::new().without_xsrf_cookie())
        .into())
}

/// The page: who the agent will act as, through which provider, with which
/// scopes, and the form that takes the code. It reads the claim through
/// [`AgentClaims`], as an application's own page does.
async fn claim(page: Arc<ClaimPage>, request: Request) -> Response {
    let token = request
        .query_param("claim_attempt_token")
        .filter(|token| !token.is_empty());
    // The engine refuses a guest before it reads the token, so the user
    // signs in before anything about the claim is shown.
    let lookup = AgentClaims::new(Arc::clone(&page.engine), request.ip())
        .pending(token.as_deref().unwrap_or_default())
        .await?;
    match (lookup, token) {
        // A session from before the user's last password change no longer
        // speaks for them either.
        (ClaimLookup::Refused(ClaimRefusal::SignIn), _) => Ok(sign_in_redirect(
            &page.sign_in_url,
            &page.return_to_parameter,
            &request,
        )),
        (_, None) => Ok(notice(
            400,
            "Nothing to claim",
            "This link does not name a claim. Ask the agent for a new one.",
        )),
        (ClaimLookup::Pending(claim), Some(token)) => {
            Ok(guarded(HttpResponse::html(render_claim(
                &claim,
                &crate::routing::url::to(&page.path),
                &token,
                &csrf_token()?,
                None,
            ))))
        }
        (ClaimLookup::Refused(refusal), Some(_)) => Ok(refused(refusal)),
    }
}

/// The form's action: confirm the claim with the code the user typed.
async fn confirm(page: Arc<ClaimPage>, request: Request) -> Response {
    let claims = AgentClaims::new(Arc::clone(&page.engine), request.ip());
    let bytes = match request.body_bytes_with_cap(MAX_FORM_BYTES).await {
        Ok((_, bytes)) => bytes,
        Err(error) if error.status_code() == 413 => {
            return Ok(notice(
                413,
                "The form is too large",
                "Open the link the agent gave you again.",
            ));
        }
        Err(error) => return Err(error.into()),
    };
    let Some(fields) = form_fields(&bytes) else {
        return Ok(notice(
            400,
            "The form is not valid",
            "Open the link the agent gave you again.",
        ));
    };
    let token = fields
        .get("claim_attempt_token")
        .cloned()
        .unwrap_or_default();
    let code = fields.get("user_code").cloned().unwrap_or_default();
    match claims.confirm(&token, &code).await? {
        ClaimConfirmation::Claimed { .. } => Ok(guarded(HttpResponse::html(render_claimed(
            page.connected_path
                .as_deref()
                .map(crate::routing::url::to)
                .as_deref(),
        )))),
        ClaimConfirmation::WrongCode { attempts_remaining } => {
            // The page again, as it stands now, with the mistake named.
            match claims.pending(&token).await? {
                ClaimLookup::Pending(claim) => Ok(guarded(
                    HttpResponse::html(render_claim(
                        &claim,
                        &crate::routing::url::to(&page.path),
                        &token,
                        &csrf_token()?,
                        Some(attempts_remaining),
                    ))
                    .status(400),
                )),
                ClaimLookup::Refused(refusal) => Ok(refused_confirmation(refusal)),
            }
        }
        ClaimConfirmation::Refused(refusal) => Ok(refused_confirmation(refusal)),
    }
}

/// The page for a confirmation the engine refused: a user who must sign in
/// again opens the agent's link once signed in, since a form post cannot be
/// replayed after sign-in.
fn refused_confirmation(refusal: ClaimRefusal) -> HttpResponse {
    match refusal {
        ClaimRefusal::SignIn => notice(
            401,
            "Sign in again",
            "Your sign-in has ended. Sign in, then open the link the agent gave you again.",
        ),
        refusal => refused(refusal),
    }
}

/// The page for a claim the engine refused.
fn refused(refusal: ClaimRefusal) -> HttpResponse {
    match refusal {
        ClaimRefusal::OtherAccount => notice(
            403,
            "This claim is for another account",
            "Sign in with the email address the agent named, then open its link again.",
        ),
        ClaimRefusal::NotFound => notice(
            404,
            "No such claim",
            "This link does not name a claim. Ask the agent for a new one.",
        ),
        ClaimRefusal::Ended => notice(
            410,
            "This code can no longer be used",
            "It expired or was entered wrongly too often. Ask the agent for a new one.",
        ),
        ClaimRefusal::Invalid | ClaimRefusal::SignIn => notice(
            400,
            "The form is not valid",
            "Open the link the agent gave you again.",
        ),
    }
}

/// The form's fields, one value per name; `None` when a name repeats, so
/// the engine never reads a value other than the one a check saw.
fn form_fields(bytes: &[u8]) -> Option<BTreeMap<String, String>> {
    let mut fields = BTreeMap::new();
    for (name, value) in url::form_urlencoded::parse(bytes) {
        if fields
            .insert(name.into_owned(), value.into_owned())
            .is_some()
        {
            return None;
        }
    }
    Some(fields)
}

/// The claim page from the pending claim. `remaining` is set after a wrong
/// code.
fn render_claim(
    claim: &PendingClaim,
    action: &str,
    attempt_token: &str,
    csrf: &str,
    remaining: Option<u64>,
) -> String {
    let mut html = Html::default();
    html.raw("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n")
        .raw("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n")
        .raw("<title>Let an agent act for you?</title>\n</head>\n<body>\n<main>\n")
        .raw("<h1>Let an agent act for you?</h1>\n<p>");
    // The provider's name is the application's trust list entry: the page
    // never shows a name the provider or the agent sent.
    match (claim.registration_type.as_str(), &claim.provider) {
        (_, Some(provider)) => {
            html.raw("An agent signed in through <strong>")
                .text(provider)
                .raw("</strong> asks to act as you.");
        }
        ("service_auth", None) => {
            html.raw("An agent that named your email address asks to act as you.");
        }
        _ => {
            html.raw("An agent that registered on its own asks to act as you.");
        }
    }
    html.raw("</p>\n<p>If you allow it, the agent will use this application as you, with the permissions below, until you revoke it.</p>\n")
        .raw("<p>You are signed in as <strong>")
        .client(&claim.signed_in_as)
        .raw("</strong>.");
    if let Some(email) = &claim.email {
        html.raw(" The agent named <strong>")
            .client(email)
            .raw("</strong>.");
    }
    html.raw("</p>\n");
    if claim.first_time_account {
        html.raw("<p><strong>Warning:</strong> no agent has acted for your account before. Allow it only if you asked an agent to connect.</p>\n");
    }
    if claim.first_time_provider {
        html.raw("<p><strong>Warning:</strong> no agent from this provider has acted for you before.</p>\n");
    }
    if claim.scopes.is_empty() {
        html.raw("<p>It asks for no permissions: it can use only what needs none.</p>\n");
    } else {
        html.raw("<h2>It will be able to</h2>\n<ul>\n");
        for scope in &claim.scopes {
            html.raw("<li>")
                .text(&scope.description)
                .raw(" (<code>")
                .text(&scope.name)
                .raw("</code>)</li>\n");
        }
        html.raw("</ul>\n");
    }
    let minutes = claim.expires_in_seconds.div_ceil(60).to_string();
    let attempts = remaining.unwrap_or(claim.attempts_remaining).to_string();
    if remaining.is_some() {
        html.raw("<p><strong>That code is wrong.</strong> ")
            .text(&attempts)
            .raw(" attempts left.</p>\n");
    }
    html.raw("<p>Enter the six-digit code the agent gave you. It works for about ")
        .text(&minutes)
        .raw(" more minutes, ")
        .text(&attempts)
        .raw(" wrong entries allowed.</p>\n<form method=\"post\" action=\"")
        .text(action)
        .raw("\">\n<input type=\"hidden\" name=\"_token\" value=\"")
        .text(csrf)
        .raw("\">\n<input type=\"hidden\" name=\"claim_attempt_token\" value=\"")
        .text(attempt_token)
        .raw("\">\n<label>Code <input type=\"text\" name=\"user_code\" inputmode=\"numeric\" ")
        .raw("autocomplete=\"one-time-code\" maxlength=\"9\" required></label>\n")
        .raw("<button type=\"submit\">Allow</button>\n</form>\n")
        .raw("<p>If you did not ask an agent to connect, close this page.</p>\n")
        .raw("</main>\n</body>\n</html>\n");
    html.into_string()
}

/// The page after a claim, linking the connected-agents page when one is
/// served.
fn render_claimed(connected: Option<&str>) -> String {
    let mut html = Html::default();
    html.raw("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n")
        .raw("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n")
        .raw("<title>The agent is connected</title>\n</head>\n<body>\n<main>\n")
        .raw("<h1>The agent is connected</h1>\n<p>It can now act as you. You can return to the agent.</p>\n");
    if let Some(connected) = connected {
        html.raw("<p><a href=\"")
            .text(connected)
            .raw("\">See and revoke the agents that act for you</a>.</p>\n");
    }
    html.raw("</main>\n</body>\n</html>\n");
    html.into_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::GrantScope;

    fn claim(registration_type: &str, signed_in_as: &str) -> PendingClaim {
        PendingClaim {
            registration_type: registration_type.to_owned(),
            provider: None,
            signed_in_as: signed_in_as.to_owned(),
            email: None,
            scopes: Vec::new(),
            expires_in_seconds: 0,
            attempts_remaining: 0,
            first_time_account: false,
            first_time_provider: false,
        }
    }

    #[test]
    fn agent_008_claim_page_escapes_and_shows_only_the_trust_list_name() {
        let claim = PendingClaim {
            provider: Some("Acme & Co".to_owned()),
            email: Some("<script>alert(1)</script>@corp.test".to_owned()),
            scopes: vec![GrantScope {
                name: "posts:read".to_owned(),
                description: "Read <your> posts".to_owned(),
            }],
            expires_in_seconds: 540,
            attempts_remaining: 5,
            first_time_provider: true,
            ..claim("identity_assertion", "alice@corp.test")
        };
        let page = render_claim(&claim, "/agent/claim", "cat_\"x", "csrf", None);
        assert!(page.contains("<strong>Acme &amp; Co</strong>"), "{page}");
        assert!(page.contains("<bdi>&lt;script&gt;alert(1)&lt;/script&gt;@corp.test</bdi>"));
        assert!(page.contains("Read &lt;your&gt; posts"));
        assert!(page.contains("value=\"cat_&quot;x\""));
        assert!(page.contains("about 9 more minutes"));
        assert!(page.contains("no agent from this provider has acted for you before"));
    }

    #[test]
    fn agent_009_signed_in_email_is_isolated() {
        // The user chose their address, so it is isolated as any text a
        // party other than the application chose.
        let page = render_claim(
            &claim("anonymous", "\u{202e}me@corp.test"),
            "/agent/claim",
            "cat_1",
            "csrf",
            None,
        );
        assert!(
            page.contains("signed in as <strong><bdi>\u{202e}me@corp.test</bdi></strong>"),
            "{page}"
        );
    }

    #[test]
    fn agent_009_repeated_form_fields_are_refused() {
        assert!(form_fields(b"user_code=1&user_code=2").is_none());
        assert_eq!(
            form_fields(b"user_code=123456&claim_attempt_token=cat_1")
                .and_then(|fields| fields.get("user_code").cloned()),
            Some("123456".to_owned())
        );
    }
}
