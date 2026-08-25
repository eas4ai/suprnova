//! Race-safe upload expiry, cleanup reconciliation, and telemetry tests.

mod component_support;

use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use suprnova_live::clock::Clock;
use suprnova_live::identity::{ComponentName, ModelField, UnixMillis};
use suprnova_live::limits::{UploadLimitConfig, UploadLimits};
use suprnova_live::resource::PermitPool;
use suprnova_live::upload::{
    AcceptedChunk, BoundedBackoff, CleanupDisposition, CleanupLeaseId, CleanupMetricSink,
    CleanupMetrics, CleanupOutcome, CleanupPolicy, ConditionalTransition, IntegrityEvidence,
    PrepareTransfer, QuarantineBytes, ReadUpload, RetryBucket, TransferGrantScope, TransferPlan,
    UploadAgeBucket, UploadChecksum, UploadCleanupService, UploadError, UploadErrorKind,
    UploadFuture, UploadHandle, UploadIdempotencyKey, UploadLedger, UploadProvider, UploadRecord,
    UploadRevision, UploadState, UploadTransition, UploadTransitionRequest, UploadValidationStore,
    UploadVolumeBucket, ValidatedUpload, ValidationStoreDisposition, VerifyTransfer,
};
use suprnova_live_test_support::{ControlledClock, MemoryUploadLedger};

use component_support::fixture_host_scope;

const MIB: usize = 1024 * 1024;
const HANDLES: [&str; 8] = [
    "018f47c1-2af0-7cc4-a001-000000000001",
    "018f47c1-2af0-7cc4-a001-000000000002",
    "018f47c1-2af0-7cc4-a001-000000000003",
    "018f47c1-2af0-7cc4-a001-000000000004",
    "018f47c1-2af0-7cc4-a001-000000000005",
    "018f47c1-2af0-7cc4-a001-000000000006",
    "018f47c1-2af0-7cc4-a001-000000000007",
    "018f47c1-2af0-7cc4-a001-000000000008",
];

fn limits() -> UploadLimits {
    UploadLimits::new(UploadLimitConfig::reference()).expect("reference upload limits")
}

fn handle(index: usize) -> UploadHandle {
    UploadHandle::parse(HANDLES[index]).expect("fixture upload handle")
}

fn generated_handle(index: usize) -> UploadHandle {
    UploadHandle::parse(&format!("018f8f3a-7b2c-4d5e-8f90-{index:012x}"))
        .expect("generated fixture handle")
}

fn generated_record(
    index: usize,
    state: UploadState,
    revision: u64,
    expires_at: u64,
) -> UploadRecord {
    UploadRecord::new(
        TransferGrantScope::new(
            generated_handle(index),
            ComponentName::parse("upload-cleanup").expect("component"),
            ModelField::parse("attachments").expect("field"),
            fixture_host_scope(),
            1,
        ),
        state,
        UploadRevision::new(revision),
        UnixMillis::new(1_000),
        UnixMillis::new(expires_at),
    )
    .expect("generated upload record")
}

fn authority(index: usize) -> TransferGrantScope {
    TransferGrantScope::new(
        handle(index),
        ComponentName::parse("upload-cleanup").expect("component"),
        ModelField::parse("attachments").expect("field"),
        fixture_host_scope(),
        1,
    )
}

fn record(index: usize, state: UploadState, revision: u64, expires_at: u64) -> UploadRecord {
    UploadRecord::new(
        authority(index),
        state,
        UploadRevision::new(revision),
        UnixMillis::new(1_000),
        UnixMillis::new(expires_at),
    )
    .expect("upload record")
}

fn lease(value: &str) -> CleanupLeaseId {
    CleanupLeaseId::parse(value).expect("cleanup lease id")
}

fn policy(batch_items: usize, batch_bytes: usize, orphan_after: u32) -> CleanupPolicy {
    CleanupPolicy::new(
        NonZeroUsize::new(batch_items).expect("nonzero items"),
        NonZeroUsize::new(batch_bytes).expect("nonzero bytes"),
        Duration::from_millis(500),
        BoundedBackoff::new(
            Duration::from_millis(100),
            Duration::from_millis(400),
            orphan_after,
        )
        .expect("bounded retry"),
    )
    .expect("cleanup policy")
}

