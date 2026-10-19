//! The MCP endpoint (AGENT-003, AGENT-004).
//!
//! One `POST` carries one JSON-RPC message. The endpoint checks, in order:
//!
//! 1. the `Origin` header, answering 403 for an origin the application does
//!    not allow, before it reads the body;
//! 2. the client address's request budget, which every request counts
//!    against, answering 429 once it is used;
//! 3. the body: JSON, one message, no larger than the configured limit;
//! 4. the protocol era: `initialize` opens the 2025-06-18 and 2025-11-25
//!    revisions, negotiating the newest of them when it asks for another;
//!    any other request names its revision in `MCP-Protocol-Version`, and a
//!    2026-07-28 request must mirror its `_meta` version, its method and
//!    its tool's name into the headers, each header sent once;
//! 5. the caller: no token is a guest, a bearer token goes to the
//!    application's resolvers, each with a deadline, and a token none of
//!    them accepts is 401.
//!
//! Then `server/discover`, `initialize`, `ping`, `tools/list` and
//! `tools/call` are answered. The endpoint mints no session id, and `GET`
//! and `DELETE` meet the router's 405 because only `POST` is routed.

pub(crate) mod protocol;
mod tools;

use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use serde_json::{Map, Value, json};

use self::protocol::{
    FORBIDDEN, INTERNAL_ERROR, INVALID_PARAMS, INVALID_REQUEST, LEGACY, META_CLIENT_CAPABILITIES,
    META_PROTOCOL_VERSION, META_SERVER_INFO, METHOD_HEADER, METHOD_NOT_FOUND, MODERN, NAME_HEADER,
    PARSE_ERROR, PROTOCOL_VERSION_HEADER, RATE_LIMITED, SUPPORTED, UNAUTHORIZED, UNAVAILABLE,
};
use super::caller::ToolCaller;
use super::dispatch::{self, ClientFacts};
use super::limit::{self, Refusal};
use super::mark::EndpointContext;
use super::metadata::{app_name, resource_metadata_url, resource_url};
use super::scope::Scope;
use super::tool::ToolRegistry;
use super::{AgentRoute, AgentsConfig, ResolveContext, ResolveError};
use crate::error::FrameworkError;
use crate::http::{HttpResponse, Request, Response};

