//! The server tools a page offers (AGENT-014), read from the element the
//! server writes into the page's head, registered with WebMCP and called
//! the way the Live runtime and the kits call them.

use std::collections::HashSet;
use std::fmt;
use std::future::Future;
use std::rc::Rc;

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::error::WebMcpError;
use crate::path::{self, Piece};
use crate::result::{self, ToolResult};
use crate::tool::{self, Annotations, ToolHandle};
use crate::{DEFAULT_MAX_RESULT_BYTES, LocalFuture, platform};

/// The most tools one page offers, the bound the Live runtime holds an
/// island to.
const MAX_TOOLS: usize = 64;

/// The methods a server tool's route answers.
const METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];

/// The framework's default `call_timeout`, for a page that lists no tools.
const DEFAULT_CALL_TIMEOUT_MS: u64 = 30_000;

/// The server tools the page offers this viewer.
///
/// The server decides the list: the tools the application shares on every
/// page, plus those the page's route or response adds, less any the
/// viewer's caller kind may not call. The front end cannot widen it, and
/// the server still checks every call.
#[derive(Clone)]
pub struct ServerTools {
    tools: Vec<ServerTool>,
    limits: Limits,
}

/// One server tool of the page.
#[derive(Debug, Clone)]
pub(crate) struct ServerTool {
    pub(crate) name: String,
    title: Option<String>,
    description: String,
    input_schema: Map<String, Value>,
    pub(crate) annotations: Annotations,
    pub(crate) method: String,
    pub(crate) path: Vec<Piece>,
}

/// The bounds the router's `AgentsConfig` sets on a browser call.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    /// The largest answer body a call returns.
    pub(crate) max_result_bytes: usize,
    /// How long a call may take before the page stops it.
    pub(crate) call_timeout_ms: u64,
}

impl ServerTools {
    /// The server tools of the current page, from its
    /// `<script id="suprnova-agent-tools">` element. A page without the
    /// element offers none.
    ///
    /// # Errors
    ///
    /// - [`WebMcpError::InvalidPage`] when the element is not what the
    ///   server writes. None of its tools are then trusted.
    /// - [`WebMcpError::Unsupported`] where there is no page to read.
    pub fn from_document() -> Result<Self, WebMcpError> {
        match platform::page_tools()? {
            Some(text) => Self::parse(&text),
            None => Ok(Self {
                tools: Vec::new(),
                limits: Limits {
                    max_result_bytes: DEFAULT_MAX_RESULT_BYTES,
                    call_timeout_ms: DEFAULT_CALL_TIMEOUT_MS,
                },
            }),
        }
    }

