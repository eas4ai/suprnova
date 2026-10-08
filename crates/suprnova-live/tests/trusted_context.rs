//! Trusted host-context construction and catalog-binding contracts.

use suprnova_live::host::{
    CheckDisposition, CheckFact, CheckKind, HostCapabilities, HostCheckFacts, HostContextErrorKind,
    HostScopeFacts, LiveRequestContextCandidate, LiveRequestContextValidator, MountCatalog,
    MountCatalogBuilder, MountCatalogEntry, MountScopeRequirements, MountSelection, PolicyReason,
    PrincipalFingerprint, ScopeRequirement, SessionFingerprint, TenantFingerprint,
};
use suprnova_live::identity::{
    BuildId, ComponentName, ContentDigest, IslandSlot, RouteIdentity, ScopeFingerprint, UnixMillis,
    ViewName,
};
use suprnova_live::metadata::{ComponentMetadata, ContractVersions};
use suprnova_live::mount::DocumentMountKey;
use suprnova_live::registry::{ComponentDescriptor, ComponentRegistry, ComponentRegistryBuilder};
use suprnova_live::snapshot::state::{SnapshotSchemaSet, StateSchema};
use suprnova_live::snapshot::{ComponentContract, ExpectedSeedV1};
use suprnova_live_test_support::SyntheticLiveRequestContextBuilder;

const NOW: UnixMillis = UnixMillis::new(1_000);
const EXPIRES: UnixMillis = UnixMillis::new(2_000);

fn bytes<const N: usize>(start: u8) -> [u8; N] {
    std::array::from_fn(|index| start.wrapping_add(index as u8))
}

fn digest(start: u8) -> ContentDigest {
    ContentDigest::from_bytes(&bytes::<32>(start)).expect("digest")
}

fn route(start: u8) -> RouteIdentity {
    RouteIdentity::from_bytes(&bytes::<32>(start)).expect("route")
}

fn scope(start: u8) -> ScopeFingerprint {
    ScopeFingerprint::from_bytes(&bytes::<32>(start)).expect("scope")
}

fn principal(start: u8) -> PrincipalFingerprint {
    PrincipalFingerprint::from_bytes(&bytes::<32>(start)).expect("principal")
}

fn tenant(start: u8) -> TenantFingerprint {
    TenantFingerprint::from_bytes(&bytes::<32>(start)).expect("tenant")
}

fn session(start: u8) -> SessionFingerprint {
    SessionFingerprint::from_bytes(&bytes::<32>(start)).expect("session")
}

fn schemas() -> SnapshotSchemaSet {
    SnapshotSchemaSet::new(
        StateSchema::new(1, vec![]).expect("state schema"),
        StateSchema::new(1, vec![]).expect("memo schema"),
        StateSchema::new(1, vec![]).expect("mount schema"),
    )
    .expect("schema set")
}

fn registry() -> ComponentRegistry {
    let metadata = ComponentMetadata::new(
        ComponentName::parse("catalog.search").expect("component"),
        ViewName::parse("live/catalog/search.html").expect("view"),
        ContractVersions::new(1, 1, 1, 1, 2).expect("versions"),
        vec![],
        vec![],
    )
    .expect("metadata");
    ComponentRegistryBuilder::new()
        .register(ComponentDescriptor::new(metadata))
        .expect("registration")
        .build()
}

fn expected_seed(
    registry: &ComponentRegistry,
    route: RouteIdentity,
    slot: IslandSlot,
) -> ExpectedSeedV1 {
    let component = ComponentName::parse("catalog.search").expect("component");
    let descriptor = registry.resolve(&component).expect("descriptor");
    ExpectedSeedV1::new(
        ComponentContract::new(component, descriptor.contract_digest().clone(), 1, 1, 1)
            .expect("component contract"),
        BuildId::parse("build-1").expect("build"),
        route,
        slot,
        schemas(),
    )
}

