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

use magnetar::agent_auth::SignedInUser;
use magnetar::plugin::WireResponse;
use serde_json::Value;

use crate::agents::page::{Html, csrf_token, guard_errors, guarded, notice, sign_in_redirect};
use crate::csrf::CsrfMiddleware;
use crate::error::FrameworkError;
use crate::http::{HttpResponse, Request, Response};
use crate::magnetar_integration::plugin_host::signed_in_session;
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

/// The signed-in user as the engine compares them with the claim.
async fn signed_in() -> Result<Option<SignedInUser>, FrameworkError> {
    Ok(signed_in_session().await?.map(|session| SignedInUser {
        user_id: session.user_id().to_owned(),
        auth_epoch: Some(session.auth_epoch()),
    }))
}

/// The page: who the agent will act as, through which provider, with which
/// scopes, and the form that takes the code.
async fn claim(page: Arc<ClaimPage>, request: Request) -> Response {
    // The user signs in before anything about the claim is shown.
    let Some(user) = signed_in().await? else {
        return Ok(sign_in_redirect(
            &page.sign_in_url,
            &page.return_to_parameter,
            &request,
        ));
    };
    let Some(token) = request
        .query_param("claim_attempt_token")
        .filter(|token| !token.is_empty())
    else {
        return Ok(notice(
            400,
            "Nothing to claim",
            "This link does not name a claim. Ask the agent for a new one.",
        ));
    };
    let answer = Answer::of(page.engine.pending_claim(&token, Some(&user)).await);
    match answer.status {
        200 => Ok(guarded(HttpResponse::html(render_claim(
            &answer.body,
            &crate::routing::url::to(&page.path),
            &token,
            &csrf_token()?,
            None,
        )))),
        // A session from before the user's last password change no longer
        // speaks for them.
        401 => Ok(sign_in_redirect(
            &page.sign_in_url,
            &page.return_to_parameter,
            &request,
        )),
        status => refused(status),
    }
}

/// The form's action: confirm the claim with the code the user typed.
async fn confirm(page: Arc<ClaimPage>, request: Request) -> Response {
    let Some(user) = signed_in().await? else {
        return Ok(notice(
            401,
            "Sign in first",
            "Sign in, then open the link the agent gave you again.",
        ));
    };
    let address = request.ip();
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
    let answer = Answer::of(
        page.engine
            .confirm_claim(&token, &code, Some(&user), address.as_deref())
            .await,
    );
    match answer.status {
        200 => Ok(guarded(HttpResponse::html(render_claimed(
            page.connected_path
                .as_deref()
                .map(crate::routing::url::to)
                .as_deref(),
        )))),
        400 if answer.error() == Some("invalid_user_code") => {
            let remaining = answer
                .body
                .get("attempts_remaining")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            // The page again, as it stands now, with the mistake named.
            let pending = Answer::of(page.engine.pending_claim(&token, Some(&user)).await);
            if pending.status != 200 {
                return refused(pending.status);
            }
            Ok(guarded(
                HttpResponse::html(render_claim(
                    &pending.body,
                    &crate::routing::url::to(&page.path),
                    &token,
                    &csrf_token()?,
                    Some(remaining),
                ))
                .status(400),
            ))
        }
        401 => Ok(notice(
            401,
            "Sign in again",
            "Your sign-in has ended. Sign in, then open the link the agent gave you again.",
        )),
        status => refused(status),
    }
}

/// The engine's answer, read as status and JSON body.
struct Answer {
    status: u16,
    body: Value,
}

impl Answer {
    fn of(response: WireResponse) -> Self {
        let effects = response.into_effects();
        Self {
            status: effects.status,
            body: effects.body.unwrap_or(Value::Null),
        }
    }

    fn error(&self) -> Option<&str> {
        self.body.get("error").and_then(Value::as_str)
    }
}