#[derive(Default)]
struct ControlledProvider {
    failures: Mutex<HashMap<UploadHandle, usize>>,
    removed: Mutex<HashSet<UploadHandle>>,
    calls: Mutex<Vec<UploadHandle>>,
    advance_clock: Mutex<HashMap<UploadHandle, (Arc<ControlledClock>, UnixMillis)>>,
    active: AtomicUsize,
    max_active: AtomicUsize,
}

impl ControlledProvider {
    fn fail(&self, upload: UploadHandle, times: usize) {
        lock(&self.failures).insert(upload, times);
    }

    fn removed(&self, upload: &UploadHandle) -> bool {
        lock(&self.removed).contains(upload)
    }

    fn calls_for(&self, upload: &UploadHandle) -> usize {
        lock(&self.calls)
            .iter()
            .filter(|candidate| *candidate == upload)
            .count()
    }

    fn advance_clock_once(
        &self,
        upload: UploadHandle,
        clock: Arc<ControlledClock>,
        now: UnixMillis,
    ) {
        lock(&self.advance_clock).insert(upload, (clock, now));
    }

    fn max_active(&self) -> usize {
        self.max_active.load(Ordering::SeqCst)
    }
}

impl UploadProvider for ControlledProvider {
    fn prepare<'a>(
        &'a self,
        _request: PrepareTransfer<'a>,
    ) -> UploadFuture<'a, Result<TransferPlan, UploadError>> {
        Box::pin(async move { Err(UploadError::new(UploadErrorKind::UploadConflict)) })
    }

    fn verify<'a>(
        &'a self,
        _request: VerifyTransfer<'a>,
    ) -> UploadFuture<'a, Result<IntegrityEvidence, UploadError>> {
        Box::pin(async move { Err(UploadError::new(UploadErrorKind::UploadConflict)) })
    }

    fn read<'a>(
        &'a self,
        _request: ReadUpload<'a>,
    ) -> UploadFuture<'a, Result<QuarantineBytes, UploadError>> {
        Box::pin(async move { Err(UploadError::new(UploadErrorKind::UploadConflict)) })
    }

    fn cancel<'a>(&'a self, upload: &'a UploadHandle) -> UploadFuture<'a, Result<(), UploadError>> {
        self.cleanup(upload)
    }

    fn cleanup<'a>(
        &'a self,
        upload: &'a UploadHandle,
    ) -> UploadFuture<'a, Result<(), UploadError>> {
        Box::pin(async move {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(active, Ordering::SeqCst);
            tokio::task::yield_now().await;
            lock(&self.calls).push(upload.clone());
            if let Some(remaining) = lock(&self.failures).get_mut(upload)
                && *remaining > 0
            {
                *remaining -= 1;
                self.active.fetch_sub(1, Ordering::SeqCst);
                return Err(UploadError::new(UploadErrorKind::ProviderUnavailable));
            }
            lock(&self.removed).insert(upload.clone());
            if let Some((clock, now)) = lock(&self.advance_clock).remove(upload) {
                clock.set(now);
            }
            self.active.fetch_sub(1, Ordering::SeqCst);
            Ok(())
        })
    }
}

#[derive(Default)]
struct MemoryEvidenceStore {
    removed: Mutex<HashSet<UploadHandle>>,
}

impl MemoryEvidenceStore {
    fn removed(&self, upload: &UploadHandle) -> bool {
        lock(&self.removed).contains(upload)
    }
}

impl UploadValidationStore for MemoryEvidenceStore {
    fn put<'a>(
        &'a self,
        _evidence: ValidatedUpload,
    ) -> UploadFuture<'a, Result<ValidationStoreDisposition, UploadError>> {
        Box::pin(async move { Ok(ValidationStoreDisposition::Stored) })
    }

    fn load<'a>(
        &'a self,
        _upload: &'a UploadHandle,
    ) -> UploadFuture<'a, Result<Option<ValidatedUpload>, UploadError>> {
        Box::pin(async move { Ok(None) })
    }

    fn remove<'a>(&'a self, upload: &'a UploadHandle) -> UploadFuture<'a, Result<(), UploadError>> {
        Box::pin(async move {
            lock(&self.removed).insert(upload.clone());
            Ok(())
        })
    }
}

#[derive(Default)]
struct Metrics {
    values: Mutex<Vec<CleanupMetrics>>,
}

