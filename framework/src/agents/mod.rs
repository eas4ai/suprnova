//! Agent tools and the MCP endpoint (AGENT-001 to AGENT-004).
//!
//! An application exposes a capability to agents by marking an ordinary
//! routed `#[handler]` with `#[tool]`. A tool call is a request to that
//! route, so it runs the route's middleware, bindings, form request
//! validation, `#[authorize]` policies and rate limits, whichever surface it
//! arrives on:
//!
//! - **The MCP endpoint.** [`Router::agents`] serves `POST /mcp` over the
//!   Streamable HTTP transport of MCP revision 2026-07-28, and the
//!   `initialize`-based revisions 2025-06-18 and 2025-11-25 without
//!   sessions. A `tools/call` builds a request for the tool's route and
//!   serves it through the same routing and middleware a request off the
//!   wire takes, with the caller the endpoint authenticated.
//! - **The browser.** The tool's route answers the page that calls it with
//!   the user's session, and checks that the viewer is a caller the tool
//!   accepts.
//!
//! Three kinds of caller exist ([`CallerKind`]): `public` callers present no
//! credential, `user` callers are a signed-in user by browser session or by
//! a token the user approved for a client, and `agent` callers are agent
//! accounts. A tool declares the kinds it accepts and the scopes a token
//! caller needs; the handler and every policy read the caller through
//! [`ToolCaller`].
//!
//! Tokens are verified by [`CallerResolver`]s the application registers on
//! [`AgentsConfig`]. The endpoint never honours a token no resolver accepts,
//! and never forwards one to the route.
//!
//! ```rust,ignore
//! use suprnova::agents::AgentsConfig;
//!
//! pub fn register() -> Router {
//!     Router::new()
//!         .post("/posts", posts::store)
//!         .name("posts.store")
//!         .agents(AgentsConfig::new().resolver(my_token_resolver()))
//! }
//! ```

mod auth;
mod caller;
mod dispatch;
mod input;
mod limit;
mod mark;
mod mcp;
mod metadata;
mod middleware;
mod scope;
#[cfg(feature = "testing")]
pub mod testing;
mod tool;

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use crate::error::FrameworkError;
use crate::routing::Router;
use hyper::Method;

pub use auth::{
    AuthorizationServerMetadata, CallerResolver, MetadataKind, ResolveContext, ResolveError,
};
pub use caller::{AgentRegistration, CallerChannel, CallerKind, ToolCaller};
pub(crate) use limit::address_bucket;
pub(crate) use mark::AgentMark;
pub use scope::{Scope, ScopeRecord, declared_scopes};
#[cfg(feature = "testing")]
pub use testing::{AgentsFake, McpResponse, McpTestClient, McpToolResult};
pub(crate) use tool::LiveTarget;
pub use tool::{
    __LiveResolver, __LiveToolDeclaration, __ResolverFuture, __ToolDeclaration, __ToolMeta,
    LiveToolRecord, RegisteredTool, ToolAnnotations, ToolRecord, ToolRegistry,
};

/// The default path of the MCP endpoint.
pub const DEFAULT_MCP_PATH: &str = "/mcp";

/// The header a browser tool call carries, naming the tool: what tells a
/// page's call of a tool apart from an ordinary request to the tool's
/// route. Only a request that carries it counts against the caller's tool
/// budget; the caller checks apply to every request to the route.
pub const TOOL_CALL_HEADER: &str = "Suprnova-Tool-Call";

/// How the application serves agents: the MCP endpoint's path and limits,
/// the origins it accepts, and the resolvers that verify tokens.
///
/// Every limit has a default, so `AgentsConfig::new()` is a working
/// endpoint at `/mcp` that serves public tools.
#[derive(Clone)]
pub struct AgentsConfig {
    pub(crate) path: String,
    pub(crate) allowed_origins: Vec<String>,
    pub(crate) max_body_bytes: usize,
    pub(crate) max_result_bytes: usize,
    pub(crate) call_timeout: Duration,
    pub(crate) caller_limit: CallLimit,
    pub(crate) address_limit: CallLimit,
    pub(crate) request_limit: CallLimit,
    pub(crate) resolver_timeout: Duration,
    pub(crate) resolvers: Vec<Arc<dyn CallerResolver>>,
    pub(crate) authorization_server: Option<Arc<dyn AuthorizationServerMetadata>>,
    pub(crate) server_name: Option<String>,
    pub(crate) server_version: Option<String>,
    pub(crate) instructions: Option<String>,
    pub(crate) resource_name: Option<String>,
}

