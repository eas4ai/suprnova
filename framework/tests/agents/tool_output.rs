//! AGENT-001: a tool's output type marked `#[tool_output]` publishes its
//! schema as the tool's MCP output schema. The type names no hidden path,
//! and neither does this test: the attribute alone points the derives at
//! the framework's own copies.

use serde_json::Value;
use suprnova::ToolCaller;

use crate::fixture::{self, PostView, TestUser, scope};

/// The draft 2020-12 schema `schemars` derives for `T`, the settings a
/// `#[tool]`'s schemas use.
fn schemars_schema<T: schemars::JsonSchema>() -> Value {
    schemars::generate::SchemaSettings::draft2020_12()
        .into_generator()
        .into_root_schema_for::<T>()
        .to_value()
}

#[tokio::test]
async fn agent_001_a_tool_output_type_publishes_its_output_schema() {
    let _container = fixture::setup();
    // `create_post` is listed to a user holding `posts:write`.
    let writer = ToolCaller::user(TestUser::arc("7")).scopes([scope::POSTS_WRITE]);
    let list = fixture::client().mcp().as_caller(writer).list().await;
    assert_eq!(list.status(), 200);
    let create = list
        .result()
        .and_then(|result| result["tools"].as_array().cloned())
        .and_then(|tools| tools.into_iter().find(|tool| tool["name"] == "create_post"))
        .expect("create_post is listed");
    let published = &create["outputSchema"];
    assert_eq!(*published, schemars_schema::<PostView>(), "{create:#}");
    assert_eq!(published["type"], "object", "{published:#}");
    assert_eq!(
        published["required"],
        serde_json::json!(["id", "title"]),
        "{published:#}"
    );
}
