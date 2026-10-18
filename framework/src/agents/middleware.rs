//! The check every request to a tool's route passes (AGENT-001, AGENT-002).
//!
//! The server puts it on a tool's route after the route's own middleware.
//! A tool call the MCP endpoint dispatched passes straight through: the
//! endpoint already checked the caller's kind, scopes and limit, and the
//! caller is current for the whole call. Any other request is the browser
//! calling the route with the user's session: the viewer is a guest
//! (`public`) or the signed-in user (`user`), never an agent account, and a
//! session holds every scope. The check refuses a viewer whose kind the
//! tool does not accept, counts the call against the viewer's limit, and
//! makes the viewer the current caller for the handler and its policies.

use std::sync::Arc;

use async_trait::async_trait;

use super::AgentsConfig;
use super::caller::{CallerKind, ToolCaller};
use super::limit::{self, Refusal};
use super::tool::RegisteredTool;
use crate::error::FrameworkError;
use crate::http::{Request, Response};
use crate::middleware::{BoxedMiddleware, Middleware, Next, into_boxed};

/// The access check of one tool's route.
struct ToolAccess {
    name: &'static str,
    callers: &'static [CallerKind],
    config: Arc<AgentsConfig>,
}

/// The access check for `tool`'s route.
pub(crate) fn access(tool: &RegisteredTool, config: Arc<AgentsConfig>) -> BoxedMiddleware {
    into_boxed(ToolAccess {
        name: tool.name(),
        callers: tool.callers(),
        config,
    })
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
        let user = crate::auth::Auth::route_user().await?;
        let address = request.ip().and_then(|ip| ip.parse().ok());
        let caller = ToolCaller::browser(user, address);
        if !self.callers.contains(&caller.kind()) {
            return Err(match caller.kind() {
                CallerKind::Public => FrameworkError::domain("Unauthenticated.", 401),
                _ => FrameworkError::domain(
                    format!("The tool `{}` does not accept signed-in users.", self.name),
                    403,
                ),
            }
            .into());
        }
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
        caller.scope(next(request)).await
    }
}