/// A number of tool calls allowed in a window, for one caller or one
/// client address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallLimit {
    /// The calls one caller may make in a window.
    pub max_calls: u32,
    /// The window's length.
    pub window: Duration,
}

impl CallLimit {
    /// `max_calls` calls per `window`.
    pub const fn new(max_calls: u32, window: Duration) -> Self {
        Self { max_calls, window }
    }

    /// `max_calls` calls per minute.
    pub const fn per_minute(max_calls: u32) -> Self {
        Self::new(max_calls, Duration::from_secs(60))
    }
}

impl Default for AgentsConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for AgentsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentsConfig")
            .field("path", &self.path)
            .field("allowed_origins", &self.allowed_origins)
            .field("max_body_bytes", &self.max_body_bytes)
            .field("max_result_bytes", &self.max_result_bytes)
            .field("call_timeout", &self.call_timeout)
            .field("caller_limit", &self.caller_limit)
            .field("address_limit", &self.address_limit)
            .field("resolvers", &self.resolvers.len())
            .field("authorization_server", &self.authorization_server.is_some())
            .finish_non_exhaustive()
    }
}

impl AgentsConfig {
    /// The endpoint at `/mcp`, accepting a 1 MiB request body and 30
    /// seconds of run time per call, 120 calls a minute per caller, 60 a
    /// minute per client address for public calls, 600 requests of any kind
    /// a minute per client address, and 5 seconds for a resolver to decide,
    /// with no resolver: only public tools can be called until one is added.
    pub fn new() -> Self {
        Self {
            path: DEFAULT_MCP_PATH.to_string(),
            allowed_origins: Vec::new(),
            max_body_bytes: 1024 * 1024,
            max_result_bytes: 8 * 1024 * 1024,
            call_timeout: Duration::from_secs(30),
            caller_limit: CallLimit::per_minute(120),
            address_limit: CallLimit::per_minute(60),
            request_limit: CallLimit::per_minute(600),
            resolver_timeout: Duration::from_secs(5),
            resolvers: Vec::new(),
            authorization_server: None,
            server_name: None,
            server_version: None,
            instructions: None,
            resource_name: None,
        }
    }

    /// Serve the endpoint at `path` instead of `/mcp`. A path without a
    /// leading `/` gets one.
    pub fn path(mut self, path: impl Into<String>) -> Self {
        let path = path.into();
        self.path = if path.starts_with('/') {
            path
        } else {
            format!("/{path}")
        };
        self
    }

    /// Accept requests whose `Origin` is `origin`, such as
    /// `https://app.example.com`, besides the application's own origin
    /// (`APP_URL`). A request with any other `Origin` gets 403, which stops
    /// a web page from reaching the endpoint through the user's browser
    /// (DNS rebinding). A request without `Origin`, which every non-browser
    /// client sends, is accepted.
    pub fn allow_origin(mut self, origin: impl Into<String>) -> Self {
        self.allowed_origins
            .push(origin.into().trim_end_matches('/').to_ascii_lowercase());
        self
    }

    /// The largest request body the endpoint reads, in bytes. A larger one
    /// gets 413 before it is read.
    pub fn max_body_bytes(mut self, bytes: usize) -> Self {
        self.max_body_bytes = bytes;
        self
    }

    /// The largest response a tool's route may give, in bytes. A larger one
    /// becomes a tool error rather than being held in memory.
    pub fn max_result_bytes(mut self, bytes: usize) -> Self {
        self.max_result_bytes = bytes;
        self
    }

    /// How long one tool call may run. A call that runs longer is stopped
    /// and answered with a tool error.
    pub fn call_timeout(mut self, timeout: Duration) -> Self {
        self.call_timeout = timeout;
        self
    }

    /// How many tool calls one user or agent may make in a window. Calls
    /// through the browser and the MCP endpoint count together.
    pub fn caller_limit(mut self, limit: CallLimit) -> Self {
        self.caller_limit = limit;
        self
    }

    /// How many public tool calls one client address may make in a window.
    pub fn address_limit(mut self, limit: CallLimit) -> Self {
        self.address_limit = limit;
        self
    }

