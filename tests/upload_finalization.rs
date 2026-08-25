//! Authorized durable upload finalization, retry, compensation, and reconciliation tests.

mod component_support;

use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use sha2::{Digest, Sha256};
use suprnova_live::action::{ActionDispatchFn, ActionEntry, ActionTable, AuthorizedAction};
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::identity::{ActionName, KeyId, ModelField, UnixMillis};
use suprnova_live::limits::{UploadLimitConfig, UploadLimits};
use suprnova_live::upload::{
    ClientUploadMetadata, DetectedUploadType, DurableUpload, DurableUploadId, FailedFinalize,
    FinalizeDisposition, FinalizeRequest, FinalizeToken, FinalizeUploadRequest, MediaDimensions,
    TransferGrantCodec, TransferGrantScope, UploadChecksum, UploadDimensionLimits, UploadError,
    UploadErrorKind, UploadFieldPolicy, UploadFinalizationService, UploadFinalizer, UploadFuture,
    UploadHandle, UploadIdempotencyKey, UploadInspection, UploadLedger, UploadMediaType,
    UploadRecord, UploadReplacementPolicy, UploadRevision, UploadScanPolicy, UploadService,
    UploadState, UploadValidationStore, ValidatedUpload, ValidationStoreDisposition,
};
use suprnova_live_test_support::{ControlledUploadAuthorization, MemoryUploadLedger};

use component_support::{fixture_host_scope, trusted_context_with_upload_authorization};

const HANDLE: &str = "018f47c1-2af0-7cc4-a001-000000000001";
const ROOT_SECRET: &[u8] = b"upload-finalize-root-secret-00000";

fn limits() -> UploadLimits {
    UploadLimits::new(UploadLimitConfig::reference()).expect("reference limits")
}

fn handle() -> UploadHandle {
    UploadHandle::parse(HANDLE).expect("fixture handle")
}

fn field() -> ModelField {
    ModelField::parse("avatar").expect("field")
}

fn action_name() -> ActionName {
    ActionName::parse("save_avatar").expect("action")
}

fn idempotency() -> UploadIdempotencyKey {
    UploadIdempotencyKey::parse("finalize-avatar").expect("idempotency")
}

fn checksum(bytes: &[u8]) -> UploadChecksum {
    let mut encoded = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(&mut encoded, "{byte:02x}").expect("checksum text");
    }
    UploadChecksum::parse(&encoded).expect("checksum")
}

fn codec() -> TransferGrantCodec {
    let key = KeyRecord::new(
        KeyId::parse("upload-finalize-key").expect("key id"),
        RootKey::new(ROOT_SECRET.to_vec()).expect("root key"),
        UnixMillis::new(0),
        UnixMillis::new(10_000),
        UnixMillis::new(20_000),
    )
    .expect("key record");
    TransferGrantCodec::new(SnapshotKeyRing::new(key, Vec::new()).expect("key ring"))
}

fn policy() -> UploadFieldPolicy {
    UploadFieldPolicy::new(
        1,
        1024,
        UploadReplacementPolicy::RetirePrevious,
        vec![UploadMediaType::Png],
        Some(UploadDimensionLimits::new(512, 512, 262_144).expect("dimensions")),
        UploadScanPolicy::Disabled,
        action_name(),
    )
    .expect("policy")
}

fn unused_dispatcher() -> ActionDispatchFn {
    |_target, _authorized, _arguments| Box::pin(std::future::pending())
}

async fn authorized_action(
    context: &suprnova_live::host::TrustedLiveRequestContext,
) -> AuthorizedAction {
    let table = ActionTable::new(vec![ActionEntry::new(
        suprnova_live::metadata::ActionMetadata::new(action_name(), 1).expect("metadata"),
        unused_dispatcher(),
    )])
    .expect("action table");
    table
        .authorize(
            context.mount().component(),
            context.capabilities(),
            &action_name(),
        )
        .await
        .expect("registered action proof")
}

#[derive(Default)]
struct SeededValidationStore {
    evidence: Mutex<Option<ValidatedUpload>>,
}

impl SeededValidationStore {
    fn seed(&self, evidence: ValidatedUpload) {
        *lock(&self.evidence) = Some(evidence);
    }
}

