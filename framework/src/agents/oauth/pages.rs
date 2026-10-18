//! The framework's default approval and connected-agents pages
//! (AGENT-017): plain server-rendered HTML with no script, style or asset,
//! for an application without a kit.
//!
//! Both pages require the signed-in user, refuse to be framed, are never
//! cached and send no referrer. Every form they render posts with the
//! session's CSRF token, and the routes that take those forms run the CSRF
//! middleware themselves. Markup comes only from `&'static str`; every
//! value, most of which a client chose, is escaped on its way in (see
//! [`Html`]).

use std::sync::Arc;

use magnetar::authorization_server::{AuthorizationServer, PendingAuthorizationView};

use super::OAuthServer;
use super::grants::{self, ClientGrant};
use crate::csrf::CsrfMiddleware;
use crate::error::FrameworkError;
use crate::http::{HttpResponse, Request, Response};
use crate::magnetar_integration::plugin_host::signed_in_session;
use crate::routing::Router;

/// What both pages need from the server's setup.
pub(super) struct Pages {
    server: Arc<AuthorizationServer>,
    route_prefix: String,
    sign_in_url: String,
    return_to_parameter: String,
}

impl Pages {
    pub(super) fn new(server: Arc<AuthorizationServer>, setup: &OAuthServer) -> Self {
        Self {
            server,
            route_prefix: setup.route_prefix.trim_end_matches('/').to_owned(),
            sign_in_url: crate::routing::url::to(&setup.sign_in_path),
            return_to_parameter: setup.return_to_parameter.clone(),
        }
    }

    /// Send a guest to sign in, to come back to this page after.
    fn sign_in(&self, request: &Request) -> HttpResponse {
        let return_to = match request.query() {
            Some(query) if !query.is_empty() => format!("{}?{query}", request.path()),
            _ => request.path().to_owned(),
        };
        let separator = if self.sign_in_url.contains('?') {
            '&'
        } else {
            '?'
        };
        let pair = url::form_urlencoded::Serializer::new(String::new())
            .append_pair(&self.return_to_parameter, &return_to)
            .finish();
        guarded(HttpResponse::new().status(302))
            .header("Location", format!("{}{separator}{pair}", self.sign_in_url))
    }
}

/// Serve the approval page at `path`.
pub(super) fn mount_approval(
    router: Router,
    pages: Arc<Pages>,
    path: &str,
) -> Result<Router, FrameworkError> {
    Ok(router
        .try_get(path, move |request: Request| {
            let pages = Arc::clone(&pages);
            async move { approval(pages, request).await }
        })?
        .into())
}

/// Serve the connected-agents page at `path` and its revoke form at
/// `{path}/{grant}/revoke`.
pub(super) fn mount_connected(
    router: Router,
    pages: Arc<Pages>,
    path: &str,
) -> Result<Router, FrameworkError> {
    let path = path.trim_end_matches('/').to_owned();
    let list_pages = Arc::clone(&pages);
    let list_path = path.clone();
    let router: Router = router
        .try_get(&path, move |request: Request| {
            let pages = Arc::clone(&list_pages);
            let path = list_path.clone();
            async move { connected(pages, path, request).await }
        })?
        .into();
    let revoke_path = path.clone();
    Ok(router
        .try_post(
            &format!("{path}/{{grant}}/revoke"),
            move |request: Request| {
                let pages = Arc::clone(&pages);
                let path = revoke_path.clone();
                async move { revoke(pages, path, request).await }
            },
        )?
        .middleware(CsrfMiddleware::new().without_xsrf_cookie())
        .into())
}