/// The protocol era a request speaks.
#[derive(Clone, Copy)]
enum Era {
    /// Revision 2026-07-28: stateless, metadata on every request.
    Modern,
    /// An `initialize`-based revision, served without a session.
    Legacy(&'static str),
}

/// The route handler of `POST /mcp`.
pub(crate) async fn handle(request: Request) -> Response {
    Ok(Box::pin(serve(request)).await)
}

async fn serve(request: Request) -> HttpResponse {
    let Some(endpoint) = request
        .agent_mark()
        .and_then(|mark| mark.as_endpoint())
        .cloned()
    else {
        return protocol::refuse(
            500,
            None,
            INTERNAL_ERROR,
            "The MCP endpoint was reached without its route's mark.",
            None,
        );
    };
    let AgentRoute::Mcp(config) = endpoint.route.clone() else {
        return protocol::refuse(500, None, INTERNAL_ERROR, "Not the MCP endpoint.", None);
    };

    if let Some(origin) = request.header("Origin")
        && !origin_allowed(origin, &config)
    {
        return protocol::refuse(
            403,
            None,
            FORBIDDEN,
            "The request's Origin is not allowed.",
            None,
        );
    }
    let address: Option<IpAddr> = request.ip().and_then(|ip| ip.parse().ok());
    // Every request counts against its address, whatever it asks and
    // however it ends, a refusal for a bad token included.
    match limit::admit_request(&config, address).await {
        Ok(()) => {}
        Err(Refusal::Limited(retry_after)) => {
            return protocol::refuse(
                429,
                None,
                RATE_LIMITED,
                "Too many requests: wait before sending more.",
                Some(json!({ "retryAfter": retry_after })),
            )
            .header("Retry-After", retry_after.to_string());
        }
        Err(Refusal::Unavailable(_)) => {
            return protocol::refuse(
                503,
                None,
                UNAVAILABLE,
                "The request limit cannot be checked right now.",
                None,
            );
        }
    }
    if !is_json(request.content_type()) {
        return protocol::refuse(
            415,
            None,
            INVALID_REQUEST,
            "The body must be one JSON-RPC message as application/json.",
            None,
        );
    }
    let headers = request.headers().clone();
    let client = ClientFacts::of(&request);
    let bytes = match request.body_bytes_with_cap(config.max_body_bytes).await {
        Ok((_, bytes)) => bytes,
        Err(error) => {
            return match error.status_code() {
                413 => protocol::refuse(
                    413,
                    None,
                    INVALID_REQUEST,
                    "The request body is larger than the endpoint accepts.",
                    None,
                ),
                408 => protocol::refuse(
                    408,
                    None,
                    INVALID_REQUEST,
                    "The request body did not arrive in time.",
                    None,
                ),
                _ => protocol::refuse(
                    400,
                    None,
                    INVALID_REQUEST,
                    "The request body could not be read.",
                    None,
                ),
            };
        }
    };

    let message = match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(message)) => message,
        Ok(Value::Array(_)) => {
            return protocol::refuse(
                400,
                None,
                INVALID_REQUEST,
                "Batches are not supported: send one JSON-RPC message per request.",
                None,
            );
        }
        Ok(_) => {
            return protocol::refuse(400, None, INVALID_REQUEST, "Invalid Request", None);
        }
        Err(_) => return protocol::refuse(400, None, PARSE_ERROR, "Parse error", None),
    };
    let readable_id = message.get("id").filter(|id| is_valid_id(id));
    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return protocol::refuse(
            400,
            readable_id,
            INVALID_REQUEST,
            "The message is not JSON-RPC 2.0.",
            None,
        );
    }
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return protocol::refuse(
            400,
            readable_id,
            INVALID_REQUEST,
            "The endpoint takes requests and notifications, never responses.",
            None,
        );
    };
    if message.get("id").is_none() {
        // A notification: accepted, and nothing is answered.
        return HttpResponse::new().status(202);
    }
    let Some(id) = readable_id.cloned() else {
        return protocol::refuse(
            400,
            None,
            INVALID_REQUEST,
            "A request id is a string or an integer.",
            None,
        );
    };
    let params = match message.get("params") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(params)) => params.clone(),
        Some(_) => {
            return protocol::refuse(
                400,
                Some(&id),
                INVALID_PARAMS,
                "params must be an object.",
                None,
            );
        }
    };

    let era = match detect_era(&id, method, &params, &headers) {
        Ok(era) => era,
        Err(response) => return response,
    };
    let caller = match authenticate(&headers, &config, address, &id).await {
        Ok(caller) => caller,
        Err(response) => return response,
    };

    match (era, method) {
        (Era::Legacy(version), "initialize") => protocol::respond(
            200,
            protocol::result(&id, initialize_result(&config, version)),
        ),
        (Era::Modern, "server/discover") => {
            protocol::respond(200, protocol::result(&id, discover_result(&config)))
        }
        (_, "ping") => protocol::respond(200, protocol::result(&id, complete(&config, Map::new()))),
        (_, "tools/list") => list_tools(&endpoint, &config, &caller, &id),
        (_, "tools/call") => {
            call_tool(
                endpoint,
                config,
                caller,
                client,
                id,
                &params,
                wants_stream(&headers),
            )
            .await
        }
        _ => protocol::refuse(404, Some(&id), METHOD_NOT_FOUND, "Method not found", None),
    }
}

