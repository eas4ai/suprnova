//! Calling tools in tests, as each kind of caller (AGENT-018).
//!
//! A test calls a tool two ways, both through `handle_request`:
//!
//! - **Over MCP**, with [`TestClient::mcp`]: a guest sends no token; a user
//!   or an agent account gets a token minted by a fake resolver, so no OAuth
//!   or auth.md ceremony runs. Only the token's verification is faked. The
//!   endpoint still checks the headers, the origin, the caller's kind and
//!   scopes, the limit and the run time, and the call still runs the route's
//!   middleware, validation and policies.
//! - **From the browser**, with [`TestClient::tool`]: the tool's route,
//!   called with the client's session, as the user the client acts as or as
//!   a guest. The browser path never carries an agent account.
//!
//! ```rust,ignore
//! let client = TestClient::new(routes(), MiddlewareRegistry::new());
//!
//! let result = client
//!     .mcp()
//!     .as_caller(ToolCaller::user(user.clone()).scopes([scope::POSTS_WRITE]))
//!     .call("posts.create", json!({ "title": "Hello" }))
//!     .await;
//! assert!(!result.is_error());
//!
//! client.acting_as(&user).tool("posts.create", json!({ "title": "Hello" }))
//!     .send().await.assert_ok();
//! ```

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock};

use serde_json::{Value, json};

use super::caller::ToolCaller;
use super::dispatch::fill_path;
use super::mcp::protocol as wire;
use super::{Agents, DEFAULT_MCP_PATH};
use crate::testing::{TestClient, TestRequest, TestResponse};

/// The tokens fakes minted, and how many fakes are alive.
#[derive(Default)]
struct FakeTokens {
    alive: usize,
    tokens: HashMap<String, ToolCaller>,
}

fn fakes() -> &'static RwLock<FakeTokens> {
    static FAKES: OnceLock<RwLock<FakeTokens>> = OnceLock::new();
    FAKES.get_or_init(|| RwLock::new(FakeTokens::default()))
}

/// The caller a fake minted `token` for, while any fake is alive.
pub(crate) fn resolve_fake(token: &str) -> Option<ToolCaller> {
    let fakes = fakes().read().unwrap_or_else(PoisonError::into_inner);
    if fakes.alive == 0 {
        return None;
    }
    fakes.tokens.get(token).cloned()
}

impl Agents {
    /// Accept tokens minted for chosen callers, for as long as the returned
    /// guard lives, so a test calls tools as a user or an agent account
    /// without an OAuth or auth.md ceremony.
    ///
    /// Each token is random, so tests running at once do not see each
    /// other's callers. [`TestClient::mcp`] uses one of these itself.
    pub fn fake() -> AgentsFake {
        fakes()
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .alive += 1;
        AgentsFake {
            minted: Mutex::new(Vec::new()),
        }
    }
}

/// A fake token issuer, alive until dropped. See [`Agents::fake`].
///
/// One exists only when a test asks for it, through [`Agents::fake`] or
/// [`TestClient::mcp`], which calls it: its field is private, so nothing
/// else can build one, and no production path does. While none is alive the
/// endpoint consults no fake token at all.
pub struct AgentsFake {
    minted: Mutex<Vec<String>>,
}

impl AgentsFake {
    /// A bearer token the MCP endpoint accepts as `caller`.
    pub fn token_for(&self, caller: ToolCaller) -> String {
        let token = format!("suprnova-fake-{}", uuid::Uuid::new_v4().simple());
        fakes()
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .tokens
            .insert(token.clone(), caller);
        self.minted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(token.clone());
        token
    }
}

impl Drop for AgentsFake {
    fn drop(&mut self) {
        let minted =
            std::mem::take(&mut *self.minted.lock().unwrap_or_else(PoisonError::into_inner));
        let mut fakes = fakes().write().unwrap_or_else(PoisonError::into_inner);
        for token in minted {
            fakes.tokens.remove(&token);
        }
        fakes.alive = fakes.alive.saturating_sub(1);
    }
}

