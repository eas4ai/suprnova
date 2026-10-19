//! The framework's default approval and connected-agents pages
//! (AGENT-017): plain server-rendered HTML with no script, style or asset,
//! for an application without a kit. With agent auth on, the
//! connected-agents page lists the user's agent registrations too.
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
use crate::agents::AgentAccount;
use crate::agents::agent_auth::MountedAgentAuth;
use crate::agents::page::{Html, csrf_token, guard_errors, guarded, notice, sign_in_redirect};
use crate::csrf::CsrfMiddleware;
use crate::error::FrameworkError;
use crate::http::{HttpResponse, Request, Response};
use crate::magnetar_integration::plugin_host::signed_in_session;
use crate::routing::Router;

/// What both pages need from the server's setup.
pub(super) struct Pages {
    server: Arc<AuthorizationServer>,
    agent: Option<Arc<MountedAgentAuth>>,
    route_prefix: String,
    sign_in_url: String,
    return_to_parameter: String,
}

impl Pages {
    pub(super) fn new(
        server: Arc<AuthorizationServer>,
        agent: Option<Arc<MountedAgentAuth>>,
        setup: &OAuthServer,
    ) -> Self {
        Self {
            server,
            agent,
            route_prefix: setup.route_prefix.trim_end_matches('/').to_owned(),
            sign_in_url: crate::routing::url::to(&setup.sign_in_path),
            return_to_parameter: setup.return_to_parameter.clone(),
        }
    }

    /// Send a guest to sign in, to come back to this page after.
    fn sign_in(&self, request: &Request) -> HttpResponse {
        sign_in_redirect(&self.sign_in_url, &self.return_to_parameter, request)
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
            async move { guard_errors(approval(pages, request).await) }
        })?
        .into())
}

/// Serve the connected-agents page at `path`, its revoke form at
/// `{path}/{grant}/revoke` and, with agent auth on, the agent revoke form
/// at `{path}/agents/{registration}/revoke`.
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
            async move { guard_errors(connected(pages, path, request).await) }
        })?
        .into();
    let revoke_path = path.clone();
    let revoke_pages = Arc::clone(&pages);
    let router: Router = router
        .try_post(
            &format!("{path}/{{grant}}/revoke"),
            move |request: Request| {
                let pages = Arc::clone(&revoke_pages);
                let path = revoke_path.clone();
                async move { guard_errors(revoke(pages, path, request).await) }
            },
        )?
        .middleware(CsrfMiddleware::new().without_xsrf_cookie())
        .into();
    if pages.agent.is_none() {
        return Ok(router);
    }
    let agent_path = path.clone();
    Ok(router
        .try_post(
            &format!("{path}/agents/{{registration}}/revoke"),
            move |request: Request| {
                let pages = Arc::clone(&pages);
                let path = agent_path.clone();
                async move { guard_errors(revoke_agent(pages, path, request).await) }
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
        // The title holds no markup to isolate a name in, so it names none.
        .raw("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n")
        .raw("<title>Allow an app to act for you?</title>\n</head>\n<body>\n<main>\n<h1>Allow ")
        .client(&pending.client_name)
        .raw(" to act for you?</h1>\n<p><strong>")
        .client(&pending.client_name)
        .raw("</strong>, from <strong>")
        .client(&pending.client_host)
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
        .client(&pending.redirect_uri)
        .raw("</code>.</p>\n<form method=\"post\" action=\"")
        .text(action)
        .raw("\">\n<input type=\"hidden\" name=\"_token\" value=\"")
        .text(token)
        .raw("\">\n<button type=\"submit\" name=\"decision\" value=\"approve\">Allow</button>\n")
        .raw("<button type=\"submit\" name=\"decision\" value=\"deny\">Deny</button>\n")
        .raw("</form>\n</main>\n</body>\n</html>\n");
    html.into_string()
}

/// The connected-agents page: every client grant of the signed-in user,
/// each with a revoke button.
async fn connected(pages: Arc<Pages>, path: String, request: Request) -> Response {
    let Some(session) = signed_in_session().await? else {
        return Ok(pages.sign_in(&request));
    };
    let grants = grants::list(&pages.server, session.user_id()).await?;
    let agents = match &pages.agent {
        Some(agent) => Some(grants::list_agent_accounts(&agent.engine, session.user_id()).await?),
        None => None,
    };
    let token = csrf_token()?;
    Ok(guarded(HttpResponse::html(render_connected(
        &grants,
        agents.as_deref(),
        &path,
        &token,
    ))))
}

fn render_connected(
    grants: &[ClientGrant],
    agents: Option<&[AgentAccount]>,
    path: &str,
    token: &str,
) -> String {
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
                .client(&grant.client_name)
                .raw("</h2>\n<p>From <strong>")
                .client(&grant.client_host)
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
                .client(&grant.client_name)
                .raw("</button>\n</form>\n</li>\n");
        }
        html.raw("</ul>\n");
    }
    if let Some(agents) = agents {
        render_agents(&mut html, agents, path, token);
    }
    html.raw("</main>\n</body>\n</html>\n");
    html.into_string()
}

