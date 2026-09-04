//! Public seed publication keeps semantic SSR while allocating no server instance authority.

mod component_support;

use std::collections::BTreeMap;
use std::sync::Arc;

use bytes::Bytes;
use component_support::{
    FailurePoint, FixtureControl, ManualClock, install, key_ring, metadata, schema_set,
    snapshot_limits, trusted_context, trusted_context_for_with_schemas,
};
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::identity::{BuildId, ComponentName, ModelField, Revision, UnixMillis, ViewName};
use suprnova_live::metadata::{ComponentMetadata, ContractVersions, FieldMetadata};
use suprnova_live::mount::{
    DocumentMountKey, DocumentMountScope, MountFlags, PublicMountProviders, PublicSeedMountRequest,
    PublicSeedMountService,
};
use suprnova_live::registry::{ComponentDescriptor, ComponentRegistryBuilder};
use suprnova_live::snapshot::state::{
    FieldCategory, FieldSpec, SnapshotSchemaSet, StateCodec, StateSchema,
};
use suprnova_live::snapshot::{
    ComponentContract, ExpectedSeedV1, SeedBodyV1, SeedFieldsV1, verify_seed,
};
use suprnova_live::view::{AssetSet, IslandRender, MountSnapshotKind, RenderLimits, ViewRenderer};

#[test]
fn public_seed_mount_uses_the_shared_root_without_instance_or_promotion_authority() {
    let context = trusted_context();
    let component = ComponentContract::new(
        metadata().identity().clone(),
        metadata().contract_digest().clone(),
        1,
        1,
        1,
    )
    .expect("component contract");
    let build_id = BuildId::parse("build-lifecycle-tests").expect("build identity");
    let route = context.mount().route().clone();
    let slot = context.mount().slot().clone();
    let schemas = schema_set();
    let expected = ExpectedSeedV1::new(
        component.clone(),
        build_id.clone(),
        route.clone(),
        slot.clone(),
        schema_set(),
    );
    let keys = Arc::new(key_ring());
    let limits = snapshot_limits();
    let seed = SeedBodyV1::new(
        SeedFieldsV1 {
            component,
            build_id,
            route,
            slot: slot.clone(),
            key_id: keys.active_key_id().clone(),
            issued_at: UnixMillis::new(1_000),
            max_age_ms: 500,
            mount: CanonicalValue::Object(BTreeMap::new()),
            state: CanonicalValue::Object(BTreeMap::new()),
            memo: CanonicalValue::Object(BTreeMap::new()),
            advisory_generations: vec![],
            refresh_on_promote: false,
            extensions: BTreeMap::new(),
        },
        &schemas,
        &limits,
    )
    .expect("public seed state validates");
    let registry = ComponentRegistryBuilder::new()
        .register(ComponentDescriptor::new(metadata().clone()))
        .expect("component registers")
        .build();
    let service = PublicSeedMountService::new(
        PublicMountProviders::new(
            Arc::new(registry),
            Arc::new(ManualClock::new(1_000)),
            keys.clone(),
        ),
        limits.clone(),
        ViewRenderer::new(RenderLimits::standard()).expect("render limits"),
        8_192,
    )
    .expect("public mount service");
    let request = PublicSeedMountRequest::new(
        DocumentMountKey::parse("public-search").expect("document key"),
        seed,
        IslandRender {
            body: Bytes::from_static(b"<p>Public search results</p>"),
            assets: AssetSet::empty(),
            children: vec![],
        },
        MountFlags::empty(),
    );
    let mut document = DocumentMountScope::new();

    let output = service
        .mount(&mut document, request, &context)
        .expect("public seed mount succeeds");

    assert_eq!(output.revision(), Revision::new(0));
    assert_eq!(
        output.metadata().snapshot_kind(),
        MountSnapshotKind::PublicSeed
    );
    let html = std::str::from_utf8(output.body()).expect("mount HTML is UTF-8");
    assert_eq!(html.matches("data-suprnova-live-island=").count(), 1);
    assert!(html.contains("data-suprnova-live-document-key=\"public-search\""));
    assert!(html.contains("data-suprnova-live-snapshot-kind=\"seed\""));
    assert!(html.contains("data-suprnova-live-revision=\"0\""));
    assert!(!html.contains("data-suprnova-live-instance="));
    assert!(!html.contains("promotion-nonce"));
    assert!(!html.contains("promotion_nonce"));
    let verified = verify_seed(
        output.metadata().signed_snapshot(),
        &expected,
        &keys,
        UnixMillis::new(1_000),
        &limits,
    )
    .expect("published seed verifies");
    assert_eq!(verified.body().slot(), &slot);
}