/// The page for a claim the engine refused.
fn refused(status: u16) -> Response {
    match status {
        403 => Ok(notice(
            403,
            "This claim is for another account",
            "Sign in with the email address the agent named, then open its link again.",
        )),
        404 => Ok(notice(
            404,
            "No such claim",
            "This link does not name a claim. Ask the agent for a new one.",
        )),
        410 => Ok(notice(
            410,
            "This code can no longer be used",
            "It expired or was entered wrongly too often. Ask the agent for a new one.",
        )),
        400 => Ok(notice(
            400,
            "The form is not valid",
            "Open the link the agent gave you again.",
        )),
        _ => Err(FrameworkError::domain("agent accounts are unavailable", 503).into()),
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

/// The claim page from the pending claim's JSON. `remaining` is set after
/// a wrong code.
fn render_claim(
    claim: &Value,
    action: &str,
    attempt_token: &str,
    csrf: &str,
    remaining: Option<u64>,
) -> String {
    let text = |name: &str| claim.get(name).and_then(Value::as_str).unwrap_or_default();
    let mut html = Html::default();
    html.raw("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n")
        .raw("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n")
        .raw("<title>Let an agent act for you?</title>\n</head>\n<body>\n<main>\n")
        .raw("<h1>Let an agent act for you?</h1>\n<p>");
    // The provider's name is the application's trust list entry: the page
    // never shows a name the provider or the agent sent.
    let provider = claim
        .get("provider")
        .and_then(|provider| provider.get("display_name"))
        .and_then(Value::as_str);
    match (text("registration_type"), provider) {
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
        .client(text("signed_in_as"))
        .raw("</strong>.");
    if let Some(email) = claim.get("email").and_then(Value::as_str) {
        html.raw(" The agent named <strong>")
            .client(email)
            .raw("</strong>.");
    }
    html.raw("</p>\n");
    let advisories: Vec<&str> = claim
        .get("advisories")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if advisories.contains(&"first_time_account") {
        html.raw("<p><strong>Warning:</strong> no agent has acted for your account before. Allow it only if you asked an agent to connect.</p>\n");
    }
    if advisories.contains(&"first_time_provider") {
        html.raw("<p><strong>Warning:</strong> no agent from this provider has acted for you before.</p>\n");
    }
    let scopes = claim
        .get("scopes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if scopes.is_empty() {
        html.raw("<p>It asks for no permissions: it can use only what needs none.</p>\n");
    } else {
        html.raw("<h2>It will be able to</h2>\n<ul>\n");
        for scope in &scopes {
            let field = |name: &str| scope.get(name).and_then(Value::as_str).unwrap_or_default();
            html.raw("<li>")
                .text(field("description"))
                .raw(" (<code>")
                .text(field("name"))
                .raw("</code>)</li>\n");
        }
        html.raw("</ul>\n");
    }
    let minutes = claim
        .get("expires_in")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .div_ceil(60)
        .to_string();
    let attempts = remaining
        .or_else(|| claim.get("attempts_remaining").and_then(Value::as_u64))
        .unwrap_or(0)
        .to_string();
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
    use serde_json::json;

    #[test]
    fn agent_008_claim_page_escapes_and_shows_only_the_trust_list_name() {
        let claim = json!({
            "registration_type": "identity_assertion",
            "signed_in_as": "alice@corp.test",
            "email": "<script>alert(1)</script>@corp.test",
            "provider": { "display_name": "Acme & Co" },
            "name": "Provider-Chosen Name",
            "scopes": [{ "name": "posts:read", "description": "Read <your> posts" }],
            "expires_in": 540,
            "attempts_remaining": 5,
            "advisories": ["first_time_provider"],
        });
        let page = render_claim(&claim, "/agent/claim", "cat_\"x", "csrf", None);
        assert!(page.contains("<strong>Acme &amp; Co</strong>"), "{page}");
        assert!(!page.contains("Provider-Chosen Name"));
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
        let claim = json!({
            "registration_type": "anonymous",
            "signed_in_as": "\u{202e}me@corp.test",
            "scopes": [],
        });
        let page = render_claim(&claim, "/agent/claim", "cat_1", "csrf", None);
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
