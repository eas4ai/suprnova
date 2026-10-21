//! AGENT-001: a tool's input schema is derived from its type, its `serde`
//! attributes and its `validator` attributes, as JSON Schema draft 2020-12.

use serde_json::Value;
use suprnova::agents::__input_schema;

use crate::fixture::CreatePost;

fn schema() -> Value {
    __input_schema::<CreatePost>()
}

#[test]
fn agent_001_schema_reads_serde_and_validator_attributes() {
    let schema = schema();
    assert_eq!(
        schema["$schema"], "https://json-schema.org/draft/2020-12/schema",
        "{schema:#}"
    );
    assert_eq!(schema["type"], "object");
    let properties = &schema["properties"];
    // `#[validate(length(min = 1, max = 200))]`
    assert_eq!(properties["title"]["minLength"], 1, "{schema:#}");
    assert_eq!(properties["title"]["maxLength"], 200, "{schema:#}");
    // `#[serde(rename = "content")]` and `#[validate(length(max = 10000))]`
    assert!(properties.get("body").is_none(), "{schema:#}");
    assert_eq!(properties["content"]["maxLength"], 10000, "{schema:#}");
    // `#[validate(range(min = 1, max = 5))]`
    assert_eq!(properties["stars"]["minimum"], 1, "{schema:#}");
    assert_eq!(properties["stars"]["maximum"], 5, "{schema:#}");
}

#[test]
fn agent_001_schema_marks_required_fields() {
    let schema = schema();
    let required: Vec<&str> = schema["required"]
        .as_array()
        .map(|fields| fields.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    assert_eq!(required, vec!["title"], "{schema:#}");
}

#[test]
fn agent_001_schema_documents_fields_from_doc_comments() {
    let schema = schema();
    assert_eq!(
        schema["properties"]["title"]["description"], "The post's title.",
        "{schema:#}"
    );
}