impl UploadValidationStore for SeededValidationStore {
    fn put<'a>(
        &'a self,
        evidence: ValidatedUpload,
    ) -> UploadFuture<'a, Result<ValidationStoreDisposition, UploadError>> {
        Box::pin(async move {
            let mut stored = lock(&self.evidence);
            match stored.as_ref() {
                Some(existing) if existing == &evidence => {
                    Ok(ValidationStoreDisposition::ExistingOutcome)
                }
                Some(_) => Err(UploadError::new(UploadErrorKind::UploadConflict)),
                None => {
                    *stored = Some(evidence);
                    Ok(ValidationStoreDisposition::Stored)
                }
            }
        })
    }

    fn load<'a>(
        &'a self,
        upload: &'a UploadHandle,
    ) -> UploadFuture<'a, Result<Option<ValidatedUpload>, UploadError>> {
        Box::pin(async move {
            Ok(lock(&self.evidence)
                .as_ref()
                .filter(|evidence| evidence.handle() == upload)
                .cloned())
        })
    }

    fn remove<'a>(&'a self, upload: &'a UploadHandle) -> UploadFuture<'a, Result<(), UploadError>> {
        Box::pin(async move {
            let mut stored = lock(&self.evidence);
            if stored
                .as_ref()
                .is_some_and(|evidence| evidence.handle() == upload)
            {
                *stored = None;
            }
            Ok(())
        })
    }
}

struct ControlledFinalizer {
    ledger: Arc<MemoryUploadLedger>,
    durable: Mutex<Option<DurableUpload>>,
    fail_prepare: AtomicBool,
    fail_commit: AtomicBool,
    fail_compensation: AtomicBool,
    fail_ledger_after_commit: AtomicBool,
    prepare_calls: AtomicUsize,
    commit_calls: AtomicUsize,
    compensation_calls: AtomicUsize,
}

impl ControlledFinalizer {
    fn new(ledger: Arc<MemoryUploadLedger>) -> Self {
        Self {
            ledger,
            durable: Mutex::new(None),
            fail_prepare: AtomicBool::new(false),
            fail_commit: AtomicBool::new(false),
            fail_compensation: AtomicBool::new(false),
            fail_ledger_after_commit: AtomicBool::new(false),
            prepare_calls: AtomicUsize::new(0),
            commit_calls: AtomicUsize::new(0),
            compensation_calls: AtomicUsize::new(0),
        }
    }
}

impl UploadFinalizer for ControlledFinalizer {
    fn prepare<'a>(
        &'a self,
        request: FinalizeRequest<'a>,
    ) -> UploadFuture<'a, Result<suprnova_live::upload::PreparedFinalize, UploadError>> {
        Box::pin(async move {
            self.prepare_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_prepare.load(Ordering::SeqCst) {
                return Err(UploadError::new(UploadErrorKind::ProviderUnavailable));
            }
            Ok(suprnova_live::upload::PreparedFinalize::new(
                &request,
                FinalizeToken::parse("prepared-avatar").expect("token"),
            ))
        })
    }

    fn commit<'a>(
        &'a self,
        prepared: suprnova_live::upload::PreparedFinalize,
    ) -> UploadFuture<'a, Result<DurableUpload, UploadError>> {
        Box::pin(async move {
            self.commit_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_commit.load(Ordering::SeqCst) {
                return Err(UploadError::new(UploadErrorKind::ProviderUnavailable));
            }
            let outcome = DurableUpload::new(
                &prepared,
                DurableUploadId::parse("durable-avatar").expect("durable id"),
            );
            let mut stored = lock(&self.durable);
            match stored.as_ref() {
                Some(existing) if existing == &outcome => {}
                Some(_) => return Err(UploadError::new(UploadErrorKind::UploadConflict)),
                None => *stored = Some(outcome.clone()),
            }
            drop(stored);
            if self.fail_ledger_after_commit.swap(false, Ordering::SeqCst) {
                self.ledger.fail_next_transition();
            }
            Ok(outcome)
        })
    }

    fn compensate<'a>(
        &'a self,
        _failed: FailedFinalize,
    ) -> UploadFuture<'a, Result<(), UploadError>> {
        Box::pin(async move {
            self.compensation_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_compensation.load(Ordering::SeqCst) {
                Err(UploadError::new(UploadErrorKind::ProviderUnavailable))
            } else {
                Ok(())
            }
        })
    }

    fn reconcile<'a>(
        &'a self,
        request: FinalizeRequest<'a>,
    ) -> UploadFuture<'a, Result<Option<DurableUpload>, UploadError>> {
        Box::pin(async move {
            Ok(lock(&self.durable)
                .as_ref()
                .filter(|durable| durable.handle() == request.evidence().handle())
                .cloned())
        })
    }
}

