//! The server tools a page offers in the browser (AGENT-014).
//!
//! A page offers the tools the application shares with every page
//! ([`Router::share_tools`]), the tools its route and route group name with
//! `with_tools`, and the tools its handler adds: [`PageTools::add`] during
//! the request, or `with_tools` on an Inertia response or a Live document.
//! The server keeps only the tools the viewer may call:
//!
//! - a guest sees the tools that accept the public;
//! - a user signed in by the browser session sees the tools that accept
//!   users or the public, with every scope;
//! - a user signed in another way, by a bearer token or basic auth, sees only
//!   the user tools that declare no scope;
//! - no viewer sees an agent-only tool, since a browser viewer is never an
//!   agent account.
//!
//! A list whose tools all accept the public is the same for every viewer,
//! so the viewer is not read at all and a cached page stays shared. A list
//! that depends on the viewer reads who the viewer is, which RenderCache
//! records as an identity read and partitions or declines the page by.
//!
//! The list travels as the Inertia page's `agentTools` field, and as a
//! `<script type="application/json" id="suprnova-agent-tools">` element in
//! the head of a Live page or any other page ([`PageTools::element`]). The
//! browser registers each tool with WebMCP and calls the tool's route with
//! the page's session and CSRF token, the request
//! [`TestClient::tool`](crate::testing::TestClient::tool) sends. The route
//! checks the caller again on every call: the list decides what a page
//! offers, never what a caller may do.
//!
//! The list filters on caller kinds and scopes, not on `#[authorize]`
//! policies. A tool a policy refuses for some users is still offered to
//! them and refused when called; offer it only where the policy allows, by
//! adding it inside `if Gate::allows(...)`.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use hyper::Method;
use serde_json::{Map, Value, json};

use super::caller::{CallerKind, ToolCaller};
use super::tool::{RegisteredTool, ToolRegistry};
use crate::error::FrameworkError;
use crate::http::{Request, Response};
use crate::middleware::{BoxedMiddleware, Middleware, Next, into_boxed};
use crate::routing::Router;

/// The most tools one page offers, the bound the Live runtime puts on an
/// island's tools: every tool's schema rides on every response of the
/// page, so a page cannot grow without limit.
pub(crate) const MAX_PAGE_TOOLS: usize = 64;

/// The `id` of the element that carries a page's tools.
pub(crate) const ELEMENT_ID: &str = "suprnova-agent-tools";

/// The server tools the current page offers (AGENT-014).
///
/// The route, its group and [`Router::share_tools`] name most of them.
/// `PageTools` adds tools for the rest of the request from any handler, and
/// writes the element a plain view or a WebAssembly shell carries in its
/// head.
///
/// ```rust,ignore
/// use suprnova::PageTools;
///
/// #[handler]
/// pub async fn show(post: Post) -> Response {
///     if Gate::allows("update", &post).await {
///         PageTools::add(["posts.update"]).await?;
///     }
///     Inertia::render("Posts/Show", props)
/// }
/// ```
pub struct PageTools;

impl PageTools {
    /// Offer the tools `names` on the current request's page, beside the
    /// ones its route names.
    ///
    /// Async because a tool that does not accept every viewer makes the
    /// list depend on who the viewer is, and that is read here, after the
    /// route's middleware signed the viewer in.
    ///
    /// # Errors
    ///
    /// Outside a request the framework serves, or when the viewer cannot
    /// be read. A name that is not a routed tool a browser can call fails
    /// the page's render, naming it.
    pub async fn add<I>(names: I) -> Result<(), FrameworkError>
    where
        I: IntoIterator<Item = &'static str>,
    {
        let state = RequestPageTools::current().ok_or_else(|| {
            FrameworkError::internal(
                "PageTools::add() offers tools on the page of a request the framework serves; \
                 call it from a handler",
            )
        })?;
        state.add(names.into_iter().collect());
        state.resolve_viewer_if_needed(&[]).await
    }

