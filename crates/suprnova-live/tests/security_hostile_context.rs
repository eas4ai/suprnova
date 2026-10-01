//! Cross-boundary hostile host, replay, and redaction conformance matrix.

mod child_parameter_support;
mod component_support;

use serde::Serialize;
use suprnova_live::action::{LiveEffectPayload, OutcomeErrorKind, RegisteredEmission};
use suprnova_live::child::{ChildParameterErrorKind, verify_child_parameters};
use suprnova_live::host::{
    CheckDisposition, CheckFact, CheckKind, HostCapabilities, HostCheckFacts, HostContextErrorKind,
    HostScopeFacts, LiveRequestContextCandidate, LiveRequestContextValidator, MountCatalog,
    MountCatalogBuilder, MountCatalogEntry, MountScopeRequirements, MountSelection,
    PrincipalFingerprint, ScopeRequirement, SessionFingerprint, TenantFingerprint,
};
use suprnova_live::identity::{
    BuildId, ComponentName, IslandSlot, RouteIdentity, ScopeFingerprint, UnixMillis,
};
use suprnova_live::limits::InputLimits;
use suprnova_live::metadata::EffectPayloadMetadata;
use suprnova_live::registry::{ComponentDescriptor, ComponentRegistryBuilder};
use suprnova_live::snapshot::state::{
    FieldCategory, FieldSpec, StateCodec, StateExposure, StateSchema, dehydrate,
};
use suprnova_live::snapshot::{ComponentContract, ExpectedSeedV1, SnapshotErrorKind};

use child_parameter_support::{NOW, issued_child};
use component_support::{bytes, metadata, schema_set, snapshot_limits};

const EXPIRES: UnixMillis = UnixMillis::new(2_000);

struct Fixture {
    catalog: MountCatalog,
    selection: MountSelection,
    scope: HostScopeFacts,
}

impl Fixture {
    fn new() -> Self {
        let descriptor = ComponentDescriptor::new(metadata().clone());
        let contract = ComponentContract::new(
            metadata().identity().clone(),
            descriptor.contract_digest().clone(),
            1,
            1,
            1,
        )
        .expect("component contract");
        let registry = ComponentRegistryBuilder::new()
            .register(descriptor)
            .expect("component registration")
            .build();
        let route = route(0x30);
        let slot = IslandSlot::parse("trace").expect("slot identity");
        let catalog = MountCatalogBuilder::new()
            .register(
                &registry,
                MountCatalogEntry::new(
                    ExpectedSeedV1::new(
                        contract,
                        BuildId::parse("build-lifecycle-tests").expect("build identity"),
                        route.clone(),
                        slot.clone(),
                        schema_set(),
                    ),
                    MountScopeRequirements::new(
                        ScopeRequirement::Required,
                        ScopeRequirement::Required,
                        ScopeRequirement::Required,
                    ),
                ),
            )
            .expect("mount catalog entry")
            .build();
        let scope = HostScopeFacts::new(
            scope(0x40),
            Some(session(0x41)),
            Some(principal(0x42)),
            Some(tenant(0x43)),
        );
        Self {
            catalog,
            selection: MountSelection::new(
                route,
                slot,
                metadata().identity().clone(),
                metadata().contract_digest().clone(),
                1,
            ),
            scope,
        }
    }

    fn candidate(
        &self,
        current_route: RouteIdentity,
        current_slot: IslandSlot,
        selection: MountSelection,
        scope: HostScopeFacts,
        capabilities: HostCapabilities,
        checks: HostCheckFacts,
    ) -> LiveRequestContextCandidate {
        LiveRequestContextCandidate::new(
            current_route,
            current_slot,
            selection,
            scope,
            checks,
            capabilities,
            EXPIRES,
        )
    }

    fn ordinary_candidate(
        &self,
        checks: HostCheckFacts,
        capabilities: HostCapabilities,
    ) -> LiveRequestContextCandidate {
        self.candidate(
            self.selection.route().clone(),
            self.selection.slot().clone(),
            self.selection.clone(),
            self.scope.clone(),
            capabilities,
            checks,
        )
    }
}

fn route(start: u8) -> RouteIdentity {
    RouteIdentity::from_bytes(&bytes::<32>(start)).expect("route identity")
}

fn scope(start: u8) -> ScopeFingerprint {
    ScopeFingerprint::from_bytes(&bytes::<32>(start)).expect("scope identity")
}

fn session(start: u8) -> SessionFingerprint {
    SessionFingerprint::from_bytes(&bytes::<32>(start)).expect("session identity")
}

fn principal(start: u8) -> PrincipalFingerprint {
    PrincipalFingerprint::from_bytes(&bytes::<32>(start)).expect("principal identity")
}

