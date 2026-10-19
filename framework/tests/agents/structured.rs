//! AGENT-001 and AGENT-003: a tool's structured result follows the
//! session's revision. Revision 2026-07-28 takes any JSON value as
//! `structuredContent` and any JSON Schema as `outputSchema`, so a tool
//! answers and lists as declared. Revisions 2025-06-18 and 2025-11-25 type
//! both as objects, so there a non-object answer is text alone and a
//! non-object output schema is left out of `tools/list`; the tool stays
//! callable.
//!
//! The tools here are routed only by the router these tests build; the
//! shared fixture router leaves them out with its boot warning.

use serde_json::{Value, json};
use suprnova::agents::AgentsConfig;
use suprnova::testing::TestClient;
use suprnova::{HttpResponse, MiddlewareRegistry, Response, Router, handler, tool, tool_output};

use crate::fixture::{self, PostView};

/// The revisions that type `structuredContent` and `outputSchema` as objects.
const OLDER_REVISIONS: [&str; 2] = ["2025-11-25", "2025-06-18"];

/// How many visible posts there are.
#[tool_output]
pub struct IdCount {
    /// The count.
    pub count: u64,
}

/// What `ids.kind` answers: a tagged enum, whose schema is not an object at
/// its root.
#[allow(
    dead_code,
    reason = "only its schema is under test; the handler answers JSON"
)]
#[tool_output]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IdsAnswer {
    /// No post is visible.
    Empty,
    /// Some posts are.
    Some {
        /// How many.
        count: u64,
    },
}

#[tool(
    name = "ids.list",
    description = "List the ids of the visible posts.",
    callers = [public],
    annotations(read_only),
)]
#[handler]
pub async fn list_ids() -> Response {
    Ok(HttpResponse::json(json!([1, 2])))
}

#[tool(
    name = "ids.typed",
    description = "List the visible posts.",
    callers = [public],
    output = Vec<PostView>,
)]
#[handler]
pub async fn typed_ids() -> Response {
    Ok(HttpResponse::json(json!([])))
}

#[tool(
    name = "ids.count",
    description = "Count the visible posts.",
    callers = [public],
    output = IdCount,
)]
#[handler]
pub async fn count_ids() -> Response {
    Ok(HttpResponse::json(json!({ "count": 2 })))
}

#[tool(
    name = "ids.kind",
    description = "Say whether any post is visible.",
    callers = [public],
    output = IdsAnswer,
)]
#[handler]
pub async fn kind_ids() -> Response {
    Ok(HttpResponse::json(json!({ "kind": "some", "count": 2 })))
}

fn client() -> TestClient {
    let router: Router = Router::new()
        .post("/ids", list_ids)
        .post("/ids/typed", typed_ids)
        .post("/ids/count", count_ids)
        .post("/ids/kind", kind_ids)
        .into();
    TestClient::new(
        router.agents(AgentsConfig::new()),
        MiddlewareRegistry::new(),
    )
}

/// The listed definition of `name` on a request of `version`.
async fn listed(client: &TestClient, version: &str, name: &str) -> Value {
    let list = client.mcp().protocol_version(version).list().await;
    assert_eq!(list.status(), 200, "{:?}", list.message());
    list.result()
        .and_then(|result| result["tools"].as_array().cloned())
        .and_then(|tools| tools.into_iter().find(|tool| tool["name"] == name))
        .unwrap_or_else(|| panic!("{name} is listed on {version}"))
}

#[tokio::test]
async fn agent_003_a_json_list_answer_is_structured_content_on_2026_07_28() {
    let _container = fixture::setup();
    let result = client().mcp().call("ids.list", json!({})).await;
    assert_eq!(result.status(), 200);
    assert!(!result.is_error(), "{:?}", result.response().message());
    assert_eq!(result.text(), "[1,2]");
    assert_eq!(result.structured(), Some(&json!([1, 2])));
}

#[tokio::test]
async fn agent_003_a_json_list_answer_is_text_alone_on_older_revisions() {
    let _container = fixture::setup();
    let client = client();
    for version in OLDER_REVISIONS {
        let result = client
            .mcp()
            .protocol_version(version)
            .call("ids.list", json!({}))
            .await;
        assert_eq!(result.status(), 200, "{version}");
        let message = result.response().message();
        assert!(!result.is_error(), "{version}: {message:?}");
        assert_eq!(result.text(), "[1,2]", "{version}");
        assert_eq!(result.structured(), None, "{version}: {message:?}");
        // An object answer is still structured there.
        let counted = client
            .mcp()
            .protocol_version(version)
            .call("ids.count", json!({}))
            .await;
        assert_eq!(
            counted.structured(),
            Some(&json!({ "count": 2 })),
            "{version}"
        );
    }
}

#[tokio::test]
async fn agent_001_output_schemas_are_listed_as_each_revision_allows() {
    let _container = fixture::setup();
    let client = client();

    // 2026-07-28 lists every output schema as declared.
    let typed = listed(&client, "2026-07-28", "ids.typed").await;
    assert_eq!(typed["outputSchema"]["type"], "array", "{typed:#}");
    let kind = listed(&client, "2026-07-28", "ids.kind").await;
    assert!(kind["outputSchema"]["oneOf"].is_array(), "{kind:#}");
    let count = listed(&client, "2026-07-28", "ids.count").await;
    assert_eq!(count["outputSchema"]["type"], "object", "{count:#}");

    // The older revisions take only an object output schema, so the others
    // are left out, and the tools stay listed and callable.
    for version in OLDER_REVISIONS {
        let typed = listed(&client, version, "ids.typed").await;
        assert!(typed.get("outputSchema").is_none(), "{version}: {typed:#}");
        let kind = listed(&client, version, "ids.kind").await;
        assert!(kind.get("outputSchema").is_none(), "{version}: {kind:#}");
        let count = listed(&client, version, "ids.count").await;
        assert_eq!(
            count["outputSchema"]["type"], "object",
            "{version}: {count:#}"
        );
        let called = client
            .mcp()
            .protocol_version(version)
            .call("ids.kind", json!({}))
            .await;
        assert!(!called.is_error(), "{version}: {}", called.text());
    }
}
