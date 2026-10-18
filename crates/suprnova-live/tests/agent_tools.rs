//! Live actions as agent tools: the schemas an agent reads, the metadata
//! and root contract the browser registers from, the checker rule on tool
//! components, and the fields a tool result carries (AGENT-012, AGENT-013).

mod component_support;

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

mod root {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use serde_json::{Value, json};
    use suprnova_live::action::{
        ActionArgumentField, ActionArgumentSchema, AuthorizationRequirement, TransactionPolicy,
    };
    use suprnova_live::canonical::CanonicalValue;
    use suprnova_live::identity::{ActionName, ComponentName, ModelField, ViewName};
    use suprnova_live::ledger::{LedgerLimits, MemoryInstanceLedger};
    use suprnova_live::metadata::{
        ActionMetadata, ActionToolMetadata, ComponentMetadata, ContractVersions, FieldMetadata,
    };
    use suprnova_live::mount::{
        DocumentMountKey, DocumentMountScope, MountErrorKind, MountFlags, MountLimits,
        MountProviders, PrivateMountRequest, PrivateMountService,
    };
    use suprnova_live::registry::{ComponentDescriptor, ComponentRegistryBuilder};
    use suprnova_live::snapshot::state::{FieldCategory, StateCodec};
    use suprnova_live::state::ModelCodec;
    use suprnova_live::validation::ValidationSelection;
    use suprnova_live::view::{RenderLimits, ViewRenderer};

    use crate::component_support::{
        FailurePoint, FixtureControl, ManualClock, SequenceGenerator, install, key_ring,
        snapshot_limits, trusted_context_for,
    };

