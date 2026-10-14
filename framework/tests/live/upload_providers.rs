use std::sync::Once;

use suprnova::Router;
use suprnova::crypto::{Crypt, EncryptionKey};
use suprnova::live::testing::{
    LiveTestRuntimeProvider, inspect_runtime, prepare_live_router_for_test,
    register_live_mount_for_test, run_upload_provider_conformance_for_test,
    validate_runtime_provider_omission_for_test,
};
use suprnova::live::{LiveComponent, LiveRegistry, live};
use suprnova::testing::{TestContainer, TestContainerGuard};

#[derive(LiveComponent)]
#[live(
    name = "tests.upload-provider-component",
    view = "live/tests/boot-component.html"
)]
pub struct UploadProviderComponent {
    count: u64,
}

#[live]
impl UploadProviderComponent {}

/// Prepares crypto (once) and this test's own `LiveRegistry` binding.
///
/// The registry is bound in a thread-local test container, not the process
/// container: other tests of this binary bind registries of their own there,
/// and `prepare_live_router_for_test` checks the test container first. A
/// registry this file left in the process container replaced theirs between
/// their binding and their runtime assembly. The caller holds the guard until
/// its runtime is assembled.
fn init_runtime_dependencies() -> TestContainerGuard {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        Crypt::init(EncryptionKey::generate());
    });
    let guard = TestContainer::fake();
    TestContainer::singleton(
        LiveRegistry::builder()
            .register::<UploadProviderComponent>()
            .expect("register upload provider component")
            .build(),
    );
    guard
}

#[test]
fn default_runtime_seals_every_distinct_upload_host_port() {
    let _guard = init_runtime_dependencies();
    let mut router = Router::new();
    register_live_mount_for_test::<UploadProviderComponent>(&mut router, "/uploads", "root")
        .expect("register upload provider mount");
    let runtime = prepare_live_router_for_test(&router).expect("assemble Live runtime");
    let report = inspect_runtime(&runtime);

    assert!(
        report.has_upload_ports(),
        "ledger, cleanup, quarantine, provider modes, authorization, validation, evidence, and finalization must all be explicit"
    );
    assert!(
        report.has_upload_services(),
        "authority, validation, finalization, and cleanup services must be assembled from the explicit ports"
    );

    let upload_providers = [
        LiveTestRuntimeProvider::UploadLedger,
        LiveTestRuntimeProvider::UploadCleanupLedger,
        LiveTestRuntimeProvider::UploadQuarantine,
        LiveTestRuntimeProvider::UploadProvider,
        LiveTestRuntimeProvider::UploadReverseProxy,
        LiveTestRuntimeProvider::UploadReverseProxyProgress,
        LiveTestRuntimeProvider::UploadDirect,
        LiveTestRuntimeProvider::UploadAuthorizationAdapter,
        LiveTestRuntimeProvider::UploadAuthorization,
        LiveTestRuntimeProvider::UploadScanner,
        LiveTestRuntimeProvider::UploadApplicationValidation,
        LiveTestRuntimeProvider::UploadEvidence,
        LiveTestRuntimeProvider::UploadFinalizer,
    ];
    for provider in upload_providers {
        let error = validate_runtime_provider_omission_for_test(&runtime, provider)
            .expect_err("every missing upload provider must fail assembly");
        assert!(
            error.to_string().contains(provider.name()),
            "the boot error must name the missing upload provider without exposing state"
        );
    }
}

#[tokio::test]
async fn framework_quarantine_and_reverse_proxy_provider_pass_the_engine_contract() {
    let report = run_upload_provider_conformance_for_test()
        .await
        .expect("framework provider conformance");

    assert_eq!(report.received_bytes(), 11);
    assert_eq!(report.next_chunk_index(), 1);
    assert!(report.cancel_removed_quarantine());
    assert!(report.direct_provider_fails_closed());
    assert!(report.storage_provider_fails_closed());
    assert!(report.quarantine_permissions_are_private());
    assert!(report.memo_exact_replay());
    assert!(report.memo_mismatch_fails_closed());
    assert!(report.memo_missing_fails_closed());
    assert!(report.memo_exhaustion_fails_closed());
    assert!(report.memo_scope_isolation());
    assert!(report.memo_lifecycle_deletion());
    assert!(report.memo_partial_order_recovered());
    assert!(report.memo_redacted());
}
