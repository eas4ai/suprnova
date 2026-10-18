//! The check every request to a tool's route passes (AGENT-001, AGENT-002).
//!
//! The server puts it on a tool's route after the route's own middleware.
//! A tool call the MCP endpoint dispatched passes straight through: the
//! endpoint already checked the caller's kind, scopes and limit, and the
//! caller is current for the whole call.
//!
//! Any other request reaches the route directly, and its caller is one of:
//!
//! - a guest (`public`);
//! - a user the session guard resolved from the browser's session, a
//!   browser caller holding every scope;
//! - a user signed in some other way, by a bearer token, basic auth or a
//!   custom guard: a `user` caller on the direct channel, holding no scope.
//!   A tool that declares scopes refuses it with 403 `insufficient_scope`.
//!
//! The caller-kind and scope checks apply to every request to the route.
//! Only a browser tool call, a request carrying the
//! [`TOOL_CALL_HEADER`](super::TOOL_CALL_HEADER) the browser helpers send,
//! counts against the caller's tool budget, so marking a handler `#[tool]`
//! adds no limit to the route's page loads and form posts, and a limiter
//! store that is down refuses only tool calls. The header is no credential:
//! a browser tool call relies on the route's own CSRF middleware, as any
//! form post does, so a tool on a `GET` route should only read.

use std::sync::Arc;

use async_trait::async_trait;

use super::AgentsConfig;
use super::TOOL_CALL_HEADER;
use super::caller::{CallerKind, ToolCaller};
use super::limit::{self, Refusal};
use super::scope::Scope;
use super::tool::RegisteredTool;
use crate::error::FrameworkError;
use crate::http::{HttpResponse, Request, Response};
use crate::middleware::{BoxedMiddleware, Middleware, Next, into_boxed};

/// The access check of one tool's route.
struct ToolAccess {
    name: &'static str,
    callers: &'static [CallerKind],
    scopes: &'static [Scope],
    config: Arc<AgentsConfig>,
}

/// The access check for `tool`'s route.
pub(crate) fn access(tool: &RegisteredTool, config: Arc<AgentsConfig>) -> BoxedMiddleware {
    into_boxed(ToolAccess {
        name: tool.name(),
        callers: tool.callers(),
        scopes: tool.scopes(),
        config,
    })
}

impl ToolAccess {
    /// Whether the tool accepts callers of `kind`; a public tool accepts
    /// every caller.
    fn accepts(&self, kind: CallerKind) -> bool {
        self.callers.contains(&kind) || self.callers.contains(&CallerKind::Public)
    }

    /// The caller of a request without the tool-call mark.
    async fn caller_of(request: &Request) -> Result<ToolCaller, FrameworkError> {
        let address = request.ip().and_then(|ip| ip.parse().ok());
        let Some(user) = crate::auth::Auth::route_user().await? else {
            return Ok(ToolCaller::route_guest(address));
        };
        let guard = crate::auth::request_state::route_guard()
            .or_else(crate::auth::request_state::used_guard)
            .unwrap_or_else(crate::auth::Auth::default_guard_name);
        if crate::auth::request_state::is_session_user(&guard, &user.get_auth_identifier()) {
            Ok(ToolCaller::session_user(user, address))
        } else {
            Ok(ToolCaller::direct_user(user, address))
        }
    }
}

#[async_trait]
impl Middleware for ToolAccess {
    async fn handle(&self, request: Request, next: Next) -> Response {
        if let Some(call) = request.agent_mark().and_then(|mark| mark.as_call()) {
            if call.tool != self.name {
                // The dispatcher checks the route it reached is the tool's;
                // reaching another tool's route is a defect, never served.
                return Err(FrameworkError::internal(format!(
                    "a call of the tool `{}` reached the route of the tool `{}`",
                    call.tool, self.name
                ))
                .into());
            }
            return next(request).await;
        }
        let caller = Self::caller_of(&request).await?;
        if !self.accepts(caller.kind()) {
            return Err(match caller.kind() {
                CallerKind::Public => FrameworkError::domain("Unauthenticated.", 401),
                _ => FrameworkError::domain(
                    format!("The tool `{}` does not accept signed-in users.", self.name),
                    403,
                ),
            }
            .into());
        }
        let missing = self.scopes.iter().any(|scope| !caller.can(scope.name()));
        if missing {
            let scope = self
                .scopes
                .iter()
                .map(Scope::name)
                .collect::<Vec<_>>()
                .join(" ");
            return Err(HttpResponse::from(FrameworkError::domain(
                format!(
                    "The tool `{}` needs a browser session or a token with its scopes.",
                    self.name
                ),
                403,
            ))
            .header(
                "WWW-Authenticate",
                format!("Bearer error=\"insufficient_scope\", scope=\"{scope}\""),
            ));
        }
        if request.header(TOOL_CALL_HEADER).is_some() {
            match limit::admit(&self.config, &caller).await {
                Ok(()) => {}
                Err(Refusal::Limited(retry_after)) => {
                    return Err(FrameworkError::rate_limited(
                        Some(std::time::Duration::from_secs(retry_after)),
                        "Too many tool calls.",
                    )
                    .into());
                }
                Err(Refusal::Unavailable(error)) => {
                    return Err(FrameworkError::domain(
                        format!("The tool call limit cannot be checked: {error}"),
                        503,
                    )
                    .into());
                }
            }
        }
        caller.scope(next(request)).await
    }
}
