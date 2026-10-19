//! AGENT-016 on a target without a browser: the crate compiles, a
//! registration answers `Unsupported::Target`, and a tool's name,
//! description, title and input are still checked, so a mistake shows in a
//! native test run before it reaches a browser.

#![cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use suprnova_webmcp::{
    PageTool, ServerTools, ToolError, ToolHandle, ToolResult, Unsupported, WebMcpError,
};
use validator::{Validate, ValidationErrors};

/// The input of a typed page tool.
#[derive(Deserialize, JsonSchema, Validate)]
struct AddTodo {
    /// What to do.
    #[validate(length(min = 1, max = 200))]
    title: String,
}

/// An input whose schema is a number, not an object.
#[derive(Deserialize, JsonSchema)]
#[serde(transparent)]
struct Count(u32);

impl Validate for Count {
    fn validate(&self) -> Result<(), ValidationErrors> {
        Ok(())
    }
}

fn register(name: &'static str) -> Result<ToolHandle, WebMcpError> {
    PageTool::<AddTodo>::new(name, "Add an item to the visible todo list.")
        .register(|input: AddTodo| async move { Ok(ToolResult::text(input.title)) })
}

#[test]
fn agent_016_native_registration_reports_unsupported() {
    assert!(matches!(
        register("todos.add"),
        Err(WebMcpError::Unsupported(Unsupported::Target))
    ));
    assert!(matches!(
        ServerTools::from_document(),
        Err(WebMcpError::Unsupported(Unsupported::Target))
    ));
}

#[test]
fn agent_016_tool_names_follow_the_agent_001_grammar() {
    let longest: &'static str = "n".repeat(128).leak();
    for name in ["a", "posts.create", "Posts_v2-draft.0", longest] {
        assert!(
            matches!(
                register(name),
                Err(WebMcpError::Unsupported(Unsupported::Target))
            ),
            "`{name}` follows the grammar"
        );
    }
    let too_long: &'static str = "n".repeat(129).leak();
    for name in [
        "",
        too_long,
        "posts create",
        "posts/create",
        "posts:create",
        "caf\u{e9}",
        "tab\t",
    ] {
        assert!(
            matches!(register(name), Err(WebMcpError::InvalidName { name: given }) if given == name),
            "`{name}` breaks the grammar"
        );
    }
}

#[test]
fn agent_016_registration_refuses_a_missing_description_a_blank_title_and_a_non_object_input() {
    let handler = |input: AddTodo| async move { Ok(ToolResult::text(input.title)) };
    assert!(matches!(
        PageTool::<AddTodo>::new("todos.add", " ").register(handler),
        Err(WebMcpError::MissingDescription { name }) if name == "todos.add"
    ));
    assert!(matches!(
        PageTool::<AddTodo>::new("todos.add", "Add an item.")
            .title("")
            .register(handler),
        Err(WebMcpError::BlankTitle { name }) if name == "todos.add"
    ));
    assert!(matches!(
        PageTool::<Count>::new("todos.count", "Count the items.")
            .register(|count: Count| async move { Ok(ToolResult::text(count.0.to_string())) }),
        Err(WebMcpError::InputNotObject { name }) if name == "todos.count"
    ));
}

#[test]
fn agent_016_tool_results_take_the_shape_webmcp_reads() {
    let json_result = ToolResult::json(&json!({ "id": 1 })).expect("JSON serializes");
    assert_eq!(
        serde_json::to_value(&json_result).expect("a result serializes"),
        json!({
            "content": [{ "type": "text", "text": "{\"id\":1}" }],
            "structuredContent": { "id": 1 },
            "isError": false
        })
    );
    assert_eq!(
        serde_json::to_value(ToolResult::empty()).expect("a result serializes"),
        json!({ "content": [], "isError": false })
    );
    assert_eq!(
        serde_json::to_value(ToolResult::error("No room left.")).expect("a result serializes"),
        json!({ "content": [{ "type": "text", "text": "No room left." }], "isError": true })
    );
    let failure: ToolError = WebMcpError::UnknownTool("posts.nope".into()).into();
    assert_eq!(
        failure.message(),
        "the page offers no server tool named `posts.nope`"
    );
}