    /// The trace fixture's component, its one action made a tool as asked.
    fn trace_metadata(
        tool: Option<ActionToolMetadata>,
        visible: bool,
    ) -> &'static ComponentMetadata {
        let mut serial = FieldMetadata::new(
            ModelField::parse("serial").expect("field"),
            FieldCategory::State,
            StateCodec::Json,
            true,
        );
        if visible {
            serial = serial.with_agent_visible().expect("visible");
        }
        let mut execute = ActionMetadata::new_with_contract(
            ActionName::parse("execute").expect("action"),
            1,
            ActionArgumentSchema::new(vec![
                ActionArgumentField::new(
                    ModelField::parse("amount").expect("argument"),
                    ModelCodec::U64,
                    true,
                )
                .expect("argument field"),
            ])
            .expect("arguments"),
            AuthorizationRequirement::Current,
            ValidationSelection::ComponentAndArguments,
            TransactionPolicy::None,
        )
        .expect("action metadata");
        if let Some(tool) = tool {
            execute = execute.with_tool(tool).expect("tool");
        }
        Box::leak(Box::new(
            ComponentMetadata::new(
                ComponentName::parse("tests.trace").expect("component"),
                ViewName::parse("tests/trace.html").expect("view"),
                ContractVersions::new(1, 1, 1, 1, 1).expect("versions"),
                vec![serial],
                vec![execute],
            )
            .expect("component metadata"),
        ))
    }

    fn browser_tool() -> ActionToolMetadata {
        ActionToolMetadata::new("trace.execute", "Run the trace.")
            .expect("tool")
            .with_title("Execute")
            .expect("title")
            .with_hints(false, true, true)
            .for_browser(true)
    }

    async fn mount(
        metadata: &'static ComponentMetadata,
        key: &str,
    ) -> Result<String, MountErrorKind> {
        let control = FixtureControl::new_with_metadata(FailurePoint::None, metadata);
        let clock = Arc::new(ManualClock::new(1_000));
        let ledger = Arc::new(MemoryInstanceLedger::new(
            clock.clone(),
            LedgerLimits::new(100, 10_000, 4, 8).expect("ledger limits"),
        ));
        let registry = ComponentRegistryBuilder::new()
            .register(ComponentDescriptor::with_hooks(
                metadata.clone(),
                install(control),
            ))
            .expect("component registers")
            .build();
        let service = PrivateMountService::new(
            MountProviders::new(
                Arc::new(registry),
                ledger,
                clock,
                Arc::new(SequenceGenerator::new(0x20)),
                Arc::new(key_ring()),
            ),
            snapshot_limits(),
            ViewRenderer::new(RenderLimits::standard()).expect("render limits"),
            MountLimits::new(1_000, 3, 64 * 1024, 8).expect("mount limits"),
        )
        .expect("mount service");
        let mut document = DocumentMountScope::new();
        let output = service
            .mount(
                &mut document,
                PrivateMountRequest::new(
                    DocumentMountKey::parse(key).expect("document key"),
                    CanonicalValue::Object(BTreeMap::new()),
                    MountFlags::empty(),
                ),
                &trusted_context_for(metadata, None),
            )
            .await
            .map_err(|error| error.kind())?;
        Ok(String::from_utf8(output.body().to_vec()).expect("UTF-8 HTML"))
    }

    /// The parsed `data-suprnova-live-tools` attribute of the island root.
    fn tools_attribute(html: &str) -> Option<Value> {
        let start = html.find("data-suprnova-live-tools=\"")? + "data-suprnova-live-tools=\"".len();
        let end = start + html[start..].find('"')?;
        let raw = html[start..end]
            .replace("&quot;", "\"")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&#39;", "'")
            .replace("&amp;", "&");
        Some(serde_json::from_str(&raw).expect("the attribute is JSON"))
    }

    #[tokio::test]
    async fn agent_012_root_carries_the_tool_contract_with_the_instance_key() {
        let html = mount(trace_metadata(Some(browser_tool()), true), "main")
            .await
            .expect("mount");
        let tools = tools_attribute(&html).expect("the root carries the tool contract");
        assert_eq!(tools["v"], 1);
        assert_eq!(tools["fields"], json!(["serial"]));
        let tool = &tools["tools"][0];
        assert_eq!(tools["tools"].as_array().map(Vec::len), Some(1));
        assert_eq!(tool["action"], "execute");
        assert_eq!(tool["name"], "trace.execute.main");
        assert_eq!(tool["title"], "Execute");
        assert_eq!(tool["description"], "Run the trace.");
        assert_eq!(
            tool["annotations"],
            json!({
                "readOnlyHint": false,
                "untrustedContentHint": true,
                "consequentialHint": true,
            })
        );
        assert_eq!(tool["inputSchema"]["required"], json!(["amount"]));
        assert_eq!(tool["inputSchema"]["additionalProperties"], false);

        // Another island of the same component names its tool by its own key.
        let html = mount(trace_metadata(Some(browser_tool()), true), "side-2")
            .await
            .expect("mount");
        assert_eq!(
            tools_attribute(&html).expect("tools")["tools"][0]["name"],
            "trace.execute.side-2"
        );
    }

    #[tokio::test]
    async fn agent_012_root_omits_agent_only_tools_and_components_without_tools() {
        let html = mount(trace_metadata(None, false), "main")
            .await
            .expect("mount");
        assert!(!html.contains("data-suprnova-live-tools"), "{html}");

        let headless_only = ActionToolMetadata::new("trace.execute", "Run the trace.")
            .expect("tool")
            .headless(true);
        let html = mount(trace_metadata(Some(headless_only), true), "main")
            .await
            .expect("mount");
        assert!(!html.contains("data-suprnova-live-tools"), "{html}");
    }

    #[tokio::test]
    async fn agent_012_root_refuses_an_instance_key_outside_the_tool_grammar() {
        for key in ["main.left", "a:b", &"k".repeat(64)] {
            assert!(
                mount(trace_metadata(Some(browser_tool()), false), key)
                    .await
                    .is_err(),
                "{key}"
            );
            // The same key is fine for a component the browser offers no
            // tool of.
            assert!(
                mount(trace_metadata(None, false), key).await.is_ok(),
                "{key}"
            );
        }
    }
}
