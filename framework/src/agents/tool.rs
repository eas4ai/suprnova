//! The tools an application declares, and their check against the router
//! (AGENT-001).
//!
//! `#[tool]` submits one [`ToolRecord`] per tool through `inventory`, keyed
//! by the type of its handler function. [`ToolRegistry::build`] finds each
//! tool's route by that key, so a tool call runs that route, and refuses an
//! application whose tools do not fit its routes.

use std::any::TypeId;
use std::collections::{BTreeMap, HashMap};

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

/// What `#[tool]` declares about one tool. Built only by the macro.
#[doc(hidden)]
pub struct __ToolDeclaration {
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
    /// The guard a user caller is signed into.
    pub guard: Option<&'static str>,
}

/// One tool `#[tool]` declared.
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
        self.decl.name
    }

    /// The tool's description.
    pub fn description(&self) -> &'static str {
        self.decl.description
    }

    /// The caller kinds the tool accepts.
    pub fn callers(&self) -> &'static [CallerKind] {
        self.decl.callers
    }

    /// The scopes a token caller needs.
    pub fn scopes(&self) -> &'static [Scope] {
        self.decl.scopes
    }

    /// The handler's path, `app::controllers::posts::store`, for errors.
    pub fn handler_path(&self) -> String {
        format!("{}::{}", self.decl.module, self.decl.handler_name)
    }
}

impl std::fmt::Debug for ToolRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRecord")
            .field("name", &self.decl.name)
            .field("handler", &self.handler_path())
            .finish_non_exhaustive()
    }
}

inventory::collect!(ToolRecord);

/// A tool, resolved to the route it calls.
pub struct RegisteredTool {
    record: &'static ToolRecord,
    method: Method,
    pattern: String,
    input_schema: Value,
    output_schema: Option<Value>,
}

impl std::fmt::Debug for RegisteredTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegisteredTool")
            .field("name", &self.name())
            .field("method", &self.method)
            .field("pattern", &self.pattern)
            .finish_non_exhaustive()
    }
}

impl RegisteredTool {
    /// The tool's name.
    pub fn name(&self) -> &'static str {
        self.record.decl.name
    }

    /// The tool's description.
    pub fn description(&self) -> &'static str {
        self.record.decl.description
    }

    /// The tool's display name.
    pub fn title(&self) -> Option<&'static str> {
        self.record.decl.title
    }

    /// The caller kinds the tool accepts.
    pub fn callers(&self) -> &'static [CallerKind] {
        self.record.decl.callers
    }

    /// Whether the tool accepts callers of `kind`.
    ///
    /// A tool that accepts `public` callers accepts every caller: a guest,
    /// a signed-in user, a user's client and an agent account. The handler
    /// still sees which one called.
    pub fn accepts(&self, kind: CallerKind) -> bool {
        let callers = self.record.decl.callers;
        callers.contains(&kind) || callers.contains(&CallerKind::Public)
    }

    /// The scopes a token caller needs.
    pub fn scopes(&self) -> &'static [Scope] {
        self.record.decl.scopes
    }

    /// The tool's hints.
    pub fn annotations(&self) -> ToolAnnotations {
        self.record.decl.annotations
    }

    /// The method of the route the tool calls.
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

    /// The guard a user caller is signed into.
    pub(crate) fn guard(&self) -> Option<&'static str> {
        self.record.decl.guard
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
    /// Sorted by name.
    tools: Vec<RegisteredTool>,
    by_route: HashMap<(Method, String), usize>,
    scopes: Vec<Scope>,
}

impl ToolRegistry {
    /// Check every `#[tool]` linked into the application against `router`.
    ///
    /// A tool whose handler the router does not serve at all is left out,
    /// with one warning naming each such tool.
    ///
    /// # Errors
    ///
    /// The first problem, naming the tool:
    ///
    /// - two tools share a name (two crates; within one crate the build
    ///   already failed);
    /// - two declarations of one scope disagree on its description, or a
    ///   tool names a scope no `tool_scopes!` declared;
    /// - a tool's handler serves several routes and the tool names none of
    ///   them with `route = "..."`, or its `route = "..."` names none of the
    ///   routes the handler serves;
    /// - a path parameter of the tool's route is not a field of its input;
    /// - a tool's input is not an object.
    pub fn build(router: &Router) -> Result<Self, FrameworkError> {
        let scopes = checked_scopes()?;
        let mut records: Vec<&'static ToolRecord> = inventory::iter::<ToolRecord>().collect();
        records.sort_by(|a, b| {
            a.name()
                .cmp(b.name())
                .then(a.handler_path().cmp(&b.handler_path()))
        });
        let mut tools: Vec<RegisteredTool> = Vec::with_capacity(records.len());
        let mut unrouted: Vec<&'static str> = Vec::new();
        let mut previous: Option<&'static ToolRecord> = None;
        for record in records {
            if let Some(previous) = previous
                && previous.name() == record.name()
            {
                return Err(FrameworkError::internal(format!(
                    "two tools are named `{}`: {} and {}; a tool name is unique in the \
                     application",
                    record.name(),
                    previous.handler_path(),
                    record.handler_path()
                )));
            }
            previous = Some(record);
            for scope in record.scopes() {
                if !scopes.contains(scope) {
                    return Err(FrameworkError::internal(format!(
                        "tool `{}` needs the scope `{}`, which no tool_scopes! declares",
                        record.name(),
                        scope.name()
                    )));
                }
            }
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
        let by_route = tools
            .iter()
            .enumerate()
            .map(|(index, tool)| ((tool.method.clone(), tool.pattern.clone()), index))
            .collect();
        Ok(Self {
            tools,
            by_route,
            scopes,
        })
    }

    /// Every tool, sorted by name.
    pub fn tools(&self) -> &[RegisteredTool] {
        &self.tools
    }

    /// The tool named `name`.
    pub fn get(&self, name: &str) -> Option<&RegisteredTool> {
        self.tools
            .binary_search_by(|tool| tool.name().cmp(name))
            .ok()
            .map(|index| &self.tools[index])
    }

    /// The tool whose route is `(method, pattern)`.
    pub(crate) fn for_route(&self, method: &Method, pattern: &str) -> Option<&RegisteredTool> {
        self.by_route
            .get(&(method.clone(), pattern.to_string()))
            .map(|index| &self.tools[*index])
    }

    /// Every scope the application declared, sorted by name.
    pub fn scopes(&self) -> &[Scope] {
        &self.scopes
    }
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
    if record.decl.callers.contains(&CallerKind::Agent)
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
        record,
        method,
        pattern,
        input_schema,
        output_schema: record.decl.output_schema.map(|schema| schema()),
    }))
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