/// The approval page: the client, its host and every scope it asks for,
/// and a form that posts the user's decision to the authorization server.
async fn approval(pages: Arc<Pages>, request: Request) -> Response {
    // The user signs in before anything about the request is shown.
    let Some(session) = signed_in_session().await? else {
        return Ok(pages.sign_in(&request));
    };
    let Some(id) = request
        .query_param("authorization")
        .filter(|id| !id.is_empty())
    else {
        return Ok(notice(
            400,
            "Nothing to approve",
            "This link does not name an authorization request.",
        ));
    };
    let pending = pages
        .server
        .pending_authorization(&id, &session)
        .await
        .map_err(unavailable)?;
    let Some(pending) = pending else {
        return Ok(notice(
            404,
            "This request has ended",
            "It expired or was already answered. Start again from the app that asked.",
        ));
    };
    let token = csrf_token()?;
    let action = crate::routing::url::to(&format!(
        "{}/authorizations/{}",
        pages.route_prefix, pending.id
    ));
    Ok(guarded(HttpResponse::html(render_approval(
        &pending, &action, &token,
    ))))
}

fn render_approval(pending: &PendingAuthorizationView, action: &str, token: &str) -> String {
    let mut html = Html::default();
    html.raw("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n")
        .raw("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>Allow ")
        .text(&pending.client_name)
        .raw(" to act for you?</title>\n</head>\n<body>\n<main>\n<h1>Allow ")
        .text(&pending.client_name)
        .raw(" to act for you?</h1>\n<p><strong>")
        .text(&pending.client_name)
        .raw("</strong>, from <strong>")
        .text(&pending.client_host)
        .raw("</strong>, wants to use this application as you.</p>\n");
    if pending.scopes.is_empty() {
        html.raw("<p>It asks for no permissions: it can use only what needs none.</p>\n");
    } else {
        html.raw("<h2>It asks to</h2>\n<ul>\n");
        for scope in &pending.scopes {
            html.raw("<li>")
                .text(&scope.description)
                .raw(" (<code>")
                .text(&scope.name)
                .raw("</code>)</li>\n");
        }
        html.raw("</ul>\n");
    }
    html.raw("<p>If you allow it, you return to <code>")
        .text(&pending.redirect_uri)
        .raw("</code>.</p>\n<form method=\"post\" action=\"")
        .text(action)
        .raw("\">\n<input type=\"hidden\" name=\"_token\" value=\"")
        .text(token)
        .raw("\">\n<button type=\"submit\" name=\"decision\" value=\"approve\">Allow</button>\n")
        .raw("<button type=\"submit\" name=\"decision\" value=\"deny\">Deny</button>\n")
        .raw("</form>\n</main>\n</body>\n</html>\n");
    html.0
}

/// The connected-agents page: every client grant of the signed-in user,
/// each with a revoke button.
async fn connected(pages: Arc<Pages>, path: String, request: Request) -> Response {
    let Some(session) = signed_in_session().await? else {
        return Ok(pages.sign_in(&request));
    };
    let grants = grants::list(&pages.server, session.user_id()).await?;
    let token = csrf_token()?;
    Ok(guarded(HttpResponse::html(render_connected(
        &grants, &path, &token,
    ))))
}

fn render_connected(grants: &[ClientGrant], path: &str, token: &str) -> String {
    let mut html = Html::default();
    html.raw("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n")
        .raw("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n")
        .raw("<title>Connected apps</title>\n</head>\n<body>\n<main>\n<h1>Connected apps</h1>\n");
    if grants.is_empty() {
        html.raw("<p>No apps can act for you.</p>\n");
    } else {
        html.raw("<p>These apps can use this application as you. Revoking one ends its access at once.</p>\n<ul>\n");
        for grant in grants {
            let action = crate::routing::url::to(&format!("{path}/{}/revoke", grant.id));
            let created = grant.created_at.format("%Y-%m-%d %H:%M UTC").to_string();
            let used = grant.last_used_at.map_or_else(
                || "never".to_owned(),
                |used| used.format("%Y-%m-%d %H:%M UTC").to_string(),
            );
            html.raw("<li>\n<h2>")
                .text(&grant.client_name)
                .raw("</h2>\n<p>From <strong>")
                .text(&grant.client_host)
                .raw("</strong>. Allowed ")
                .text(&created)
                .raw(", last used ")
                .text(&used)
                .raw(".</p>\n");
            if grant.scopes.is_empty() {
                html.raw("<p>No permissions.</p>\n");
            } else {
                html.raw("<ul>\n");
                for scope in &grant.scopes {
                    html.raw("<li>")
                        .text(&scope.description)
                        .raw(" (<code>")
                        .text(&scope.name)
                        .raw("</code>)</li>\n");
                }
                html.raw("</ul>\n");
            }
            html.raw("<form method=\"post\" action=\"")
                .text(&action)
                .raw("\">\n<input type=\"hidden\" name=\"_token\" value=\"")
                .text(token)
                .raw("\">\n<button type=\"submit\">Revoke ")
                .text(&grant.client_name)
                .raw("</button>\n</form>\n</li>\n");
        }
        html.raw("</ul>\n");
    }
    html.raw("</main>\n</body>\n</html>\n");
    html.0
}

