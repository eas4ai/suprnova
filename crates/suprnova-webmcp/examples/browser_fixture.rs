//! The WebAssembly front end of the `agents_browser` suite (AGENT-016).
//!
//! `suprnova-cli/tests/agents_browser/wasm.rs` builds it for
//! `wasm32-unknown-unknown`, binds it with `wasm-bindgen --target web`,
//! and loads it in the page `framework/examples/agents_wasm_host.rs`
//! serves. On start it:
//!
//! - registers `todos.add`, a typed page tool that adds to a list the page
//!   keeps, and writes the schema the crate derives for its input to
//!   `data-expected-schema` on `<html>`;
//! - registers the page's server tools from its element, counting each
//!   refresh after a write tool in `data-refreshes`;
//! - marks `<html>` `data-fixture="ready"`, and `data-registered` with how
//!   the browser settled the `todos.add` registration.
//!
//! `drop_todo_tool`, exported to the page, drops the `todos.add` handle.
//!
//! Outside `wasm32-unknown-unknown` the file is empty, so a native build of
//! the crate's examples compiles it to nothing.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use std::cell::{Cell, RefCell};

use serde_json::json;
use suprnova_webmcp::{PageTool, ServerTools, ToolHandle, ToolResult, input_schema, tool_input};
use wasm_bindgen::prelude::*;

/// An item to add to the visible todo list.
#[tool_input]
#[serde(rename_all = "camelCase")]
struct AddTodo {
    /// What to do.
    #[validate(length(min = 1, max = 80))]
    todo_title: String,
}

thread_local! {
    static TODO_TOOL: RefCell<Option<ToolHandle>> = const { RefCell::new(None) };
    static SERVER_TOOLS: RefCell<Vec<ToolHandle>> = const { RefCell::new(Vec::new()) };
    static TODOS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static REFRESHES: Cell<u32> = const { Cell::new(0) };
}

/// Set `name` on the page's `<html>` element.
fn mark(name: &str, value: &str) {
    if let Some(root) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.document_element())
    {
        let _ = root.set_attribute(name, value);
    }
}

#[wasm_bindgen(start)]
pub fn start() {
    wasm_bindgen_futures::spawn_local(async {
        if let Err(error) = boot().await {
            mark("data-fixture", &format!("failed: {error}"));
        }
    });
}

/// Register the tools, then mark the page ready. The handle is stored only
/// once its registration settled, so `drop_todo_tool` can never run while
/// another owner holds it.
async fn boot() -> Result<(), String> {
    let schema = serde_json::to_string(&input_schema::<AddTodo>()).map_err(|e| e.to_string())?;
    mark("data-expected-schema", &schema);
    let handle = PageTool::<AddTodo>::new("todos.add", "Add an item to the visible todo list.")
        .title("Add todo")
        .register(|input: AddTodo| async move {
            let count = TODOS.with(|todos| {
                let mut todos = todos.borrow_mut();
                todos.push(input.todo_title.clone());
                todos.len()
            });
            mark("data-todos", &count.to_string());
            Ok(ToolResult::json(
                &json!({ "added": input.todo_title, "count": count }),
            )?)
        })
        .map_err(|e| e.to_string())?;
    let settled = handle.registered().await;
    mark(
        "data-registered",
        &settled.map_or_else(|error| error.to_string(), |()| "ok".to_string()),
    );
    TODO_TOOL.with(|tool| *tool.borrow_mut() = Some(handle));
    let server = ServerTools::from_document().map_err(|e| e.to_string())?;
    let handles = server
        .register_all(|| async {
            let count = REFRESHES.with(|refreshes| {
                refreshes.set(refreshes.get() + 1);
                refreshes.get()
            });
            mark("data-refreshes", &count.to_string());
        })
        .map_err(|e| e.to_string())?;
    SERVER_TOOLS.with(|tools| *tools.borrow_mut() = handles);
    mark("data-fixture", "ready");
    Ok(())
}

/// Drop the `todos.add` handle, which removes the tool from the page.
#[wasm_bindgen]
pub fn drop_todo_tool() {
    TODO_TOOL.with(|tool| drop(tool.borrow_mut().take()));
}
