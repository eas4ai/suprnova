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
//!
//! A Live tool's action follows the same rules on the Live action route
//! (AGENT-012): a request with the header is checked and counted here,
//! and every other request carries a [`LiveToolGate`] the action kernel
//! checks once it knows which action the request invokes.

use std::sync::Arc;

use async_trait::async_trait;

use super::AgentsConfig;
use super::TOOL_CALL_HEADER;
use super::caller::{CallerKind, ToolCaller};
use super::limit::{self, Refusal};
use super::scope::Scope;
use super::tool::{RegisteredTool, ToolRegistry};
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

/// The access check of the Live action route for browser tool calls
/// (AGENT-012).
struct LiveToolAccess {
    registry: Arc<ToolRegistry>,
    config: Arc<AgentsConfig>,
}

/// Why a browser caller may not invoke a Live tool's action.
#[derive(Debug)]
pub(crate) enum BrowserToolRefusal {
    /// A guest, for a tool that does not accept the public.
    Unauthenticated,
    /// A signed-in user, for a tool that does not accept users; it holds
    /// the tool's name.
    NotAccepted(String),
    /// A caller without the tool's scopes.
    MissingScope {
        /// The tool's name.
        name: String,
        /// The tool's scopes, space-separated.
        scope: String,
    },
    /// Who the caller is could not be read.
    Failed(FrameworkError),
}

impl BrowserToolRefusal {
    /// The answer to the refused request: 401 for a guest, 403 otherwise,
    /// with `insufficient_scope` for a missing scope, as a route tool
    /// answers.
    pub(crate) fn into_response(self) -> HttpResponse {
        match self {
            Self::Unauthenticated => FrameworkError::domain("Unauthenticated.", 401).into(),
            Self::NotAccepted(name) => FrameworkError::domain(
                format!("The tool `{name}` does not accept signed-in users."),
                403,
            )
            .into(),
            Self::MissingScope { name, scope } => HttpResponse::from(FrameworkError::domain(
                format!("The tool `{name}` needs a browser session or a token with its scopes."),
                403,
            ))
            .header(
                "WWW-Authenticate",
                format!("Bearer error=\"insufficient_scope\", scope=\"{scope}\""),
            ),
            Self::Failed(error) => error.into(),
        }
    }
}

/// The caller-kind and scope check of the browser caller of `tool`, named
/// `name` in a refusal. Every request to a Live tool's action runs it, with
/// or without the tool-call header, as every request to a route tool's
/// route runs [`ToolAccess`] (AGENT-002).
fn admit_browser_caller(
    tool: &RegisteredTool,
    caller: &ToolCaller,
    name: &str,
) -> Result<(), BrowserToolRefusal> {
    if !tool.accepts(caller.kind()) {
        return Err(match caller.kind() {
            CallerKind::Public => BrowserToolRefusal::Unauthenticated,
            _ => BrowserToolRefusal::NotAccepted(name.to_string()),
        });
    }
    if !tool.missing_scopes(caller).is_empty() {
        let scope = tool
            .scopes()
            .iter()
            .map(Scope::name)
            .collect::<Vec<_>>()
            .join(" ");
        return Err(BrowserToolRefusal::MissingScope {
            name: name.to_string(),
            scope,
        });
    }
    Ok(())
}

/// The browser caller check of every Live tool action, for a request
/// without the tool-call header (AGENT-002, AGENT-012).
///
/// The Live action route's tool access puts it on every request it sees,
/// and the action kernel runs it once the snapshot is verified and it knows
/// which component and action the request invokes: the route's middleware
/// sees only bytes, and a check that read the action from them could
/// disagree with the kernel. A request with the header was checked, and
/// counted against the caller's tool budget, before the kernel, which then
/// holds it to the claimed tool; one without it is never counted.
#[derive(Clone)]
pub(crate) struct LiveToolGate {
    registry: Arc<ToolRegistry>,
}

impl LiveToolGate {
    /// Check the browser caller of `action` on `component`, from `address`,
    /// when that action is a tool's.
    pub(crate) async fn admit(
        &self,
        component: &suprnova_live::identity::ComponentName,
        action: &str,
        address: Option<std::net::IpAddr>,
    ) -> Result<(), BrowserToolRefusal> {
        let Some(tool) = self.registry.live_tool_of(component, action) else {
            return Ok(());
        };
        let caller = ToolAccess::caller_at(address)
            .await
            .map_err(BrowserToolRefusal::Failed)?;
        admit_browser_caller(tool, &caller, tool.name())
    }
}