/// The revoke form's action: end one of the signed-in user's grants and
/// return to the page.
async fn revoke(pages: Arc<Pages>, path: String, request: Request) -> Response {
    let Some(session) = signed_in_session().await? else {
        return Ok(notice(
            401,
            "Sign in first",
            "Sign in to manage your connected apps.",
        ));
    };
    let grant = request.param("grant").unwrap_or_default().to_owned();
    if !grants::revoke(&pages.server, session.user_id(), &grant).await? {
        return Ok(notice(
            404,
            "No such app",
            "You have no connected app by that name. It may already be revoked.",
        ));
    }
    Ok(guarded(HttpResponse::new().status(303)).header("Location", crate::routing::url::to(&path)))
}

/// A short page that says one thing.
fn notice(status: u16, title: &'static str, message: &'static str) -> HttpResponse {
    let mut html = Html::default();
    html.raw("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n")
        .raw("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>")
        .raw(title)
        .raw("</title>\n</head>\n<body>\n<main>\n<h1>")
        .raw(title)
        .raw("</h1>\n<p>")
        .raw(message)
        .raw("</p>\n</main>\n</body>\n</html>\n");
    guarded(HttpResponse::html(html.0).status(status))
}

/// The headers every answer of these pages carries: never framed, so no
/// page can overlay a button and trick the user into pressing it; never
/// cached; and no referrer, since the address names a pending request.
fn guarded(response: HttpResponse) -> HttpResponse {
    response
        .header("Cache-Control", "no-store")
        .header("X-Frame-Options", "DENY")
        .header("Content-Security-Policy", "frame-ancestors 'none'")
        .header("Referrer-Policy", "no-referrer")
}

/// The session's CSRF token for the page's forms.
fn csrf_token() -> Result<String, FrameworkError> {
    crate::session::get_csrf_token().ok_or_else(|| {
        FrameworkError::internal(
            "the OAuth server's pages post their forms with the session's CSRF token; install \
             the session middleware",
        )
    })
}

fn unavailable(error: magnetar::Error) -> FrameworkError {
    tracing::error!(%error, "the authorization server's store failed");
    FrameworkError::domain("the authorization server is unavailable", 503)
}

/// An HTML document under construction. Markup only comes in as
/// `&'static str`, through [`Self::raw`]; every value the page shows goes
/// through [`Self::text`], which escapes it for both text and quoted
/// attributes. So no client-supplied name, host or address can become
/// markup.
#[derive(Default)]
struct Html(String);

impl Html {
    /// Append fixed markup.
    fn raw(&mut self, markup: &'static str) -> &mut Self {
        self.0.push_str(markup);
        self
    }

    /// Append `value` as text, escaped.
    fn text(&mut self, value: &str) -> &mut Self {
        for ch in value.chars() {
            match ch {
                '&' => self.0.push_str("&amp;"),
                '<' => self.0.push_str("&lt;"),
                '>' => self.0.push_str("&gt;"),
                '"' => self.0.push_str("&quot;"),
                '\'' => self.0.push_str("&#39;"),
                _ => self.0.push(ch),
            }
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_017_html_escapes_every_value() {
        let mut html = Html::default();
        html.raw("<p title=\"").text("\"><script>'&").raw("\">");
        assert_eq!(html.0, "<p title=\"&quot;&gt;&lt;script&gt;&#39;&amp;\">");
    }
}