    /// The element carrying the page's tools, for the `<head>` of a plain
    /// view or of the page a WebAssembly front end boots in:
    /// `<script type="application/json" id="suprnova-agent-tools">`, or
    /// nothing when the page offers no tool. A Live page gets it from its
    /// document's bootstrap markup, and an Inertia page in its page object,
    /// so neither needs this.
    ///
    /// It takes the request whose page it is: the list is the one that
    /// request's route and handler built.
    ///
    /// # Errors
    ///
    /// When a name the request added is not a routed tool a browser can
    /// call.
    pub fn element(request: &Request) -> Result<crate::view::TrustedHtml, FrameworkError> {
        let _ = request;
        let html = match payload_now(&[])? {
            Some(payload) => element_html(&payload),
            None => String::new(),
        };
        trusted(html)
    }
}

/// `html`, the element this module wrote, as markup a template inserts
/// unescaped.
pub(crate) fn trusted(html: String) -> Result<crate::view::TrustedHtml, FrameworkError> {
    let reason = crate::view::TrustedMarkupReason::new(
        "the page tools element, JSON escaped so it cannot close its script element",
    )
    .map_err(|_| FrameworkError::internal("the page tools element could not be marked"))?;
    crate::view::TrustedHtml::framework_generated(html, reason)
        .map_err(|_| FrameworkError::internal("the page tools element is too large"))
}

