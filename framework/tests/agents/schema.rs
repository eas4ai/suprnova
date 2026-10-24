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

/// Integer formats are Rust names that schema clients do not recognize.
#[allow(dead_code, reason = "only the published schemas are under test")]
#[suprnova::tool_input]
struct IntegerFields {
    /// A byte.
    small: u8,
    /// An unsigned count.
    count: u32,
    /// A large unsigned count.
    large: u64,
    /// A signed count with exclusive schema bounds.
    #[schemars(extend("exclusiveMinimum" = -100, "exclusiveMaximum" = 100))]
    signed: i32,
    /// A large signed count.
    signed_large: i64,
    /// An optional count.
    optional: Option<u64>,
    /// A list of counts.
    values: Vec<u16>,
    /// A nested count.
    nested: NestedInteger,
    /// A count from 1 to 100.
    #[validate(range(min = 1, max = 100))]
    bounded: u32,
    /// An email address.
    #[validate(email)]
    email: String,
    /// A fraction.
    fraction: f64,
}

/// A referenced schema also needs portable integer fields.
#[allow(dead_code, reason = "only the published schemas are under test")]
#[suprnova::tool_schema]
#[derive(serde::Deserialize)]
struct NestedInteger {
    /// A nested unsigned count.
    count: u64,
}

fn assert_integer_formats_absent(schema: &Value) {
    match schema {
        Value::Object(object) => {
            let integer = match object.get("type") {
                Some(Value::String(kind)) => kind == "integer",
                Some(Value::Array(kinds)) => kinds.iter().any(|kind| kind == "integer"),
                _ => false,
            };
            if integer {
                assert!(!object.contains_key("format"), "{schema:#}");
            }
            for subschema in object.values() {
                assert_integer_formats_absent(subschema);
            }
        }
        Value::Array(schemas) => {
            for subschema in schemas {
                assert_integer_formats_absent(subschema);
            }
        }
        _ => {}
    }
}

fn assert_portable_integer_schema(schema: &Value) {
    assert_integer_formats_absent(schema);
    let properties = &schema["properties"];
    for field in ["small", "count", "large"] {
        assert_eq!(properties[field]["type"], "integer", "{schema:#}");
        assert_eq!(properties[field]["minimum"], 0, "{schema:#}");
    }
    for field in ["signed", "signed_large"] {
        assert_eq!(properties[field]["type"], "integer", "{schema:#}");
    }
    assert_eq!(properties["signed"]["exclusiveMinimum"], -100, "{schema:#}");
    assert_eq!(properties["signed"]["exclusiveMaximum"], 100, "{schema:#}");
    assert_eq!(properties["small"]["maximum"], 255, "{schema:#}");
    assert_eq!(
        properties["optional"]["type"],
        serde_json::json!(["integer", "null"])
    );
    assert_eq!(properties["optional"]["minimum"], 0, "{schema:#}");
    assert_eq!(
        properties["values"]["items"]["type"], "integer",
        "{schema:#}"
    );
    assert_eq!(properties["values"]["items"]["minimum"], 0, "{schema:#}");
    assert_eq!(
        properties["values"]["items"]["maximum"], 65535,
        "{schema:#}"
    );
    assert_eq!(
        properties["nested"]["$ref"], "#/$defs/NestedInteger",
        "{schema:#}"
    );
    assert_eq!(
        schema["$defs"]["NestedInteger"]["properties"]["count"]["type"],
        "integer"
    );
    assert_eq!(
        schema["$defs"]["NestedInteger"]["properties"]["count"]["minimum"],
        0
    );
    assert_eq!(properties["bounded"]["minimum"], 1, "{schema:#}");
    assert_eq!(properties["bounded"]["maximum"], 100, "{schema:#}");
    assert_eq!(properties["email"]["type"], "string", "{schema:#}");
    assert_eq!(properties["email"]["format"], "email", "{schema:#}");
    assert_eq!(properties["fraction"]["format"], "double", "{schema:#}");
}

#[test]
fn agent_001_input_integer_schemas_keep_bounds_without_formats() {
    assert_portable_integer_schema(&__input_schema::<IntegerFields>());
}

#[test]
fn agent_001_output_integer_schemas_keep_bounds_without_formats() {
    assert_portable_integer_schema(&suprnova::agents::__output_schema::<IntegerFields>());
    assert_integer_formats_absent(&suprnova::agents::__output_schema::<u64>());
}

#[test]
fn agent_001_browser_integer_schemas_keep_bounds_without_formats() {
    assert_portable_integer_schema(&suprnova_webmcp::input_schema::<IntegerFields>());
}
