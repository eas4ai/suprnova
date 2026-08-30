//! Typed model proposal authorization, decoding, and mutation contracts.

use std::collections::BTreeMap;

use serde::Deserialize;
use suprnova_live::canonical::{CanonicalValue, parse_canonical_value};
use suprnova_live::limits::InputLimits;
use suprnova_live::snapshot::state::FieldCategory;
use suprnova_live::state::{
    ModelBindingSchema, ModelCodec, ModelFieldBinding, ModelPath, ProposalApplication,
    ProposalBatch, ProposalErrorKind, ProposalLimits, ProposedValue, RawModelProposal,
};
use time::{Date, OffsetDateTime};
use uuid::Uuid;

fn value(json: &str) -> CanonicalValue {
    parse_canonical_value(json.as_bytes(), &InputLimits::default()).expect("canonical test value")
}

fn path(value: &str) -> ModelPath {
    ModelPath::parse(value).expect("registered model path")
}

fn binding(path: &str, category: FieldCategory, codec: ModelCodec) -> ModelFieldBinding {
    ModelFieldBinding::new(path, category, codec).expect("registered binding")
}

fn schema() -> ModelBindingSchema {
    ModelBindingSchema::new(vec![
        binding("name", FieldCategory::Model, ModelCodec::String),
        binding("age", FieldCategory::Model, ModelCodec::I64),
        binding("nickname", FieldCategory::Model, ModelCodec::String),
        binding(
            "tags",
            FieldCategory::Model,
            ModelCodec::list(ModelCodec::String),
        ),
        binding(
            "preferences",
            FieldCategory::Model,
            ModelCodec::map(ModelCodec::Boolean),
        ),
        binding("address", FieldCategory::Model, ModelCodec::Json),
        binding(
            "status",
            FieldCategory::Model,
            ModelCodec::enumeration(["draft", "published"]).expect("enum codec"),
        ),
        binding("birthday", FieldCategory::Model, ModelCodec::Date),
        binding("seen_at", FieldCategory::Model, ModelCodec::DateTime),
        binding("account_id", FieldCategory::Model, ModelCodec::Uuid),
        binding("accepted", FieldCategory::Model, ModelCodec::Boolean),
        binding(
            "roles",
            FieldCategory::Model,
            ModelCodec::list(ModelCodec::String),
        ),
        binding("password", FieldCategory::Transient, ModelCodec::String),
        binding("owner_id", FieldCategory::Locked, ModelCodec::U64),
        binding(
            "items[sku-1].quantity",
            FieldCategory::Model,
            ModelCodec::U64,
        ),
        binding("profile", FieldCategory::Model, ModelCodec::Json),
        binding("profile.name", FieldCategory::Model, ModelCodec::String),
    ])
    .expect("binding schema")
}

fn proposal(path: &str, value: CanonicalValue) -> RawModelProposal {
    RawModelProposal::new(path, value)
}

#[derive(Debug, Deserialize, PartialEq)]
struct Address {
    city: String,
    postal_code: String,
}

