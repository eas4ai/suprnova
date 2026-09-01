//! Conditional upload authority and service-admission contract tests.

mod component_support;

use std::sync::Arc;

use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::identity::{ActionName, KeyId, ModelField, UnixMillis};
use suprnova_live::limits::{UploadLimitConfig, UploadLimits};
use suprnova_live::upload::{
    ConditionalUploadCreate, TransferGrant, TransferGrantCodec, TransferGrantRequest,
    TransferGrantScope, TransitionDisposition, UploadAuthorizationDecision, UploadControlKind,
    UploadCreationRequest, UploadErrorKind, UploadFieldPolicy, UploadHandle, UploadIdempotencyKey,
    UploadLedger, UploadMediaType, UploadReacquireRequest, UploadRecord, UploadReplacementPolicy,
    UploadRevision, UploadScanPolicy, UploadService, UploadState, UploadTransition,
    UploadTransitionAdmission, UploadTransitionRequest,
};
use suprnova_live_test_support::{ControlledUploadAuthorization, MemoryUploadLedger};

use component_support::{
    fixture_host_scope, trusted_context, trusted_context_with_upload_authorization,
};

const HANDLE: &str = "018f47c1-2af0-7cc4-a001-000000000001";
const OTHER_HANDLE: &str = "018f8f3a-7b2c-4d5e-8f90-abcdef012345";
const THIRD_HANDLE: &str = "018f8f3a-7b2c-4d5e-8f90-abcdef012346";
const ROOT_SECRET: &[u8] = b"upload-service-root-secret-000000";

fn handle(value: &str) -> UploadHandle {
    UploadHandle::parse(value).expect("fixture handle")
}

fn field() -> ModelField {
    ModelField::parse("serial").expect("fixture upload field")
}

fn idempotency(value: &str) -> UploadIdempotencyKey {
    UploadIdempotencyKey::parse(value).expect("fixture idempotency key")
}

fn codec() -> TransferGrantCodec {
    let key = KeyRecord::new(
        KeyId::parse("upload-service-key").expect("key id"),
        RootKey::new(ROOT_SECRET.to_vec()).expect("root key"),
        UnixMillis::new(0),
        UnixMillis::new(10_000),
        UnixMillis::new(20_000),
    )
    .expect("key record");
    TransferGrantCodec::new(SnapshotKeyRing::new(key, Vec::new()).expect("key ring"))
}

fn limits() -> UploadLimits {
    UploadLimits::new(UploadLimitConfig::reference()).expect("reference limits")
}

fn policy() -> UploadFieldPolicy {
    UploadFieldPolicy::new(
        3,
        1_024,
        UploadReplacementPolicy::PreservePrevious,
        vec![UploadMediaType::Png],
        None,
        UploadScanPolicy::Disabled,
        ActionName::parse("finalize_upload").expect("fixture action"),
    )
    .expect("fixture upload policy")
}

fn scope(
    context: &suprnova_live::host::TrustedLiveRequestContext,
    upload_handle: UploadHandle,
) -> TransferGrantScope {
    TransferGrantScope::new(
        upload_handle,
        context.mount().component().clone(),
        field(),
        fixture_host_scope(),
        1,
    )
}

fn grant(authority: &TransferGrantScope) -> TransferGrant {
    let issued = codec()
        .issue(
            TransferGrantRequest::new(authority.clone(), UnixMillis::new(1_900)),
            UnixMillis::new(1_000),
        )
        .expect("transfer grant");
    TransferGrant::parse(issued.grant().expose_bearer()).expect("grant round trip")
}

fn record(authority: TransferGrantScope, state: UploadState, revision: u64) -> UploadRecord {
    UploadRecord::new(
        authority,
        state,
        UploadRevision::new(revision),
        UnixMillis::new(1_000),
        UnixMillis::new(1_900),
    )
    .expect("upload record")
}

