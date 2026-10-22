//! AGENT-001: a type nested in a tool's input, here an enum, takes
//! `#[tool_schema]` for its JSON Schema and names no hidden path. Its schema
//! is part of the tool's input schema, and the route refuses a value the
//! schema does not allow.
//!
//! The tool is routed only by the router this test builds; the shared
//! fixture router leaves it out with its boot warning.

use serde::Deserialize;
use serde_json::json;
use suprnova::agents::AgentsConfig;
use suprnova::{
    MiddlewareRegistry, Response, Router, handler, json_response, tool, tool_input, tool_schema,
};

use crate::fixture;

/// How to order the notes.
#[tool_schema]
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    /// Newest first.
    Newest,
    /// Oldest first.
    Oldest,
}

/// The input of `sort_notes`.
#[tool_input]
pub struct SortNotes {
    /// The order to list the notes in.
    pub order: SortOrder,
}

#[tool(
    name = "sort_notes",
    description = "List the notes in an order.",
    callers = [public],
    input = SortNotes,
    annotations(read_only),
)]
#[handler]
pub async fn sort_notes(form: SortNotes) -> Response {
    let order = match form.order {
        SortOrder::Newest => "newest",
        SortOrder::Oldest => "oldest",
    };
    json_response!({ "order": order })
}

fn client() -> suprnova::testing::TestClient {
    let router: Router = Router::new().post("/notes/sort", sort_notes).into();
    suprnova::testing::TestClient::new(
        router.agents(AgentsConfig::new()),
        MiddlewareRegistry::new(),
    )
}

#[tokio::test]
async fn agent_001_a_tool_schema_enum_is_part_of_the_input_schema() {
    let _container = fixture::setup();
    let client = client();
    let list = client.mcp().list().await;
    let tool = list
        .result()
        .and_then(|result| result["tools"].as_array().cloned())
        .and_then(|tools| tools.into_iter().find(|tool| tool["name"] == "sort_notes"))
        .expect("sort_notes is listed");
    let schema = &tool["inputSchema"];
    assert_eq!(
        schema["properties"]["order"]["$ref"], "#/$defs/SortOrder",
        "{schema:#}"
    );
    let order = &schema["$defs"]["SortOrder"];
    // Documented variants are described one by one, each a `const`.
    let variants: Vec<&serde_json::Value> = order["oneOf"]
        .as_array()
        .map(|variants| variants.iter().map(|variant| &variant["const"]).collect())
        .unwrap_or_default();
    assert_eq!(variants, [&json!("newest"), &json!("oldest")], "{schema:#}");
    assert_eq!(
        order["description"], "How to order the notes.",
        "{schema:#}"
    );

    let sorted = client
        .mcp()
        .call("sort_notes", json!({ "order": "oldest" }))
        .await;
    assert!(!sorted.is_error(), "{}", sorted.text());
    assert_eq!(sorted.structured(), Some(&json!({ "order": "oldest" })));

    let refused = client
        .mcp()
        .call("sort_notes", json!({ "order": "sideways" }))
        .await;
    assert!(refused.is_error(), "{}", refused.text());
    assert!(refused.text().contains("order"), "{}", refused.text());
}