/// The era the request speaks, after every header check its era requires.
fn detect_era(
    id: &Value,
    method: &str,
    params: &Map<String, Value>,
    headers: &hyper::HeaderMap,
) -> Result<Era, HttpResponse> {
    // A metadata header sent twice leaves unclear which value the body must
    // match, whatever the era.
    for (name, label) in [
        (PROTOCOL_VERSION_HEADER, "MCP-Protocol-Version"),
        (METHOD_HEADER, "Mcp-Method"),
        (NAME_HEADER, "Mcp-Name"),
    ] {
        if let Err(problem) = protocol::header_text(headers, name) {
            return Err(protocol::header_mismatch(
                Some(id),
                &problem.describe(label),
            ));
        }
    }
    if method == "initialize" {
        // The lifecycle of the initialize-based revisions negotiates: a
        // version this endpoint does not serve in that era is answered with
        // the newest one it does, which the client may accept or not.
        let Some(requested) = params.get("protocolVersion").and_then(Value::as_str) else {
            return Err(protocol::refuse(
                400,
                Some(id),
                INVALID_PARAMS,
                "initialize needs params.protocolVersion, a string naming a revision.",
                None,
            ));
        };
        let version = LEGACY
            .iter()
            .find(|version| **version == requested)
            .unwrap_or(&LEGACY[0]);
        return Ok(Era::Legacy(version));
    }
    let version = match protocol::header_text(headers, PROTOCOL_VERSION_HEADER) {
        Err(problem) => {
            return Err(protocol::header_mismatch(
                Some(id),
                &problem.describe("MCP-Protocol-Version"),
            ));
        }
        // Streamable HTTP, "Protocol Version Header" and "Server
        // Validation" (revision 2026-07-28): a server that does not serve
        // revisions before 2025-06-18 rejects a request without the header,
        // and a missing required header is a `HeaderMismatch`.
        Ok(None) => {
            return Err(protocol::header_mismatch(
                Some(id),
                "The MCP-Protocol-Version header is missing.",
            ));
        }
        Ok(Some(version)) => version,
    };
    if version == MODERN {
        check_modern(id, method, params, headers, &version)?;
        return Ok(Era::Modern);
    }
    match LEGACY.iter().find(|legacy| **legacy == version) {
        Some(legacy) => Ok(Era::Legacy(legacy)),
        None => Err(protocol::unsupported_version(Some(id), Some(&version))),
    }
}

/// The rules of a 2026-07-28 request: its `_meta` names the version the
/// header names and the client's capabilities, and `Mcp-Method` and, for a
/// named method, `Mcp-Name` mirror the body.
fn check_modern(
    id: &Value,
    method: &str,
    params: &Map<String, Value>,
    headers: &hyper::HeaderMap,
    version: &str,
) -> Result<(), HttpResponse> {
    let meta = params.get("_meta").and_then(Value::as_object);
    let Some(meta_version) = meta
        .and_then(|meta| meta.get(META_PROTOCOL_VERSION))
        .and_then(Value::as_str)
    else {
        return Err(protocol::refuse(
            400,
            Some(id),
            INVALID_PARAMS,
            "The request's _meta has no io.modelcontextprotocol/protocolVersion.",
            None,
        ));
    };
    if meta_version != version {
        return Err(protocol::header_mismatch(
            Some(id),
            &format!(
                "Header mismatch: MCP-Protocol-Version header value '{version}' does not match \
                 body value '{meta_version}'"
            ),
        ));
    }
    if meta
        .and_then(|meta| meta.get(META_CLIENT_CAPABILITIES))
        .is_none()
    {
        return Err(protocol::refuse(
            400,
            Some(id),
            INVALID_PARAMS,
            "The request's _meta has no io.modelcontextprotocol/clientCapabilities.",
            None,
        ));
    }
    match protocol::header_text(headers, METHOD_HEADER) {
        Ok(Some(header)) if header == method => {}
        Ok(Some(header)) => {
            return Err(protocol::header_mismatch(
                Some(id),
                &format!(
                    "Header mismatch: Mcp-Method header value '{header}' does not match body \
                     value '{method}'"
                ),
            ));
        }
        Ok(None) => {
            return Err(protocol::header_mismatch(
                Some(id),
                "The Mcp-Method header is missing.",
            ));
        }
        Err(problem) => {
            return Err(protocol::header_mismatch(
                Some(id),
                &problem.describe("Mcp-Method"),
            ));
        }
    }
    if let Some(field) = protocol::name_field(method) {
        let body = params.get(field).and_then(Value::as_str);
        let header = match protocol::header_text(headers, NAME_HEADER) {
            Ok(Some(header)) => protocol::decode_name(&header),
            Ok(None) => {
                return Err(protocol::header_mismatch(
                    Some(id),
                    "The Mcp-Name header is missing.",
                ));
            }
            Err(_) => None,
        };
        match (header, body) {
            (Some(header), Some(body)) if header == body => {}
            (Some(header), Some(body)) => {
                return Err(protocol::header_mismatch(
                    Some(id),
                    &format!(
                        "Header mismatch: Mcp-Name header value '{header}' does not match body \
                         value '{body}'"
                    ),
                ));
            }
            (None, _) => {
                return Err(protocol::header_mismatch(
                    Some(id),
                    "The Mcp-Name header is not a valid value.",
                ));
            }
            (Some(_), None) => {
                return Err(protocol::header_mismatch(
                    Some(id),
                    &format!("The Mcp-Name header names a {field} the body does not have."),
                ));
            }
        }
    }
    Ok(())
}