impl std::fmt::Debug for AgentsFake {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentsFake").finish_non_exhaustive()
    }
}

impl TestClient {
    /// A client of the application's MCP endpoint, as a guest until
    /// [`McpTestClient::as_caller`] names a caller. It speaks revision
    /// 2026-07-28 unless [`McpTestClient::protocol_version`] picks another.
    pub fn mcp(&self) -> McpTestClient {
        let path = self
            .router()
            .agents
            .config_path()
            .unwrap_or_else(|| DEFAULT_MCP_PATH.to_string());
        McpTestClient {
            client: self.clone(),
            fake: Arc::new(Agents::fake()),
            token: None,
            version: wire::MODERN.to_string(),
            headers: Vec::new(),
            path,
            ids: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Call the tool `name` from the browser: a request to the tool's route
    /// with `arguments` as a JSON body, its path parameters filled from the
    /// arguments, the [`TOOL_CALL_HEADER`](super::TOOL_CALL_HEADER) the
    /// browser helpers send, this client's cookies and the user it acts as.
    ///
    /// When the client holds the session's `XSRF-TOKEN` cookie, the request
    /// sends it back as `X-XSRF-TOKEN`, as a page's script does; visit a page
    /// first to get one, as the browser would.
    ///
    /// # Panics
    ///
    /// Panics when the application has no tool named `name`, or its path
    /// parameters are missing from `arguments`: the test asked for a call
    /// that cannot be made.
    pub fn tool(&self, name: &str, arguments: Value) -> TestRequest {
        let registry = self
            .router()
            .tool_registry()
            .unwrap_or_else(|error| panic!("TestClient::tool(): the tools do not check: {error}"));
        let tool = registry
            .get(name)
            .unwrap_or_else(|| panic!("TestClient::tool(): no tool is named `{name}`"));
        let object = arguments.as_object().cloned().unwrap_or_default();
        let path = fill_path(tool.pattern(), &object)
            .unwrap_or_else(|error| panic!("TestClient::tool(`{name}`): {error}"));
        let mut request = self
            .send(tool.method().clone(), path)
            .json(&Value::Object(object))
            .header("Accept", "application/json")
            .header(super::TOOL_CALL_HEADER, name);
        if let Some(token) = self.cookie("XSRF-TOKEN") {
            request = request.header("X-XSRF-TOKEN", token);
        }
        request
    }
}

impl TestClient {
    /// Call the Live tool `name` from the browser, as the Live runtime's
    /// WebMCP registration does: a Live action request to `/__live/action`
    /// carrying the island's snapshot from `page`, a page this client
    /// rendered, invoking the tool's action with `arguments`, with the
    /// [`TOOL_CALL_HEADER`](super::TOOL_CALL_HEADER), `Sec-Fetch-Site:
    /// same-origin`, this client's cookies and its `XSRF-TOKEN` as
    /// `X-XSRF-TOKEN` (AGENT-012, AGENT-018).
    ///
    /// The request takes the path a click takes: the session, CSRF and
    /// origin checks, the snapshot verification, the
    /// `live:{component}.{action}` authorization and the transaction, plus
    /// the tool's caller check and budget.
    ///
    /// # Panics
    ///
    /// Panics when no island of `page` registers a tool named `name`: the
    /// test asked for a call the page does not offer.
    pub fn live_tool(&self, name: &str, page: &str, arguments: Value) -> TestRequest {
        let island = LiveIsland::offering(page, name).unwrap_or_else(|| {
            panic!("TestClient::live_tool(): no island of the page offers the tool `{name}`")
        });
        let snapshot = if island.seed {
            json!({
                "kind": "seed_promotion",
                "envelope": island.snapshot,
                "browser_nonce": random_identity(),
            })
        } else {
            json!({ "kind": "instance", "envelope": island.snapshot })
        };
        let body = json!({
            "protocol_version": 2,
            "runtime_contract_version": 2,
            "snapshot_schema_version": 1,
            "correlation_id": random_identity(),
            "idempotency_key": random_identity(),
            "component": island.component,
            "base_revision": island.revision,
            "snapshot": snapshot,
            "child_parameters": null,
            "model_proposals": {},
            "operations": [{
                "kind": "invoke_action",
                "name": island.action,
                "arguments": arguments,
            }],
            "extensions": { "x_suprnova_live_document_key_v1": island.document_key },
        });
        let mut request = self
            .post("/__live/action")
            .json(&body)
            .header(
                "Content-Type",
                "application/vnd.suprnova.live+json; charset=utf-8; version=2",
            )
            .header("Sec-Fetch-Site", "same-origin")
            .header(super::TOOL_CALL_HEADER, name);
        if let Some(token) = self.cookie("XSRF-TOKEN") {
            request = request.header("X-XSRF-TOKEN", token);
        }
        request
    }
}

/// The island of a rendered page that registers a tool, as the browser
/// runtime reads it from the island root.
struct LiveIsland {
    component: String,
    document_key: String,
    revision: String,
    seed: bool,
    snapshot: Value,
    action: String,
}

impl LiveIsland {
    /// The island of `page` whose root lists the tool `name`.
    fn offering(page: &str, name: &str) -> Option<Self> {
        use base64::Engine as _;

        for root in page.split("<div").skip(1) {
            let attributes = root.split('>').next().unwrap_or_default();
            let Some(tools) = attribute(attributes, "data-suprnova-live-tools") else {
                continue;
            };
            let tools: Value = serde_json::from_str(&tools).ok()?;
            let Some(action) = tools["tools"]
                .as_array()
                .and_then(|tools| tools.iter().find(|tool| tool["name"] == name))
                .and_then(|tool| tool["action"].as_str())
            else {
                continue;
            };
            let snapshot = attribute(attributes, "data-suprnova-live-snapshot")?;
            let snapshot = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(snapshot)
                .ok()?;
            return Some(Self {
                component: attribute(attributes, "data-suprnova-live-component")?,
                document_key: attribute(attributes, "data-suprnova-live-document-key")?,
                revision: attribute(attributes, "data-suprnova-live-revision")?,
                seed: attribute(attributes, "data-suprnova-live-snapshot-kind")?.as_str() == "seed",
                snapshot: serde_json::from_slice(&snapshot).ok()?,
                action: action.to_string(),
            });
        }
        None
    }
}

/// The value of the attribute `name` among an element's `attributes`, with
/// the escapes the island root writes undone.
fn attribute(attributes: &str, name: &str) -> Option<String> {
    let prefix = format!(" {name}=\"");
    let start = attributes.find(&prefix)? + prefix.len();
    let end = start + attributes[start..].find('"')?;
    Some(
        attributes[start..end]
            .replace("&quot;", "\"")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&#39;", "'")
            .replace("&amp;", "&"),
    )
}

/// 16 random bytes as unpadded base64url, the form of a Live correlation
/// id, idempotency key and browser nonce.
fn random_identity() -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(uuid::Uuid::new_v4().as_bytes())
}

/// A test client of the MCP endpoint. See [`TestClient::mcp`].
#[derive(Clone)]
pub struct McpTestClient {
    client: TestClient,
    fake: Arc<AgentsFake>,
    token: Option<String>,
    version: String,
    headers: Vec<(String, String)>,
    path: String,
    ids: Arc<AtomicU64>,
}

impl McpTestClient {
    /// Call as `caller`: a guest sends no token, any other caller a token
    /// minted for it.
    pub fn as_caller(mut self, caller: ToolCaller) -> Self {
        self.token = if caller.is_guest() {
            None
        } else {
            Some(self.fake.token_for(caller))
        };
        self
    }