/// The page's "Agents" section: every agent registration bound to the
/// user, each with a revoke button.
fn render_agents(html: &mut Html, agents: &[AgentAccount], path: &str, token: &str) {
    html.raw("<h2>Agents</h2>\n");
    if agents.is_empty() {
        html.raw("<p>No agents can act for you.</p>\n");
        return;
    }
    html.raw("<p>These agents can use this application as you. Revoking one ends its access at once.</p>\n<ul>\n");
    for agent in agents {
        let action = crate::routing::url::to(&format!("{path}/agents/{}/revoke", agent.id));
        let created = agent.created_at.format("%Y-%m-%d %H:%M UTC").to_string();
        // The provider's name is the application's trust list entry, never
        // one the provider sent.
        html.raw("<li>\n<h3>");
        match &agent.provider {
            Some(provider) => {
                html.raw("Agent through ").text(provider);
            }
            None => {
                html.raw("Agent");
            }
        }
        html.raw("</h3>\n<p>")
            .raw(registration_type_words(&agent.registration_type))
            .raw(". Registered ")
            .text(&created);
        if let Some(claimed) = agent.claimed_at {
            html.raw(", claimed ")
                .text(&claimed.format("%Y-%m-%d %H:%M UTC").to_string());
        }
        html.raw(".</p>\n");
        if agent.scopes.is_empty() {
            html.raw("<p>No permissions.</p>\n");
        } else {
            html.raw("<ul>\n");
            for scope in &agent.scopes {
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
            .raw("\">\n<button type=\"submit\">Revoke this agent</button>\n</form>\n</li>\n");
    }
    html.raw("</ul>\n");
}

/// How an agent registered, in words.
pub(crate) fn registration_type_words(registration_type: &str) -> &'static str {
    match registration_type {
        "anonymous" => "Registered on its own and claimed by you",
        "service_auth" => "Registered with your email address",
        "identity_assertion" => "Signed in through a provider you use",
        _ => "An agent account",
    }
}

/// The agent revoke form's action: end one of the signed-in user's agent
/// registrations and return to the page.
async fn revoke_agent(pages: Arc<Pages>, path: String, request: Request) -> Response {
    let Some(session) = signed_in_session().await? else {
        return Ok(notice(
            401,
            "Sign in first",
            "Sign in to manage your connected agents.",
        ));
    };
    let Some(agent) = &pages.agent else {
        return Ok(notice(404, "No such agent", "Agent accounts are not on."));
    };
    let registration = request.param("registration").unwrap_or_default().to_owned();
    let revoked = agent
        .engine
        .revoke_registration(session.user_id(), &registration, request.ip().as_deref())
        .await
        .map_err(unavailable)?;
    if !revoked {
        return Ok(notice(
            404,
            "No such agent",
            "You have no connected agent by that name. It may already be revoked.",
        ));
    }
    Ok(guarded(HttpResponse::new().status(303)).header("Location", crate::routing::url::to(&path)))
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

fn unavailable(error: magnetar::Error) -> FrameworkError {
    tracing::error!(%error, "the authorization server's store failed");
    FrameworkError::domain("the authorization server is unavailable", 503)
}