fn tenant(start: u8) -> TenantFingerprint {
    TenantFingerprint::from_bytes(&bytes::<32>(start)).expect("tenant identity")
}

fn validator() -> LiveRequestContextValidator {
    LiveRequestContextValidator::new(300_000).expect("context validator")
}

fn checks(
    omitted: Option<CheckKind>,
    replacement: Option<(CheckKind, CheckDisposition)>,
    expires_at: UnixMillis,
) -> HostCheckFacts {
    let mut facts = HostCheckFacts::new();
    for kind in CheckKind::ALL {
        if omitted == Some(kind) {
            continue;
        }
        let disposition = replacement
            .filter(|(replacement_kind, _)| *replacement_kind == kind)
            .map_or(CheckDisposition::Passed, |(_, disposition)| disposition);
        facts
            .record(kind, CheckFact::new(disposition, expires_at))
            .expect("unique check fact");
    }
    facts
}

fn checks_with_expired(expired_kind: CheckKind) -> HostCheckFacts {
    let mut facts = HostCheckFacts::new();
    for kind in CheckKind::ALL {
        let expires_at = if kind == expired_kind { NOW } else { EXPIRES };
        facts
            .record(kind, CheckFact::new(CheckDisposition::Passed, expires_at))
            .expect("unique check fact");
    }
    facts
}

#[test]
fn every_authenticity_check_fails_closed_when_absent_inconsistent_or_expired() {
    let fixture = Fixture::new();
    for kind in CheckKind::ALL {
        let absent = fixture.ordinary_candidate(
            checks(Some(kind), None, EXPIRES),
            HostCapabilities::bound_to(fixture.scope.clone()),
        );
        assert_eq!(
            validator()
                .validate(&fixture.catalog, absent, NOW)
                .expect_err("absent host check")
                .kind(),
            HostContextErrorKind::MissingCheck,
            "absent {kind:?}"
        );

        let wrong_reason = if kind == CheckKind::Middleware {
            suprnova_live::host::PolicyReason::TrustedInternalOrigin
        } else {
            suprnova_live::host::PolicyReason::NoAdditionalMiddleware
        };
        let inconsistent = fixture.ordinary_candidate(
            checks(
                None,
                Some((kind, CheckDisposition::NotRequired(wrong_reason))),
                EXPIRES,
            ),
            HostCapabilities::bound_to(fixture.scope.clone()),
        );
        assert_eq!(
            validator()
                .validate(&fixture.catalog, inconsistent, NOW)
                .expect_err("inconsistent host check")
                .kind(),
            HostContextErrorKind::InvalidCheckDisposition,
            "inconsistent {kind:?}"
        );

        let expired = fixture.ordinary_candidate(
            checks_with_expired(kind),
            HostCapabilities::bound_to(fixture.scope.clone()),
        );
        assert_eq!(
            validator()
                .validate(&fixture.catalog, expired, NOW)
                .expect_err("expired host check")
                .kind(),
            HostContextErrorKind::CheckExpired,
            "expired {kind:?}"
        );
    }
}