/// What the server stamps on a request whose router has tools: the checked
/// registry, the tools every page offers, and the limits the browser
/// applies to a call. A private value of the request's container scope, so
/// nothing the request carries can set it.
#[derive(Clone)]
pub(crate) struct PageToolsContext {
    registry: Arc<ToolRegistry>,
    shared: Arc<[&'static str]>,
    max_result_bytes: usize,
    call_timeout: Duration,
}

/// The page tools of one request: its context, the names its route layers
/// and handler added, and the viewer, once read.
#[derive(Default)]
pub(crate) struct RequestPageTools(Mutex<State>);

#[derive(Default)]
struct State {
    context: Option<PageToolsContext>,
    names: Vec<&'static str>,
    viewer: Option<ToolCaller>,
}

impl RequestPageTools {
    /// The current request's page tools, or `None` outside a container
    /// scope.
    fn current() -> Option<Arc<Self>> {
        crate::container::scope::ContainerScope::current()?;
        let value = crate::container::scope::resolve(
            std::any::TypeId::of::<Self>(),
            std::any::type_name::<Self>(),
            &|| Arc::new(Self::default()) as Arc<dyn std::any::Any + Send + Sync>,
        )
        .ok()?;
        value.downcast::<Self>().ok()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn add(&self, names: Vec<&'static str>) {
        self.lock().names.extend(names);
    }

    /// Read the viewer when the list, with `extra`, holds a tool that does
    /// not accept every viewer and the viewer is not read yet.
    async fn resolve_viewer_if_needed(&self, extra: &[&'static str]) -> Result<(), FrameworkError> {
        let needed = {
            let state = self.lock();
            state.viewer.is_none()
                && match listed(&state, extra) {
                    Ok(Some(listed)) => !listed.public(),
                    // The render reports a name it cannot list.
                    Ok(None) | Err(_) => false,
                }
        };
        if needed {
            let viewer = super::middleware::browser_caller(None).await?;
            self.lock().viewer = Some(viewer);
        }
        Ok(())
    }
}

/// Stamp `context` on the current request, before its middleware runs.
pub(crate) fn stamp(context: PageToolsContext) {
    if let Some(state) = RequestPageTools::current() {
        state.lock().context = Some(context);
    }
}

/// The step the server runs after a route's middleware, right before its
/// handler: when the tools the route names do not all accept every viewer,
/// it reads the viewer, now that the route's `auth` has signed them in.
pub(crate) fn viewer_step() -> BoxedMiddleware {
    into_boxed(ViewerStep)
}

struct ViewerStep;

#[async_trait]
impl Middleware for ViewerStep {
    async fn handle(&self, request: Request, next: Next) -> Response {
        if let Some(state) = RequestPageTools::current() {
            state.resolve_viewer_if_needed(&[]).await?;
        }
        next(request).await
    }
}

/// The route middleware `with_tools` adds: it offers its tools on the
/// page of every request the route or group serves. The router reads its
/// names back at boot to check them ([`check`]).
pub(crate) struct PageToolsLayer {
    names: Arc<[&'static str]>,
}

impl PageToolsLayer {
    /// The layer offering `names`.
    pub(crate) fn new<I: IntoIterator<Item = &'static str>>(names: I) -> Self {
        Self {
            names: names.into_iter().collect(),
        }
    }

    /// The tools it offers.
    pub(crate) fn names(&self) -> Arc<[&'static str]> {
        Arc::clone(&self.names)
    }
}

#[async_trait]
impl Middleware for PageToolsLayer {
    async fn handle(&self, request: Request, next: Next) -> Response {
        match RequestPageTools::current() {
            Some(state) => state.add(self.names.to_vec()),
            None => tracing::warn!("page tools outside a container scope were dropped"),
        }
        next(request).await
    }
}

/// The tools a page lists before the viewer filter.
struct Listed {
    registry: Arc<ToolRegistry>,
    tools: Vec<&'static str>,
    viewer: Option<ToolCaller>,
    max_result_bytes: usize,
    call_timeout: Duration,
}

impl Listed {
    /// The listed tools.
    fn tools(&self) -> impl Iterator<Item = &RegisteredTool> {
        self.tools.iter().filter_map(|name| self.registry.get(name))
    }

    /// Whether every listed tool accepts every viewer.
    fn public(&self) -> bool {
        self.tools()
            .all(|tool| tool.callers().contains(&CallerKind::Public))
    }
}

/// The tools the request's page lists with `extra`, checked, before the
/// viewer filter; `None` when it lists none.
fn listed(state: &State, extra: &[&'static str]) -> Result<Option<Listed>, FrameworkError> {
    let Some(context) = &state.context else {
        return match state.names.iter().chain(extra).next() {
            Some(name) => Err(not_offered(
                name,
                "the application has no tool a browser can call",
            )),
            None => Ok(None),
        };
    };
    let mut names: Vec<&'static str> = context
        .shared
        .iter()
        .chain(&state.names)
        .chain(extra)
        .copied()
        .collect();
    names.sort_unstable();
    names.dedup();
    if names.is_empty() {
        return Ok(None);
    }
    for name in &names {
        offerable(&context.registry, name).map_err(|reason| not_offered(name, reason))?;
    }
    if names.len() > MAX_PAGE_TOOLS {
        return Err(FrameworkError::internal(format!(
            "the page offers {} tools; a page offers at most {MAX_PAGE_TOOLS}",
            names.len()
        )));
    }
    Ok(Some(Listed {
        registry: Arc::clone(&context.registry),
        tools: names,
        viewer: state.viewer.clone(),
        max_result_bytes: context.max_result_bytes,
        call_timeout: context.call_timeout,
    }))
}

/// Whether a page may offer the tool `name`: a routed tool a browser
/// viewer can call. The reason it may not, otherwise.
fn offerable(registry: &ToolRegistry, name: &str) -> Result<(), &'static str> {
    match registry.get(name) {
        Some(tool) if tool.live_target().is_some() => {
            Err("it is a Live tool, which its island offers")
        }
        Some(tool)
            if !tool
                .callers()
                .iter()
                .any(|kind| matches!(kind, CallerKind::Public | CallerKind::User)) =>
        {
            Err("it accepts only agent accounts, and a browser viewer is never one")
        }
        Some(_) => Ok(()),
        None if registry.live_tool(name).is_some() => {
            Err("it is a Live tool, which its island offers")
        }
        None => Err("the application has no routed tool of that name"),
    }
}

fn not_offered(name: &str, reason: &str) -> FrameworkError {
    FrameworkError::internal(format!("the page offers the tool `{name}`, but {reason}"))
}

/// The payload of the page's tools, `extra` added, as the browser reads it,
/// reading the viewer when the list needs it; `None` when the page offers
/// the viewer no tool.
pub(crate) async fn payload(extra: &[&'static str]) -> Result<Option<Value>, FrameworkError> {
    let Some(state) = RequestPageTools::current() else {
        return match extra.first() {
            Some(name) => Err(not_offered(
                name,
                "the response was not built inside a request the framework serves",
            )),
            None => Ok(None),
        };
    };
    state.resolve_viewer_if_needed(extra).await?;
    let state = state.lock();
    render(listed(&state, extra)?)
}

/// [`payload`] where the viewer is already read, for a render that cannot
/// wait: every way a tool reaches the list reads the viewer as it does.
pub(crate) fn payload_now(extra: &[&'static str]) -> Result<Option<Value>, FrameworkError> {
    let Some(state) = RequestPageTools::current() else {
        return match extra.first() {
            Some(name) => Err(not_offered(
                name,
                "the page was not built inside a request the framework serves",
            )),
            None => Ok(None),
        };
    };
    let state = state.lock();
    render(listed(&state, extra)?)
}

/// The payload of `listed` after the viewer filter.
fn render(listed: Option<Listed>) -> Result<Option<Value>, FrameworkError> {
    let Some(listed) = listed else {
        return Ok(None);
    };
    let public = listed.public();
    let tools: Vec<Value> = match (&listed.viewer, public) {
        (_, true) => listed.tools().map(definition).collect(),
        (Some(viewer), false) => listed
            .tools()
            .filter(|tool| tool.callable_by(viewer))
            .map(definition)
            .collect(),
        (None, false) => {
            return Err(FrameworkError::internal(
                "the page's tools depend on the viewer, who was not read before the render",
            ));
        }
    };
    if tools.is_empty() {
        return Ok(None);
    }
    let timeout_ms = u64::try_from(listed.call_timeout.as_millis()).unwrap_or(u64::MAX);
    Ok(Some(json!({
        "v": 1,
        "limits": {
            "maxResultBytes": listed.max_result_bytes,
            "callTimeoutMs": timeout_ms,
        },
        "tools": tools,
    })))
}

/// One tool as the page carries it: what WebMCP registers, plus the method
/// and the pre-parsed path the browser calls.
fn definition(tool: &RegisteredTool) -> Value {
    let annotations = tool.annotations();
    let mut out = Map::new();
    out.insert("name".into(), json!(tool.name()));
    if let Some(title) = tool.title() {
        out.insert("title".into(), json!(title));
    }
    out.insert("description".into(), json!(tool.description()));
    out.insert("inputSchema".into(), tool.input_schema().clone());
    out.insert(
        "annotations".into(),
        json!({
            "readOnlyHint": annotations.read_only() == Some(true),
            "untrustedContentHint": annotations.untrusted_content() == Some(true),
            "consequentialHint": annotations.destructive() == Some(true),
        }),
    );
    out.insert("method".into(), json!(tool.method().as_str()));
    out.insert("path".into(), path_pieces(tool.pattern()));
    Value::Object(out)
}

/// `pattern` as the pieces the browser fills: literal text, with the
/// application's public root joined into the first piece, and parameters
/// with their optional and catch-all flags. The browser never parses the
/// router's placeholder grammar.
pub(crate) fn path_pieces(pattern: &str) -> Value {
    let mut pieces: Vec<Value> = Vec::new();
    let mut text = crate::routing::root::current().to_string();
    let mut rest = pattern;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}').map(|close| open + close) else {
            break;
        };
        text.push_str(&rest[..open]);
        if !text.is_empty() {
            pieces.push(json!({ "text": std::mem::take(&mut text) }));
        }
        let inner = &rest[open + 1..close];
        let (name, _field, optional) = crate::routing::binding::split_placeholder(inner);
        pieces.push(json!({
            "param": name,
            "optional": optional,
            "catchAll": inner.starts_with('*'),
        }));
        rest = &rest[close + 1..];
    }
    text.push_str(rest);
    if !text.is_empty() {
        pieces.push(json!({ "text": text }));
    }
    Value::Array(pieces)
}

/// The element carrying `payload`, its JSON escaped so no value can close
/// the script element or change how the tokenizer reads it.
pub(crate) fn element_html(payload: &Value) -> String {
    let mut html = Vec::with_capacity(256);
    html.extend_from_slice(b"<script type=\"application/json\" id=\"");
    html.extend_from_slice(ELEMENT_ID.as_bytes());
    html.extend_from_slice(b"\">");
    // Writing a `Value` into a `Vec` cannot fail.
    let _ = serde_json::to_writer(crate::inertia::SlashEscaping(&mut html), payload);
    html.extend_from_slice(b"</script>");
    String::from_utf8(html).unwrap_or_default()
}

/// The island filter of `component` for the current request: which of its
/// Live tools its island roots list. Every tool when they all accept the
/// public, so the viewer is not read and a public island stays shared;
/// otherwise the ones the viewer may call (AGENT-014).
pub(crate) async fn island_filter(
    component: &suprnova_live::identity::ComponentName,
) -> Result<suprnova_live::host::AgentToolFilter, FrameworkError> {
    use suprnova_live::host::AgentToolFilter;

    let registry = RequestPageTools::current().and_then(|state| {
        state
            .lock()
            .context
            .as_ref()
            .map(|context| Arc::clone(&context.registry))
    });
    let Some(registry) = registry else {
        return Ok(AgentToolFilter::All);
    };
    let tools = registry.live_tools_of(component);
    if tools
        .iter()
        .all(|tool| tool.callers().contains(&CallerKind::Public))
    {
        return Ok(AgentToolFilter::All);
    }
    let viewer = super::middleware::browser_caller(None).await?;
    Ok(AgentToolFilter::Only(
        tools
            .into_iter()
            .filter(|tool| tool.callable_by(&viewer))
            .map(|tool| tool.name().to_owned())
            .collect(),
    ))
}

impl Router {
    /// Offer the tools `names` on every page the application serves, as
    /// Inertia's `share` gives every page a prop (AGENT-014). A route, its
    /// group and its handler add their own with `with_tools` and
    /// [`PageTools::add`].
    ///
    /// Each name must be a routed `#[tool]` that accepts the public or
    /// users: the server refuses to start otherwise, naming the tool. Each
    /// page lists only the tools its viewer may call.
    pub fn share_tools<I: IntoIterator<Item = &'static str>>(mut self, names: I) -> Router {
        self.agents.shared_tools.extend(names);
        self
    }

