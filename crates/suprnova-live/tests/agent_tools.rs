//! Live actions as agent tools: the schemas an agent reads, the metadata
//! and root contract the browser registers from, the checker rule on tool
//! components, and the fields a tool result carries (AGENT-012, AGENT-013).

use std::collections::BTreeMap;

use serde_json::json;
use suprnova_live::action::{ActionArgumentField, ActionArgumentSchema};
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::component::composition::{ChildParameterField, ChildParameterSchema};
use suprnova_live::identity::ModelField;
use suprnova_live::limits::InputLimits;
use suprnova_live::state::{BindingIssueKind, FieldIssueKind, ModelCodec};

fn field(name: &str) -> ModelField {
    ModelField::parse(name).expect("field identity")
}

fn object(entries: &[(&str, CanonicalValue)]) -> CanonicalValue {
    CanonicalValue::Object(
        entries
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect::<BTreeMap<_, _>>(),
    )
}

fn number(value: f64) -> CanonicalValue {
    CanonicalValue::number(value).expect("finite number")
}

fn text(value: &str) -> CanonicalValue {
    CanonicalValue::String(value.to_owned())
}

#[test]
fn agent_012_codec_schemas_follow_each_codec() {
    let integer = json!({
        "anyOf": [
            {
                "type": "integer",
                "minimum": -9_007_199_254_740_991_i64,
                "maximum": 9_007_199_254_740_991_i64,
            },
            { "type": "string", "pattern": "^(0|-?[1-9][0-9]*)$" },
        ]
    });
    let unsigned = json!({
        "anyOf": [
            { "type": "integer", "minimum": 0, "maximum": 9_007_199_254_740_991_i64 },
            { "type": "string", "pattern": "^(0|[1-9][0-9]*)$" },
        ]
    });
    let uuid = json!({
        "type": "string",
        "format": "uuid",
        "pattern": "^[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}$",
    });
    let cases = [
        (ModelCodec::String, Some(json!({ "type": "string" }))),
        (ModelCodec::Boolean, Some(json!({ "type": "boolean" }))),
        (ModelCodec::I64, Some(integer.clone())),
        (ModelCodec::U64, Some(unsigned)),
        (ModelCodec::F64, Some(json!({ "type": "number" }))),
        (
            ModelCodec::Date,
            Some(json!({ "type": "string", "format": "date" })),
        ),
        (
            ModelCodec::DateTime,
            Some(json!({ "type": "string", "format": "date-time" })),
        ),
        (ModelCodec::Uuid, Some(uuid)),
        (
            ModelCodec::enumeration(["low", "high"]).expect("enumeration"),
            Some(json!({ "type": "string", "enum": ["high", "low"] })),
        ),
        (
            ModelCodec::list(ModelCodec::I64),
            Some(json!({ "type": "array", "items": integer })),
        ),
        (
            ModelCodec::map(ModelCodec::Boolean),
            Some(json!({ "type": "object", "additionalProperties": { "type": "boolean" } })),
        ),
        (ModelCodec::Json, None),
        (ModelCodec::list(ModelCodec::Json), None),
        (ModelCodec::map(ModelCodec::list(ModelCodec::Json)), None),
    ];
    for (codec, expected) in cases {
        assert_eq!(codec.json_schema(), expected, "{codec:?}");
    }

    // The patterns accept exactly what the codecs accept.
    let limits = InputLimits::default();
    for (codec, value, accepted) in [
        (ModelCodec::I64, text("-42"), true),
        (ModelCodec::I64, text("042"), false),
        (ModelCodec::I64, text("-0"), false),
        (ModelCodec::U64, text("-1"), false),
        (ModelCodec::U64, number(7.0), true),
        (
            ModelCodec::Uuid,
            text("6F9619FF-8B86-D011-B42D-00C04FC964FF"),
            true,
        ),
        (
            ModelCodec::Uuid,
            text("6f9619ff8b86d011b42d00c04fc964ff"),
            false,
        ),
    ] {
        let issues = ActionArgumentSchema::new(vec![
            ActionArgumentField::new(field("value"), codec.clone(), true).expect("field"),
        ])
        .expect("schema")
        .field_issues(&object(&[("value", value.clone())]), &limits);
        assert_eq!(issues.is_empty(), accepted, "{codec:?} {value:?}");
    }
}

