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
            (
                "note",
                FieldIssueKind::Invalid(BindingIssueKind::InvalidType)
            ),
            ("team", FieldIssueKind::Unknown),
            (
                "title",
                FieldIssueKind::Invalid(BindingIssueKind::InvalidType)
            ),
        ]
    );

    let opaque = ChildParameterSchema::new(
        1,
        vec![ChildParameterField::new(
            field("filter"),
            ModelCodec::Json,
            true,
        )],
    )
    .expect("parameter schema");
    assert_eq!(opaque.json_schema(&[]), None);
    assert!(opaque.json_schema(&["filter"]).is_some());
}

mod metadata {
    use suprnova_live::action::{
        ActionArgumentField, ActionArgumentSchema, AuthorizationRequirement, TransactionPolicy,
    };
    use suprnova_live::identity::{ActionName, ComponentName, ModelField, ViewName};
    use suprnova_live::metadata::{
        ActionMetadata, ActionToolMetadata, ComponentMetadata, ContractVersions, FieldMetadata,
        MetadataErrorKind,
    };
    use suprnova_live::snapshot::state::{FieldCategory, StateCodec};
    use suprnova_live::state::ModelCodec;
    use suprnova_live::validation::ValidationSelection;

    pub(crate) fn action(name: &str, codec: ModelCodec) -> ActionMetadata {
        ActionMetadata::new_with_contract(
            ActionName::parse(name).expect("action"),
            1,
            ActionArgumentSchema::new(vec![
                ActionArgumentField::new(
                    ModelField::parse("amount").expect("argument"),
                    codec,
                    true,
                )
                .expect("argument field"),
            ])
            .expect("arguments"),
            AuthorizationRequirement::Current,
            ValidationSelection::None,
            TransactionPolicy::None,
        )
        .expect("action metadata")
    }

    pub(crate) fn tool(name: &str) -> ActionToolMetadata {
        ActionToolMetadata::new(name, "Add to the counter.")
            .expect("tool metadata")
            .with_title("Increment")
            .expect("title")
            .with_hints(false, false, true)
            .for_browser(true)
    }

    pub(crate) fn field(name: &str, category: FieldCategory) -> FieldMetadata {
        FieldMetadata::new(
            ModelField::parse(name).expect("field"),
            category,
            StateCodec::Json,
            true,
        )
    }

    pub(crate) fn component(
        fields: Vec<FieldMetadata>,
        actions: Vec<ActionMetadata>,
    ) -> ComponentMetadata {
        ComponentMetadata::new(
            ComponentName::parse("tests.counter").expect("component"),
            ViewName::parse("tests/counter.html").expect("view"),
            ContractVersions::new(1, 1, 1, 1, 1).expect("versions"),
            fields,
            actions,
        )
        .expect("component metadata")
    }

    #[test]
    fn agent_012_tool_metadata_changes_the_digest_only_when_present() {
        // The digest of a component that declares no tool and marks no field
        // is the one it had before tools existed, byte for byte, so every
        // deployed snapshot stays valid.
        let plain = component(
            vec![field("count", FieldCategory::State)],
            vec![action("increment", ModelCodec::I64)],
        );
        assert_eq!(
            plain.contract_digest().to_base64url(),
            "_hkz7JpgBFiNlO2qVPF02uedM7h13MYF7XKxInZDLVE"
        );

        let with_tool = component(
            vec![field("count", FieldCategory::State)],
            vec![
                action("increment", ModelCodec::I64)
                    .with_tool(tool("counter.increment"))
                    .expect("tool"),
            ],
        );
        assert_ne!(with_tool.contract_digest(), plain.contract_digest());
        assert_eq!(
            with_tool.actions()[0].tool().map(ActionToolMetadata::name),
            Some("counter.increment")
        );

        let visible = component(
            vec![
                field("count", FieldCategory::State)
                    .with_agent_visible()
                    .expect("visible"),
            ],
            vec![action("increment", ModelCodec::I64)],
        );
        assert_ne!(visible.contract_digest(), plain.contract_digest());
        assert_ne!(visible.contract_digest(), with_tool.contract_digest());
        assert!(visible.fields()[0].agent_visible());

        // Each declared fact is in the digest.
        let renamed = component(
            vec![field("count", FieldCategory::State)],
            vec![
                action("increment", ModelCodec::I64)
                    .with_tool(tool("counter.bump"))
                    .expect("tool"),
            ],
        );
        assert_ne!(renamed.contract_digest(), with_tool.contract_digest());
        let headless = component(
            vec![field("count", FieldCategory::State)],
            vec![
                action("increment", ModelCodec::I64)
                    .with_tool(tool("counter.increment").headless(true))
                    .expect("tool"),
            ],
        );
        assert_ne!(headless.contract_digest(), with_tool.contract_digest());
    }