impl CleanupMetricSink for Metrics {
    fn record(&self, metrics: CleanupMetrics) {
        lock(&self.values).push(metrics);
    }
}

struct Fixture {
    ledger: Arc<MemoryUploadLedger>,
    provider: Arc<ControlledProvider>,
    evidence: Arc<MemoryEvidenceStore>,
    clock: Arc<ControlledClock>,
    metrics: Arc<Metrics>,
    service: UploadCleanupService,
}

fn fixture(policy: CleanupPolicy) -> Fixture {
    let ledger = Arc::new(MemoryUploadLedger::new(limits()).expect("ledger"));
    let provider = Arc::new(ControlledProvider::default());
    let evidence = Arc::new(MemoryEvidenceStore::default());
    let clock = Arc::new(ControlledClock::new(UnixMillis::new(2_000)));
    let metrics = Arc::new(Metrics::default());
    let service = UploadCleanupService::new(
        ledger.clone(),
        provider.clone(),
        evidence.clone(),
        clock.clone() as Arc<dyn Clock>,
        PermitPool::new(2).expect("shared permit pool"),
        policy,
        limits(),
    )
    .expect("cleanup service")
    .with_metrics(metrics.clone());
    Fixture {
        ledger,
        provider,
        evidence,
        clock,
        metrics,
        service,
    }
}

#[tokio::test]
async fn expired_active_states_are_claimed_atomically_and_reclaimed_without_browser() {
    let fixture = fixture(policy(8, 64 * MIB, 3));
    for (index, state) in [
        UploadState::Created,
        UploadState::Queued,
        UploadState::Transferring,
        UploadState::Verifying,
        UploadState::Ready,
    ]
    .into_iter()
    .enumerate()
    {
        fixture
            .ledger
            .seed(record(index, state, 7, 1_900))
            .expect("seed active upload");
    }

    let outcome = fixture
        .service
        .run_once(lease("expire-active"))
        .await
        .expect("cleanup run");

    assert_eq!(outcome.disposition(), CleanupDisposition::Complete);
    assert_eq!(outcome.claimed(), 5);
    assert_eq!(outcome.reclaimed(), 5);
    for index in 0..5 {
        let upload = handle(index);
        assert!(fixture.ledger.load(&upload).await.expect("load").is_none());
        assert!(fixture.provider.removed(&upload));
        assert!(fixture.evidence.removed(&upload));
    }
}

#[tokio::test]
async fn cleanup_never_claims_finalizing_or_finalized_records() {
    let fixture = fixture(policy(8, 64 * MIB, 3));
    fixture
        .ledger
        .seed(record(0, UploadState::Finalizing, 8, 1_900))
        .expect("seed finalizing");
    fixture
        .ledger
        .seed(record(1, UploadState::Finalized, 9, 1_900))
        .expect("seed finalized");

    let outcome = fixture
        .service
        .run_once(lease("skip-finalized"))
        .await
        .expect("cleanup run");

    assert_eq!(outcome.disposition(), CleanupDisposition::Idle);
    assert_eq!(fixture.provider.calls_for(&handle(0)), 0);
    assert_eq!(fixture.provider.calls_for(&handle(1)), 0);
}

#[tokio::test]
async fn cleanup_cannot_delete_a_committed_finalize() {
    let fixture = fixture(policy(1, 64 * MIB, 3));
    let authority = authority(0);
    fixture
        .ledger
        .seed(record(0, UploadState::Ready, 7, 2_000))
        .expect("seed ready");

    let finalize = fixture.ledger.transition(ConditionalTransition::new(
        authority,
        UploadTransitionRequest::new(
            handle(0),
            UploadRevision::new(7),
            UploadIdempotencyKey::parse("begin-finalize").expect("idempotency"),
            UploadTransition::BeginFinalize,
        ),
        UnixMillis::new(1_999),
    ));
    let cleanup = fixture.service.run_once(lease("finalize-race"));
    let (finalized, cleaned) = tokio::join!(finalize, cleanup);
    let cleaned = cleaned.expect("cleanup outcome");
    let stored = fixture.ledger.load(&handle(0)).await.expect("load");

    if finalized.is_ok() {
        let stored = stored.expect("finalizing record remains authoritative");
        assert_eq!(stored.state(), UploadState::Finalizing);
        assert_eq!(cleaned.disposition(), CleanupDisposition::Idle);
        assert!(!fixture.provider.removed(&handle(0)));
    } else {
        assert!(stored.is_none());
        assert_eq!(cleaned.disposition(), CleanupDisposition::Complete);
    }
}

