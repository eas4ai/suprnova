//! The tools an application declares, and their check against the router
//! (AGENT-001).
//!
//! `#[tool]` submits one [`ToolRecord`] per tool through `inventory`, keyed
//! by the type of its handler function. [`ToolRegistry::build`] finds each
//! tool's route by that key, so a tool call runs that route, and refuses an
//! application whose tools do not fit its routes.

use std::any::TypeId;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde_json::{Value, json};

use super::caller::{CallerKind, ToolCaller};
use super::scope::{Scope, ScopeRecord};
use crate::error::FrameworkError;
use crate::routing::Router;
use hyper::Method;

/// Hints a tool gives its clients about what it does. Clients treat them as
/// hints, never as guarantees.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ToolAnnotations {
    read_only: Option<bool>,
    destructive: Option<bool>,
    idempotent: Option<bool>,
    open_world: Option<bool>,
    untrusted_content: Option<bool>,
}

impl ToolAnnotations {
    /// The hints `#[tool(annotations(...))]` set.
    #[doc(hidden)]
    pub const fn __declare(
        read_only: Option<bool>,
        destructive: Option<bool>,
        idempotent: Option<bool>,
        open_world: Option<bool>,
        untrusted_content: Option<bool>,
    ) -> Self {
        Self {
            read_only,
            destructive,
            idempotent,
            open_world,
            untrusted_content,
        }
    }

    /// Whether the tool changes nothing.
    pub fn read_only(&self) -> Option<bool> {
        self.read_only
    }

    /// Whether the tool may destroy or overwrite data.
    pub fn destructive(&self) -> Option<bool> {
        self.destructive
    }

    /// Whether calling the tool twice with the same input changes nothing
    /// more than calling it once.
    pub fn idempotent(&self) -> Option<bool> {
        self.idempotent
    }

    /// Whether the tool reaches things outside the application.
    pub fn open_world(&self) -> Option<bool> {
        self.open_world
    }

    /// Whether the tool's result may carry content from untrusted sources.
    pub fn untrusted_content(&self) -> Option<bool> {
        self.untrusted_content
    }

    /// The MCP `annotations` object, or `None` when no hint is set.
    fn to_mcp(self, title: Option<&str>) -> Option<Value> {
        let mut out = serde_json::Map::new();
        if let Some(title) = title {
            out.insert("title".into(), json!(title));
        }
        for (key, value) in [
            ("readOnlyHint", self.read_only),
            ("destructiveHint", self.destructive),
            ("idempotentHint", self.idempotent),
            ("openWorldHint", self.open_world),
        ] {
            if let Some(value) = value {
                out.insert(key.into(), json!(value));
            }
        }
        (!out.is_empty()).then_some(Value::Object(out))
    }
}

/// What `#[tool]` declares about one tool, whatever it calls: a route's
/// handler or a Live action. Built only by the macro.
#[doc(hidden)]
pub struct __ToolMeta {
    /// The tool's name.
    pub name: &'static str,
    /// What the tool does, for an agent to read.
    pub description: &'static str,
    /// A display name.
    pub title: Option<&'static str>,
    /// The caller kinds it accepts.
    pub callers: &'static [CallerKind],
    /// The scopes a token caller needs.
    pub scopes: &'static [Scope],
    /// Hints for clients.
    pub annotations: ToolAnnotations,
    /// The guard a user caller is signed into.
    pub guard: Option<&'static str>,
}

/// What `#[tool]` declares about a tool on a routed handler. Built only by
/// the macro.
#[doc(hidden)]
pub struct __ToolDeclaration {
    /// The facts every tool declares.
    pub meta: __ToolMeta,
    /// The input type's JSON Schema.
    pub input_schema: fn() -> Value,
    /// The output type's JSON Schema.
    pub output_schema: Option<fn() -> Value>,
    /// The type of the handler function.
    pub handler: fn() -> TypeId,
    /// The handler's name, for errors.
    pub handler_name: &'static str,
    /// The handler's module, for errors.
    pub module: &'static str,
    /// The name of the route to call, when the handler serves several.
    pub route: Option<&'static str>,
}

/// One tool `#[tool]` declared on a routed handler.
pub struct ToolRecord {
    decl: __ToolDeclaration,
}

impl ToolRecord {
    /// The record of `decl`. Built by `#[tool]`.
    #[doc(hidden)]
    pub const fn __declare(decl: __ToolDeclaration) -> Self {
        Self { decl }
    }

