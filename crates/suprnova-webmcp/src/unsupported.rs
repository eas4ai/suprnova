//! Every target that is not a browser: the same functions as `browser.rs`,
//! each answering [`Unsupported::Target`], so an application's shared
//! crate builds and runs its tests natively.

use std::rc::Rc;

use serde_json::Value;

use crate::LocalFuture;
use crate::error::{Unsupported, WebMcpError};
use crate::result::ToolResult;
use crate::server::{Limits, ServerTool};

/// A registration, which no target without a browser can make.
pub(crate) enum Registration {}

impl Registration {
    pub(crate) async fn registered(&self, _name: &str) -> Result<(), WebMcpError> {
        match *self {}
    }
}

const TARGET: WebMcpError = WebMcpError::Unsupported(Unsupported::Target);

pub(crate) fn available() -> Result<(), WebMcpError> {
    Err(TARGET)
}

pub(crate) fn register_page_tool(
    _name: &str,
    _descriptor: &Value,
    _execute: Rc<dyn Fn(String) -> LocalFuture<ToolResult>>,
) -> Result<Registration, WebMcpError> {
    Err(TARGET)
}

pub(crate) fn register_server_tool(
    _tool: &ServerTool,
    _limits: Limits,
    _on_write: Rc<dyn Fn() -> LocalFuture<()>>,
) -> Result<Registration, WebMcpError> {
    Err(TARGET)
}

pub(crate) fn page_tools() -> Result<Option<String>, WebMcpError> {
    Err(TARGET)
}

pub(crate) async fn call_tool(
    _tool: &ServerTool,
    _limits: Limits,
    _arguments: &Value,
) -> Result<ToolResult, WebMcpError> {
    Err(TARGET)
}
