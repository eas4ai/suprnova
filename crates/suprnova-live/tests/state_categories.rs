//! Snapshot exposure contracts for every Live component state category.

use std::collections::BTreeMap;

use suprnova_live::canonical::CanonicalValue;
use suprnova_live::snapshot::SnapshotErrorKind;
use suprnova_live::snapshot::state::{
    FieldCategory, FieldSpec, StateCodec, StateExposure, StateSchema,
};

fn schema() -> StateSchema {
    StateSchema::new(
        1,
        vec![
            field("ordinary", FieldCategory::State, true),
            field("public", FieldCategory::Public, true),
            field("model", FieldCategory::Model, true),
            field("locked", FieldCategory::Locked, true),
            field("server", FieldCategory::ServerOnly, true),
            field("session", FieldCategory::Session, true),
            field("computed", FieldCategory::Computed, true),
            field("transient", FieldCategory::Transient, true),
            field("secret", FieldCategory::Secret, true),
        ],
    )
    .expect("category schema")
}

fn field(name: &str, category: FieldCategory, required: bool) -> FieldSpec {
    FieldSpec::new(name, StateCodec::Json, category, required).expect("field spec")
}

fn object(names: &[&str]) -> CanonicalValue {
    CanonicalValue::Object(
        names
            .iter()
            .map(|name| {
                (
                    (*name).to_owned(),
                    CanonicalValue::String("value".to_owned()),
                )
            })
            .collect::<BTreeMap<_, _>>(),
    )
}

#[test]
fn public_seed_requires_only_public_fields_and_rejects_every_other_category() {
    schema()
        .validate(&object(&["public"]), StateExposure::PublicSeed)
        .expect("instance-only and nondehydrated fields are intentionally absent from a seed");

    for forbidden in [
        "ordinary",
        "model",
        "locked",
        "server",
        "session",
        "computed",
        "transient",
        "secret",
    ] {
        let error = schema()
            .validate(&object(&["public", forbidden]), StateExposure::PublicSeed)
            .expect_err("only explicitly public state is seed-eligible");
        assert_eq!(error.kind(), SnapshotErrorKind::ForbiddenStateField);
    }
}

#[test]
fn instanced_state_requires_only_dehydratable_fields() {
    schema()
        .validate(
            &object(&["ordinary", "public", "model", "locked"]),
            StateExposure::Instanced,
        )
        .expect("all instanced categories are present");

    for forbidden in ["server", "session", "computed", "transient", "secret"] {
        let error = schema()
            .validate(
                &object(&["ordinary", "public", "model", "locked", forbidden]),
                StateExposure::Instanced,
            )
            .expect_err("nondehydratable state cannot enter an instance snapshot");
        assert_eq!(error.kind(), SnapshotErrorKind::ForbiddenStateField);
    }
}

#[test]
fn required_checks_are_specific_to_the_selected_exposure() {
    let seed_error = schema()
        .validate(&object(&[]), StateExposure::PublicSeed)
        .expect_err("required public state is missing");
    assert_eq!(seed_error.kind(), SnapshotErrorKind::MissingStateField);

    let instance_error = schema()
        .validate(
            &object(&["ordinary", "public", "model"]),
            StateExposure::Instanced,
        )
        .expect_err("required locked state is missing");
    assert_eq!(instance_error.kind(), SnapshotErrorKind::MissingStateField);
}