    /// The tool's name.
    pub fn name(&self) -> &'static str {
        self.decl.meta.name
    }

    /// The tool's description.
    pub fn description(&self) -> &'static str {
        self.decl.meta.description
    }

    /// The caller kinds the tool accepts.
    pub fn callers(&self) -> &'static [CallerKind] {
        self.decl.meta.callers
    }

    /// The scopes a token caller needs.
    pub fn scopes(&self) -> &'static [Scope] {
        self.decl.meta.scopes
    }

    /// The handler's path, `app::controllers::posts::store`, for errors.
    pub fn handler_path(&self) -> String {
        format!("{}::{}", self.decl.module, self.decl.handler_name)
    }
}

impl std::fmt::Debug for ToolRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRecord")
            .field("name", &self.decl.meta.name)
            .field("handler", &self.handler_path())
            .finish_non_exhaustive()
    }
}

inventory::collect!(ToolRecord);

/// A mount property the application fills itself for a headless Live tool
/// call, from the caller: the agent cannot set it (AGENT-013). Built only by
/// `#[resolve(...)]`.
#[doc(hidden)]
pub struct __LiveResolver {
    /// The mount parameter it fills.
    pub property: &'static str,
    /// The resolver, encoding its value with the parameter's codec.
    pub resolve: for<'a> fn(&'a ToolCaller) -> __ResolverFuture<'a>,
}

/// What a generated resolver adapter returns.
#[doc(hidden)]
pub type __ResolverFuture<'a> = std::pin::Pin<
    Box<
        dyn std::future::Future<
                Output = Result<suprnova_live::canonical::CanonicalValue, FrameworkError>,
            > + Send
            + 'a,
    >,
>;

/// What `#[tool]` declares about a tool on a Live action. Built only by the
/// macro.
#[doc(hidden)]
pub struct __LiveToolDeclaration {
    /// The facts every tool declares.
    pub meta: __ToolMeta,
    /// The Live action the tool runs.
    pub action: &'static str,
    /// Whether the MCP endpoint offers the tool with no browser.
    pub headless: bool,
    /// The component's generated registration.
    pub registration:
        fn() -> Result<crate::live::__private::ComponentRegistration, crate::live::RegistryError>,
    /// The component's type, for errors.
    pub component_type: &'static str,
    /// The component's module, for errors.
    pub module: &'static str,
    /// The mount properties the application fills itself.
    pub resolvers: &'static [__LiveResolver],
}

/// One tool `#[tool]` declared on a Live action (AGENT-012).
pub struct LiveToolRecord {
    decl: __LiveToolDeclaration,
}

impl LiveToolRecord {
    /// The record of `decl`. Built by `#[tool]` inside `#[live]`.
    #[doc(hidden)]
    pub const fn __declare(decl: __LiveToolDeclaration) -> Self {
        Self { decl }
    }

    /// The tool's base name.
    pub fn name(&self) -> &'static str {
        self.decl.meta.name
    }

    /// Whether the MCP endpoint offers the tool with no browser.
    pub fn is_headless(&self) -> bool {
        self.decl.headless
    }

    /// The action's path, `app::live::Counter::increment`, for errors.
    pub fn action_path(&self) -> String {
        format!(
            "{}::{}::{}",
            self.decl.module, self.decl.component_type, self.decl.action
        )
    }

    /// The component's generated registration.
    pub(crate) fn registration(
        &self,
    ) -> Result<crate::live::__private::ComponentRegistration, crate::live::RegistryError> {
        (self.decl.registration)()
    }
}

impl std::fmt::Debug for LiveToolRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveToolRecord")
            .field("name", &self.decl.meta.name)
            .field("action", &self.action_path())
            .field("headless", &self.decl.headless)
            .finish_non_exhaustive()
    }
}

inventory::collect!(LiveToolRecord);

/// The description of a headless Live tool's `mount` property, fixed at
/// build like every other text an agent reads.
const MOUNT_DESCRIPTION: &str = "The component's mount properties. Send them to start a new \
     component; leave them out when you send a handle.";

/// The description of a headless Live tool's `handle` property.
const HANDLE_DESCRIPTION: &str = "The handle a previous call of a tool of this component \
     returned, to act on the same component. Leave it out, and send `mount`, to start a new one.";

/// What a tool runs.
#[derive(Clone)]
pub(crate) enum ToolTarget {
    /// The route its handler serves.
    Route,
    /// A Live action.
    Live(Arc<LiveTarget>),
}

/// The Live action a tool runs, resolved from the component's metadata.
pub(crate) struct LiveTarget {
    /// The tool's base name.
    pub(crate) tool: &'static str,
    /// The component.
    pub(crate) component: suprnova_live::identity::ComponentName,
    /// The action.
    pub(crate) action: &'static str,
    /// Whether the MCP endpoint offers the tool with no browser.
    pub(crate) headless: bool,
    /// The mount properties the application fills itself.
    pub(crate) resolvers: &'static [__LiveResolver],
}