fn checks_with(
    omitted: Option<CheckKind>,
    replacement: Option<(CheckKind, CheckDisposition)>,
    expiry: UnixMillis,
) -> HostCheckFacts {
    let mut checks = HostCheckFacts::new();
    for kind in CheckKind::ALL {
        if omitted == Some(kind) {
            continue;
        }
        let disposition = replacement
            .filter(|(replacement_kind, _)| *replacement_kind == kind)
            .map_or(CheckDisposition::Passed, |(_, disposition)| disposition);
        checks
            .record(kind, CheckFact::new(disposition, expiry))
            .expect("unique check");
    }
    checks
}

struct Fixture {
    catalog: MountCatalog,
    selection: MountSelection,
    scope: HostScopeFacts,
}

impl Fixture {
    fn new() -> Self {
        Self::with_requirements(MountScopeRequirements::new(
            ScopeRequirement::Required,
            ScopeRequirement::Required,
            ScopeRequirement::Required,
        ))
    }

    fn with_requirements(requirements: MountScopeRequirements) -> Self {
        let registry = registry();
        let route = route(0x20);
        let slot = IslandSlot::parse("primary").expect("slot");
        let expected = expected_seed(&registry, route.clone(), slot.clone());
        let component = ComponentName::parse("catalog.search").expect("component");
        let contract = registry
            .resolve(&component)
            .expect("descriptor")
            .contract_digest()
            .clone();
        let catalog = MountCatalogBuilder::new()
            .register(
                &registry,
                MountCatalogEntry::new(expected, requirements).with_document_key(
                    DocumentMountKey::parse("catalog-primary").expect("document key"),
                ),
            )
            .expect("catalog entry")
            .build();
        Self {
            catalog,
            selection: MountSelection::new(route, slot, component, contract, 2),
            scope: HostScopeFacts::new(
                scope(0x40),
                Some(session(0x50)),
                Some(principal(0x60)),
                Some(tenant(0x70)),
            ),
        }
    }

    fn candidate(
        &self,
        selection: MountSelection,
        scope: HostScopeFacts,
        capabilities: HostCapabilities,
        checks: HostCheckFacts,
        expires_at: UnixMillis,
    ) -> LiveRequestContextCandidate {
        LiveRequestContextCandidate::new(
            selection.route().clone(),
            selection.slot().clone(),
            selection,
            scope,
            checks,
            capabilities,
            expires_at,
        )
    }

    fn valid_candidate(&self) -> LiveRequestContextCandidate {
        self.candidate(
            self.selection.clone(),
            self.scope.clone(),
            HostCapabilities::bound_to(self.scope.clone()),
            checks_with(None, None, EXPIRES),
            EXPIRES,
        )
    }
}

fn anonymous_checks() -> HostCheckFacts {
    let mut checks = HostCheckFacts::new();
    for kind in CheckKind::ALL {
        let disposition = match kind {
            CheckKind::Session => CheckDisposition::NotRequired(PolicyReason::StatelessRequest),
            CheckKind::Principal => CheckDisposition::NotRequired(PolicyReason::AnonymousPrincipal),
            CheckKind::Tenant => CheckDisposition::NotRequired(PolicyReason::TenantlessRoute),
            _ => CheckDisposition::Passed,
        };
        checks
            .record(kind, CheckFact::new(disposition, EXPIRES))
            .expect("unique check");
    }
    checks
}

fn validator() -> LiveRequestContextValidator {
    LiveRequestContextValidator::new(5_000).expect("validation policy")
}

#[test]
fn complete_current_host_facts_create_a_redacted_request_capability() {
    let fixture = Fixture::new();
    let trusted = validator()
        .validate(&fixture.catalog, fixture.valid_candidate(), NOW)
        .expect("trusted context");

    assert_eq!(trusted.scope(), fixture.scope.scope());
    assert_eq!(trusted.expires_at(), EXPIRES);
    assert_eq!(trusted.mount().component().as_str(), "catalog.search");
    assert_eq!(trusted.mount().minimum_protocol(), 2);
    assert_eq!(trusted.mount().protocol(), 2);
    assert_eq!(trusted.mount().document_key().as_str(), "catalog-primary");
    assert_eq!(
        trusted.checks().get(CheckKind::Csrf),
        CheckDisposition::Passed
    );
    assert!(!format!("{trusted:?}").contains("catalog.search"));
    assert_eq!(trusted.for_promotion().scope(), fixture.scope.scope());
}