#[test]
fn route_slot_catalog_and_all_capability_bindings_cannot_cross() {
    let fixture = Fixture::new();
    let current_route = fixture.candidate(
        route(0x31),
        fixture.selection.slot().clone(),
        fixture.selection.clone(),
        fixture.scope.clone(),
        HostCapabilities::bound_to(fixture.scope.clone()),
        checks(None, None, EXPIRES),
    );
    assert_eq!(
        validator()
            .validate(&fixture.catalog, current_route, NOW)
            .expect_err("current route mismatch")
            .kind(),
        HostContextErrorKind::RouteMismatch
    );

    let current_slot = fixture.candidate(
        fixture.selection.route().clone(),
        IslandSlot::parse("other").expect("other slot"),
        fixture.selection.clone(),
        fixture.scope.clone(),
        HostCapabilities::bound_to(fixture.scope.clone()),
        checks(None, None, EXPIRES),
    );
    assert_eq!(
        validator()
            .validate(&fixture.catalog, current_slot, NOW)
            .expect_err("current slot mismatch")
            .kind(),
        HostContextErrorKind::SlotMismatch
    );

    let catalog_cases = [
        (
            MountSelection::new(
                fixture.selection.route().clone(),
                fixture.selection.slot().clone(),
                ComponentName::parse("tests.other").expect("other component"),
                fixture.selection.contract_digest().clone(),
                1,
            ),
            HostContextErrorKind::ComponentMismatch,
        ),
        (
            MountSelection::new(
                fixture.selection.route().clone(),
                fixture.selection.slot().clone(),
                fixture.selection.component().clone(),
                suprnova_live::identity::ContentDigest::from_bytes(&bytes::<32>(0x90))
                    .expect("other contract"),
                1,
            ),
            HostContextErrorKind::ContractMismatch,
        ),
    ];
    for (selection, expected) in catalog_cases {
        let candidate = fixture.candidate(
            selection.route().clone(),
            selection.slot().clone(),
            selection,
            fixture.scope.clone(),
            HostCapabilities::bound_to(fixture.scope.clone()),
            checks(None, None, EXPIRES),
        );
        assert_eq!(
            validator()
                .validate(&fixture.catalog, candidate, NOW)
                .expect_err("catalog mismatch")
                .kind(),
            expected
        );
    }

    let capability_cases = [
        (
            HostScopeFacts::new(
                scope(0x44),
                fixture.scope.session().cloned(),
                fixture.scope.principal().cloned(),
                fixture.scope.tenant().cloned(),
            ),
            HostContextErrorKind::ScopeMismatch,
        ),
        (
            HostScopeFacts::new(
                fixture.scope.scope().clone(),
                Some(session(0x45)),
                fixture.scope.principal().cloned(),
                fixture.scope.tenant().cloned(),
            ),
            HostContextErrorKind::SessionMismatch,
        ),
        (
            HostScopeFacts::new(
                fixture.scope.scope().clone(),
                fixture.scope.session().cloned(),
                Some(principal(0x46)),
                fixture.scope.tenant().cloned(),
            ),
            HostContextErrorKind::PrincipalMismatch,
        ),
        (
            HostScopeFacts::new(
                fixture.scope.scope().clone(),
                fixture.scope.session().cloned(),
                fixture.scope.principal().cloned(),
                Some(tenant(0x47)),
            ),
            HostContextErrorKind::TenantMismatch,
        ),
    ];
    for (capability_scope, expected) in capability_cases {
        let candidate = fixture.ordinary_candidate(
            checks(None, None, EXPIRES),
            HostCapabilities::bound_to(capability_scope),
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

#[derive(Serialize)]
struct UntrustedEffect {
    target: String,
}

impl EffectPayloadMetadata for UntrustedEffect {
    const NAME: &'static str = "untrusted.effect";
    const VERSION: u16 = 1;
}

impl LiveEffectPayload for UntrustedEffect {}

#[tokio::test]
async fn replay_secrets_transients_and_unregistered_browser_output_never_cross_authority() {
    let issued = issued_child("sensitive-query").await;
    verify_child_parameters(
        &issued.encoded,
        &issued.expected,
        &issued.keys,
        NOW,
        &issued.limits,
    )
    .expect("first child delivery verifies");
    let replay = issued
        .expected
        .clone()
        .after_applied_parent_revision(issued.parent_revision);
    assert_eq!(
        verify_child_parameters(&issued.encoded, &replay, &issued.keys, NOW, &issued.limits)
            .expect_err("child delivery replay")
            .kind(),
        ChildParameterErrorKind::ParentRevisionMismatch
    );

    #[derive(Serialize)]
    struct UnsafeState {
        public: String,
        secret: String,
        transient: String,
    }
    let state_schema = StateSchema::new(
        1,
        vec![
            FieldSpec::new("public", StateCodec::Json, FieldCategory::Public, true)
                .expect("public field"),
            FieldSpec::new("secret", StateCodec::Json, FieldCategory::Secret, false)
                .expect("secret field"),
            FieldSpec::new(
                "transient",
                StateCodec::Json,
                FieldCategory::Transient,
                false,
            )
            .expect("transient field"),
        ],
    )
    .expect("state schema");
    let unsafe_state = UnsafeState {
        public: "visible".to_owned(),
        secret: "must-not-escape".to_owned(),
        transient: "must-not-persist".to_owned(),
    };
    for exposure in [StateExposure::PublicSeed, StateExposure::Instanced] {
        let error = dehydrate(
            &unsafe_state,
            &state_schema,
            exposure,
            snapshot_limits().input(),
        )
        .expect_err("nondehydrated fields fail closed");
        assert_eq!(error.kind(), SnapshotErrorKind::ForbiddenStateField);
        let diagnostic = format!("{error:?}");
        assert!(!diagnostic.contains("must-not-escape"));
        assert!(!diagnostic.contains("must-not-persist"));
    }

    let descriptor = ComponentDescriptor::new(metadata().clone());
    let effect_error = RegisteredEmission::effect(
        &descriptor,
        &UntrustedEffect {
            target: "browser-secret".to_owned(),
        },
        &InputLimits::default(),
    )
    .expect_err("unregistered effect");
    assert_eq!(effect_error.kind(), OutcomeErrorKind::UnregisteredEmission);
    assert!(!format!("{effect_error:?}").contains("browser-secret"));

    assert!(RouteIdentity::from_bytes(b"https://evil.test/").is_err());
}