impl LiveTarget {
    /// The names of the mount properties an agent may not send.
    pub(crate) fn resolved_properties(&self) -> Vec<&'static str> {
        self.resolvers
            .iter()
            .map(|resolver| resolver.property)
            .collect()
    }
}

impl std::fmt::Debug for LiveTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveTarget")
            .field("tool", &self.tool)
            .field("component", &self.component.as_str())
            .field("action", &self.action)
            .field("headless", &self.headless)
            .finish_non_exhaustive()
    }
}

/// A tool, resolved to what it runs: a route, or a Live action.
pub struct RegisteredTool {
    meta: &'static __ToolMeta,
    origin: String,
    target: ToolTarget,
    method: Method,
    pattern: String,
    input_schema: Value,
    output_schema: Option<Value>,
}

impl std::fmt::Debug for RegisteredTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegisteredTool")
            .field("name", &self.name())
            .field("origin", &self.origin)
            .field("method", &self.method)
            .field("pattern", &self.pattern)
            .finish_non_exhaustive()
    }
}

impl RegisteredTool {
    /// The tool's name.
    pub fn name(&self) -> &'static str {
        self.meta.name
    }

    /// The tool's description.
    pub fn description(&self) -> &'static str {
        self.meta.description
    }

    /// The tool's display name.
    pub fn title(&self) -> Option<&'static str> {
        self.meta.title
    }

    /// The caller kinds the tool accepts.
    pub fn callers(&self) -> &'static [CallerKind] {
        self.meta.callers
    }

    /// Whether the tool accepts callers of `kind`.
    ///
    /// A tool that accepts `public` callers accepts every caller: a guest,
    /// a signed-in user, a user's client and an agent account. The handler
    /// still sees which one called.
    pub fn accepts(&self, kind: CallerKind) -> bool {
        let callers = self.meta.callers;
        callers.contains(&kind) || callers.contains(&CallerKind::Public)
    }

    /// The scopes a token caller needs.
    pub fn scopes(&self) -> &'static [Scope] {
        self.meta.scopes
    }

    /// The tool's hints.
    pub fn annotations(&self) -> ToolAnnotations {
        self.meta.annotations
    }

    /// The method of the route the tool calls. A Live tool calls the
    /// framework's own `POST /__live/agent`.
    pub fn method(&self) -> &Method {
        &self.method
    }

    /// The pattern of the route the tool calls, `/posts/{post}`.
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// The input schema.
    pub fn input_schema(&self) -> &Value {
        &self.input_schema
    }

    /// The output schema, when the tool declares an output type.
    pub fn output_schema(&self) -> Option<&Value> {
        self.output_schema.as_ref()
    }

    /// The Live component and action the tool runs, for a Live tool.
    pub fn live_action(&self) -> Option<(&str, &str)> {
        match &self.target {
            ToolTarget::Live(target) => Some((target.component.as_str(), target.action)),
            ToolTarget::Route => None,
        }
    }

    /// The Live action the tool runs, for a Live tool.
    pub(crate) fn live_target(&self) -> Option<&Arc<LiveTarget>> {
        match &self.target {
            ToolTarget::Live(target) => Some(target),
            ToolTarget::Route => None,
        }
    }

    /// The guard a user caller is signed into.
    pub(crate) fn guard(&self) -> Option<&'static str> {
        self.meta.guard
    }

    /// The scopes of the tool that `caller` does not hold.
    pub(crate) fn missing_scopes(&self, caller: &ToolCaller) -> Vec<&'static str> {
        self.scopes()
            .iter()
            .filter(|scope| !caller.can(scope.name()))
            .map(Scope::name)
            .collect()
    }

    /// Whether `caller` may call the tool: it accepts the caller's kind, and
    /// the caller holds every scope it needs. A public tool declares no
    /// scope, which the build enforces, so every caller may call it.
    pub(crate) fn callable_by(&self, caller: &ToolCaller) -> bool {
        self.accepts(caller.kind()) && self.missing_scopes(caller).is_empty()
    }

    /// The tool as MCP's `tools/list` shows it.
    pub(crate) fn definition(&self) -> Value {
        let mut out = serde_json::Map::new();
        out.insert("name".into(), json!(self.name()));
        if let Some(title) = self.title() {
            out.insert("title".into(), json!(title));
        }
        out.insert("description".into(), json!(self.description()));
        out.insert("inputSchema".into(), self.input_schema.clone());
        if let Some(schema) = &self.output_schema {
            out.insert("outputSchema".into(), schema.clone());
        }
        if let Some(annotations) = self.annotations().to_mcp(self.title()) {
            out.insert("annotations".into(), annotations);
        }
        Value::Object(out)
    }
}