#[test]
fn browser_mount_selection_must_match_independent_current_route_and_slot_facts() {
    let fixture = Fixture::new();
    let candidate = LiveRequestContextCandidate::new(
        route(0x21),
        fixture.selection.slot().clone(),
        fixture.selection.clone(),
        fixture.scope.clone(),
        checks_with(None, None, EXPIRES),
        HostCapabilities::bound_to(fixture.scope.clone()),
        EXPIRES,
    );
    assert_eq!(
        validator()
            .validate(&fixture.catalog, candidate, NOW)
            .expect_err("browser route cannot replace current route")
            .kind(),
        HostContextErrorKind::RouteMismatch
    );

    let candidate = LiveRequestContextCandidate::new(
        fixture.selection.route().clone(),
        IslandSlot::parse("other").expect("slot"),
        fixture.selection.clone(),
        fixture.scope.clone(),
        checks_with(None, None, EXPIRES),
        HostCapabilities::bound_to(fixture.scope.clone()),
        EXPIRES,
    );
    assert_eq!(
        validator()
            .validate(&fixture.catalog, candidate, NOW)
            .expect_err("browser slot cannot replace current slot")
            .kind(),
        HostContextErrorKind::SlotMismatch
    );
}

#[test]
fn policy_declared_anonymous_tenantless_mounts_remain_explicit_and_complete() {
    let fixture = Fixture::with_requirements(MountScopeRequirements::new(
        ScopeRequirement::Absent,
        ScopeRequirement::Absent,
        ScopeRequirement::Absent,
    ));
    let anonymous = HostScopeFacts::new(scope(0x80), None, None, None);
    let candidate = fixture.candidate(
        fixture.selection.clone(),
        anonymous.clone(),
        HostCapabilities::bound_to(anonymous),
        anonymous_checks(),
        EXPIRES,
    );
    let trusted = validator()
        .validate(&fixture.catalog, candidate, NOW)
        .expect("explicit anonymous policy");

    assert_eq!(
        trusted.checks().get(CheckKind::Principal),
        CheckDisposition::NotRequired(PolicyReason::AnonymousPrincipal)
    );
}

#[test]
fn required_identity_dimensions_cannot_be_omitted_under_not_required_dispositions() {
    let fixture = Fixture::new();
    let cases = [
        (
            HostScopeFacts::new(
                fixture.scope.scope().clone(),
                None,
                fixture.scope.principal().cloned(),
                fixture.scope.tenant().cloned(),
            ),
            CheckKind::Session,
            PolicyReason::StatelessRequest,
            HostContextErrorKind::SessionRequirement,
        ),
        (
            HostScopeFacts::new(
                fixture.scope.scope().clone(),
                fixture.scope.session().cloned(),
                None,
                fixture.scope.tenant().cloned(),
            ),
            CheckKind::Principal,
            PolicyReason::AnonymousPrincipal,
            HostContextErrorKind::PrincipalRequirement,
        ),
        (
            HostScopeFacts::new(
                fixture.scope.scope().clone(),
                fixture.scope.session().cloned(),
                fixture.scope.principal().cloned(),
                None,
            ),
            CheckKind::Tenant,
            PolicyReason::TenantlessRoute,
            HostContextErrorKind::TenantRequirement,
        ),
    ];

    for (scope, kind, reason, expected) in cases {
        let candidate = fixture.candidate(
            fixture.selection.clone(),
            scope.clone(),
            HostCapabilities::bound_to(scope),
            checks_with(
                None,
                Some((kind, CheckDisposition::NotRequired(reason))),
                EXPIRES,
            ),
            EXPIRES,
        );
        assert_eq!(
            validator()
                .validate(&fixture.catalog, candidate, NOW)
                .expect_err("required identity omitted")
                .kind(),
            expected
        );
    }
}