    /// How many requests of any kind one client address may send the MCP
    /// endpoint in a window: 600 a minute unless set. It bounds what the
    /// tool budget does not count, such as `tools/list`, `initialize`,
    /// `ping` and requests refused for a bad token.
    pub fn request_limit(mut self, limit: CallLimit) -> Self {
        self.request_limit = limit;
        self
    }

    /// How long a resolver may take to decide on a token: 5 seconds unless
    /// set. A resolver that takes longer is treated as unavailable, and the
    /// request gets 503.
    pub fn resolver_timeout(mut self, timeout: Duration) -> Self {
        self.resolver_timeout = timeout;
        self
    }

    /// Add a resolver that verifies bearer tokens. Resolvers are asked in
    /// the order they were added; the first that recognizes a token decides.
    pub fn resolver(mut self, resolver: Arc<dyn CallerResolver>) -> Self {
        self.resolvers.push(resolver);
        self
    }

    /// Serve authorization server metadata (RFC 8414) from `provider` at
    /// `/.well-known/oauth-authorization-server` and
    /// `/.well-known/openid-configuration` under the application's public
    /// root, and, when the provider's issuer is a path of this application
    /// such as `https://app.example/oauth`, at the path-inserted locations
    /// for that path too. Without a provider those paths answer 404.
    pub fn authorization_server(mut self, provider: Arc<dyn AuthorizationServerMetadata>) -> Self {
        self.authorization_server = Some(provider);
        self
    }

    /// The name and version the endpoint reports as its `serverInfo`. The
    /// name defaults to the application's name (`APP_NAME`), the version to
    /// the framework's.
    pub fn server_info(mut self, name: impl Into<String>, version: impl Into<String>) -> Self {
        self.server_name = Some(name.into());
        self.server_version = Some(version.into());
        self
    }

    /// Guidance for agents on how to use the application's tools, sent as
    /// the `instructions` of discovery and `initialize`.
    pub fn instructions(mut self, instructions: impl Into<String>) -> Self {
        self.instructions = Some(instructions.into());
        self
    }

    /// The `resource_name` of the protected resource metadata. Defaults to
    /// the application's name.
    pub fn resource_name(mut self, name: impl Into<String>) -> Self {
        self.resource_name = Some(name.into());
        self
    }

    /// The endpoint's path.
    pub fn mcp_path(&self) -> &str {
        &self.path
    }
}

/// What a route `Router::agents` registered serves.
#[derive(Clone)]
pub(crate) enum AgentRoute {
    /// The MCP endpoint.
    Mcp(Arc<AgentsConfig>),
    /// Protected resource metadata (RFC 9728).
    ProtectedResource(Arc<AgentsConfig>),
    /// Authorization server metadata (RFC 8414) of the given kind.
    AuthorizationServer(Arc<AgentsConfig>, MetadataKind),
}

/// The agent state of a [`Router`]: the routes `Router::agents` registered
/// and, once checked, the tool registry.
#[derive(Default)]
pub(crate) struct RouterAgents {
    config: Option<Arc<AgentsConfig>>,
    routes: HashMap<(Method, String), AgentRoute>,
    registry: OnceLock<Result<Arc<ToolRegistry>, FrameworkError>>,
}

impl RouterAgents {
    /// The MCP endpoint's path, when the router serves one.
    #[cfg(feature = "testing")]
    pub(crate) fn config_path(&self) -> Option<String> {
        self.config.as_ref().map(|config| config.path.clone())
    }
}