fn transition(
    authority: &TransferGrantScope,
    bearer: TransferGrant,
    revision: u64,
    key: &str,
    operation: UploadTransition,
) -> UploadTransitionAdmission {
    UploadTransitionAdmission::new(
        bearer,
        field(),
        UploadTransitionRequest::new(
            authority.handle().clone(),
            UploadRevision::new(revision),
            idempotency(key),
            operation,
        ),
    )
}

#[tokio::test]
async fn concurrent_completion_accepts_one_committed_revision() {
    let authorization = Arc::new(ControlledUploadAuthorization::new());
    let context = trusted_context_with_upload_authorization(authorization);
    let authority = scope(&context, handle(HANDLE));
    let ledger = Arc::new(MemoryUploadLedger::new(limits()).expect("ledger"));
    ledger
        .seed(record(authority.clone(), UploadState::Transferring, 7))
        .expect("seed record");
    let service = UploadService::new(ledger.clone(), codec(), limits()).expect("service");

    let left = transition(
        &authority,
        grant(&authority),
        7,
        "complete-left",
        UploadTransition::Complete,
    );
    let right = transition(
        &authority,
        grant(&authority),
        7,
        "complete-right",
        UploadTransition::Complete,
    );
    let (left, right) = tokio::join!(
        service.transition(&context, left, UnixMillis::new(1_001)),
        service.transition(&context, right, UnixMillis::new(1_001)),
    );

    assert_eq!(
        [left, right].iter().filter(|result| result.is_ok()).count(),
        1
    );
    let stored = ledger
        .load(authority.handle())
        .await
        .expect("load")
        .expect("record");
    assert_eq!(stored.state(), UploadState::Verifying);
    assert_eq!(stored.revision(), UploadRevision::new(8));
}

#[tokio::test]
async fn exact_duplicate_replays_the_committed_outcome() {
    let authorization = Arc::new(ControlledUploadAuthorization::new());
    let context = trusted_context_with_upload_authorization(authorization);
    let authority = scope(&context, handle(HANDLE));
    let ledger = Arc::new(MemoryUploadLedger::new(limits()).expect("ledger"));
    ledger
        .seed(record(authority.clone(), UploadState::Created, 1))
        .expect("seed record");
    let service = UploadService::new(ledger, codec(), limits()).expect("service");

    let first = service
        .transition(
            &context,
            transition(
                &authority,
                grant(&authority),
                1,
                "queue",
                UploadTransition::Queue,
            ),
            UnixMillis::new(1_001),
        )
        .await
        .expect("first transition");
    let duplicate = service
        .transition(
            &context,
            transition(
                &authority,
                grant(&authority),
                1,
                "queue",
                UploadTransition::Queue,
            ),
            UnixMillis::new(1_002),
        )
        .await
        .expect("duplicate transition");

    assert_eq!(first.disposition(), TransitionDisposition::Applied);
    assert_eq!(
        duplicate.disposition(),
        TransitionDisposition::ExistingOutcome
    );
    assert_eq!(duplicate.revision(), UploadRevision::new(2));
}

#[tokio::test]
async fn failed_ledger_transition_does_not_consume_revision_authority() {
    let authorization = Arc::new(ControlledUploadAuthorization::new());
    let context = trusted_context_with_upload_authorization(authorization);
    let authority = scope(&context, handle(HANDLE));
    let ledger = Arc::new(MemoryUploadLedger::new(limits()).expect("ledger"));
    ledger
        .seed(record(authority.clone(), UploadState::Created, 1))
        .expect("seed record");
    ledger.fail_next_transition();
    let service = UploadService::new(ledger.clone(), codec(), limits()).expect("service");

    let failed = service
        .transition(
            &context,
            transition(
                &authority,
                grant(&authority),
                1,
                "queue",
                UploadTransition::Queue,
            ),
            UnixMillis::new(1_001),
        )
        .await
        .expect_err("injected ledger failure");
    assert_eq!(failed.kind(), UploadErrorKind::LedgerUnavailable);
    assert_eq!(
        ledger
            .load(authority.handle())
            .await
            .expect("load")
            .expect("record")
            .revision(),
        UploadRevision::new(1)
    );

    let retry = service
        .transition(
            &context,
            transition(
                &authority,
                grant(&authority),
                1,
                "queue",
                UploadTransition::Queue,
            ),
            UnixMillis::new(1_002),
        )
        .await
        .expect("retry succeeds");
    assert_eq!(retry.revision(), UploadRevision::new(2));
}