/// The caller the request's credential stands for: a guest without one, or
/// the caller a resolver turns its bearer token into.
async fn authenticate(
    headers: &hyper::HeaderMap,
    config: &AgentsConfig,
    address: Option<IpAddr>,
    id: &Value,
) -> Result<ToolCaller, HttpResponse> {
    let Some(header) = headers.get(hyper::header::AUTHORIZATION) else {
        return Ok(ToolCaller::guest().with_address(address));
    };
    let token = header
        .to_str()
        .ok()
        .and_then(|value| {
            let (scheme, rest) = value.split_once(' ')?;
            scheme.eq_ignore_ascii_case("bearer").then(|| rest.trim())
        })
        .filter(|token| !token.is_empty());
    let Some(token) = token else {
        return Err(invalid_token(
            config,
            id,
            "The Authorization header is not a bearer token.",
        ));
    };
    #[cfg(feature = "testing")]
    if let Some(caller) = super::testing::resolve_fake(token) {
        return Ok(caller.with_address(address));
    }
    let resource = resource_url(config);
    let context = ResolveContext {
        resource: &resource,
        client_ip: address,
    };
    for resolver in &config.resolvers {
        // A resolver that does not decide in time counts as unavailable.
        let decided =
            match tokio::time::timeout(config.resolver_timeout, resolver.resolve(token, &context))
                .await
            {
                Ok(decided) => decided,
                Err(_) => Err(ResolveError::Unavailable(FrameworkError::internal(
                    format!(
                        "a token resolver did not decide within {} ms",
                        config.resolver_timeout.as_millis()
                    ),
                ))),
            };
        match decided {
            Ok(caller) if caller.is_guest() => {
                return Err(invalid_token(
                    config,
                    id,
                    "The token stands for no user or agent.",
                ));
            }
            Ok(caller) => return Ok(caller.with_address(address)),
            Err(ResolveError::NotMine) => {}
            Err(ResolveError::Invalid(reason)) => return Err(invalid_token(config, id, reason)),
            Err(ResolveError::Unavailable(error)) => {
                tracing::error!(
                    error = %crate::error::render_error_chain(&error),
                    "a token resolver could not decide; the MCP request is refused"
                );
                return Err(protocol::refuse(
                    503,
                    Some(id),
                    UNAVAILABLE,
                    "The token cannot be checked right now.",
                    None,
                ));
            }
        }
    }
    Err(invalid_token(config, id, "The token is not recognized."))
}

/// The tools the caller may call.
fn list_tools(
    endpoint: &EndpointContext,
    config: &AgentsConfig,
    caller: &ToolCaller,
    id: &Value,
) -> HttpResponse {
    let registry = match endpoint.router.tool_registry() {
        Ok(registry) => registry,
        Err(error) => return internal(id, &error),
    };
    let mut result = Map::new();
    result.insert("tools".into(), Value::Array(tools::list(&registry, caller)));
    result.insert("ttlMs".into(), json!(0));
    result.insert("cacheScope".into(), json!("private"));
    protocol::respond(200, protocol::result(id, complete(config, result)))
}