impl Router {
    /// Serve agents: the MCP endpoint at the configured path (`/mcp` by
    /// default) and the OAuth discovery documents (AGENT-003, AGENT-004).
    ///
    /// Registers `POST` at the endpoint's path, the protected resource
    /// metadata at `/.well-known/oauth-protected-resource` and its
    /// path-inserted form for the endpoint, and the authorization server
    /// metadata at `/.well-known/oauth-authorization-server` and
    /// `/.well-known/openid-configuration`. Every path is relative to the
    /// application's public root, so an application served under a path
    /// prefix is discoverable without the host root. `GET` and `DELETE` on
    /// the endpoint answer 405.
    ///
    /// # Panics
    ///
    /// Panics when one of these routes is already registered, or when this
    /// router already serves agents. Use [`Router::try_agents`] for an
    /// error instead.
    pub fn agents(self, config: AgentsConfig) -> Router {
        self.try_agents(config).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Fallible sibling of [`Router::agents`].
    ///
    /// # Errors
    ///
    /// Returns an error when one of the routes is already registered, or
    /// when this router already serves agents.
    pub fn try_agents(self, config: AgentsConfig) -> Result<Router, FrameworkError> {
        if self.agents.config.is_some() {
            return Err(FrameworkError::internal(
                "Router::agents was called twice: an application serves one MCP endpoint",
            ));
        }
        let config = Arc::new(config);
        let mcp_path = config.path.clone();
        let mut router: Router = self.try_post(&mcp_path, mcp::handle)?.into();
        router.agents.routes.insert(
            (Method::POST, mcp_path.clone()),
            AgentRoute::Mcp(config.clone()),
        );
        let resource_paths = [
            metadata::PROTECTED_RESOURCE_PATH.to_string(),
            format!("{}{mcp_path}", metadata::PROTECTED_RESOURCE_PATH),
        ];
        for path in resource_paths {
            router = router.try_get(&path, metadata::protected_resource)?.into();
            router.agents.routes.insert(
                (Method::GET, path),
                AgentRoute::ProtectedResource(config.clone()),
            );
        }
        for kind in [
            MetadataKind::OAuthAuthorizationServer,
            MetadataKind::OpenIdConfiguration,
        ] {
            let path = kind.well_known_path().to_string();
            router = router
                .try_get(&path, metadata::authorization_server)?
                .into();
            router.agents.routes.insert(
                (Method::GET, path),
                AgentRoute::AuthorizationServer(config.clone(), kind),
            );
        }
        // An issuer at a path of this application's own origin is also
        // served at its path-inserted location (RFC 8414): an issuer on
        // another host serves its own metadata.
        if let Some(provider) = &config.authorization_server
            && let Some(issuer_path) = metadata::issuer_path(provider.as_ref())
        {
            for kind in [
                MetadataKind::OAuthAuthorizationServer,
                MetadataKind::OpenIdConfiguration,
            ] {
                let path = format!("{}{issuer_path}", kind.well_known_path());
                router = router
                    .try_get(&path, metadata::authorization_server)?
                    .into();
                router.agents.routes.insert(
                    (Method::GET, path),
                    AgentRoute::AuthorizationServer(config.clone(), kind),
                );
            }
        }
        router.agents.config = Some(config);
        Ok(router)
    }

    /// What the route `(method, pattern)` serves, when `Router::agents`
    /// registered it.
    pub(crate) fn agent_route(&self, method: &Method, pattern: &str) -> Option<AgentRoute> {
        if self.agents.routes.is_empty() {
            return None;
        }
        self.agents
            .routes
            .get(&(method.clone(), pattern.to_string()))
            .cloned()
    }

    /// The agents configuration: the one `Router::agents` was given, or
    /// the defaults for a router that serves tools only to the browser.
    pub(crate) fn agents_config(&self) -> Arc<AgentsConfig> {
        self.agents
            .config
            .clone()
            .unwrap_or_else(|| Arc::new(AgentsConfig::new()))
    }

    /// The application's tools, checked against this router once (see
    /// [`Router::prepare_tools`]).
    ///
    /// # Errors
    ///
    /// The problem [`ToolRegistry::build`] found, on every call.
    pub fn tool_registry(&self) -> Result<Arc<ToolRegistry>, FrameworkError> {
        self.checked_tools().clone()
    }

    /// The registry, built and checked on the first call.
    pub(crate) fn checked_tools(&self) -> &Result<Arc<ToolRegistry>, FrameworkError> {
        self.agents
            .registry
            .get_or_init(|| ToolRegistry::build(self).map(Arc::new))
    }

    /// The tool access check of the route `(method, pattern)`, when a tool's
    /// handler serves it.
    pub(crate) fn tool_access(
        &self,
        method: &Method,
        pattern: &str,
    ) -> Option<crate::middleware::BoxedMiddleware> {
        let registry = self.checked_tools().as_ref().ok()?;
        // The Live action route serves every island's actions; a browser's
        // call of a Live tool is one of them, marked by the tool-call
        // header (AGENT-012).
        if *method == Method::POST
            && pattern == crate::live::LIVE_UPDATE_PATH
            && registry.has_browser_tools()
        {
            return Some(middleware::live_access(
                Arc::clone(registry),
                self.agents_config(),
            ));
        }
        if registry.tools().is_empty() {
            return None;
        }
        let tool = registry.for_route(method, pattern)?;
        Some(middleware::access(tool, self.agents_config()))
    }
}

/// The input schema of a `#[tool(input = T)]`: the JSON Schema (draft
/// 2020-12) `schemars` derives from `T`, its `serde` attributes and its
/// `validator` attributes. `T` must also be what the handler validates.
#[doc(hidden)]
pub fn __input_schema<T>() -> serde_json::Value
where
    T: schemars::JsonSchema + serde::de::DeserializeOwned + validator::Validate,
{
    schema_of::<T>()
}

/// The output schema of a `#[tool(output = U)]`.
#[doc(hidden)]
pub fn __output_schema<U: schemars::JsonSchema>() -> serde_json::Value {
    schema_of::<U>()
}

/// The input schema of a tool that takes no input: an object with no
/// properties, as MCP recommends.
#[doc(hidden)]
pub fn __empty_input_schema() -> serde_json::Value {
    serde_json::json!({ "type": "object", "additionalProperties": false })
}

/// The draft 2020-12 schema of `T`.
fn schema_of<T: schemars::JsonSchema>() -> serde_json::Value {
    schemars::generate::SchemaSettings::draft2020_12()
        .into_generator()
        .into_root_schema_for::<T>()
        .to_value()
}

/// A value a `#[resolve(...)]` resolver returned, encoded with its mount
/// parameter's codec so the mount reads it as if a page had sent it.
///
/// # Errors
///
/// A 500 naming the property when the value does not fit the codec: the
/// resolver and the `#[mount]` parameter disagree, an application defect.
#[doc(hidden)]
pub fn __encode_resolved<T: serde::Serialize + 'static>(
    property: &str,
    value: &T,
    codec: &suprnova_live::state::ModelCodec,
) -> Result<suprnova_live::canonical::CanonicalValue, FrameworkError> {
    codec
        .encode(value, &suprnova_live::limits::InputLimits::default())
        .map_err(|issue| {
            FrameworkError::internal(format!(
                "the resolver of the Live mount property `{property}` returned a value its \
                 codec refuses: {issue}"
            ))
        })
}