#[tokio::test]
async fn current_authorization_is_rechecked_at_every_control_boundary() {
    let authorization = Arc::new(ControlledUploadAuthorization::new());
    let context = trusted_context_with_upload_authorization(authorization.clone());
    let authority = scope(&context, handle(HANDLE));
    let ledger = Arc::new(MemoryUploadLedger::new(limits()).expect("ledger"));
    ledger
        .seed(record(authority.clone(), UploadState::Created, 1))
        .expect("seed record");
    let service = UploadService::new(ledger, codec(), limits()).expect("service");

    service
        .status(
            &context,
            grant(&authority),
            field(),
            authority.handle().clone(),
            UnixMillis::new(1_001),
        )
        .await
        .expect("authorized status");
    authorization.set_decision(UploadAuthorizationDecision::Deny);
    let denied = service
        .status(
            &context,
            grant(&authority),
            field(),
            authority.handle().clone(),
            UnixMillis::new(1_002),
        )
        .await
        .expect_err("authorization changed");

    assert_eq!(denied.kind(), UploadErrorKind::AuthorizationDenied);
    assert_eq!(authorization.call_count(), 2);
    assert_eq!(
        authorization.last_control(),
        Some(UploadControlKind::Status)
    );
}

#[tokio::test]
async fn reacquisition_reauthorizes_resumable_state_and_issues_a_fresh_bounded_grant() {
    let authorization = Arc::new(ControlledUploadAuthorization::new());
    let context = trusted_context_with_upload_authorization(authorization.clone());
    let authority = scope(&context, handle(HANDLE));
    let ledger = Arc::new(MemoryUploadLedger::new(limits()).expect("ledger"));
    ledger
        .seed(record(authority.clone(), UploadState::Transferring, 7))
        .expect("seed record");
    let service = UploadService::new(ledger, codec(), limits()).expect("service");

    let outcome = service
        .reacquire(
            &context,
            UploadReacquireRequest::new(
                authority.handle().clone(),
                field(),
                UnixMillis::new(1_500),
            ),
            UnixMillis::new(1_001),
        )
        .await
        .expect("reacquire resumable upload");

    assert_eq!(outcome.record().state(), UploadState::Transferring);
    assert_eq!(outcome.record().revision(), UploadRevision::new(7));
    codec()
        .verify(outcome.grant(), &authority, UnixMillis::new(1_002))
        .expect("fresh grant remains bound to exact upload authority");
    assert_eq!(authorization.call_count(), 1);
    assert_eq!(
        authorization.last_control(),
        Some(UploadControlKind::Reacquire)
    );
}