/// A `tools/call`: the access checks, the limit, then the call through the
/// tool's route.
async fn call_tool(
    endpoint: EndpointContext,
    config: Arc<AgentsConfig>,
    caller: ToolCaller,
    client: ClientFacts,
    id: Value,
    params: &Map<String, Value>,
    stream: bool,
) -> HttpResponse {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return protocol::respond(
            200,
            protocol::error(
                Some(&id),
                INVALID_PARAMS,
                "tools/call needs params.name.",
                None,
            ),
        );
    };
    let arguments = match params.get("arguments") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(arguments)) => arguments.clone(),
        Some(_) => {
            return protocol::respond(
                200,
                protocol::error(
                    Some(&id),
                    INVALID_PARAMS,
                    "tools/call arguments must be an object.",
                    None,
                ),
            );
        }
    };
    let registry = match endpoint.router.tool_registry() {
        Ok(registry) => registry,
        Err(error) => return internal(&id, &error),
    };
    let Some(tool) = registry.get(name) else {
        return protocol::respond(
            200,
            protocol::error(
                Some(&id),
                INVALID_PARAMS,
                &format!("Unknown tool: {name}"),
                None,
            ),
        );
    };
    if !tool.accepts(caller.kind()) {
        if caller.is_guest() {
            return challenge(&config, &id, tool.scopes());
        }
        return protocol::refuse(
            403,
            Some(&id),
            FORBIDDEN,
            &format!(
                "The tool `{name}` does not accept {} callers.",
                caller.kind().as_str()
            ),
            None,
        );
    }
    if !tool.missing_scopes(&caller).is_empty() {
        return insufficient_scope(&config, &id, tool.scopes());
    }
    // A tool with its own guard signs in the user that guard's provider
    // knows by the identifier the resolver returned, never the resolver's
    // record; a user the provider does not know cannot call it.
    let caller = match guard_user(tool.guard(), caller).await {
        Ok(caller) => caller,
        Err(GuardRefusal::Unknown) => {
            return invalid_token(
                &config,
                &id,
                "The token's user is not known to the tool's guard.",
            );
        }
        Err(GuardRefusal::Misconfigured(error)) => return internal(&id, &error),
        Err(GuardRefusal::Failed(error)) => {
            tracing::error!(
                error = %crate::error::render_error_chain(&error),
                "a tool's guard could not load the caller's user"
            );
            return protocol::refuse(
                503,
                Some(&id),
                UNAVAILABLE,
                "The tool's guard cannot load the user right now.",
                None,
            );
        }
    };
    match limit::admit(&config, &caller).await {
        Ok(()) => {}
        Err(Refusal::Limited(retry_after)) => {
            return protocol::refuse(
                429,
                Some(&id),
                RATE_LIMITED,
                "Too many tool calls: wait before calling again.",
                Some(json!({ "retryAfter": retry_after })),
            )
            .header("Retry-After", retry_after.to_string());
        }
        Err(Refusal::Unavailable(_)) => {
            return protocol::refuse(
                503,
                Some(&id),
                UNAVAILABLE,
                "The tool call limit cannot be checked right now.",
                None,
            );
        }
    }

    let call = run_call(
        endpoint,
        registry,
        name.to_string(),
        arguments,
        caller,
        client,
        config,
        id,
        crate::routing::root::current(),
    );
    // The call runs as the response body is polled, in both forms of the
    // answer. A client that closes the connection drops the body and the
    // call with it, which is the cancellation (AGENT-003). And the call is
    // polled below hyper rather than below this request's middleware, so a
    // call's own middleware stack does not sit on top of the endpoint's.
    let call: CallFuture = Box::pin(call);
    if stream {
        HttpResponse::sse(CallStream::new(call, |answer| {
            crate::sse::SseEvent::data(answer.to_string()).with_event("message")
        }))
    } else {
        HttpResponse::stream_bytes(CallStream::new(call, |answer| {
            Ok(bytes::Bytes::from(answer.to_string()))
        }))
        .header("Content-Type", "application/json")
    }
}

/// A tool call's JSON-RPC answer, still to run.
type CallFuture = Pin<Box<dyn Future<Output = Value> + Send>>;

/// Run the call and build its JSON-RPC answer. Owned, so a streamed answer
/// can run it after the handler returned; `root` is the public root of the
/// request, which a streamed body polled later no longer has in scope.
#[allow(clippy::too_many_arguments)]
async fn run_call(
    endpoint: EndpointContext,
    registry: Arc<ToolRegistry>,
    name: String,
    arguments: Map<String, Value>,
    caller: ToolCaller,
    client: ClientFacts,
    config: Arc<AgentsConfig>,
    id: Value,
    root: Arc<str>,
) -> Value {
    let Some(tool) = registry.get(&name) else {
        return protocol::error(
            Some(&id),
            INVALID_PARAMS,
            &format!("Unknown tool: {name}"),
            None,
        );
    };
    // Every field that breaks the schema's shape is named at once; serde
    // and the `validator` rules, which stop at the first, run after.
    let report = super::input::check(tool.input_schema(), &arguments);
    if !report.is_empty() {
        return protocol::result(
            &id,
            match tools::invalid_input_result(&report, config.max_result_bytes) {
                Value::Object(result) => complete(&config, result),
                other => other,
            },
        );
    }
    let answer = crate::routing::root::scope(
        root,
        dispatch::call(
            &endpoint,
            tool,
            arguments,
            caller,
            client,
            config.call_timeout,
            config.max_result_bytes,
        ),
    )
    .await;
    let live = tool.live_target().is_some();
    let result = match answer {
        Ok(answer) => match live.then(|| tools::live_result(&answer)).flatten() {
            Some(result) => result,
            None => tools::answer_result(&answer, config.max_result_bytes),
        },
        Err(failure) => tools::failure_result(&failure),
    };
    let result = match result {
        Value::Object(result) => complete(&config, result),
        other => other,
    };
    protocol::result(&id, result)
}