#[tokio::test]
async fn public_seed_mount_runs_the_registered_component_lifecycle_without_instance_authority() {
    let public_metadata = Box::leak(Box::new(
        ComponentMetadata::new(
            ComponentName::parse("tests.trace").expect("component identity"),
            ViewName::parse("tests/trace.html").expect("view identity"),
            ContractVersions::new(1, 1, 1, 1, 1).expect("versions"),
            vec![FieldMetadata::new(
                ModelField::parse("serial").expect("field identity"),
                FieldCategory::Public,
                StateCodec::Json,
                true,
            )],
            vec![],
        )
        .expect("public component metadata"),
    ));
    let public_schemas = SnapshotSchemaSet::new(
        StateSchema::new(
            1,
            vec![
                FieldSpec::new("serial", StateCodec::Json, FieldCategory::Public, true)
                    .expect("public state field"),
            ],
        )
        .expect("public state schema"),
        StateSchema::new(1, vec![]).expect("memo schema"),
        StateSchema::new(1, vec![]).expect("mount schema"),
    )
    .expect("public schemas");
    let control = FixtureControl::new_with_metadata(FailurePoint::None, public_metadata);
    let context = trusted_context_for_with_schemas(public_metadata, None, public_schemas);
    let keys = Arc::new(key_ring());
    let limits = snapshot_limits();
    let registry = ComponentRegistryBuilder::new()
        .register(ComponentDescriptor::with_hooks(
            public_metadata.clone(),
            install(control.clone()),
        ))
        .expect("component registers")
        .build();
    let service = PublicSeedMountService::new(
        PublicMountProviders::new(Arc::new(registry), Arc::new(ManualClock::new(1_000)), keys),
        limits,
        ViewRenderer::new(RenderLimits::standard()).expect("render limits"),
        8_192,
    )
    .expect("public mount service");
    let mut document = DocumentMountScope::new();

    let output = service
        .mount_component(
            &mut document,
            DocumentMountKey::parse("public-lifecycle").expect("document key"),
            CanonicalValue::Object(BTreeMap::new()),
            MountFlags::empty(),
            &context,
        )
        .await
        .expect("registered public component mount succeeds");

    let html = std::str::from_utf8(output.body()).expect("mount HTML is UTF-8");
    assert!(html.contains("<p>1</p>"));
    assert!(!html.contains("data-suprnova-live-instance="));
    assert_eq!(
        control.values(),
        [
            "mount",
            "rendering",
            "render",
            "rendered",
            "dehydrating",
            "dehydrate",
            "memo",
            "teardown",
        ]
    );
}

#[test]
fn a_public_seed_output_reports_its_non_authoritative_expiry() {
    let context = trusted_context();
    let component = ComponentContract::new(
        metadata().identity().clone(),
        metadata().contract_digest().clone(),
        1,
        1,
        1,
    )
    .expect("component contract");
    let build_id = BuildId::parse("build-lifecycle-tests").expect("build identity");
    let route = context.mount().route().clone();
    let slot = context.mount().slot().clone();
    let schemas = schema_set();
    let keys = Arc::new(key_ring());
    let limits = snapshot_limits();
    let seed = SeedBodyV1::new(
        SeedFieldsV1 {
            component,
            build_id,
            route,
            slot: slot.clone(),
            key_id: keys.active_key_id().clone(),
            issued_at: UnixMillis::new(1_000),
            max_age_ms: 500,
            mount: CanonicalValue::Object(BTreeMap::new()),
            state: CanonicalValue::Object(BTreeMap::new()),
            memo: CanonicalValue::Object(BTreeMap::new()),
            advisory_generations: vec![],
            refresh_on_promote: false,
            extensions: BTreeMap::new(),
        },
        &schemas,
        &limits,
    )
    .expect("public seed state validates");
    let registry = ComponentRegistryBuilder::new()
        .register(ComponentDescriptor::new(metadata().clone()))
        .expect("component registers")
        .build();
    let now = 1_000;
    let service = PublicSeedMountService::new(
        PublicMountProviders::new(
            Arc::new(registry),
            Arc::new(ManualClock::new(now)),
            keys.clone(),
        ),
        limits.clone(),
        ViewRenderer::new(RenderLimits::standard()).expect("render limits"),
        8_192,
    )
    .expect("public mount service");
    let request = PublicSeedMountRequest::new(
        DocumentMountKey::parse("public-search-expiry").expect("document key"),
        seed,
        IslandRender {
            body: Bytes::from_static(b"<p>Public search results</p>"),
            assets: AssetSet::empty(),
            children: vec![],
        },
        MountFlags::empty(),
    );
    let mut document = DocumentMountScope::new();

    let output = service
        .mount(&mut document, request, &context)
        .expect("public seed mount succeeds");

    // The engine's non-authoritative expiry is `now` plus the service's own
    // configured `max_seed_age_ms` (10_000, set by `snapshot_limits()` in
    // `component_support`), not the seed's own `max_age_ms` field (500
    // above): it mirrors `mount/output.rs`'s `PrivateMountOutput::expires_at`,
    // which is likewise `now + max_seed_age_ms` from the service's limits.
    assert_eq!(output.expires_at().get(), now + 10_000);
}
