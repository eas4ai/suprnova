//! The mark agent traffic carries on its [`Request`](crate::Request).
//!
//! Two kinds of request are agent traffic: a request to a route
//! `Router::agents` registered, which the server marks from its own route
//! table, and a tool call the MCP endpoint built after it authenticated the
//! caller. The CSRF, session, bearer-token and RenderCache middleware step
//! aside for both. The mark lives in a private field of the request that
//! nothing parsed from the wire can set, and only this crate builds one, so
//! a client cannot forge it with a header or a cookie.

use std::net::IpAddr;
use std::sync::Arc;

use super::AgentRoute;
#[cfg(doc)]
use super::caller::ToolCaller;
use crate::middleware::MiddlewareRegistry;
use crate::routing::Router;

/// The mark of agent traffic.
pub(crate) enum AgentMark {
    /// A request to a route `Router::agents` registered.
    Endpoint(EndpointContext),
    /// A tool call the MCP endpoint dispatched.
    Call(CallContext),
}

/// What the MCP endpoint needs to dispatch a tool call: the router and the
/// middleware the request it serves came through.
#[derive(Clone)]
pub(crate) struct EndpointContext {
    pub(crate) router: Arc<Router>,
    pub(crate) registry: Arc<MiddlewareRegistry>,
    pub(crate) route: AgentRoute,
    pub(crate) peer_ip: Option<IpAddr>,
}

/// The tool call a request carries out. Its caller is current for the
/// whole call ([`ToolCaller::current`]).
pub(crate) struct CallContext {
    /// The tool's name.
    pub(crate) tool: &'static str,
}

impl AgentMark {
    /// The mark of a request to a route `Router::agents` registered.
    pub(crate) fn endpoint(
        router: Arc<Router>,
        registry: Arc<MiddlewareRegistry>,
        route: AgentRoute,
        peer_ip: Option<IpAddr>,
    ) -> Self {
        Self::Endpoint(EndpointContext {
            router,
            registry,
            route,
            peer_ip,
        })
    }

    /// The endpoint context, for a request to an agents route.
    pub(crate) fn as_endpoint(&self) -> Option<&EndpointContext> {
        match self {
            Self::Endpoint(context) => Some(context),
            Self::Call(_) => None,
        }
    }

    /// The tool call, for a request the endpoint dispatched.
    pub(crate) fn as_call(&self) -> Option<&CallContext> {
        match self {
            Self::Call(call) => Some(call),
            Self::Endpoint(_) => None,
        }
    }
}