    /// The names of the page's server tools, in the server's order.
    pub fn names(&self) -> impl Iterator<Item = &str> + '_ {
        self.tools.iter().map(|tool| tool.name.as_str())
    }

    /// Call the server tool `name` with `arguments`, a JSON object, as an
    /// agent's call would: a same-origin request with the session cookie,
    /// the page's CSRF token and the `Suprnova-Tool-Call` header.
    ///
    /// A call the server refused or failed, or that could not be sent, is
    /// still a result, with `is_error` set and the reason in its text.
    ///
    /// # Errors
    ///
    /// - [`WebMcpError::UnknownTool`] when the page offers no such tool.
    /// - [`WebMcpError::Unsupported`] where there is no page to call from.
    pub async fn call(&self, name: &str, arguments: &Value) -> Result<ToolResult, WebMcpError> {
        let tool = self
            .tool(name)
            .ok_or_else(|| WebMcpError::UnknownTool(name.to_string()))?;
        platform::call_tool(tool, self.limits, arguments).await
    }

    /// Register every server tool of the page with `document.modelContext`.
    ///
    /// After an agent's call of a tool not marked read-only reaches the
    /// server, `on_write` runs before the call's result goes back, and is
    /// awaited up to the call timeout, so the page shows what the agent
    /// did before the agent reads it again. A refresh that fails or runs
    /// long does not change the result.
    ///
    /// The tools stay registered until the returned handles are dropped.
    /// When the browser refuses one tool at once, none of them stays
    /// registered.
    ///
    /// # Errors
    ///
    /// [`WebMcpError::Unsupported`] where there is no WebMCP to register
    /// with, and [`WebMcpError::Rejected`] when the browser refuses a tool
    /// at once.
    pub fn register_all<F, Fut>(&self, on_write: F) -> Result<Vec<ToolHandle>, WebMcpError>
    where
        F: Fn() -> Fut + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        platform::available()?;
        let on_write: Rc<dyn Fn() -> LocalFuture<()>> = Rc::new(move || Box::pin(on_write()));
        self.tools
            .iter()
            .map(|tool| {
                platform::register_server_tool(tool, self.limits, Rc::clone(&on_write))
                    .map(|registration| ToolHandle::new(&tool.name, registration))
            })
            .collect()
    }

    /// The page's tool named `name`.
    pub(crate) fn tool(&self, name: &str) -> Option<&ServerTool> {
        self.tools.iter().find(|tool| tool.name == name)
    }

    /// Read and check the element's JSON as strictly as the Live runtime
    /// reads an island's tools: an unknown key, a missing hint, a name
    /// outside the grammar or a path that does not start at the page's
    /// root refuses the whole element.
    pub(crate) fn parse(text: &str) -> Result<Self, WebMcpError> {
        let invalid = |reason: String| WebMcpError::InvalidPage(reason);
        let page: RawPage =
            serde_json::from_str(text).map_err(|error| invalid(error.to_string()))?;
        if page.v != 1 {
            return Err(invalid(format!("version {} is not 1", page.v)));
        }
        let max_result_bytes = usize::try_from(page.limits.max_result_bytes)
            .ok()
            .filter(|bytes| *bytes > 0)
            .ok_or_else(|| invalid("maxResultBytes is not a positive size".into()))?;
        if page.limits.call_timeout_ms == 0 {
            return Err(invalid("callTimeoutMs is zero".into()));
        }
        if page.tools.len() > MAX_TOOLS {
            return Err(invalid(format!(
                "{} tools is more than {MAX_TOOLS}",
                page.tools.len()
            )));
        }
        let mut names = HashSet::new();
        let tools = page
            .tools
            .into_iter()
            .map(|raw| {
                let name = raw.name;
                if !tool::valid_name(&name) {
                    return Err(invalid(format!("the tool name `{name}` is not valid")));
                }
                if !names.insert(name.clone()) {
                    return Err(invalid(format!("the tool `{name}` is listed twice")));
                }
                if raw
                    .title
                    .as_deref()
                    .is_some_and(|title| title.trim().is_empty())
                {
                    return Err(invalid(format!("the tool `{name}` has a blank title")));
                }
                if raw.description.trim().is_empty() {
                    return Err(invalid(format!("the tool `{name}` has no description")));
                }
                if !METHODS.contains(&raw.method.as_str()) {
                    return Err(invalid(format!(
                        "the tool `{name}` has the method {}",
                        raw.method
                    )));
                }
                check_path(&raw.path).map_err(|reason| {
                    invalid(format!("the tool `{name}` has a path that {reason}"))
                })?;
                Ok(ServerTool {
                    name,
                    title: raw.title,
                    description: raw.description,
                    input_schema: raw.input_schema,
                    annotations: raw.annotations,
                    method: raw.method,
                    path: raw.path,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            tools,
            limits: Limits {
                max_result_bytes,
                call_timeout_ms: page.limits.call_timeout_ms,
            },
        })
    }
}

impl fmt::Debug for ServerTools {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerTools")
            .field("tools", &self.names().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl ServerTool {
    /// The tool as `registerTool` receives it, less `execute`.
    pub(crate) fn descriptor(&self) -> Value {
        tool::descriptor(
            &self.name,
            self.title.as_deref(),
            &self.description,
            Value::Object(self.input_schema.clone()),
            self.annotations,
        )
    }

    /// Whether a call of the tool is followed by the page's refresh: any
    /// tool not marked read-only may have changed what the page shows.
    pub(crate) fn refreshes(&self) -> bool {
        !self.annotations.read_only_hint
    }
}

/// The request a call sends, before the browser adds the origin and the
/// headers.
#[derive(Debug)]
pub(crate) struct Plan {
    /// The tool's method.
    pub(crate) method: String,
    /// The path and, for a GET call, the query.
    pub(crate) target: String,
    /// The JSON body of a call that is not a GET: every argument, path
    /// arguments included, as the MCP dispatcher sends them.
    pub(crate) body: Option<String>,
}

/// The request for a call of `tool` with `arguments`, or the error result
/// of arguments it cannot send.
pub(crate) fn plan(tool: &ServerTool, arguments: &Value) -> Result<Plan, ToolResult> {
    let Value::Object(arguments) = arguments else {
        return Err(result::invalid_input("the arguments must be an object"));
    };
    let path =
        path::fill(&tool.path, arguments).map_err(|reason| result::invalid_input(&reason))?;
    if tool.method == "GET" {
        let target = match path::query(&tool.path, arguments)
            .map_err(|reason| result::invalid_input(&reason))?
        {
            Some(query) => format!("{path}?{query}"),
            None => path,
        };
        return Ok(Plan {
            method: tool.method.clone(),
            target,
            body: None,
        });
    }
    let body = serde_json::to_string(arguments)
        .map_err(|error| result::invalid_input(&error.to_string()))?;
    Ok(Plan {
        method: tool.method.clone(),
        target: path,
        body: Some(body),
    })
}

/// Why `pieces` is not a path the server writes, if it is not: it starts
/// with text at the page's root, never with `//`, which a URL reads as
/// another host, and no piece is empty.
fn check_path(pieces: &[Piece]) -> Result<(), &'static str> {
    match pieces.first() {
        None => return Err("is empty"),
        Some(Piece::Text(first)) => {
            if !first.text.starts_with('/') {
                return Err("does not start at the root");
            }
            if first.text.starts_with("//") || first.text.starts_with("/\\") {
                return Err("names another host");
            }
        }
        Some(Piece::Param(_)) => return Err("starts with a parameter"),
    }
    for piece in pieces {
        match piece {
            Piece::Text(text) if text.text.is_empty() => return Err("has an empty text piece"),
            Piece::Param(param) if param.param.is_empty() => {
                return Err("has a parameter without a name");
            }
            _ => {}
        }
    }
    Ok(())
}

/// The element's JSON, as the contract writes it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPage {
    v: u64,
    limits: RawLimits,
    tools: Vec<RawTool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawLimits {
    max_result_bytes: u64,
    call_timeout_ms: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawTool {
    name: String,
    #[serde(default)]
    title: Option<String>,
    description: String,
    input_schema: Map<String, Value>,
    annotations: Annotations,
    method: String,
    path: Vec<Piece>,
}