#[tokio::test]
async fn cleanup_races_with_transfer_verification_scan_cancel_and_remove() {
    let checksum = UploadChecksum::parse(&"0".repeat(64)).expect("checksum");
    let cases = [
        (
            UploadState::Transferring,
            UploadTransition::PutChunk(
                AcceptedChunk::new(0, 32, checksum).expect("accepted chunk"),
            ),
        ),
        (UploadState::Verifying, UploadTransition::Accept),
        (UploadState::Verifying, UploadTransition::Reject),
        (UploadState::Created, UploadTransition::Cancel),
        (UploadState::Ready, UploadTransition::Cancel),
    ];

    for (index, (state, transition)) in cases.into_iter().enumerate() {
        let fixture = fixture(policy(1, 64 * MIB, 3));
        let authority = authority(index);
        fixture
            .ledger
            .seed(record(index, state, 7, 2_000))
            .expect("seed racing state");
        let operation = fixture.ledger.transition(ConditionalTransition::new(
            authority,
            UploadTransitionRequest::new(
                handle(index),
                UploadRevision::new(7),
                UploadIdempotencyKey::parse(&format!("race-{index}")).expect("idempotency"),
                transition,
            ),
            UnixMillis::new(1_999),
        ));
        let cleanup = fixture
            .service
            .run_once(lease(&format!("cleanup-race-{index}")));
        let (_operation, cleanup) = tokio::join!(operation, cleanup);
        let cleanup = cleanup.expect("cleanup race");
        assert_eq!(cleanup.reclaimed(), 1);
        assert!(
            fixture
                .ledger
                .load(&handle(index))
                .await
                .expect("load")
                .is_none()
        );
        assert!(fixture.provider.removed(&handle(index)));
    }
}

#[tokio::test]
async fn cleanup_batches_are_bounded_by_items_and_retained_bytes() {
    let fixture = fixture(policy(2, 64 * MIB, 3));
    for index in 0..3 {
        fixture
            .ledger
            .seed(record(index, UploadState::Canceled, 8, 10_000))
            .expect("seed canceled");
        fixture
            .ledger
            .seed_cleanup_bytes(&handle(index), 40 * MIB as u64)
            .expect("seed retained bytes");
    }

    let first = fixture
        .service
        .run_once(lease("bounded-one"))
        .await
        .expect("first cleanup");
    assert_eq!(first.claimed(), 1);
    assert_eq!(first.reclaimed_bytes(), 40 * MIB as u64);

    let second = fixture
        .service
        .run_once(lease("bounded-two"))
        .await
        .expect("second cleanup");
    assert_eq!(second.claimed(), 1);
    assert_eq!(second.reclaimed_bytes(), 40 * MIB as u64);
}

#[tokio::test]
async fn cleanup_selection_never_scans_or_allocates_the_entire_ledger() {
    let fixture = fixture(policy(2, 64 * MIB, 3));
    for index in 100..228 {
        fixture
            .ledger
            .seed(generated_record(index, UploadState::Created, 1, 10_000))
            .expect("seed future upload");
    }
    for index in 0..3 {
        let upload = generated_handle(index);
        fixture
            .ledger
            .seed(generated_record(index, UploadState::Canceled, 8, 10_000))
            .expect("seed due upload");
        fixture
            .ledger
            .seed_cleanup_bytes(&upload, 40 * MIB as u64)
            .expect("seed retained bytes");
    }

    let outcome = fixture
        .service
        .run_once(lease("bounded-selection"))
        .await
        .expect("bounded cleanup selection");

    assert_eq!(outcome.claimed(), 1);
    assert_eq!(fixture.ledger.cleanup_examined_last_run(), 2);
    assert_eq!(fixture.ledger.len(), 130);
}

