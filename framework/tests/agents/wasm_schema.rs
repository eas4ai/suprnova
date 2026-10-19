//! AGENT-016: the WebAssembly crate derives a typed page tool's input
//! schema exactly as the server derives a `#[tool]`'s (AGENT-001), so a type
//! shared between the server and a Rust front end tells an agent the same
//! thing on both sides.

use serde::Deserialize;
use serde_json::Value;
use suprnova::agents::__input_schema;
use suprnova::{JsonSchema, Validate, tool_input};

use crate::fixture::CreatePost;

/// How a task is filed.
#[derive(Deserialize, JsonSchema)]
#[schemars(crate = "suprnova::__schemars")]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    High,
}

/// Where a task is due.
#[tool_input]
#[serde(rename_all = "camelCase")]
pub struct DuePlace {
    /// The room.
    #[validate(length(min = 1, max = 40))]
    pub room_name: String,
}

/// A task, with the serde and `validator` attributes a form request uses.
#[allow(
    dead_code,
    reason = "only the schema is under test; no handler reads the fields"
)]
#[tool_input]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileTask {
    /// What to do.
    #[validate(length(min = 3, max = 120))]
    pub task_title: String,
    /// Who to tell.
    #[validate(email)]
    pub notify_email: Option<String>,
    /// How many hours, a quarter to forty.
    #[validate(range(min = 0.25, max = 40.0))]
    pub estimate_hours: f64,
    /// Labels, at most eight.
    #[validate(length(max = 8))]
    #[serde(default)]
    pub labels: Vec<String>,
    /// How urgent.
    pub priority: Priority,
    /// Where.
    #[validate(nested)]
    pub place: Option<DuePlace>,
    /// Filled by the server, never read from input.
    #[serde(skip_deserializing)]
    pub filed_by: Option<u64>,
}

fn assert_agree(server: Value, page: Value) {
    assert_eq!(
        server["$schema"], "https://json-schema.org/draft/2020-12/schema",
        "{server:#}"
    );
    assert_eq!(page, server, "page:\n{page:#}\nserver:\n{server:#}");
}

#[test]
fn agent_016_typed_tool_schema_matches_the_server_derivation() {
    assert_agree(
        __input_schema::<CreatePost>(),
        suprnova_webmcp::input_schema::<CreatePost>(),
    );
    assert_agree(
        __input_schema::<FileTask>(),
        suprnova_webmcp::input_schema::<FileTask>(),
    );
}