#[test]
fn agent_012_argument_schema_names_required_and_nullable_fields() {
    let schema = ActionArgumentSchema::new(vec![
        ActionArgumentField::new(field("title"), ModelCodec::String, true).expect("title"),
        ActionArgumentField::new(field("limit"), ModelCodec::U64, false).expect("limit"),
    ])
    .expect("schema");
    let json = schema.json_schema().expect("schemable arguments");
    assert_eq!(
        json["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert_eq!(json["type"], "object");
    assert_eq!(json["additionalProperties"], false);
    assert_eq!(json["required"], json!(["title"]));
    assert_eq!(json["properties"]["title"], json!({ "type": "string" }));
    assert_eq!(
        json["properties"]["limit"]["anyOf"][1],
        json!({ "type": "null" })
    );
    assert_eq!(
        json["properties"]["limit"]["anyOf"][0],
        ModelCodec::U64.json_schema().expect("u64 schema")
    );

    // An action without arguments takes an empty object.
    let empty = ActionArgumentSchema::empty()
        .json_schema()
        .expect("empty schema");
    assert_eq!(empty["properties"], json!({}));
    assert!(empty.get("required").is_none(), "{empty}");

    // One Json argument leaves the whole action without a schema.
    let opaque = ActionArgumentSchema::new(vec![
        ActionArgumentField::new(field("title"), ModelCodec::String, true).expect("title"),
        ActionArgumentField::new(field("payload"), ModelCodec::Json, true).expect("payload"),
    ])
    .expect("schema");
    assert_eq!(opaque.json_schema(), None);
}

#[test]
fn agent_013_argument_field_issues_name_every_field_preparation_refuses() {
    let schema = ActionArgumentSchema::new(vec![
        ActionArgumentField::new(field("title"), ModelCodec::String, true).expect("title"),
        ActionArgumentField::new(field("limit"), ModelCodec::U64, false).expect("limit"),
        ActionArgumentField::new(field("due"), ModelCodec::Date, true).expect("due"),
    ])
    .expect("schema");
    let limits = InputLimits::default();

    assert!(
        schema
            .field_issues(
                &object(&[
                    ("title", text("Hello")),
                    ("limit", CanonicalValue::Null),
                    ("due", text("2026-10-18")),
                ]),
                &limits,
            )
            .is_empty()
    );

    let issues = schema.field_issues(
        &object(&[
            ("title", CanonicalValue::Null),
            ("limit", text("-3")),
            ("extra", text("x")),
        ]),
        &limits,
    );
    let named = issues
        .iter()
        .map(|issue| (issue.field(), issue.kind()))
        .collect::<Vec<_>>();
    assert_eq!(
        named,
        vec![
            ("due", FieldIssueKind::Missing),
            ("extra", FieldIssueKind::Unknown),
            (
                "limit",
                FieldIssueKind::Invalid(BindingIssueKind::InvalidValue)
            ),
            ("title", FieldIssueKind::Null),
        ]
    );

    let issues = schema.field_issues(&text("not an object"), &limits);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].kind(), FieldIssueKind::NotAnObject);
}

#[test]
fn agent_013_parameter_schema_and_issues_exclude_resolver_properties() {
    let schema = ChildParameterSchema::new(
        1,
        vec![
            ChildParameterField::new(field("title"), ModelCodec::String, true),
            ChildParameterField::new(field("team"), ModelCodec::U64, true),
            ChildParameterField::new(field("note"), ModelCodec::String, false),
        ],
    )
    .expect("parameter schema");
    let json = schema.json_schema(&["team"]).expect("schemable parameters");
    assert_eq!(json["type"], "object");
    assert_eq!(json["additionalProperties"], false);
    assert_eq!(json["required"], json!(["title"]));
    assert!(json["properties"].get("team").is_none(), "{json}");
    assert_eq!(json["properties"]["note"], json!({ "type": "string" }));

    let limits = InputLimits::default();
    assert!(
        schema
            .field_issues(&object(&[("title", text("a"))]), &["team"], &limits)
            .is_empty()
    );
    // A resolver property is not the agent's to set: it is unknown to it.
    let issues = schema.field_issues(
        &object(&[
            ("title", number(3.0)),
            ("team", number(9.0)),
            ("note", CanonicalValue::Null),
        ]),
        &["team"],
        &limits,
    );
    let named = issues
        .iter()
        .map(|issue| (issue.field(), issue.kind()))
        .collect::<Vec<_>>();
    assert_eq!(
        named,
        vec![
            ("note", FieldIssueKind::Invalid(BindingIssueKind::InvalidType)),
            ("team", FieldIssueKind::Unknown),
            ("title", FieldIssueKind::Invalid(BindingIssueKind::InvalidType)),
        ]
    );

    let opaque = ChildParameterSchema::new(
        1,
        vec![ChildParameterField::new(field("filter"), ModelCodec::Json, true)],
    )
    .expect("parameter schema");
    assert_eq!(opaque.json_schema(&[]), None);
    assert!(opaque.json_schema(&["filter"]).is_some());
}