/// The application's tools, checked against a router (AGENT-001).
#[derive(Debug)]
pub struct ToolRegistry {
    /// The tools the MCP endpoint offers, sorted by name: every routed
    /// handler tool and every headless Live tool.
    tools: Vec<RegisteredTool>,
    /// The Live tools only a browser registers, sorted by name: checked like
    /// every tool, but never listed, and never found by [`Self::get`]
    /// (AGENT-013).
    browser_tools: Vec<RegisteredTool>,
    by_route: HashMap<(Method, String), usize>,
    scopes: Vec<Scope>,
}

impl ToolRegistry {
    /// Check every `#[tool]` linked into the application against `router`.
    ///
    /// A tool whose handler the router does not serve at all is left out,
    /// with one warning naming each such tool, and so is a headless Live
    /// tool on a router that does not serve Live.
    ///
    /// # Errors
    ///
    /// The first problem, naming the tool:
    ///
    /// - two tools share a name (two crates; within one crate the build
    ///   already failed), or a routed tool's name is a Live tool's base, a
    ///   `.` and an instance key, which a browser would also register;
    /// - two declarations of one scope disagree on its description, or a
    ///   tool names a scope no `tool_scopes!` declared;
    /// - a tool's handler serves several routes and the tool names none of
    ///   them with `route = "..."`, or its `route = "..."` names none of the
    ///   routes the handler serves;
    /// - a path parameter of the tool's route is not a field of its input;
    /// - a tool's input is not an object;
    /// - a headless Live tool's component has session fields, its action
    ///   finalizes an upload, or a mount property the agent sends has no
    ///   schema.
    pub fn build(router: &Router) -> Result<Self, FrameworkError> {
        let scopes = checked_scopes()?;
        let mut records: Vec<&'static ToolRecord> = inventory::iter::<ToolRecord>().collect();
        records.sort_by(|a, b| {
            a.name()
                .cmp(b.name())
                .then(a.handler_path().cmp(&b.handler_path()))
        });
        let mut live_records: Vec<&'static LiveToolRecord> =
            inventory::iter::<LiveToolRecord>().collect();
        live_records.sort_by(|a, b| {
            a.name()
                .cmp(b.name())
                .then(a.action_path().cmp(&b.action_path()))
        });
        check_unique_names(&records, &live_records)?;
        for (name, needed) in records
            .iter()
            .map(|record| (record.name(), record.scopes()))
            .chain(
                live_records
                    .iter()
                    .map(|record| (record.name(), record.decl.meta.scopes)),
            )
        {
            for scope in needed {
                if !scopes.contains(scope) {
                    return Err(FrameworkError::internal(format!(
                        "tool `{name}` needs the scope `{}`, which no tool_scopes! declares",
                        scope.name()
                    )));
                }
            }
        }
        let mut tools: Vec<RegisteredTool> = Vec::with_capacity(records.len());
        let mut browser_tools: Vec<RegisteredTool> = Vec::new();
        let mut unrouted: Vec<&'static str> = Vec::new();
        for record in records {
            match resolve(router, record)? {
                Some(tool) => tools.push(tool),
                None => unrouted.push(record.name()),
            }
        }
        // A router that leaves a tool unrouted, such as one a test builds for
        // a few routes, serves without it rather than refusing every request.
        if !unrouted.is_empty() {
            tracing::warn!(
                tools = %unrouted.join(", "),
                "these tools are not routed and are left out: register a route for each \
                 handler, or remove #[tool]"
            );
        }
        let serves_live = router
            .match_route(&Method::POST, crate::live::LIVE_AGENT_PATH)
            .is_some_and(|(pattern, _, _)| pattern == crate::live::LIVE_AGENT_PATH);
        let mut unserved: Vec<&'static str> = Vec::new();
        for record in live_records {
            let tool = resolve_live(record)?;
            if !record.decl.headless {
                browser_tools.push(tool);
            } else if serves_live {
                tools.push(tool);
            } else {
                unserved.push(record.name());
            }
        }
        if !unserved.is_empty() {
            tracing::warn!(
                tools = %unserved.join(", "),
                "these headless Live tools are left out because the router does not serve \
                 Live: install Live with `Router::try_live_with` to serve headless Live tools"
            );
        }
        tools.sort_by(|a, b| a.name().cmp(b.name()));
        check_browser_prefixes(&tools, &browser_tools)?;
        let by_route = tools
            .iter()
            .enumerate()
            .filter(|(_, tool)| matches!(tool.target, ToolTarget::Route))
            .map(|(index, tool)| ((tool.method.clone(), tool.pattern.clone()), index))
            .collect();
        Ok(Self {
            tools,
            browser_tools,
            by_route,
            scopes,
        })
    }