#[tokio::test]
async fn reacquisition_rejects_non_resumable_state_and_expiry_beyond_upload_authority() {
    let authorization = Arc::new(ControlledUploadAuthorization::new());
    let context = trusted_context_with_upload_authorization(authorization);
    let authority = scope(&context, handle(HANDLE));
    let ledger = Arc::new(MemoryUploadLedger::new(limits()).expect("ledger"));
    ledger
        .seed(record(authority.clone(), UploadState::Ready, 7))
        .expect("seed record");
    let service = UploadService::new(ledger.clone(), codec(), limits()).expect("service");

    let terminal = service
        .reacquire(
            &context,
            UploadReacquireRequest::new(
                authority.handle().clone(),
                field(),
                UnixMillis::new(1_500),
            ),
            UnixMillis::new(1_001),
        )
        .await
        .expect_err("ready uploads are not resumable transfers");
    assert_eq!(terminal.kind(), UploadErrorKind::InvalidTransition);

    let resumable_ledger = Arc::new(MemoryUploadLedger::new(limits()).expect("resumable ledger"));
    resumable_ledger
        .seed(record(authority.clone(), UploadState::Transferring, 8))
        .expect("seed resumable record");
    let resumable_service =
        UploadService::new(resumable_ledger, codec(), limits()).expect("resumable service");
    let overlong = resumable_service
        .reacquire(
            &context,
            UploadReacquireRequest::new(
                authority.handle().clone(),
                field(),
                UnixMillis::new(1_901),
            ),
            UnixMillis::new(1_002),
        )
        .await
        .expect_err("grant cannot outlive upload authority");
    assert_eq!(overlong.kind(), UploadErrorKind::GrantExpired);

    let cross_field = resumable_service
        .reacquire(
            &context,
            UploadReacquireRequest::new(
                authority.handle().clone(),
                ModelField::parse("another_upload").expect("different upload field"),
                UnixMillis::new(1_500),
            ),
            UnixMillis::new(1_003),
        )
        .await
        .expect_err("a handle cannot be rebound to another model field");
    assert_eq!(cross_field.kind(), UploadErrorKind::ScopeMismatch);

    let expired = resumable_service
        .reacquire(
            &context,
            UploadReacquireRequest::new(
                authority.handle().clone(),
                field(),
                UnixMillis::new(1_901),
            ),
            UnixMillis::new(1_900),
        )
        .await
        .expect_err("temporary upload authority expired independently");
    assert_eq!(expired.kind(), UploadErrorKind::UploadExpired);
}

#[tokio::test]
async fn grant_scope_failure_precedes_current_authorization_and_ledger_access() {
    let authorization = Arc::new(ControlledUploadAuthorization::new());
    let context = trusted_context_with_upload_authorization(authorization.clone());
    let authority = scope(&context, handle(HANDLE));
    let ledger = Arc::new(MemoryUploadLedger::new(limits()).expect("ledger"));
    ledger
        .seed(record(authority.clone(), UploadState::Created, 1))
        .expect("seed record");
    let service = UploadService::new(ledger.clone(), codec(), limits()).expect("service");

    let error = service
        .status(
            &context,
            grant(&authority),
            ModelField::parse("another_upload").expect("different field"),
            authority.handle().clone(),
            UnixMillis::new(1_001),
        )
        .await
        .expect_err("cross-field grant reuse");

    assert_eq!(error.kind(), UploadErrorKind::ScopeMismatch);
    assert_eq!(authorization.call_count(), 0);
    assert_eq!(
        ledger
            .load(authority.handle())
            .await
            .expect("load")
            .expect("record")
            .revision(),
        UploadRevision::new(1)
    );
}

#[tokio::test]
async fn missing_current_authorization_capability_fails_closed() {
    let context = trusted_context();
    let authority = scope(&context, handle(HANDLE));
    let ledger = Arc::new(MemoryUploadLedger::new(limits()).expect("ledger"));
    ledger
        .seed(record(authority.clone(), UploadState::Created, 1))
        .expect("seed record");
    let service = UploadService::new(ledger, codec(), limits()).expect("service");

    let error = service
        .status(
            &context,
            grant(&authority),
            field(),
            authority.handle().clone(),
            UnixMillis::new(1_001),
        )
        .await
        .expect_err("missing current authorization");

    assert_eq!(error.kind(), UploadErrorKind::AuthorizationUnavailable);
}