    #[test]
    fn agent_012_tool_metadata_refuses_what_an_agent_could_not_read() {
        for name in ["", "has space", "colon:name", &"a".repeat(65)] {
            assert_eq!(
                ActionToolMetadata::new(name, "d")
                    .map(|_| ())
                    .unwrap_err()
                    .kind(),
                MetadataErrorKind::InvalidActionMetadata,
                "{name:?}"
            );
        }
        assert!(ActionToolMetadata::new("counter.increment", "  ").is_err());
        assert!(ActionToolMetadata::new(&"a".repeat(64), "d").is_ok());

        // An argument with no schema cannot be part of a tool.
        assert_eq!(
            action("increment", ModelCodec::Json)
                .with_tool(tool("counter.increment"))
                .map(|_| ())
                .unwrap_err()
                .kind(),
            MetadataErrorKind::InvalidActionMetadata
        );

        // Only state the instanced snapshot carries can be shown to an agent.
        for category in [
            FieldCategory::State,
            FieldCategory::Public,
            FieldCategory::Model,
            FieldCategory::Locked,
        ] {
            assert!(field("value", category).with_agent_visible().is_ok());
        }
        for category in [
            FieldCategory::ServerOnly,
            FieldCategory::Session,
            FieldCategory::Computed,
            FieldCategory::Transient,
            FieldCategory::Secret,
        ] {
            assert!(
                field("value", category).with_agent_visible().is_err(),
                "{category:?}"
            );
        }

        // Two actions of one component cannot share a tool name.
        let duplicate = ComponentMetadata::new(
            ComponentName::parse("tests.counter").expect("component"),
            ViewName::parse("tests/counter.html").expect("view"),
            ContractVersions::new(1, 1, 1, 1, 1).expect("versions"),
            vec![],
            vec![
                action("increment", ModelCodec::I64)
                    .with_tool(tool("counter.change"))
                    .expect("tool"),
                action("decrement", ModelCodec::I64)
                    .with_tool(tool("counter.change"))
                    .expect("tool"),
            ],
        );
        assert_eq!(
            duplicate.map(|_| ()).unwrap_err().kind(),
            MetadataErrorKind::InvalidActionMetadata
        );
    }

    #[test]
    fn agent_012_agent_fields_carry_only_marked_fields() {
        use std::collections::BTreeMap;
        use suprnova_live::canonical::CanonicalValue;
        use suprnova_live::metadata::agent_fields;

        let metadata = component(
            vec![
                field("count", FieldCategory::State)
                    .with_agent_visible()
                    .expect("visible"),
                field("label", FieldCategory::Public)
                    .with_agent_visible()
                    .expect("visible"),
                field("owner", FieldCategory::Locked),
                field("draft", FieldCategory::Model)
                    .with_model_binding(
                        ModelCodec::String,
                        suprnova_live::state::BindingTiming::Change,
                    )
                    .expect("model binding"),
            ],
            vec![action("increment", ModelCodec::I64)],
        );
        let state = CanonicalValue::Object(BTreeMap::from([
            ("count".to_owned(), CanonicalValue::String("3".to_owned())),
            (
                "label".to_owned(),
                CanonicalValue::String("Main".to_owned()),
            ),
            (
                "owner".to_owned(),
                CanonicalValue::String("alice".to_owned()),
            ),
            (
                "draft".to_owned(),
                CanonicalValue::String("secret plan".to_owned()),
            ),
            ("unexpected".to_owned(), CanonicalValue::Bool(true)),
        ]));
        assert_eq!(
            agent_fields(&metadata, &state),
            CanonicalValue::Object(BTreeMap::from([
                ("count".to_owned(), CanonicalValue::String("3".to_owned())),
                (
                    "label".to_owned(),
                    CanonicalValue::String("Main".to_owned())
                ),
            ]))
        );
        // A component that marks nothing shows nothing.
        let unmarked = component(
            vec![field("count", FieldCategory::State)],
            vec![action("increment", ModelCodec::I64)],
        );
        assert_eq!(
            agent_fields(&unmarked, &state),
            CanonicalValue::Object(BTreeMap::new())
        );
    }
}