/// A request-scoped answer body: one item carrying the call's JSON-RPC
/// answer, a JSON body or an SSE `message` event, then the end of the body.
/// The mutex only makes the body `Sync`, as a response body must be; it is
/// never contended.
struct CallStream<T> {
    call: std::sync::Mutex<Option<CallFuture>>,
    finish: fn(Value) -> T,
}

impl<T> CallStream<T> {
    fn new(call: CallFuture, finish: fn(Value) -> T) -> Self {
        Self {
            call: std::sync::Mutex::new(Some(call)),
            finish,
        }
    }
}

impl<T> futures::Stream for CallStream<T> {
    type Item = T;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let finish = this.finish;
        let slot = match this.call.get_mut() {
            Ok(slot) => slot,
            Err(poisoned) => poisoned.into_inner(),
        };
        let Some(call) = slot.as_mut() else {
            return Poll::Ready(None);
        };
        match call.as_mut().poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(answer) => {
                *slot = None;
                Poll::Ready(Some(finish(answer)))
            }
        }
    }
}

/// Why a tool's guard did not sign the caller's user in.
enum GuardRefusal {
    /// The guard's provider does not know the user.
    Unknown,
    /// The application has no AuthManager, or the guard names no provider.
    Misconfigured(FrameworkError),
    /// The guard's provider failed to answer.
    Failed(FrameworkError),
}

/// The caller with its user loaded through the guard `guard`, when the
/// tool names one and the caller has a user.
async fn guard_user(guard: Option<&str>, caller: ToolCaller) -> Result<ToolCaller, GuardRefusal> {
    let (Some(guard), Some(user)) = (guard, caller.acting_user()) else {
        return Ok(caller);
    };
    let id = user.get_auth_identifier();
    let manager = crate::App::get::<crate::auth::AuthManager>().ok_or_else(|| {
        GuardRefusal::Misconfigured(FrameworkError::internal(format!(
            "the tool's guard '{guard}' needs an AuthManager"
        )))
    })?;
    let provider = manager
        .guard_provider(guard)
        .map_err(GuardRefusal::Misconfigured)?;
    match provider.retrieve_by_id(&id).await {
        Ok(Some(loaded)) => Ok(caller.with_acting_user(loaded)),
        Ok(None) => Err(GuardRefusal::Unknown),
        Err(error) => Err(GuardRefusal::Failed(error)),
    }
}

/// The `initialize` result of a 2025-06-18 or 2025-11-25 client. No session
/// id is minted: every later request stands alone.
fn initialize_result(config: &AgentsConfig, version: &str) -> Value {
    let mut result = Map::new();
    result.insert("protocolVersion".into(), json!(version));
    result.insert("capabilities".into(), capabilities());
    result.insert("serverInfo".into(), server_info(config));
    if let Some(instructions) = &config.instructions {
        result.insert("instructions".into(), json!(instructions));
    }
    complete(config, result)
}

/// The `server/discover` result.
fn discover_result(config: &AgentsConfig) -> Value {
    let mut result = Map::new();
    result.insert("supportedVersions".into(), json!(SUPPORTED));
    result.insert("capabilities".into(), capabilities());
    if let Some(instructions) = &config.instructions {
        result.insert("instructions".into(), json!(instructions));
    }
    result.insert("ttlMs".into(), json!(0));
    result.insert("cacheScope".into(), json!("private"));
    complete(config, result)
}

/// The capabilities the endpoint serves: tools, whose list does not change
/// while the application runs.
fn capabilities() -> Value {
    json!({ "tools": { "listChanged": false } })
}

/// The endpoint's name and version.
fn server_info(config: &AgentsConfig) -> Value {
    json!({
        "name": config.server_name.clone().unwrap_or_else(app_name),
        "version": config
            .server_version
            .clone()
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string()),
    })
}

