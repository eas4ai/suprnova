//! WebMCP for Rust front ends compiled to WebAssembly (AGENT-016).
//!
//! A Suprnova application's browser pages offer agents two kinds of tools
//! through `document.modelContext`:
//!
//! - **Page tools** the front end defines itself. [`PageTool`] registers
//!   one with a typed input: its JSON Schema is derived from the input type
//!   exactly as the server derives a `#[tool]`'s (AGENT-001), so what an
//!   agent is told matches what the handler accepts. The registration lasts
//!   as long as its [`ToolHandle`]; dropping the handle removes it.
//! - **Server tools** the server lists for the page (AGENT-014), in the
//!   `<script id="suprnova-agent-tools">` element `PageTools::element`
//!   writes into the page's head. [`ServerTools`] reads that element,
//!   registers its tools, and calls them the way the Live runtime and the
//!   kits do: a same-origin request with the user's session cookie, the
//!   page's CSRF token and the `Suprnova-Tool-Call` header, answered as a
//!   [`ToolResult`].
//!
//! ```no_run
//! use schemars::JsonSchema;
//! use serde::Deserialize;
//! use suprnova_webmcp::{PageTool, ServerTools, ToolResult, WebMcpError};
//! use validator::Validate;
//!
//! /// An item to add to the visible todo list.
//! #[derive(Deserialize, JsonSchema, Validate)]
//! struct AddTodo {
//!     /// What to do.
//!     #[validate(length(min = 1, max = 200))]
//!     title: String,
//! }
//!
//! fn boot() -> Result<(), WebMcpError> {
//!     let add = PageTool::<AddTodo>::new("todos.add", "Add an item to the visible todo list.")
//!         .title("Add todo")
//!         .register(|input: AddTodo| async move { Ok(ToolResult::json(&input.title)?) })?;
//!     let server = ServerTools::from_document()?;
//!     let tools = server.register_all(|| async { /* refetch the app's data */ })?;
//!     // Keep `add` and `tools` alive for as long as the tools apply.
//!     # drop((add, tools));
//!     Ok(())
//! }
//! ```
//!
//! An input type derives `serde::Deserialize`, `schemars::JsonSchema`
//! (schemars 1.x) and `validator::Validate` (validator 0.20), the three
//! derives `#[tool_input]` adds on the server. Sharing one input type
//! between the server and the front end keeps the two in step.
//!
//! # Without a browser
//!
//! Every type and function compiles on every target, so an application's
//! shared crate builds natively too. Outside `wasm32-unknown-unknown`,
//! registering a tool, reading the page's server tools and calling one
//! return [`Unsupported::Target`]. Where there is no document, in a worker
//! for example, they return [`Unsupported::NoDocument`]. In a browser
//! without WebMCP, registering returns [`Unsupported::NoModelContext`],
//! while reading and calling server tools still work, since they need only
//! the page.

#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]
#![deny(rustdoc::private_intra_doc_links)]
// Outside the browser the call path (path filling, the CSRF header, the
// answer mapping) still compiles, so its tests run natively, but nothing
// calls it. Dead code is judged on the browser target, where all of it is
// used.
#![cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    allow(
        dead_code,
        reason = "the browser call path compiles on every target and runs only in a browser"
    )
)]

mod csrf;
mod error;
mod path;
mod result;
mod server;
mod tool;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod browser;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use browser as platform;

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
mod unsupported;
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
use unsupported as platform;

#[cfg(test)]
mod tests;

use std::future::Future;
use std::pin::Pin;

pub use error::{Unsupported, WebMcpError};
pub use result::{Content, ToolError, ToolResult};
pub use server::ServerTools;
pub use tool::{PageTool, ToolHandle};

/// The input schema of a typed tool: the JSON Schema (draft 2020-12)
/// `schemars` derives from `T`, its `serde` attributes and its `validator`
/// attributes.
///
/// These are the settings the framework's `#[tool]` uses, so a type shared
/// between the server and the front end gets one schema on both sides.
pub fn input_schema<T: schemars::JsonSchema>() -> serde_json::Value {
    schemars::generate::SchemaSettings::draft2020_12()
        .into_generator()
        .into_root_schema_for::<T>()
        .to_value()
}

/// The bound on a page tool's validation text when no server limit
/// applies: the framework's default `max_result_bytes`.
pub(crate) const DEFAULT_MAX_RESULT_BYTES: usize = 8 * 1024 * 1024;

/// A future the browser's single thread runs, so it need not be `Send`.
pub(crate) type LocalFuture<T> = Pin<Box<dyn Future<Output = T>>>;