#[tokio::test]
async fn replayed_chunks_do_not_double_cleanup_volume_or_break_idempotency() {
    let fixture = fixture(policy(1, 64 * MIB, 3));
    let authority = authority(0);
    fixture
        .ledger
        .seed(record(0, UploadState::Transferring, 7, 10_000))
        .expect("seed transfer");
    fixture
        .ledger
        .seed_cleanup_bytes(&handle(0), limits().max_file_bytes() - 32)
        .expect("seed partial bytes");
    let transition = ConditionalTransition::new(
        authority,
        UploadTransitionRequest::new(
            handle(0),
            UploadRevision::new(7),
            UploadIdempotencyKey::parse("last-chunk").expect("idempotency"),
            UploadTransition::PutChunk(
                AcceptedChunk::new(
                    1,
                    32,
                    UploadChecksum::parse(&"0".repeat(64)).expect("checksum"),
                )
                .expect("accepted chunk"),
            ),
        ),
        UnixMillis::new(1_999),
    );

    fixture
        .ledger
        .transition(transition.clone())
        .await
        .expect("first chunk outcome");
    fixture
        .ledger
        .transition(transition)
        .await
        .expect("exact replay");

    assert_eq!(
        fixture
            .ledger
            .cleanup_observation(&handle(0))
            .expect("observation")
            .retained_bytes(),
        limits().max_file_bytes()
    );
}

#[tokio::test]
async fn failed_cleanup_retries_with_bounded_backoff_and_marks_orphans() {
    let fixture = fixture(policy(1, 64 * MIB, 2));
    fixture
        .ledger
        .seed(record(0, UploadState::Canceled, 8, 10_000))
        .expect("seed canceled");
    fixture.provider.fail(handle(0), 2);

    let first = fixture
        .service
        .run_once(lease("retry-one"))
        .await
        .expect("first retry");
    assert_eq!(first.retry_scheduled(), 1);
    assert_eq!(first.orphaned(), 0);

    fixture.clock.set(UnixMillis::new(2_099));
    let early = fixture
        .service
        .run_once(lease("retry-early"))
        .await
        .expect("early run");
    assert_eq!(early.disposition(), CleanupDisposition::Idle);

    fixture.clock.set(UnixMillis::new(2_100));
    let second = fixture
        .service
        .run_once(lease("retry-two"))
        .await
        .expect("second retry");
    assert_eq!(second.retry_scheduled(), 1);
    assert_eq!(second.orphaned(), 1);
    let observation = fixture
        .ledger
        .cleanup_observation(&handle(0))
        .expect("cleanup observation");
    assert_eq!(observation.retries(), 2);
    assert!(observation.orphaned());

    fixture.clock.set(UnixMillis::new(2_300));
    let recovered = fixture
        .service
        .run_once(lease("retry-recovered"))
        .await
        .expect("recovered cleanup");
    assert_eq!(recovered.reclaimed(), 1);
    assert!(fixture.provider.removed(&handle(0)));
    assert!(
        fixture
            .ledger
            .load(&handle(0))
            .await
            .expect("load")
            .is_none()
    );
}

#[tokio::test]
async fn expired_cleanup_leases_are_fenced_then_reconciled_idempotently() {
    let fixture = fixture(policy(1, 64 * MIB, 3));
    fixture
        .ledger
        .seed(record(0, UploadState::Canceled, 8, 10_000))
        .expect("seed canceled");
    fixture
        .provider
        .advance_clock_once(handle(0), fixture.clock.clone(), UnixMillis::new(2_600));

    let fenced = fixture
        .service
        .run_once(lease("lease-will-expire"))
        .await
        .expect("fenced cleanup");
    assert_eq!(fenced.disposition(), CleanupDisposition::Deferred);
    assert_eq!(fenced.deferred(), 1);
    assert!(
        fixture
            .ledger
            .cleanup_observation(&handle(0))
            .expect("observation")
            .leased()
    );

    let reconciled = fixture
        .service
        .run_once(lease("lease-reconciled"))
        .await
        .expect("idempotent reconciliation");
    assert_eq!(reconciled.reclaimed(), 1);
    assert_eq!(fixture.provider.calls_for(&handle(0)), 2);
    assert!(
        fixture
            .ledger
            .load(&handle(0))
            .await
            .expect("load")
            .is_none()
    );
}

