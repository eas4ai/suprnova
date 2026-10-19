//! Typed page tools: a front end's own tools, registered with
//! `document.modelContext` for as long as their handle lives.

use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::rc::Rc;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use validator::Validate;

use crate::error::WebMcpError;
use crate::result::{self, ToolError, ToolResult};
use crate::{DEFAULT_MAX_RESULT_BYTES, LocalFuture, platform};

/// A page tool with a typed input, ready to register.
///
/// The input schema is derived from `T` as the server derives a
/// `#[tool]`'s, and a call's input is read into `T` and checked against its
/// `validator` rules before the handler sees it, so the handler only ever
/// receives input that matches what the agent was told.
///
/// The name and description are `&'static str` because an agent reads them
/// as instructions: they belong in the code, never in data a user can
/// change (AGENT-001).
pub struct PageTool<T> {
    name: &'static str,
    description: &'static str,
    title: Option<&'static str>,
    annotations: Annotations,
    _input: PhantomData<fn() -> T>,
}

impl<T> PageTool<T>
where
    T: DeserializeOwned + JsonSchema + Validate + 'static,
{
    /// A tool named `name`, which an agent chooses by `description`.
    ///
    /// The name follows the AGENT-001 grammar, 1 to 128 ASCII letters,
    /// digits, `_`, `-` and `.`; [`register`](Self::register) refuses any
    /// other.
    pub fn new(name: &'static str, description: &'static str) -> Self {
        Self {
            name,
            description,
            title: None,
            annotations: Annotations::default(),
            _input: PhantomData,
        }
    }

    /// A short label an agent's interface can show instead of the name.
    pub fn title(mut self, title: &'static str) -> Self {
        self.title = Some(title);
        self
    }

    /// Marks the tool as changing nothing, WebMCP's `readOnlyHint`, so an
    /// agent may call it without asking the user.
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.annotations.read_only_hint = read_only;
        self
    }

    /// Marks the tool's result as holding content the application does not
    /// vouch for, such as what other users wrote: WebMCP's
    /// `untrustedContentHint`, so an agent does not follow instructions
    /// inside it.
    pub fn untrusted_content(mut self, untrusted: bool) -> Self {
        self.annotations.untrusted_content_hint = untrusted;
        self
    }

    /// Marks the tool as having effects the user would want to confirm,
    /// WebMCP's `consequentialHint`. The server's `#[tool]` sets the same
    /// hint from `destructive`.
    pub fn consequential(mut self, consequential: bool) -> Self {
        self.annotations.consequential_hint = consequential;
        self
    }

    /// Register the tool with `document.modelContext`, with `handler`
    /// answering each call. The tool stays registered until the returned
    /// handle is dropped.
    ///
    /// # Errors
    ///
    /// - [`WebMcpError::InvalidName`], [`WebMcpError::MissingDescription`],
    ///   [`WebMcpError::BlankTitle`] or [`WebMcpError::InputNotObject`] for
    ///   a tool an agent could not use. These are checked on every target,
    ///   so a native test run finds them.
    /// - [`WebMcpError::Unsupported`] where there is no WebMCP to register
    ///   with.
    /// - [`WebMcpError::Rejected`] when the browser refuses the tool at
    ///   once. A refusal the browser reports later arrives through
    ///   [`ToolHandle::registered`].
    pub fn register<F, Fut>(self, handler: F) -> Result<ToolHandle, WebMcpError>
    where
        F: Fn(T) -> Fut + 'static,
        Fut: Future<Output = Result<ToolResult, ToolError>> + 'static,
    {
        let descriptor = self.descriptor()?;
        platform::available()?;
        let handler = Rc::new(handler);
        let execute: Rc<dyn Fn(String) -> LocalFuture<ToolResult>> =
            Rc::new(move |input: String| {
                let handler = Rc::clone(&handler);
                Box::pin(async move { run(&*handler, &input).await })
            });
        let registration = platform::register_page_tool(self.name, &descriptor, execute)?;
        Ok(ToolHandle::new(self.name, registration))
    }

    /// The tool as `registerTool` receives it, less `execute`, after the
    /// checks every target runs.
    pub(crate) fn descriptor(&self) -> Result<Value, WebMcpError> {
        let name = self.name;
        if !valid_name(name) {
            return Err(WebMcpError::InvalidName {
                name: name.to_string(),
            });
        }
        if self.description.trim().is_empty() {
            return Err(WebMcpError::MissingDescription {
                name: name.to_string(),
            });
        }
        if self.title.is_some_and(|title| title.trim().is_empty()) {
            return Err(WebMcpError::BlankTitle {
                name: name.to_string(),
            });
        }
        let schema = crate::input_schema::<T>();
        if schema.get("type").and_then(Value::as_str) != Some("object") {
            return Err(WebMcpError::InputNotObject {
                name: name.to_string(),
            });
        }
        Ok(descriptor(
            name,
            self.title,
            self.description,
            schema,
            self.annotations,
        ))
    }
}