    /// The context the server stamps on each request: `None` when the
    /// application has no tool, or its tools do not check.
    pub(crate) fn page_tools_context(&self) -> Option<PageToolsContext> {
        let registry = self.checked_tools().as_ref().ok()?;
        if !registry.has_tools() {
            return None;
        }
        let config = self.agents_config();
        Some(PageToolsContext {
            registry: Arc::clone(registry),
            shared: self.agents.shared_tools.iter().copied().collect(),
            max_result_bytes: config.max_result_bytes,
            call_timeout: config.call_timeout,
        })
    }
}

/// Check every tool the router offers on its pages, once, at boot: the
/// shared tools and every route's `with_tools`. Each must be a routed tool
/// a browser can call, a route offers at most [`MAX_PAGE_TOOLS`], and a
/// `GET` tool's input must hold only fields a query can carry.
pub(crate) fn check(router: &Router, registry: &ToolRegistry) -> Result<(), FrameworkError> {
    let shared = &router.agents.shared_tools;
    for name in shared {
        offerable(registry, name).map_err(|reason| {
            FrameworkError::internal(format!(
                "Router::share_tools names the tool `{name}`, but {reason}"
            ))
        })?;
    }
    let mut routes = router.page_tools_by_route();
    routes.sort_by(|a, b| (a.0.as_str(), a.1.as_str()).cmp(&(b.0.as_str(), b.1.as_str())));
    let mut checked: Vec<&'static str> = shared.clone();
    for (method, pattern, names) in &routes {
        for name in names {
            offerable(registry, name).map_err(|reason| {
                FrameworkError::internal(format!(
                    "the route {method} {pattern} offers the tool `{name}` with `with_tools`, \
                     but {reason}"
                ))
            })?;
        }
        let mut offered: Vec<&'static str> = shared.iter().chain(names).copied().collect();
        offered.sort_unstable();
        offered.dedup();
        if offered.len() > MAX_PAGE_TOOLS {
            return Err(FrameworkError::internal(format!(
                "the route {method} {pattern} offers {} tools; a page offers at most \
                 {MAX_PAGE_TOOLS}",
                offered.len()
            )));
        }
        checked.extend(names);
    }
    checked.sort_unstable();
    checked.dedup();
    for name in checked {
        if let Some(tool) = registry.get(name) {
            check_get_input(tool)?;
        }
    }
    Ok(())
}

/// Refuse a `GET` page tool whose input has a field the query cannot
/// carry. A form request on a `GET` reads its query, and a browser sends a
/// `GET` tool's arguments there as scalars only, so an object or a list
/// would never arrive.
fn check_get_input(tool: &RegisteredTool) -> Result<(), FrameworkError> {
    if *tool.method() != Method::GET {
        return Ok(());
    }
    let root = tool.input_schema();
    let Some(properties) = root.get("properties").and_then(Value::as_object) else {
        return Ok(());
    };
    for (field, schema) in properties {
        if !query_carries(root, schema, 0) {
            return Err(FrameworkError::internal(format!(
                "the GET tool `{}` is offered on a page, but its input field `{field}` is an \
                 object or a list, which a browser cannot send in a GET tool's query; make it \
                 a scalar, or serve the tool on POST",
                tool.name()
            )));
        }
    }
    Ok(())
}

/// Whether a query can carry a value of `schema`: a string, a number, an
/// integer, a boolean or null, or a choice among them. A schema that says
/// nothing of its type is taken at its word.
fn query_carries(root: &Value, schema: &Value, depth: usize) -> bool {
    // A schema that refers to itself would loop; nothing a query carries
    // nests that deep.
    if depth > 16 {
        return false;
    }
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        let target = reference
            .strip_prefix("#/")
            .map(|path| {
                path.split('/')
                    .try_fold(root, |value, key| value.get(key))
                    .cloned()
            })
            .unwrap_or_default();
        return match target {
            Some(target) => query_carries(root, &target, depth + 1),
            None => false,
        };
    }
    let scalar = |kind: &str| matches!(kind, "string" | "number" | "integer" | "boolean" | "null");
    match schema.get("type") {
        Some(Value::String(kind)) if !scalar(kind) => return false,
        Some(Value::Array(kinds)) if !kinds.iter().filter_map(Value::as_str).all(scalar) => {
            return false;
        }
        _ => {}
    }
    for choice in ["anyOf", "oneOf", "allOf"] {
        if let Some(branches) = schema.get(choice).and_then(Value::as_array)
            && !branches
                .iter()
                .all(|branch| query_carries(root, branch, depth + 1))
        {
            return false;
        }
    }
    for values in [
        schema.get("enum").and_then(Value::as_array).cloned(),
        schema.get("const").cloned().map(|value| vec![value]),
    ]
    .into_iter()
    .flatten()
    {
        if values
            .iter()
            .any(|value| value.is_object() || value.is_array())
        {
            return false;
        }
    }
    true
}