struct Fixture {
    context: suprnova_live::host::TrustedLiveRequestContext,
    authorization: Arc<ControlledUploadAuthorization>,
    ledger: Arc<MemoryUploadLedger>,
    finalizer: Arc<ControlledFinalizer>,
    service: UploadFinalizationService,
    policy: UploadFieldPolicy,
}

fn fixture() -> Fixture {
    let authorization = Arc::new(ControlledUploadAuthorization::new());
    let context = trusted_context_with_upload_authorization(authorization.clone());
    let authority = TransferGrantScope::new(
        handle(),
        context.mount().component().clone(),
        field(),
        fixture_host_scope(),
        1,
    );
    let ledger = Arc::new(MemoryUploadLedger::new(limits()).expect("ledger"));
    ledger
        .seed(
            UploadRecord::new(
                authority.clone(),
                UploadState::Ready,
                UploadRevision::new(8),
                UnixMillis::new(1_000),
                UnixMillis::new(1_900),
            )
            .expect("record"),
        )
        .expect("seed");
    let policy = policy();
    let bytes = b"authoritative-png";
    let inspection = UploadInspection::from_store(
        handle(),
        ClientUploadMetadata::new("avatar.png", Some("image/png")).expect("client"),
        DetectedUploadType::Png,
        Some(UploadMediaType::Png.into()),
        bytes.len() as u64,
        checksum(bytes),
        Some(MediaDimensions::new(320, 240).expect("dimensions")),
        UnixMillis::new(1_001),
    )
    .expect("stored inspection");
    let evidence = ValidatedUpload::from_store(
        authority,
        UploadRevision::new(8),
        policy.contract_digest().clone(),
        inspection,
    )
    .expect("validated evidence");
    let evidence_store = Arc::new(SeededValidationStore::default());
    evidence_store.seed(evidence);
    let authority_service =
        Arc::new(UploadService::new(ledger.clone(), codec(), limits()).expect("authority service"));
    let finalizer = Arc::new(ControlledFinalizer::new(ledger.clone()));
    let service =
        UploadFinalizationService::new(authority_service, evidence_store, finalizer.clone());
    Fixture {
        context,
        authorization,
        ledger,
        finalizer,
        service,
        policy,
    }
}

async fn request(fixture: &Fixture) -> FinalizeUploadRequest {
    request_with_policy(fixture, fixture.policy.clone()).await
}

async fn request_with_policy(
    fixture: &Fixture,
    policy: UploadFieldPolicy,
) -> FinalizeUploadRequest {
    FinalizeUploadRequest::new(
        handle(),
        field(),
        UploadRevision::new(8),
        idempotency(),
        authorized_action(&fixture.context).await,
        policy,
    )
}

#[tokio::test]
async fn ready_content_is_not_durable_until_authorized_finalize() {
    let fixture = fixture();
    fixture
        .authorization
        .set_decision(suprnova_live::upload::UploadAuthorizationDecision::Deny);

    let error = fixture
        .service
        .finalize(
            &fixture.context,
            request(&fixture).await,
            UnixMillis::new(1_002),
        )
        .await
        .expect_err("denied finalization");

    assert_eq!(error.kind(), UploadErrorKind::AuthorizationDenied);
    assert_eq!(fixture.finalizer.prepare_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        fixture
            .ledger
            .load(&handle())
            .await
            .expect("load")
            .expect("record")
            .state(),
        UploadState::Ready
    );
}