/// `result` as a complete result carrying the server's identity.
fn complete(config: &AgentsConfig, mut result: Map<String, Value>) -> Value {
    result.insert("resultType".into(), json!("complete"));
    let mut meta = match result.remove("_meta") {
        Some(Value::Object(meta)) => meta,
        _ => Map::new(),
    };
    meta.insert(META_SERVER_INFO.into(), server_info(config));
    result.insert("_meta".into(), Value::Object(meta));
    Value::Object(result)
}

/// The 401 of a call that needs a token it did not present, naming where to
/// get one and the scopes the tool needs (RFC 6750, RFC 9728).
fn challenge(config: &AgentsConfig, id: &Value, scopes: &[Scope]) -> HttpResponse {
    let mut header = format!(
        "Bearer resource_metadata=\"{}\"",
        quoted(&resource_metadata_url(config))
    );
    let scope = scope_list(scopes);
    if !scope.is_empty() {
        header.push_str(&format!(", scope=\"{scope}\""));
    }
    protocol::refuse(401, Some(id), UNAUTHORIZED, "The tool needs a token.", None)
        .header("WWW-Authenticate", header)
}

/// The 403 of a token that lacks a scope, naming every scope the call
/// needs, not only the missing ones.
fn insufficient_scope(config: &AgentsConfig, id: &Value, scopes: &[Scope]) -> HttpResponse {
    let header = format!(
        "Bearer error=\"insufficient_scope\", scope=\"{}\", resource_metadata=\"{}\", \
         error_description=\"The token lacks a scope the tool needs.\"",
        scope_list(scopes),
        quoted(&resource_metadata_url(config))
    );
    protocol::refuse(
        403,
        Some(id),
        FORBIDDEN,
        "The token lacks a scope the tool needs.",
        Some(json!({ "requiredScopes": scopes.iter().map(Scope::name).collect::<Vec<_>>() })),
    )
    .header("WWW-Authenticate", header)
}

/// The 401 of a token no resolver accepts.
fn invalid_token(config: &AgentsConfig, id: &Value, description: &str) -> HttpResponse {
    let header = format!(
        "Bearer error=\"invalid_token\", error_description=\"{}\", resource_metadata=\"{}\"",
        quoted(description),
        quoted(&resource_metadata_url(config))
    );
    protocol::refuse(401, Some(id), UNAUTHORIZED, description, None)
        .header("WWW-Authenticate", header)
}

/// A 500 that names nothing internal; the error is logged.
fn internal(id: &Value, error: &crate::error::FrameworkError) -> HttpResponse {
    tracing::error!(
        error = %crate::error::render_error_chain(error),
        "the MCP endpoint failed"
    );
    protocol::refuse(500, Some(id), INTERNAL_ERROR, "Internal error", None)
}

/// The scope names, space-joined, as a `scope` parameter carries them.
fn scope_list(scopes: &[Scope]) -> String {
    quoted(&scopes.iter().map(Scope::name).collect::<Vec<_>>().join(" "))
}

/// `value` with what would end a quoted header parameter removed.
fn quoted(value: &str) -> String {
    value.chars().filter(|c| *c != '"' && *c != '\\').collect()
}

/// Whether `origin` may call the endpoint: the application's own origin,
/// from `APP_URL`, or one the configuration allows.
fn origin_allowed(origin: &str, config: &AgentsConfig) -> bool {
    let origin = origin.trim_end_matches('/').to_ascii_lowercase();
    let app_url = crate::routing::url::app_url();
    let (own, _) = crate::routing::root::split_app_url(&app_url);
    origin == own.trim_end_matches('/').to_ascii_lowercase()
        || config.allowed_origins.contains(&origin)
}

/// Whether the client asked for an SSE answer: it accepts
/// `text/event-stream` and not JSON.
fn wants_stream(headers: &hyper::HeaderMap) -> bool {
    let accept = headers
        .get_all(hyper::header::ACCEPT)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect::<Vec<_>>()
        .join(",")
        .to_ascii_lowercase();
    let json = accept.contains("application/json")
        || accept.contains("application/*")
        || accept.contains("*/*");
    accept.contains("text/event-stream") && !json
}

/// Whether the request's `Content-Type` is JSON.
fn is_json(content_type: Option<&str>) -> bool {
    content_type.is_some_and(|value| {
        let media = value
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        media == "application/json" || media.ends_with("+json")
    })
}

/// Whether `id` is an id MCP allows: a string or an integer, never `null`.
fn is_valid_id(id: &Value) -> bool {
    id.is_string() || id.is_i64() || id.is_u64()
}