/// The access check the Live action route runs: a request that carries
/// the [`TOOL_CALL_HEADER`](super::TOOL_CALL_HEADER) is checked and counted
/// here, and every other request carries the [`LiveToolGate`] the action
/// kernel checks it with.
pub(crate) fn live_access(
    registry: Arc<ToolRegistry>,
    config: Arc<AgentsConfig>,
) -> BoxedMiddleware {
    into_boxed(LiveToolAccess { registry, config })
}

/// The browser caller of the current request, which came from `address`:
/// who the route's guard signed in, if anyone. What a page's tool list is
/// filtered by (AGENT-014), and what a browser tool call is checked as.
pub(crate) async fn browser_caller(
    address: Option<std::net::IpAddr>,
) -> Result<ToolCaller, FrameworkError> {
    ToolAccess::caller_at(address).await
}

/// The refusal of a request that carries the
/// [`TOOL_CALL_HEADER`](super::TOOL_CALL_HEADER) to a route that serves no
/// tool, naming the tool it claims (AGENT-014).
///
/// A browser fills a tool's path from the agent's arguments and cannot see
/// which route the path reached: `post = "new"` on `/posts/{post}` would
/// reach a literal `/posts/new`. The server refuses such a call rather than
/// serve another route as the tool.
pub(crate) fn no_tool(name: String) -> BoxedMiddleware {
    into_boxed(NoTool { name })
}

struct NoTool {
    name: String,
}

#[async_trait]
impl Middleware for NoTool {
    async fn handle(&self, _request: Request, _next: Next) -> Response {
        Err(FrameworkError::domain(
            format!(
                "The tool call names the tool `{}`, but this route serves no tool.",
                self.name
            ),
            400,
        )
        .into())
    }
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
        Self::caller_at(request.ip().and_then(|ip| ip.parse().ok())).await
    }

    /// The caller of the current request without the tool-call mark, which
    /// came from `address`: who the route's guard signed in, if anyone.
    async fn caller_at(address: Option<std::net::IpAddr>) -> Result<ToolCaller, FrameworkError> {
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
        // A browser tool call names the tool it calls; reaching another
        // tool's route means its path reached a route other than the
        // tool's, which the browser cannot see (AGENT-014).
        if let Some(claimed) = request.header(TOOL_CALL_HEADER)
            && claimed != self.name
        {
            return Err(FrameworkError::domain(
                format!(
                    "The tool call names the tool `{claimed}`, but this route serves the tool \
                     `{}`.",
                    self.name
                ),
                400,
            )
            .into());
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

/// A browser's call of a Live tool reaches the ordinary Live action route,
/// with the island's snapshot, the session and the CSRF proof a click
/// carries. An ordinary action request passes straight through, uncounted.
/// One that carries the tool-call header is the call of the tool it names:
/// the tool must accept the browser's caller, a browser caller is never an
/// agent account, and the call counts against the caller's tool budget, per
/// user or per address for a guest. The Live kernel then requires the
/// request to invoke that tool's action on that tool's island.
#[async_trait]
impl Middleware for LiveToolAccess {
    async fn handle(&self, mut request: Request, next: Next) -> Response {
        request.set_live_tool_gate(LiveToolGate {
            registry: Arc::clone(&self.registry),
        });
        let Some(name) = request.header(TOOL_CALL_HEADER).map(str::to_string) else {
            return next(request).await;
        };
        let Some((tool, instance)) = self.registry.browser_tool(&name) else {
            return Err(
                FrameworkError::domain(format!("No Live tool is named `{name}`."), 400).into(),
            );
        };
        let Some(target) = tool.live_target() else {
            return Err(
                FrameworkError::domain(format!("No Live tool is named `{name}`."), 400).into(),
            );
        };
        let claim = crate::live::action::LiveToolClaim {
            component: target.component.clone(),
            action: target.action,
            instance: instance.to_string(),
        };
        let caller = ToolAccess::caller_of(&request).await?;
        admit_browser_caller(tool, &caller, &name).map_err(BrowserToolRefusal::into_response)?;
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
        request.set_live_tool_claim(claim);
        caller.scope(next(request)).await
    }
}