#[tokio::test]
async fn successful_finalize_and_exact_retry_produce_one_durable_outcome() {
    let fixture = fixture();
    let first = fixture
        .service
        .finalize(
            &fixture.context,
            request(&fixture).await,
            UnixMillis::new(1_002),
        )
        .await
        .expect("finalize");
    let replay = fixture
        .service
        .finalize(
            &fixture.context,
            request(&fixture).await,
            UnixMillis::new(1_003),
        )
        .await
        .expect("exact retry");

    assert_eq!(first.disposition(), FinalizeDisposition::Finalized);
    assert_eq!(replay.disposition(), FinalizeDisposition::ExistingOutcome);
    assert_eq!(first.durable(), replay.durable());
    assert_eq!(first.revision(), UploadRevision::new(10));
    assert_eq!(fixture.finalizer.prepare_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.finalizer.commit_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn failed_commit_is_compensated_and_same_logical_request_can_retry() {
    let fixture = fixture();
    fixture.finalizer.fail_commit.store(true, Ordering::SeqCst);

    let error = fixture
        .service
        .finalize(
            &fixture.context,
            request(&fixture).await,
            UnixMillis::new(1_002),
        )
        .await
        .expect_err("injected provider failure");
    assert_eq!(error.kind(), UploadErrorKind::ProviderUnavailable);
    assert_eq!(
        fixture.finalizer.compensation_calls.load(Ordering::SeqCst),
        1
    );
    assert_eq!(
        fixture
            .ledger
            .load(&handle())
            .await
            .expect("load")
            .expect("record")
            .state(),
        UploadState::Finalizing
    );

    fixture.finalizer.fail_commit.store(false, Ordering::SeqCst);
    fixture
        .service
        .finalize(
            &fixture.context,
            request(&fixture).await,
            UnixMillis::new(1_003),
        )
        .await
        .expect("retry");
    assert_eq!(fixture.finalizer.prepare_calls.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.finalizer.commit_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn failed_prepare_leaves_reconcilable_claim_and_retry_finishes_once() {
    let fixture = fixture();
    fixture.finalizer.fail_prepare.store(true, Ordering::SeqCst);

    let error = fixture
        .service
        .finalize(
            &fixture.context,
            request(&fixture).await,
            UnixMillis::new(1_002),
        )
        .await
        .expect_err("injected prepare failure");
    assert_eq!(error.kind(), UploadErrorKind::ProviderUnavailable);
    assert_eq!(fixture.finalizer.prepare_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.finalizer.commit_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        fixture
            .ledger
            .load(&handle())
            .await
            .expect("load")
            .expect("record")
            .state(),
        UploadState::Finalizing
    );

    fixture
        .finalizer
        .fail_prepare
        .store(false, Ordering::SeqCst);
    fixture
        .service
        .finalize(
            &fixture.context,
            request(&fixture).await,
            UnixMillis::new(1_003),
        )
        .await
        .expect("retry");
    assert_eq!(fixture.finalizer.prepare_calls.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.finalizer.commit_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn changed_policy_cannot_reuse_ready_validation_evidence() {
    let fixture = fixture();
    let changed_policy = UploadFieldPolicy::new(
        1,
        2_048,
        UploadReplacementPolicy::RetirePrevious,
        vec![UploadMediaType::Png],
        Some(UploadDimensionLimits::new(512, 512, 262_144).expect("dimensions")),
        UploadScanPolicy::Disabled,
        action_name(),
    )
    .expect("changed policy");

    let error = fixture
        .service
        .finalize(
            &fixture.context,
            request_with_policy(&fixture, changed_policy).await,
            UnixMillis::new(1_002),
        )
        .await
        .expect_err("changed policy must invalidate evidence");

    assert_eq!(error.kind(), UploadErrorKind::ValidationEvidenceUnavailable);
    assert_eq!(fixture.finalizer.prepare_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        fixture
            .ledger
            .load(&handle())
            .await
            .expect("load")
            .expect("record")
            .state(),
        UploadState::Ready
    );
}

#[tokio::test]
async fn durable_commit_with_ledger_failure_requires_and_completes_reconciliation() {
    let fixture = fixture();
    fixture
        .finalizer
        .fail_ledger_after_commit
        .store(true, Ordering::SeqCst);

    let error = fixture
        .service
        .finalize(
            &fixture.context,
            request(&fixture).await,
            UnixMillis::new(1_002),
        )
        .await
        .expect_err("ledger failed after durable commit");
    assert_eq!(error.kind(), UploadErrorKind::ReconciliationRequired);
    assert_eq!(fixture.finalizer.commit_calls.load(Ordering::SeqCst), 1);

    let recovered = fixture
        .service
        .finalize(
            &fixture.context,
            request(&fixture).await,
            UnixMillis::new(1_003),
        )
        .await
        .expect("reconcile");
    assert_eq!(
        recovered.disposition(),
        FinalizeDisposition::ExistingOutcome
    );
    assert_eq!(fixture.finalizer.commit_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture
            .ledger
            .load(&handle())
            .await
            .expect("load")
            .expect("record")
            .state(),
        UploadState::Finalized
    );
}

#[tokio::test]
async fn failed_compensation_is_a_distinct_reconciliation_signal() {
    let fixture = fixture();
    fixture.finalizer.fail_commit.store(true, Ordering::SeqCst);
    fixture
        .finalizer
        .fail_compensation
        .store(true, Ordering::SeqCst);

    let error = fixture
        .service
        .finalize(
            &fixture.context,
            request(&fixture).await,
            UnixMillis::new(1_002),
        )
        .await
        .expect_err("compensation failure");
    assert_eq!(error.kind(), UploadErrorKind::CompensationFailed);
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