    /// Send `token` as the bearer token, unchanged, to test a resolver.
    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }

    /// Speak `version`: `2026-07-28` (the default), or `2025-11-25` or
    /// `2025-06-18`, which send the version header without the 2026-07-28
    /// request metadata.
    pub fn protocol_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    /// Send `name: value` on every request, as a browser's `Origin` or a
    /// client's `Accept`.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        let name = name.into();
        self.headers
            .retain(|(held, _)| !held.eq_ignore_ascii_case(&name));
        self.headers.push((name, value.into()));
        self
    }

    /// The bearer token the client sends, if any.
    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    /// `server/discover`.
    pub async fn discover(&self) -> McpResponse {
        self.request("server/discover", json!({})).await
    }

    /// `initialize`, offering the client's protocol version.
    pub async fn initialize(&self) -> McpResponse {
        let params = json!({
            "protocolVersion": self.version,
            "capabilities": {},
            "clientInfo": { "name": "suprnova-test", "version": "1.0.0" },
        });
        self.send("initialize", params, false).await
    }

    /// `tools/list`.
    pub async fn list(&self) -> McpResponse {
        self.request("tools/list", json!({})).await
    }

    /// `tools/call` of `name` with `arguments`.
    pub async fn call(&self, name: &str, arguments: Value) -> McpToolResult {
        McpToolResult {
            inner: self
                .request(
                    "tools/call",
                    json!({ "name": name, "arguments": arguments }),
                )
                .await,
        }
    }

    /// Any request, with the headers and metadata the client's revision
    /// requires.
    pub async fn request(&self, method: &str, params: Value) -> McpResponse {
        self.send(method, params, true).await
    }

    /// POST `body` to the endpoint with only `headers`, the client's
    /// headers and its token: no protocol header or metadata is added, to
    /// test how the endpoint answers a request that breaks its rules.
    pub async fn raw(&self, headers: &[(&str, &str)], body: Value) -> TestResponse {
        let mut request = self.base_request(&body);
        for (header, value) in &self.headers {
            request = request.header(header.clone(), value.clone());
        }
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        request.send().await
    }

    async fn send(&self, method: &str, params: Value, with_version: bool) -> McpResponse {
        let mut params = match params {
            Value::Object(params) => params,
            _ => serde_json::Map::new(),
        };
        let modern = self.version == wire::MODERN;
        if modern && with_version {
            let mut meta = serde_json::Map::new();
            meta.insert(wire::META_PROTOCOL_VERSION.into(), json!(self.version));
            meta.insert(
                "io.modelcontextprotocol/clientInfo".into(),
                json!({ "name": "suprnova-test", "version": "1.0.0" }),
            );
            meta.insert(wire::META_CLIENT_CAPABILITIES.into(), json!({}));
            params.insert("_meta".into(), Value::Object(meta));
        }
        let name = params
            .get("name")
            .or_else(|| params.get("uri"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let body = json!({
            "jsonrpc": "2.0",
            "id": self.ids.fetch_add(1, Ordering::Relaxed),
            "method": method,
            "params": Value::Object(params),
        });
        let mut request = self
            .base_request(&body)
            .header("Accept", "application/json, text/event-stream");
        if with_version {
            request = request.header("MCP-Protocol-Version", self.version.clone());
            if modern {
                request = request.header("Mcp-Method", method);
                if let Some(name) = name.filter(|_| wire::name_field(method).is_some()) {
                    request = request.header("Mcp-Name", encode_header(&name));
                }
            }
        }
        for (header, value) in &self.headers {
            request = request.header(header.clone(), value.clone());
        }
        McpResponse::new(request.send().await)
    }

    fn base_request(&self, body: &Value) -> TestRequest {
        let mut request = self.client.post(self.path.clone()).json(body);
        if let Some(token) = &self.token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        request
    }
}

impl std::fmt::Debug for McpTestClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpTestClient")
            .field("path", &self.path)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

/// A header value as MCP carries it: plain when it is visible ASCII, the
/// Base64 sentinel otherwise.
fn encode_header(value: &str) -> String {
    use base64::Engine;
    let plain = !value.is_empty()
        && value.bytes().all(|byte| (0x21..=0x7E).contains(&byte))
        && !(value.starts_with("=?base64?") && value.ends_with("?="));
    if plain {
        value.to_string()
    } else {
        format!(
            "=?base64?{}?=",
            base64::engine::general_purpose::STANDARD.encode(value)
        )
    }
}

/// One answer of the MCP endpoint: the HTTP response and the JSON-RPC
/// message it carries, read from a JSON body or the `message` event of an
/// SSE body.
pub struct McpResponse {
    response: TestResponse,
    message: Value,
}

impl McpResponse {
    fn new(response: TestResponse) -> Self {
        let text = response.body_text();
        let streamed = response
            .header("content-type")
            .is_some_and(|value| value.starts_with("text/event-stream"));
        let message = if streamed {
            text.lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(str::trim)
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            text
        };
        let message = serde_json::from_str(&message).unwrap_or(Value::Null);
        Self { response, message }
    }

    /// The HTTP status.
    pub fn status(&self) -> u16 {
        self.response.status()
    }

    /// A response header.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.response.header(name)
    }

    /// The JSON-RPC message, `null` when the body held none (a 202).
    pub fn message(&self) -> &Value {
        &self.message
    }

    /// The JSON-RPC result, when the answer is one.
    pub fn result(&self) -> Option<&Value> {
        self.message.get("result")
    }

    /// The JSON-RPC error code, when the answer is an error.
    pub fn error_code(&self) -> Option<i64> {
        self.message
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_i64)
    }

    /// The names of the tools a `tools/list` answer lists, in its order.
    pub fn tool_names(&self) -> Vec<String> {
        self.result()
            .and_then(|result| result.get("tools"))
            .and_then(Value::as_array)
            .map(|tools| {
                tools
                    .iter()
                    .filter_map(|tool| tool.get("name").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The HTTP response itself.
    pub fn response(&self) -> &TestResponse {
        &self.response
    }
}

impl std::fmt::Debug for McpResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpResponse")
            .field("status", &self.status())
            .field("message", &self.message)
            .finish()
    }
}

/// The answer of a `tools/call`.
#[derive(Debug)]
pub struct McpToolResult {
    inner: McpResponse,
}

impl McpToolResult {
    /// Whether the call answered a tool result with `isError: true`. A
    /// protocol error is not a tool result: check [`Self::error_code`].
    pub fn is_error(&self) -> bool {
        self.inner
            .result()
            .and_then(|result| result.get("isError"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// The structured content.
    pub fn structured(&self) -> Option<&Value> {
        self.inner
            .result()
            .and_then(|result| result.get("structuredContent"))
    }

    /// The state handle a headless Live tool returned, to pass as the
    /// `handle` argument of a later call (AGENT-013).
    pub fn handle(&self) -> Option<&str> {
        self.structured()
            .and_then(|structured| structured.get("handle"))
            .and_then(Value::as_str)
    }

    /// The text of every text content item, joined by newlines.
    pub fn text(&self) -> String {
        self.inner
            .result()
            .and_then(|result| result.get("content"))
            .and_then(Value::as_array)
            .map(|content| {
                content
                    .iter()
                    .filter_map(|item| item.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    }

    /// The HTTP status.
    pub fn status(&self) -> u16 {
        self.inner.status()
    }

    /// The JSON-RPC error code, for a call refused as a protocol error.
    pub fn error_code(&self) -> Option<i64> {
        self.inner.error_code()
    }

    /// The whole answer.
    pub fn response(&self) -> &McpResponse {
        &self.inner
    }
}