#[test]
fn proposals_preserve_missing_null_invalid_and_valid_values() {
    let batch = ProposalBatch::prepare(
        &schema(),
        vec![
            proposal("name", value(r#""Ada""#)),
            proposal("nickname", CanonicalValue::Null),
            proposal("age", value(r#""not-an-integer""#)),
        ],
        &ProposalLimits::default(),
    )
    .expect("authorized batch retains field-level conversion issues");

    assert_eq!(
        batch.proposed::<String>(&path("name")),
        ProposedValue::Valid("Ada".to_owned())
    );
    assert_eq!(
        batch.proposed::<String>(&path("nickname")),
        ProposedValue::Null
    );
    assert!(matches!(
        batch.proposed::<i64>(&path("age")),
        ProposedValue::Invalid(_)
    ));
    assert_eq!(
        batch.proposed::<String>(&path("password")),
        ProposedValue::Missing
    );
    assert_eq!(batch.issues().len(), 1);
}

#[test]
fn registered_codecs_cover_nested_collections_enums_dates_uuid_and_controls() {
    let account_id = Uuid::parse_str("018f4f4a-7c2b-7a13-9ca4-63aa1f6590f4").expect("UUID");
    let batch = ProposalBatch::prepare(
        &schema(),
        vec![
            proposal("tags", value(r#"["rust","web"]"#)),
            proposal("preferences", value(r#"{"email":true,"sms":false}"#)),
            proposal(
                "address",
                value(r#"{"city":"Boston","postal_code":"02110"}"#),
            ),
            proposal("status", value(r#""published""#)),
            proposal("birthday", value(r#""1980-06-15""#)),
            proposal("seen_at", value(r#""2026-08-21T14:30:00Z""#)),
            proposal("account_id", CanonicalValue::String(account_id.to_string())),
            proposal("accepted", CanonicalValue::Bool(false)),
            proposal("roles", value(r#"["admin","editor"]"#)),
        ],
        &ProposalLimits::default(),
    )
    .expect("typed proposal batch");

    assert_eq!(
        batch.proposed::<Vec<String>>(&path("tags")),
        ProposedValue::Valid(vec!["rust".to_owned(), "web".to_owned()])
    );
    assert_eq!(
        batch.proposed::<BTreeMap<String, bool>>(&path("preferences")),
        ProposedValue::Valid(BTreeMap::from([
            ("email".to_owned(), true),
            ("sms".to_owned(), false),
        ]))
    );
    assert_eq!(
        batch.proposed::<Address>(&path("address")),
        ProposedValue::Valid(Address {
            city: "Boston".to_owned(),
            postal_code: "02110".to_owned(),
        })
    );
    assert_eq!(
        batch.proposed::<String>(&path("status")),
        ProposedValue::Valid("published".to_owned())
    );
    assert!(matches!(
        batch.proposed::<Date>(&path("birthday")),
        ProposedValue::Valid(_)
    ));
    assert!(matches!(
        batch.proposed::<OffsetDateTime>(&path("seen_at")),
        ProposedValue::Valid(_)
    ));
    assert_eq!(
        batch.proposed::<Uuid>(&path("account_id")),
        ProposedValue::Valid(account_id)
    );
    assert_eq!(
        batch.proposed::<bool>(&path("accepted")),
        ProposedValue::Valid(false)
    );
    assert_eq!(
        batch.proposed::<Vec<String>>(&path("roles")),
        ProposedValue::Valid(vec!["admin".to_owned(), "editor".to_owned()])
    );
}

#[test]
fn integer_number_inputs_outside_the_lossless_browser_range_are_invalid() {
    let batch = ProposalBatch::prepare(
        &schema(),
        vec![proposal("age", value("1e30"))],
        &ProposalLimits::default(),
    )
    .expect("range failure remains a field issue");

    assert!(matches!(
        batch.proposed::<i64>(&path("age")),
        ProposedValue::Invalid(_)
    ));
}

#[test]
fn failed_or_absent_proposals_never_call_a_setter() {
    #[derive(Debug, PartialEq)]
    struct State {
        age: i64,
        nickname: Option<String>,
    }

    let batch = ProposalBatch::prepare(
        &schema(),
        vec![
            proposal("age", value(r#""invalid""#)),
            proposal("nickname", CanonicalValue::Null),
        ],
        &ProposalLimits::default(),
    )
    .expect("invalid typed values remain field issues");
    let mut state = State {
        age: 42,
        nickname: Some("before".to_owned()),
    };

    let age = batch.apply_required(&path("age"), &mut state, |state, value: i64| {
        state.age = value;
    });
    assert!(matches!(age, ProposalApplication::Invalid(_)));
    assert_eq!(state.age, 42);

    let missing = batch.apply_required(&path("name"), &mut state, |_state, _value: String| {
        panic!("missing setter must not run")
    });
    assert_eq!(missing, ProposalApplication::Missing);

    let optional = batch.apply_optional(
        &path("nickname"),
        &mut state,
        |state, value: Option<String>| state.nickname = value,
    );
    assert_eq!(optional, ProposalApplication::Applied);
    assert_eq!(state.nickname, None);
}

#[test]
fn authorization_rejects_paths_before_generated_setters_can_run() {
    let cases = [
        (
            vec![proposal("unknown", value("1"))],
            ProposalErrorKind::UnknownField,
        ),
        (
            vec![proposal("owner_id", value("1"))],
            ProposalErrorKind::ForbiddenField,
        ),
        (
            vec![proposal("items.0.quantity", value("1"))],
            ProposalErrorKind::UnstableCollectionPath,
        ),
        (
            vec![
                proposal("profile", value(r#"{"name":"Ada"}"#)),
                proposal("profile.name", value(r#""Grace""#)),
            ],
            ProposalErrorKind::ConflictingPaths,
        ),
        (
            vec![
                proposal("name", value(r#""Ada""#)),
                proposal("name", value(r#""Grace""#)),
            ],
            ProposalErrorKind::DuplicatePath,
        ),
    ];

    for (proposals, expected) in cases {
        let error = ProposalBatch::prepare(&schema(), proposals, &ProposalLimits::default())
            .expect_err("hostile proposal batch");
        assert_eq!(error.kind(), expected);
    }
}

#[test]
fn proposal_count_path_shape_and_value_size_are_bounded() {
    let tiny = ProposalLimits::new(
        1,
        1,
        InputLimits::new(64, 4, 8, 16).expect("tiny input limits"),
    )
    .expect("proposal limits");
    let too_many = ProposalBatch::prepare(
        &schema(),
        vec![
            proposal("name", value(r#""Ada""#)),
            proposal("age", value(r#""42""#)),
        ],
        &tiny,
    )
    .expect_err("proposal count bound");
    assert_eq!(too_many.kind(), ProposalErrorKind::TooManyProposals);

    let oversized = ProposalBatch::prepare(
        &schema(),
        vec![proposal("name", CanonicalValue::String("x".repeat(128)))],
        &tiny,
    )
    .expect_err("proposal string bound");
    assert_eq!(oversized.kind(), ProposalErrorKind::StringTooLong);

    let oversized_value = ProposalBatch::prepare(
        &schema(),
        vec![proposal(
            "address",
            value(r#"{"a":"1234567890123456","b":"1234567890123456","c":"1234567890123456"}"#),
        )],
        &tiny,
    )
    .expect_err("proposal value byte bound");
    assert_eq!(oversized_value.kind(), ProposalErrorKind::InputTooLarge);

    let issue_limited = ProposalLimits::new(
        2,
        1,
        InputLimits::new(1_024, 8, 64, 128).expect("issue input limits"),
    )
    .expect("issue limits");
    let too_many_issues = ProposalBatch::prepare(
        &schema(),
        vec![
            proposal("age", value(r#""invalid""#)),
            proposal("items[sku-1].quantity", value(r#""invalid""#)),
        ],
        &issue_limited,
    )
    .expect_err("issue count bound");
    assert_eq!(too_many_issues.kind(), ProposalErrorKind::TooManyIssues);

    assert!(ModelPath::parse(&"segment.".repeat(40)).is_err());
}

#[test]
fn manually_constructed_invalid_codec_metadata_is_rejected_before_requests() {
    let error = ModelBindingSchema::new(vec![binding(
        "status",
        FieldCategory::Model,
        ModelCodec::Enumeration(vec![]),
    )])
    .expect_err("empty enum contract");

    assert_eq!(error.kind(), ProposalErrorKind::InvalidSchema);
}