#[tokio::test]
async fn creation_rate_is_bounded_and_exact_retries_do_not_consume_capacity() {
    let authorization = Arc::new(ControlledUploadAuthorization::new());
    let context = trusted_context_with_upload_authorization(authorization);
    let mut config = UploadLimitConfig::reference();
    config.max_creations_per_window = 2;
    let bounded = UploadLimits::new(config).expect("bounded limits");
    let ledger = Arc::new(MemoryUploadLedger::new(bounded).expect("ledger"));
    let service = UploadService::new(ledger.clone(), codec(), bounded).expect("service");

    let first = UploadCreationRequest::new(
        handle(HANDLE),
        field(),
        idempotency("create-one"),
        UnixMillis::new(1_900),
        64,
        policy(),
    );
    let replay = first.clone();
    let second = UploadCreationRequest::new(
        handle(OTHER_HANDLE),
        field(),
        idempotency("create-two"),
        UnixMillis::new(1_900),
        64,
        policy(),
    );
    let third = UploadCreationRequest::new(
        handle(THIRD_HANDLE),
        field(),
        idempotency("create-three"),
        UnixMillis::new(1_900),
        64,
        policy(),
    );

    let created = service
        .create(&context, first, UnixMillis::new(1_001))
        .await
        .expect("first create");
    let existing = service
        .create(&context, replay, UnixMillis::new(1_002))
        .await
        .expect("exact retry");
    service
        .create(&context, second, UnixMillis::new(1_003))
        .await
        .expect("second create");
    let rejected = service
        .create(&context, third, UnixMillis::new(1_004))
        .await
        .expect_err("creation rate exceeded");

    assert_eq!(created.disposition(), ConditionalUploadCreate::Created);
    assert_eq!(
        existing.disposition(),
        ConditionalUploadCreate::ExistingOutcome
    );
    assert_eq!(rejected.kind(), UploadErrorKind::CreationRateExceeded);
    assert_eq!(ledger.len(), 2);
}

#[tokio::test]
async fn expired_request_authority_precedes_grant_and_policy_failures() {
    let authorization = Arc::new(ControlledUploadAuthorization::new());
    authorization.set_decision(UploadAuthorizationDecision::Deny);
    let context = trusted_context_with_upload_authorization(authorization.clone());
    let authority = scope(&context, handle(HANDLE));
    let ledger = Arc::new(MemoryUploadLedger::new(limits()).expect("ledger"));
    ledger
        .seed(record(authority.clone(), UploadState::Created, 1))
        .expect("seed record");
    let service = UploadService::new(ledger, codec(), limits()).expect("service");

    let error = service
        .status(
            &context,
            grant(&authority),
            field(),
            authority.handle().clone(),
            UnixMillis::new(2_000),
        )
        .await
        .expect_err("expired trusted request");

    assert_eq!(error.kind(), UploadErrorKind::RequestAuthorityExpired);
    assert_eq!(authorization.call_count(), 0);
}

#[tokio::test]
async fn service_lifecycle_and_transfer_concurrency_use_shared_resource_primitives() {
    let authorization = Arc::new(ControlledUploadAuthorization::new());
    let context = trusted_context_with_upload_authorization(authorization);
    let authority = scope(&context, handle(HANDLE));
    let ledger = Arc::new(MemoryUploadLedger::new(limits()).expect("ledger"));
    ledger
        .seed(record(authority.clone(), UploadState::Created, 1))
        .expect("seed record");
    let service = UploadService::new(ledger, codec(), limits()).expect("service");

    let permits = (0..limits().max_concurrent_transfers())
        .map(|_| {
            service
                .transfer_permits()
                .try_acquire()
                .expect("bounded permit")
        })
        .collect::<Vec<_>>();
    assert!(service.transfer_permits().try_acquire().is_err());
    drop(permits);
    assert_eq!(service.transfer_permits().active(), 0);

    let cancellation = service.cancellation();
    let retirement = service.retire();
    assert!(retirement.canceled);
    assert!(cancellation.is_canceled());
    let error = service
        .status(
            &context,
            grant(&authority),
            field(),
            authority.handle().clone(),
            UnixMillis::new(1_001),
        )
        .await
        .expect_err("retired service");
    assert_eq!(error.kind(), UploadErrorKind::ServiceRetired);
}
