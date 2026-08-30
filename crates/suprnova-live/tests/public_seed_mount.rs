//! Public seed publication keeps semantic SSR while allocating no server instance authority.

mod component_support;

use std::collections::BTreeMap;
use std::sync::Arc;

use bytes::Bytes;
use component_support::{
    ManualClock, key_ring, metadata, schema_set, snapshot_limits, trusted_context,
};
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::identity::{BuildId, Revision, UnixMillis};
use suprnova_live::mount::{
    DocumentMountKey, DocumentMountScope, MountFlags, PublicMountProviders, PublicSeedMountRequest,
    PublicSeedMountService,
};
use suprnova_live::registry::{ComponentDescriptor, ComponentRegistryBuilder};
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