    /// Every tool the MCP endpoint offers, sorted by name: the routed
    /// handler tools and the headless Live tools.
    pub fn tools(&self) -> &[RegisteredTool] {
        &self.tools
    }

    /// The tool named `name` the MCP endpoint offers. A Live tool only a
    /// browser registers is never found here.
    pub fn get(&self, name: &str) -> Option<&RegisteredTool> {
        self.tools
            .binary_search_by(|tool| tool.name().cmp(name))
            .ok()
            .map(|index| &self.tools[index])
    }

    /// The Live tool with the base name `base`, headless or not: what a
    /// browser registers as `{base}.{instance}`.
    pub fn live_tool(&self, base: &str) -> Option<&RegisteredTool> {
        self.get(base)
            .filter(|tool| tool.live_target().is_some())
            .or_else(|| {
                self.browser_tools
                    .binary_search_by(|tool| tool.name().cmp(base))
                    .ok()
                    .map(|index| &self.browser_tools[index])
            })
    }

    /// The Live tool whose action is `action` of `component`, offered to an
    /// agent, to a browser or to both. A Live action is at most one tool's.
    pub(crate) fn live_tool_of(
        &self,
        component: &suprnova_live::identity::ComponentName,
        action: &str,
    ) -> Option<&RegisteredTool> {
        self.tools.iter().chain(&self.browser_tools).find(|tool| {
            tool.live_target()
                .is_some_and(|target| &target.component == component && target.action == action)
        })
    }

    /// The Live tool a browser registers as `name`, `{base}.{instance}`,
    /// with the instance.
    pub(crate) fn browser_tool<'a>(&self, name: &'a str) -> Option<(&RegisteredTool, &'a str)> {
        let (base, instance) = name.rsplit_once('.')?;
        if !suprnova_live::metadata::is_tool_instance_key(instance) {
            return None;
        }
        let tool = self.live_tool(base)?;
        offers_browser(tool.meta).then_some((tool, instance))
    }

    /// The tool whose route is `(method, pattern)`. A Live tool has no route
    /// of its own: every one shares `/__live/agent`.
    pub(crate) fn for_route(&self, method: &Method, pattern: &str) -> Option<&RegisteredTool> {
        self.by_route
            .get(&(method.clone(), pattern.to_string()))
            .map(|index| &self.tools[*index])
    }

    /// Whether any Live tool exists, offered to a browser, to an agent with
    /// no browser, or both: every one of their actions needs the Live tool
    /// gate's caller check, whoever clicks it.
    pub(crate) fn has_live_tools(&self) -> bool {
        self.tools
            .iter()
            .chain(&self.browser_tools)
            .any(|tool| tool.live_target().is_some())
    }

    /// Whether the application has any tool at all, routed or Live: the
    /// server stamps page tools on its requests only then (AGENT-014).
    pub(crate) fn has_tools(&self) -> bool {
        !self.tools.is_empty() || !self.browser_tools.is_empty()
    }

    /// The Live tools of `component` a browser registers on its islands,
    /// sorted by name.
    pub(crate) fn live_tools_of(
        &self,
        component: &suprnova_live::identity::ComponentName,
    ) -> Vec<&RegisteredTool> {
        let mut tools: Vec<&RegisteredTool> = self
            .tools
            .iter()
            .chain(&self.browser_tools)
            .filter(|tool| {
                offers_browser(tool.meta)
                    && tool
                        .live_target()
                        .is_some_and(|target| &target.component == component)
            })
            .collect();
        tools.sort_by(|a, b| a.name().cmp(b.name()));
        tools
    }

    /// Every scope the application declared, sorted by name.
    pub fn scopes(&self) -> &[Scope] {
        &self.scopes
    }
}

/// Whether a browser registers a tool of `meta`: it accepts a public or a
/// signed-in caller, the callers a page has. A browser viewer is never an
/// agent account.
fn offers_browser(meta: &__ToolMeta) -> bool {
    meta.callers
        .iter()
        .any(|kind| matches!(kind, CallerKind::Public | CallerKind::User))
}