/// What a headless Live tool call asks to mount, as the Gate policy of
/// `live:{component}.mount` reads it (AGENT-013): the component and the
/// mount properties the agent sent, after their codecs accepted them and
/// before anything is loaded. Properties the application fills itself with
/// `#[resolve(...)]` are not among them: they are filled after the policy
/// allows the mount.
///
/// Each property holds the one JSON form its codec decodes, whatever form
/// the agent wrote: a 64-bit integer is a JSON integer whether the agent
/// sent a number or a decimal string, a UUID is hyphenated in lower case,
/// and a date-time is in RFC 3339. Write a policy so a property it cannot
/// read refuses the mount:
///
/// ```rust,ignore
/// Gate::define::<ToolCaller, LiveMountRequest>("live:app.counter.mount", |caller, mount| {
///     caller.is_user()
///         && mount
///             .property("start")
///             .and_then(|v| v.as_u64())
///             .is_some_and(|start| start < 100)
/// });
/// ```
#[derive(Debug, Clone)]
pub struct LiveMountRequest {
    component: String,
    properties: serde_json::Map<String, serde_json::Value>,
}

impl LiveMountRequest {
    pub(crate) fn new(
        component: impl Into<String>,
        properties: serde_json::Map<String, serde_json::Value>,
    ) -> Self {
        Self {
            component: component.into(),
            properties,
        }
    }

    /// The component's registered name, `app.counter`.
    pub fn component(&self) -> &str {
        &self.component
    }

    /// Every mount property the agent sent, each in its codec's one form.
    pub fn properties(&self) -> &serde_json::Map<String, serde_json::Value> {
        &self.properties
    }

    /// The mount property `name`, in its codec's one form, or `None` when
    /// the agent did not send it.
    pub fn property(&self, name: &str) -> Option<&serde_json::Value> {
        self.properties.get(name)
    }
}

/// The `Agents` facade: test helpers for tool calls.
pub struct Agents;