#[tokio::test]
async fn one_scope_failure_does_not_block_unrelated_scope_cleanup() {
    let fixture = fixture(policy(2, 128 * MIB, 3));
    fixture
        .ledger
        .seed(record(0, UploadState::Canceled, 8, 10_000))
        .expect("seed failing scope");
    fixture
        .ledger
        .seed(record(1, UploadState::Canceled, 8, 10_000))
        .expect("seed healthy scope");
    fixture.provider.fail(handle(0), 1);

    let outcome = fixture
        .service
        .run_once(lease("scope-availability"))
        .await
        .expect("bounded cleanup");

    assert_eq!(outcome.reclaimed(), 1);
    assert_eq!(outcome.retry_scheduled(), 1);
    assert!(fixture.provider.removed(&handle(1)));
}

#[tokio::test]
async fn concurrent_cleanup_runs_use_the_shared_permit_pool() {
    let fixture = fixture(policy(1, 64 * MIB, 3));
    fixture
        .ledger
        .seed(record(0, UploadState::Canceled, 8, 10_000))
        .expect("seed first upload");
    fixture
        .ledger
        .seed(record(1, UploadState::Canceled, 8, 10_000))
        .expect("seed second upload");

    let first = fixture.service.run_once(lease("concurrent-one"));
    let second = fixture.service.run_once(lease("concurrent-two"));
    let (first, second) = tokio::join!(first, second);

    assert_eq!(
        first.expect("first cleanup").reclaimed() + second.expect("second cleanup").reclaimed(),
        2
    );
    assert_eq!(fixture.provider.max_active(), 2);
}

#[tokio::test]
async fn cleanup_cancellation_is_advisory_and_never_claims_new_work() {
    let fixture = fixture(policy(1, 64 * MIB, 3));
    fixture
        .ledger
        .seed(record(0, UploadState::Canceled, 8, 10_000))
        .expect("seed canceled");
    assert!(fixture.service.cancel());
    assert!(!fixture.service.cancel());

    let outcome = fixture
        .service
        .run_once(lease("canceled-service"))
        .await
        .expect("canceled run");

    assert_eq!(outcome.disposition(), CleanupDisposition::Deferred);
    assert_eq!(fixture.provider.calls_for(&handle(0)), 0);
    assert!(
        !fixture
            .ledger
            .cleanup_observation(&handle(0))
            .expect("observation")
            .leased()
    );
}

#[tokio::test]
async fn cleanup_telemetry_is_closed_redacted_and_observational() {
    let fixture = fixture(policy(1, 64 * MIB, 3));
    fixture
        .ledger
        .seed(record(0, UploadState::Canceled, 8, 10_000))
        .expect("seed canceled");
    fixture
        .service
        .run_once(lease("telemetry-run"))
        .await
        .expect("cleanup run");

    let metrics = lock(&fixture.metrics.values);
    assert_eq!(metrics.len(), 1);
    let metric = metrics[0];
    assert_eq!(metric.outcome(), CleanupOutcome::Reclaimed);
    assert_eq!(metric.retry_bucket(), RetryBucket::None);
    assert!(UploadAgeBucket::ALL.contains(&metric.age_bucket()));
    assert!(UploadVolumeBucket::ALL.contains(&metric.volume_bucket()));
    let debug = format!("{metric:?}");
    assert!(!debug.contains(HANDLES[0]));
    assert!(!debug.contains("attachments"));
    assert!(!debug.contains("telemetry-run"));
}

struct PanickingMetrics;

impl CleanupMetricSink for PanickingMetrics {
    fn record(&self, _metrics: CleanupMetrics) {
        panic!("host telemetry panic");
    }
}

#[tokio::test]
async fn telemetry_panics_cannot_rewrite_a_committed_cleanup() {
    let fixture = fixture(policy(1, 64 * MIB, 3));
    fixture
        .ledger
        .seed(record(0, UploadState::Canceled, 8, 10_000))
        .expect("seed canceled");
    let service = UploadCleanupService::new(
        fixture.ledger.clone(),
        fixture.provider.clone(),
        fixture.evidence.clone(),
        fixture.clock.clone() as Arc<dyn Clock>,
        PermitPool::new(1).expect("permit pool"),
        policy(1, 64 * MIB, 3),
        limits(),
    )
    .expect("cleanup service")
    .with_metrics(Arc::new(PanickingMetrics));

    let outcome = service
        .run_once(lease("panic-metrics"))
        .await
        .expect("telemetry is observational");

    assert_eq!(outcome.reclaimed(), 1);
    assert!(
        fixture
            .ledger
            .load(&handle(0))
            .await
            .expect("load")
            .is_none()
    );
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