#[test]
fn every_required_check_must_be_present_current_and_policy_coherent() {
    let fixture = Fixture::new();
    for omitted in CheckKind::ALL {
        let candidate = fixture.candidate(
            fixture.selection.clone(),
            fixture.scope.clone(),
            HostCapabilities::bound_to(fixture.scope.clone()),
            checks_with(Some(omitted), None, EXPIRES),
            EXPIRES,
        );
        let error = validator()
            .validate(&fixture.catalog, candidate, NOW)
            .expect_err("missing check");
        assert_eq!(error.kind(), HostContextErrorKind::MissingCheck);
    }

    let incoherent = fixture.candidate(
        fixture.selection.clone(),
        fixture.scope.clone(),
        HostCapabilities::bound_to(fixture.scope.clone()),
        checks_with(
            None,
            Some((
                CheckKind::Origin,
                CheckDisposition::NotRequired(PolicyReason::TenantlessRoute),
            )),
            EXPIRES,
        ),
        EXPIRES,
    );
    assert_eq!(
        validator()
            .validate(&fixture.catalog, incoherent, NOW)
            .expect_err("wrong not-required reason")
            .kind(),
        HostContextErrorKind::InvalidCheckDisposition
    );

    let expired_fact = fixture.candidate(
        fixture.selection.clone(),
        fixture.scope.clone(),
        HostCapabilities::bound_to(fixture.scope.clone()),
        checks_with(None, None, NOW),
        EXPIRES,
    );
    assert_eq!(
        validator()
            .validate(&fixture.catalog, expired_fact, NOW)
            .expect_err("expired check")
            .kind(),
        HostContextErrorKind::CheckExpired
    );

    let expired_context = fixture.candidate(
        fixture.selection.clone(),
        fixture.scope.clone(),
        HostCapabilities::bound_to(fixture.scope.clone()),
        checks_with(None, None, EXPIRES),
        NOW,
    );
    assert_eq!(
        validator()
            .validate(&fixture.catalog, expired_context, NOW)
            .expect_err("expired context")
            .kind(),
        HostContextErrorKind::ContextExpired
    );
}

#[test]
fn trusted_lifetime_is_bounded_and_never_outlives_its_earliest_check() {
    let fixture = Fixture::new();
    let short_check_expiry = UnixMillis::new(1_500);
    let candidate = fixture.candidate(
        fixture.selection.clone(),
        fixture.scope.clone(),
        HostCapabilities::bound_to(fixture.scope.clone()),
        checks_with(None, None, short_check_expiry),
        EXPIRES,
    );
    let trusted = validator()
        .validate(&fixture.catalog, candidate, NOW)
        .expect("bounded context");
    assert_eq!(trusted.expires_at(), short_check_expiry);
    assert!(!trusted.is_current(short_check_expiry));

    let fixture = Fixture::new();
    let excessive = UnixMillis::new(6_001);
    let candidate = fixture.candidate(
        fixture.selection.clone(),
        fixture.scope.clone(),
        HostCapabilities::bound_to(fixture.scope.clone()),
        checks_with(None, None, excessive),
        excessive,
    );
    assert_eq!(
        validator()
            .validate(&fixture.catalog, candidate, NOW)
            .expect_err("context exceeds configured lifetime")
            .kind(),
        HostContextErrorKind::ContextLifetimeExceeded
    );
}

#[test]
fn catalog_resolution_binds_route_slot_component_contract_and_protocol() {
    let fixture = Fixture::new();
    let cases = [
        (
            MountSelection::new(
                route(0x21),
                fixture.selection.slot().clone(),
                fixture.selection.component().clone(),
                fixture.selection.contract_digest().clone(),
                2,
            ),
            HostContextErrorKind::RouteMismatch,
        ),
        (
            MountSelection::new(
                fixture.selection.route().clone(),
                IslandSlot::parse("other").expect("slot"),
                fixture.selection.component().clone(),
                fixture.selection.contract_digest().clone(),
                2,
            ),
            HostContextErrorKind::SlotMismatch,
        ),
        (
            MountSelection::new(
                fixture.selection.route().clone(),
                fixture.selection.slot().clone(),
                ComponentName::parse("catalog.other").expect("component"),
                fixture.selection.contract_digest().clone(),
                2,
            ),
            HostContextErrorKind::ComponentMismatch,
        ),
        (
            MountSelection::new(
                fixture.selection.route().clone(),
                fixture.selection.slot().clone(),
                fixture.selection.component().clone(),
                digest(0x91),
                2,
            ),
            HostContextErrorKind::ContractMismatch,
        ),
        (
            MountSelection::new(
                fixture.selection.route().clone(),
                fixture.selection.slot().clone(),
                fixture.selection.component().clone(),
                fixture.selection.contract_digest().clone(),
                1,
            ),
            HostContextErrorKind::ProtocolMismatch,
        ),
    ];

    for (selection, expected) in cases {
        let candidate = fixture.candidate(
            selection,
            fixture.scope.clone(),
            HostCapabilities::bound_to(fixture.scope.clone()),
            checks_with(None, None, EXPIRES),
            EXPIRES,
        );
        assert_eq!(
            validator()
                .validate(&fixture.catalog, candidate, NOW)
                .expect_err("catalog mismatch")
                .kind(),
            expected
        );
    }
}