/// Refuse two tools of one name, whatever each runs.
fn check_unique_names(
    records: &[&'static ToolRecord],
    live_records: &[&'static LiveToolRecord],
) -> Result<(), FrameworkError> {
    let mut seen: BTreeMap<&'static str, String> = BTreeMap::new();
    let all = records
        .iter()
        .map(|record| (record.name(), record.handler_path()))
        .chain(
            live_records
                .iter()
                .map(|record| (record.name(), record.action_path())),
        );
    for (name, origin) in all {
        if let Some(previous) = seen.insert(name, origin.clone()) {
            return Err(FrameworkError::internal(format!(
                "two tools are named `{name}`: {previous} and {origin}; a tool name is unique in \
                 the application"
            )));
        }
    }
    Ok(())
}

/// Refuse a routed tool whose name a browser could also register for a Live
/// tool: `{base}.{instance}` with an instance key in the tool grammar. A page
/// registers both kinds side by side (AGENT-014), and two tools of one name
/// would make an agent's call ambiguous.
fn check_browser_prefixes(
    tools: &[RegisteredTool],
    browser_tools: &[RegisteredTool],
) -> Result<(), FrameworkError> {
    let bases: Vec<&'static str> = tools
        .iter()
        .chain(browser_tools)
        .filter(|tool| tool.live_target().is_some() && offers_browser(tool.meta))
        .map(RegisteredTool::name)
        .collect();
    for tool in tools
        .iter()
        .filter(|tool| matches!(tool.target, ToolTarget::Route))
    {
        let shadowed = tool
            .name()
            .rsplit_once('.')
            .is_some_and(|(base, instance)| {
                bases.contains(&base) && suprnova_live::metadata::is_tool_instance_key(instance)
            });
        if shadowed {
            return Err(FrameworkError::internal(format!(
                "tool `{}` ({}) has the name a browser gives the Live tool `{}` on the island \
                 keyed `{}`; rename one of them",
                tool.name(),
                tool.origin,
                tool.name().rsplit_once('.').map_or("", |(base, _)| base),
                tool.name().rsplit_once('.').map_or("", |(_, key)| key),
            )));
        }
    }
    Ok(())
}

/// Every declared scope, refusing two declarations of one name that
/// disagree.
fn checked_scopes() -> Result<Vec<Scope>, FrameworkError> {
    let mut by_name: BTreeMap<&'static str, Scope> = BTreeMap::new();
    for record in inventory::iter::<ScopeRecord>() {
        let scope = record.scope();
        if let Some(previous) = by_name.insert(scope.name(), scope)
            && previous.description() != scope.description()
        {
            return Err(FrameworkError::internal(format!(
                "the scope `{}` is declared twice with different descriptions: {:?} and {:?}",
                scope.name(),
                previous.description(),
                scope.description()
            )));
        }
    }
    Ok(by_name.into_values().collect())
}

/// Find the route `record`'s handler serves and check the tool fits it.
/// `None` when the handler serves no route.
fn resolve(
    router: &Router,
    record: &'static ToolRecord,
) -> Result<Option<RegisteredTool>, FrameworkError> {
    let handler = (record.decl.handler)();
    let mut routes = router.bindings.routes_for(handler);
    // Only a handler this router does not serve at all is left out. A
    // `route = "..."` that names none of the routes the handler serves is a
    // wrong declaration, and leaving the tool out would serve its route
    // without the tool's caller checks.
    if routes.is_empty() {
        return Ok(None);
    }
    if let Some(name) = record.decl.route {
        let served = routes
            .iter()
            .map(
                |(method, pattern)| match router.route_name(method, pattern) {
                    Some(named) => format!("{method} {pattern} (named `{named}`)"),
                    None => format!("{method} {pattern} (unnamed)"),
                },
            )
            .collect::<Vec<_>>()
            .join(", ");
        routes.retain(|(method, pattern)| router.route_name(method, pattern) == Some(name));
        if routes.is_empty() {
            return Err(FrameworkError::internal(format!(
                "tool `{}` declares #[tool(route = \"{name}\")], but its handler {} serves no \
                 route of that name; it serves {served}",
                record.name(),
                record.handler_path()
            )));
        }
    }
    let (method, pattern) = match routes.len() {
        1 => routes.remove(0),
        _ => {
            let listed = routes
                .iter()
                .map(|(method, pattern)| format!("{method} {pattern}"))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(FrameworkError::internal(format!(
                "tool `{}`: its handler {} serves {} routes ({listed}); name the one to \
                 call with #[tool(route = \"...\")]",
                record.name(),
                record.handler_path(),
                routes.len()
            )));
        }
    };
    let input_schema = (record.decl.input_schema)();
    if input_schema.get("type").and_then(Value::as_str) != Some("object") {
        return Err(FrameworkError::internal(format!(
            "tool `{}`: its input must be a struct, whose schema is an object",
            record.name()
        )));
    }
    let properties = input_schema.get("properties").and_then(Value::as_object);
    for placeholder in crate::routing::binding::placeholders(&pattern) {
        if !properties.is_some_and(|properties| properties.contains_key(&placeholder.name)) {
            return Err(FrameworkError::internal(format!(
                "tool `{}`: its route {method} {pattern} takes the parameter `{}`, which its \
                 input has no field for",
                record.name(),
                placeholder.name
            )));
        }
    }
    if record.decl.meta.callers.contains(&CallerKind::Agent)
        && route_requires_auth(router, &method, &pattern)
    {
        tracing::warn!(
            tool = record.name(),
            route = %format!("{method} {pattern}"),
            "a tool that accepts agent callers is behind AuthMiddleware: an agent account \
             bound to no user is refused with 401. Serve agent tools on routes without \
             `auth` and decide in their policies with ToolCaller::current()"
        );
    }
    Ok(Some(RegisteredTool {
        meta: &record.decl.meta,
        origin: record.handler_path(),
        target: ToolTarget::Route,
        method,
        pattern,
        input_schema,
        output_schema: record.decl.output_schema.map(|schema| schema()),
    }))
}

/// Resolve a Live tool against its component's generated metadata, and
/// check that a headless one can run without a browser (AGENT-013).
fn resolve_live(record: &'static LiveToolRecord) -> Result<RegisteredTool, FrameworkError> {
    let name = record.name();
    let path = record.action_path();
    let fail =
        |reason: String| FrameworkError::internal(format!("Live tool `{name}` ({path}): {reason}"));
    let registration = record
        .registration()
        .map_err(|error| fail(format!("its component does not register: {error}")))?;
    let descriptor = registration.descriptor();
    let metadata = descriptor.metadata();
    let action = metadata
        .actions()
        .iter()
        .find(|action| action.name().as_str() == record.decl.action)
        .ok_or_else(|| fail("its component does not register the action".to_string()))?;
    // The engine metadata and the framework record come from one `#[tool]`;
    // disagreeing, they would offer the browser and the endpoint two tools.
    let declared = action
        .tool()
        .ok_or_else(|| fail("the action's metadata declares no tool".to_string()))?;
    if declared.name() != name
        || declared.is_headless() != record.decl.headless
        || declared.browser() != offers_browser(&record.decl.meta)
    {
        return Err(fail(
            "the action's metadata disagrees with the tool's declaration".to_string(),
        ));
    }
    let arguments = action.arguments().json_schema().ok_or_else(|| {
        fail("an argument's codec has no JSON Schema; take scalar, Option, Vec or BTreeMap arguments".to_string())
    })?;
    let target = Arc::new(LiveTarget {
        tool: name,
        component: metadata.identity().clone(),
        action: record.decl.action,
        headless: record.decl.headless,
        resolvers: record.decl.resolvers,
    });
    if !record.decl.headless {
        return Ok(RegisteredTool {
            meta: &record.decl.meta,
            origin: path,
            target: ToolTarget::Live(target),
            method: Method::POST,
            pattern: crate::live::LIVE_UPDATE_PATH.to_string(),
            input_schema: arguments,
            output_schema: None,
        });
    }
    // A headless call has no session, so session-only state would start
    // empty on every call; an upload is finished by the browser that sent it.
    if metadata
        .fields()
        .iter()
        .any(|field| field.category() == suprnova_live::snapshot::state::FieldCategory::Session)
    {
        return Err(fail(
            "a headless tool's component cannot have #[session] fields: a call without a              browser has no session"
                .to_string(),
        ));
    }
    if metadata.fields().iter().any(|field| {
        field
            .upload_policy()
            .is_some_and(|policy| policy.finalize_action().as_str() == record.decl.action)
    }) {
        return Err(fail(
            "an action that finalizes an upload cannot be headless: the browser that sent the              file finishes it"
                .to_string(),
        ));
    }
    let schema = descriptor.parameter_schema();
    let resolved = target.resolved_properties();
    for property in &resolved {
        if !schema
            .fields()
            .any(|field| field.name().as_str() == *property)
        {
            return Err(fail(format!(
                "#[resolve] names the mount property `{property}`, which the component's                  #[mount] does not take"
            )));
        }
    }
    let mount = schema.json_schema(&resolved).ok_or_else(|| {
        fail("a mount property an agent sends has no JSON Schema; take scalar, enumeration,               Vec or BTreeMap mount properties, or fill it with #[resolve]"
            .to_string())
    })?;
    let required_arguments = arguments
        .get("required")
        .and_then(Value::as_array)
        .is_some_and(|required| !required.is_empty());
    let input_schema = headless_input_schema(mount, arguments, required_arguments);
    let output_schema = headless_output_schema(metadata);
    Ok(RegisteredTool {
        meta: &record.decl.meta,
        origin: path,
        target: ToolTarget::Live(target),
        method: Method::POST,
        pattern: crate::live::LIVE_AGENT_PATH.to_string(),
        input_schema,
        output_schema: Some(output_schema),
    })
}

/// The three-part input of a headless Live tool: the mount properties, an
/// optional handle, and the action's arguments (AGENT-013).
fn headless_input_schema(mount: Value, arguments: Value, required_arguments: bool) -> Value {
    let mut mount = without_dialect(mount);
    if let Value::Object(object) = &mut mount {
        object.insert("description".into(), json!(MOUNT_DESCRIPTION));
    }
    let mut schema = json!({
        "$schema": suprnova_live::state::JSON_SCHEMA_DIALECT,
        "type": "object",
        "properties": {
            "mount": mount,
            "handle": { "type": "string", "description": HANDLE_DESCRIPTION },
            "arguments": without_dialect(arguments),
        },
        "additionalProperties": false,
    });
    if required_arguments && let Value::Object(object) = &mut schema {
        object.insert("required".into(), json!(["arguments"]));
    }
    schema
}

/// `schema` without its `$schema`, which only a schema's root names.
fn without_dialect(mut schema: Value) -> Value {
    if let Value::Object(object) = &mut schema {
        object.remove("$schema");
    }
    schema
}

/// The result of a headless Live tool: the outcome, the agent-visible
/// fields as the snapshot encodes them, any validation issues, and the
/// handle a later call passes with its expiry.
fn headless_output_schema(metadata: &suprnova_live::metadata::ComponentMetadata) -> Value {
    use suprnova_live::snapshot::state::StateCodec;

    let mut fields = serde_json::Map::new();
    for field in metadata
        .fields()
        .iter()
        .filter(|field| field.agent_visible())
    {
        let schema = match field.codec() {
            StateCodec::I64Decimal => json!({ "type": "string", "pattern": "^(0|-?[1-9][0-9]*)$" }),
            StateCodec::U64Decimal => json!({ "type": "string", "pattern": "^(0|[1-9][0-9]*)$" }),
            StateCodec::BytesBase64Url => json!({ "type": "string" }),
            _ => json!({}),
        };
        fields.insert(field.name().as_str().to_owned(), schema);
    }
    json!({
        "$schema": suprnova_live::state::JSON_SCHEMA_DIALECT,
        "type": "object",
        "properties": {
            "outcome": {
                "type": "string",
                "enum": ["accepted", "no_render", "redirect", "invalid"],
            },
            "fields": {
                "type": "object",
                "properties": Value::Object(fields),
                "additionalProperties": false,
            },
            "validation": {
                "type": "object",
                "additionalProperties": { "type": "array", "items": { "type": "string" } },
            },
            "handle": { "type": "string" },
            "expiresAt": { "type": "integer", "minimum": 0 },
        },
        "required": ["outcome", "fields", "handle", "expiresAt"],
    })
}

/// Whether the route `(method, pattern)` runs `AuthMiddleware`.
fn route_requires_auth(router: &Router, method: &Method, pattern: &str) -> bool {
    router
        .get_route_middleware(method, pattern)
        .iter()
        .any(|middleware| {
            crate::middleware::type_of(middleware)
                == Some(TypeId::of::<crate::auth::AuthMiddleware>())
                || crate::middleware::alias_of(middleware)
                    .is_some_and(|alias| alias == "auth" || alias.starts_with("auth:"))
        })
}

/// Registries built by hand, for unit tests of what reads one.
#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::Arc;

    use super::{
        __ToolMeta, CallerKind, HashMap, LiveTarget, Method, RegisteredTool, ToolAnnotations,
        ToolRegistry, ToolTarget,
    };

    /// A Live tool of `action` on `component` accepting `callers`, offered
    /// headless; `browser` puts it among the tools only a browser registers.
    pub(crate) fn live_registry(
        name: &'static str,
        component: &str,
        action: &'static str,
        callers: &'static [CallerKind],
        browser: bool,
    ) -> ToolRegistry {
        let meta: &'static __ToolMeta = Box::leak(Box::new(__ToolMeta {
            name,
            description: "A test tool.",
            title: None,
            callers,
            scopes: &[],
            annotations: ToolAnnotations::default(),
            guard: None,
        }));
        let tool = RegisteredTool {
            meta,
            origin: "tests".to_owned(),
            target: ToolTarget::Live(Arc::new(LiveTarget {
                tool: name,
                component: suprnova_live::identity::ComponentName::parse(component)
                    .expect("a component name"),
                action,
                headless: !browser,
                resolvers: &[],
            })),
            method: Method::POST,
            pattern: crate::live::LIVE_AGENT_PATH.to_owned(),
            input_schema: serde_json::json!({ "type": "object" }),
            output_schema: None,
        };
        let (tools, browser_tools) = if browser {
            (Vec::new(), vec![tool])
        } else {
            (vec![tool], Vec::new())
        };
        ToolRegistry {
            tools,
            browser_tools,
            by_route: HashMap::new(),
            scopes: Vec::new(),
        }
    }
}