impl<T> fmt::Debug for PageTool<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PageTool")
            .field("name", &self.name)
            .field("title", &self.title)
            .finish_non_exhaustive()
    }
}

/// A registered tool. Dropping it removes the tool from the page.
///
/// It is neither `Send` nor `Sync` on any target, as a browser
/// registration is not, so code that compiles natively compiles for the
/// browser too.
#[must_use = "dropping the handle unregisters the tool at once"]
pub struct ToolHandle {
    name: String,
    registration: platform::Registration,
    _local: PhantomData<Rc<()>>,
}

impl ToolHandle {
    pub(crate) fn new(name: &str, registration: platform::Registration) -> Self {
        Self {
            name: name.to_string(),
            registration,
            _local: PhantomData,
        }
    }

    /// The registered tool's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Wait until the browser has settled the registration.
    ///
    /// The WebMCP draft lets `registerTool` answer with a promise, so a
    /// refusal, such as a name another tool on the page already holds, can
    /// arrive after [`PageTool::register`] returned.
    ///
    /// # Errors
    ///
    /// [`WebMcpError::Rejected`] when the browser refused the tool.
    pub async fn registered(&self) -> Result<(), WebMcpError> {
        self.registration.registered(&self.name).await
    }
}

impl fmt::Debug for ToolHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolHandle")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// The three WebMCP hints, always all sent, so an agent never falls back
/// on a default the marking did not choose.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Annotations {
    pub(crate) read_only_hint: bool,
    pub(crate) untrusted_content_hint: bool,
    pub(crate) consequential_hint: bool,
}

/// Whether `name` follows the AGENT-001 grammar: 1 to 128 ASCII letters,
/// digits, `_`, `-` and `.`.
pub(crate) fn valid_name(name: &str) -> bool {
    (1..=128).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

/// A tool as `registerTool` receives it, less `execute`.
pub(crate) fn descriptor(
    name: &str,
    title: Option<&str>,
    description: &str,
    input_schema: Value,
    annotations: Annotations,
) -> Value {
    let mut tool = Map::new();
    tool.insert("name".into(), json!(name));
    if let Some(title) = title {
        tool.insert("title".into(), json!(title));
    }
    tool.insert("description".into(), json!(description));
    tool.insert("inputSchema".into(), input_schema);
    tool.insert(
        "annotations".into(),
        json!({
            "readOnlyHint": annotations.read_only_hint,
            "untrustedContentHint": annotations.untrusted_content_hint,
            "consequentialHint": annotations.consequential_hint,
        }),
    );
    Value::Object(tool)
}

/// Answer one call of a page tool whose input arrived as the JSON text
/// `input`: read it into `T`, check its `validator` rules, then run the
/// handler. Input that fails either check is an error result naming what
/// is wrong, as a server tool's 422 is, and the handler never sees it.
pub(crate) async fn run<T, F, Fut>(handler: &F, input: &str) -> ToolResult
where
    T: DeserializeOwned + Validate,
    F: Fn(T) -> Fut,
    Fut: Future<Output = Result<ToolResult, ToolError>>,
{
    let value: T = match serde_json::from_str(input) {
        Ok(value) => value,
        Err(error) => return result::invalid_input(&error.to_string()),
    };
    if let Err(errors) = value.validate() {
        return result::input_validation(&errors, DEFAULT_MAX_RESULT_BYTES);
    }
    match handler(value).await {
        Ok(output) => output,
        Err(error) => result::handler_failure(&error, DEFAULT_MAX_RESULT_BYTES),
    }
}