#[test]
fn capabilities_cannot_cross_scope_principal_tenant_or_session() {
    let fixture = Fixture::new();
    let cases = [
        (
            HostScopeFacts::new(
                scope(0x41),
                fixture.scope.session().cloned(),
                fixture.scope.principal().cloned(),
                fixture.scope.tenant().cloned(),
            ),
            HostContextErrorKind::ScopeMismatch,
        ),
        (
            HostScopeFacts::new(
                fixture.scope.scope().clone(),
                Some(session(0x51)),
                fixture.scope.principal().cloned(),
                fixture.scope.tenant().cloned(),
            ),
            HostContextErrorKind::SessionMismatch,
        ),
        (
            HostScopeFacts::new(
                fixture.scope.scope().clone(),
                fixture.scope.session().cloned(),
                Some(principal(0x61)),
                fixture.scope.tenant().cloned(),
            ),
            HostContextErrorKind::PrincipalMismatch,
        ),
        (
            HostScopeFacts::new(
                fixture.scope.scope().clone(),
                fixture.scope.session().cloned(),
                fixture.scope.principal().cloned(),
                Some(tenant(0x71)),
            ),
            HostContextErrorKind::TenantMismatch,
        ),
    ];

    for (capability_scope, expected) in cases {
        let candidate = fixture.candidate(
            fixture.selection.clone(),
            fixture.scope.clone(),
            HostCapabilities::bound_to(capability_scope),
            checks_with(None, None, EXPIRES),
            EXPIRES,
        );
        assert_eq!(
            validator()
                .validate(&fixture.catalog, candidate, NOW)
                .expect_err("capability binding mismatch")
                .kind(),
            expected
        );
    }
}

#[test]
fn catalog_rejects_a_seed_contract_that_drifted_from_the_registry() {
    let registry = registry();
    let expected = ExpectedSeedV1::new(
        ComponentContract::new(
            ComponentName::parse("catalog.search").expect("component"),
            digest(0xa0),
            1,
            1,
            1,
        )
        .expect("component contract"),
        BuildId::parse("build-1").expect("build"),
        route(0x20),
        IslandSlot::parse("primary").expect("slot"),
        schemas(),
    );
    let error = MountCatalogBuilder::new()
        .register(
            &registry,
            MountCatalogEntry::new(
                expected,
                MountScopeRequirements::new(
                    ScopeRequirement::Required,
                    ScopeRequirement::Required,
                    ScopeRequirement::Required,
                ),
            ),
        )
        .expect_err("catalog drift");
    assert_eq!(error.kind(), HostContextErrorKind::ContractMismatch);
}

#[test]
fn dev_only_synthetic_builder_still_runs_the_complete_production_validator() {
    let fixture = Fixture::new();
    let error = SyntheticLiveRequestContextBuilder::new(
        fixture.catalog,
        fixture.selection,
        fixture.scope,
        NOW,
        EXPIRES,
    )
    .with_check(
        CheckKind::Origin,
        CheckFact::new(
            CheckDisposition::NotRequired(PolicyReason::TenantlessRoute),
            EXPIRES,
        ),
    )
    .build()
    .expect_err("test support cannot bypass production validation");

    assert_eq!(error.kind(), HostContextErrorKind::InvalidCheckDisposition);
}
