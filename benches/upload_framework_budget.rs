//! U4/16 upload control-framework benchmark with every external work port excluded.

#[path = "../tests/component_support.rs"]
mod component_support;

use std::collections::{BTreeSet, HashMap};
use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use serde::Serialize;
use sha2::{Digest, Sha256};
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::identity::{ActionName, KeyId, ModelField, UnixMillis};
use suprnova_live::limits::{UploadLimitConfig, UploadLimits};
use suprnova_live::upload::{
    AcceptedChunk, ApplicationValidationDecision, ApplicationValidationInput, ChunkBody,
    ChunkDisposition, ClientUploadMetadata, IntegrityEvidence, PrepareTransfer, QuarantineBytes,
    QuarantineObject, QuarantineOperation, QuarantineStore, QuarantinedFileProvider, ReadUpload,
    RemoveDisposition, ReverseProxyUploadProvider, ScanDisposition, ScanFailurePolicy, ScanInput,
    TransferGrant, TransferGrantCodec, TransferGrantRequest, TransferGrantScope, TransferPlan,
    TransitionDisposition, UploadApplicationValidator, UploadChecksum, UploadDimensionLimits,
    UploadError, UploadErrorKind, UploadFieldPolicy, UploadFuture, UploadHandle,
    UploadIdempotencyKey, UploadLedger, UploadMediaType, UploadOperation, UploadProtocolCodec,
    UploadProvider, UploadRecord, UploadReplacementPolicy, UploadRevision, UploadScanPolicy,
    UploadScanner, UploadService, UploadState, UploadTransition, UploadTransitionAdmission,
    UploadTransitionRequest, UploadValidationDisposition, UploadValidationRequest,
    UploadValidationService, UploadValidationStore, ValidatedUpload, ValidationStoreDisposition,
    VerifyTransfer, WriteChunk,
};
use suprnova_live_test_support::{ControlledUploadAuthorization, MemoryUploadLedger};

const WORKLOAD: &str = "U4/16";
const FILES: usize = 4;
const FILE_BYTES: usize = 16 * 1024 * 1024;
const CHUNK_BYTES: usize = 256 * 1024;
const ACTIVE_TRANSFERS: usize = 4;
const WARMUP_ITERATIONS: usize = 50;
const MEASURED_SAMPLES: usize = 40;
const P95_CAP_MICROSECONDS: f64 = 2_000.0;
const NOW: UnixMillis = UnixMillis::new(1_001);
const EXPIRES_AT: UnixMillis = UnixMillis::new(1_900);
const ROOT_SECRET: &[u8] = b"upload-budget-root-secret-000000";

#[derive(Default)]
struct ExcludedPortCounters {
    body_io: AtomicUsize,
    provider: AtomicUsize,
    scanner: AtomicUsize,
    application_validation: AtomicUsize,
}

struct NullChunkBody {
    counters: Arc<ExcludedPortCounters>,
}

impl ChunkBody for NullChunkBody {
    fn next_chunk<'a>(
        &'a mut self,
        _maximum_bytes: usize,
    ) -> UploadFuture<'a, Result<Option<QuarantineBytes>, UploadError>> {
        self.counters.body_io.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(UploadError::new(UploadErrorKind::ProviderUnavailable)) })
    }
}

struct NullProvider {
    counters: Arc<ExcludedPortCounters>,
    bytes: QuarantineBytes,
    checksum: UploadChecksum,
    handle: UploadHandle,
}

impl UploadProvider for NullProvider {
    fn prepare<'a>(
        &'a self,
        _request: PrepareTransfer<'a>,
    ) -> UploadFuture<'a, Result<TransferPlan, UploadError>> {
        self.counters.provider.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(UploadError::new(UploadErrorKind::ProviderUnavailable)) })
    }

    fn verify<'a>(
        &'a self,
        request: VerifyTransfer<'a>,
    ) -> UploadFuture<'a, Result<IntegrityEvidence, UploadError>> {
        self.counters.provider.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if request.handle() != &self.handle || request.checksum() != &self.checksum {
                return Err(UploadError::new(UploadErrorKind::ChecksumMismatch));
            }
            Ok(IntegrityEvidence::from_provider(
                self.bytes.len() as u64,
                self.checksum.clone(),
            ))
        })
    }

    fn read<'a>(
        &'a self,
        request: ReadUpload<'a>,
    ) -> UploadFuture<'a, Result<QuarantineBytes, UploadError>> {
        self.counters.provider.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if request.handle() != &self.handle || request.maximum_bytes() == 0 {
                return Err(UploadError::new(UploadErrorKind::ScopeMismatch));
            }
            let start = usize::try_from(request.offset())
                .map_err(|_| UploadError::new(UploadErrorKind::InvalidField))?;
            let end = start
                .saturating_add(request.maximum_bytes())
                .min(self.bytes.len());
            Ok(self.bytes.slice(start..end))
        })
    }

    fn cancel<'a>(
        &'a self,
        _handle: &'a UploadHandle,
    ) -> UploadFuture<'a, Result<(), UploadError>> {
        self.counters.provider.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(UploadError::new(UploadErrorKind::ProviderUnavailable)) })
    }

    fn cleanup<'a>(
        &'a self,
        _handle: &'a UploadHandle,
    ) -> UploadFuture<'a, Result<(), UploadError>> {
        self.counters.provider.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(UploadError::new(UploadErrorKind::ProviderUnavailable)) })
    }
}

struct NullScanner {
    counters: Arc<ExcludedPortCounters>,
}

impl UploadScanner for NullScanner {
    fn scan<'a>(
        &'a self,
        _input: ScanInput<'a>,
    ) -> UploadFuture<'a, Result<ScanDisposition, UploadError>> {
        self.counters.scanner.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(ScanDisposition::Clean) })
    }
}

struct NullApplicationValidator {
    counters: Arc<ExcludedPortCounters>,
}

impl UploadApplicationValidator for NullApplicationValidator {
    fn validate<'a>(
        &'a self,
        _input: ApplicationValidationInput<'a>,
    ) -> UploadFuture<'a, Result<ApplicationValidationDecision, UploadError>> {
        self.counters
            .application_validation
            .fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(ApplicationValidationDecision::Allow) })
    }
}

struct NullValidationStore;

impl UploadValidationStore for NullValidationStore {
    fn put<'a>(
        &'a self,
        _evidence: ValidatedUpload,
    ) -> UploadFuture<'a, Result<ValidationStoreDisposition, UploadError>> {
        Box::pin(async { Ok(ValidationStoreDisposition::Stored) })
    }

    fn load<'a>(
        &'a self,
        _upload: &'a UploadHandle,
    ) -> UploadFuture<'a, Result<Option<ValidatedUpload>, UploadError>> {
        Box::pin(async { Ok(None) })
    }

    fn remove<'a>(
        &'a self,
        _upload: &'a UploadHandle,
    ) -> UploadFuture<'a, Result<(), UploadError>> {
        Box::pin(async { Ok(()) })
    }
}

struct NullExternalPorts {
    application: Arc<dyn UploadApplicationValidator>,
    body: NullChunkBody,
    provider: Arc<dyn UploadProvider>,
    scanner: Arc<dyn UploadScanner>,
    counters: Arc<ExcludedPortCounters>,
    handle: UploadHandle,
}

impl NullExternalPorts {
    fn new(handle: UploadHandle, bytes: QuarantineBytes, checksum: UploadChecksum) -> Self {
        let counters = Arc::new(ExcludedPortCounters::default());
        Self {
            application: Arc::new(NullApplicationValidator {
                counters: Arc::clone(&counters),
            }),
            body: NullChunkBody {
                counters: Arc::clone(&counters),
            },
            provider: Arc::new(NullProvider {
                counters: Arc::clone(&counters),
                bytes,
                checksum,
                handle: handle.clone(),
            }),
            scanner: Arc::new(NullScanner {
                counters: Arc::clone(&counters),
            }),
            counters,
            handle,
        }
    }

    fn require_zero(&self) -> Result<(), Box<dyn Error>> {
        self.counters.require_zero()
    }
}

impl ExcludedPortCounters {
    fn snapshot(&self) -> ExcludedCalls {
        ExcludedCalls {
            application_validation: self.application_validation.load(Ordering::SeqCst),
            body_io: self.body_io.load(Ordering::SeqCst),
            provider: self.provider.load(Ordering::SeqCst),
            scanner: self.scanner.load(Ordering::SeqCst),
        }
    }

    fn require_zero(&self) -> Result<(), Box<dyn Error>> {
        let calls = self.snapshot();
        if calls != ExcludedCalls::default() {
            return Err(
                std::io::Error::other("external upload work entered timed control path").into(),
            );
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
struct ExcludedCalls {
    #[serde(rename = "applicationValidation")]
    application_validation: usize,
    #[serde(rename = "bodyIo")]
    body_io: usize,
    provider: usize,
    scanner: usize,
}

fn validation_png() -> Vec<u8> {
    let mut bytes = vec![
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, b'I', b'H', b'D', b'R',
    ];
    bytes.extend_from_slice(&320_u32.to_be_bytes());
    bytes.extend_from_slice(&240_u32.to_be_bytes());
    bytes.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0]);
    bytes
}

fn checksum(bytes: &[u8]) -> Result<UploadChecksum, UploadError> {
    let mut encoded = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(&mut encoded, "{byte:02x}")
            .map_err(|_| UploadError::new(UploadErrorKind::InvalidField))?;
    }
    UploadChecksum::parse(&encoded)
}

fn validation_policy() -> Result<UploadFieldPolicy, UploadError> {
    UploadFieldPolicy::new(
        FILES,
        FILE_BYTES as u64,
        UploadReplacementPolicy::RetirePrevious,
        vec![UploadMediaType::Png],
        Some(UploadDimensionLimits::new(1_024, 1_024, 1_048_576)?),
        UploadScanPolicy::Required {
            on_timeout: ScanFailurePolicy::Retry,
            on_unavailable: ScanFailurePolicy::Reject,
        },
        ActionName::parse("upload_budget_validate")
            .map_err(|_| UploadError::new(UploadErrorKind::InvalidField))?,
    )
}

struct Fixture {
    service: UploadService,
    context: suprnova_live::host::TrustedLiveRequestContext,
    grant: TransferGrant,
    request: Vec<u8>,
    codec: UploadProtocolCodec,
    authorization: Arc<ControlledUploadAuthorization>,
    excluded: NullExternalPorts,
    validation_request: Option<UploadValidationRequest>,
    validation_service: UploadValidationService,
}

#[allow(
    dead_code,
    reason = "non-control variants are deliberate mutation probes for excluded external work"
)]
enum HarnessOperation {
    BodyIo,
    Control,
    Provider,
    Validation,
}

impl Fixture {
    fn new(index: usize) -> Result<Self, Box<dyn Error>> {
        let limits = UploadLimits::new(UploadLimitConfig::reference())?;
        let authorization = Arc::new(ControlledUploadAuthorization::new());
        let context = component_support::trusted_context_with_upload_authorization(Arc::clone(
            &authorization,
        )
            as Arc<dyn suprnova_live::upload::UploadAuthorizationPort>);
        let handle = UploadHandle::parse(&format!("018f47c1-2af0-7cc4-a001-{index:012}"))?;
        let field = ModelField::parse("serial")?;
        let authority = TransferGrantScope::new(
            handle.clone(),
            context.mount().component().clone(),
            field,
            component_support::fixture_host_scope(),
            1,
        );
        let codec = grant_codec()?;
        let issued = codec.issue(
            TransferGrantRequest::new(authority.clone(), EXPIRES_AT),
            UnixMillis::new(1_000),
        )?;
        let grant = TransferGrant::parse(issued.grant().expose_bearer())?;
        let ledger = Arc::new(MemoryUploadLedger::new(limits)?);
        ledger.seed(UploadRecord::new(
            authority,
            UploadState::Transferring,
            UploadRevision::new(7),
            UnixMillis::new(1_000),
            EXPIRES_AT,
        )?)?;
        let service_ledger: Arc<dyn UploadLedger> = ledger;
        let service = UploadService::new(service_ledger, codec, limits)?;
        let validation_bytes = validation_png();
        let validation_checksum = checksum(&validation_bytes)?;
        let excluded = NullExternalPorts::new(
            handle.clone(),
            QuarantineBytes::copy_from_slice(&validation_bytes),
            validation_checksum.clone(),
        );
        let validation_ledger = Arc::new(MemoryUploadLedger::new(limits)?);
        validation_ledger.seed(UploadRecord::new(
            TransferGrantScope::new(
                handle.clone(),
                context.mount().component().clone(),
                ModelField::parse("serial")?,
                component_support::fixture_host_scope(),
                1,
            ),
            UploadState::Verifying,
            UploadRevision::new(7),
            UnixMillis::new(1_000),
            EXPIRES_AT,
        )?)?;
        let validation_authority = Arc::new(UploadService::new(
            validation_ledger,
            grant_codec()?,
            limits,
        )?);
        let validation_service = UploadValidationService::new(
            validation_authority,
            Arc::clone(&excluded.provider),
            Arc::new(NullValidationStore),
            Some(Arc::clone(&excluded.scanner)),
            Some(Arc::clone(&excluded.application)),
            limits,
        )?;
        let validation_request = UploadValidationRequest::new(
            handle.clone(),
            ModelField::parse("serial")?,
            UploadRevision::new(7),
            UploadIdempotencyKey::parse(&format!("u4-16-validation-{index}"))?,
            ClientUploadMetadata::new("u4-16.png", Some("image/png"))?,
            validation_bytes.len() as u64,
            validation_checksum,
            validation_policy()?,
        );
        let request = serde_json_canonicalizer::to_vec(&serde_json::json!({
            "checksum": "ab".repeat(32),
            "chunk_index": 0,
            "expected_revision": "7",
            "handle": handle.to_string(),
            "idempotency_key": format!("u4-16-{index}"),
            "operation": "put_chunk",
            "protocol_version": 1,
            "size": CHUNK_BYTES,
        }))?;
        Ok(Self {
            service,
            context,
            grant,
            request,
            codec: UploadProtocolCodec::v1(),
            authorization,
            excluded,
            validation_request: Some(validation_request),
            validation_service,
        })
    }

    async fn process_once(&mut self) -> Result<(), Box<dyn Error>> {
        self.dispatch(HarnessOperation::Control).await
    }

    async fn dispatch(&mut self, operation: HarnessOperation) -> Result<(), Box<dyn Error>> {
        match operation {
            HarnessOperation::Control => self.process_control().await,
            HarnessOperation::BodyIo => {
                let _ = self.excluded.body.next_chunk(CHUNK_BYTES).await;
                Ok(())
            }
            HarnessOperation::Provider => {
                let _ = self.excluded.provider.cancel(&self.excluded.handle).await;
                Ok(())
            }
            HarnessOperation::Validation => {
                let request = self
                    .validation_request
                    .take()
                    .ok_or_else(|| std::io::Error::other("validation probe already consumed"))?;
                let outcome = self
                    .validation_service
                    .validate(&self.context, request, NOW)
                    .await?;
                if outcome.disposition() != UploadValidationDisposition::Ready {
                    return Err(
                        std::io::Error::other("validation probe did not reach Ready").into(),
                    );
                }
                Ok(())
            }
        }
    }

    async fn process_control(&self) -> Result<(), Box<dyn Error>> {
        let operation = self.codec.decode(&self.request)?;
        let UploadOperation::PutChunk(chunk) = operation else {
            return Err(std::io::Error::other("U4/16 control did not decode as put_chunk").into());
        };
        let transition = UploadTransitionRequest::new(
            chunk.handle().clone(),
            chunk.expected_revision(),
            chunk.idempotency_key().clone(),
            UploadTransition::PutChunk(AcceptedChunk::new(
                chunk.chunk_index(),
                chunk.size(),
                chunk.checksum().clone(),
            )?),
        );
        let admission = UploadTransitionAdmission::new(
            self.grant.clone(),
            ModelField::parse("serial")?,
            transition,
        );
        let applied = self
            .service
            .transition(&self.context, admission.clone(), NOW)
            .await?;
        let applied_response = response_bytes(applied)?;
        let duplicate = self
            .service
            .transition(&self.context, admission, NOW)
            .await?;
        let duplicate_response = response_bytes(duplicate)?;
        if applied.disposition() != TransitionDisposition::Applied
            || duplicate.disposition() != TransitionDisposition::ExistingOutcome
            || applied.state() != UploadState::Transferring
            || duplicate.state() != applied.state()
            || duplicate.revision() != applied.revision()
            || applied_response.is_empty()
            || duplicate_response.is_empty()
            || self.authorization.call_count() != 2
        {
            return Err(std::io::Error::other("U4/16 control/idempotency contract failed").into());
        }
        self.excluded.require_zero()?;
        black_box((applied_response, duplicate_response));
        Ok(())
    }
}

fn response_bytes(
    outcome: suprnova_live::upload::TransitionOutcome,
) -> Result<Vec<u8>, serde_json::Error> {
    serde_json_canonicalizer::to_vec(&serde_json::json!({
        "disposition": match outcome.disposition() {
            TransitionDisposition::Applied => "applied",
            TransitionDisposition::ExistingOutcome => "existing_outcome",
        },
        "revision": outcome.revision().get().to_string(),
        "state": outcome.state().as_str(),
    }))
}

fn grant_codec() -> Result<TransferGrantCodec, Box<dyn Error>> {
    let key = KeyRecord::new(
        KeyId::parse("upload-budget-key")?,
        RootKey::new(ROOT_SECRET.to_vec())?,
        UnixMillis::new(0),
        UnixMillis::new(10_000),
        UnixMillis::new(20_000),
    )?;
    Ok(TransferGrantCodec::new(SnapshotKeyRing::new(
        key,
        Vec::new(),
    )?))
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResourceWorkloadMutation {
    ExtraTransfer,
    None,
    SkewFirst,
}

#[derive(Clone, Copy, Debug, Default)]
struct TransferChunkState {
    body_buffers: usize,
    body_bytes: usize,
    body_high_water: usize,
    provider_buffers: usize,
    provider_bytes: usize,
    provider_high_water: usize,
    total_high_water: usize,
    total_high_water_bytes: usize,
}

#[derive(Default)]
struct ServerChunkTracker {
    transfers: Mutex<HashMap<UploadHandle, TransferChunkState>>,
}

impl ServerChunkTracker {
    fn body_started(&self, handle: &UploadHandle, bytes: usize) {
        let mut transfers = lock_unpoisoned(&self.transfers);
        let state = transfers.entry(handle.clone()).or_default();
        state.body_buffers = 1;
        state.body_bytes = bytes;
        Self::record_highs(state);
    }

    fn body_finished(&self, handle: &UploadHandle) {
        let mut transfers = lock_unpoisoned(&self.transfers);
        let state = transfers.entry(handle.clone()).or_default();
        state.body_buffers = 0;
        state.body_bytes = 0;
        Self::record_highs(state);
    }

    fn provider_started(&self, handle: &UploadHandle, buffers: usize, bytes: usize) {
        let mut transfers = lock_unpoisoned(&self.transfers);
        let state = transfers.entry(handle.clone()).or_default();
        state.provider_buffers = buffers;
        state.provider_bytes = bytes;
        Self::record_highs(state);
    }

    fn provider_finished(&self, handle: &UploadHandle) {
        let mut transfers = lock_unpoisoned(&self.transfers);
        let state = transfers.entry(handle.clone()).or_default();
        state.provider_buffers = 0;
        state.provider_bytes = 0;
        Self::record_highs(state);
    }

    fn record_highs(state: &mut TransferChunkState) {
        state.body_high_water = state.body_high_water.max(state.body_buffers);
        state.provider_high_water = state.provider_high_water.max(state.provider_buffers);
        state.total_high_water = state
            .total_high_water
            .max(state.body_buffers.saturating_add(state.provider_buffers));
        state.total_high_water_bytes = state
            .total_high_water_bytes
            .max(state.body_bytes.saturating_add(state.provider_bytes));
    }

    fn snapshot(&self) -> Vec<ServerTransferChunkBuffers> {
        let transfers = lock_unpoisoned(&self.transfers);
        let mut observed = transfers
            .iter()
            .map(|(handle, state)| ServerTransferChunkBuffers {
                body_high_water: state.body_high_water,
                current_body_buffers: state.body_buffers,
                current_bytes: state.body_bytes.saturating_add(state.provider_bytes),
                current_provider_buffers: state.provider_buffers,
                current_total_buffers: state.body_buffers.saturating_add(state.provider_buffers),
                handle: handle.to_string(),
                provider_high_water: state.provider_high_water,
                total_high_water: state.total_high_water,
                total_high_water_bytes: state.total_high_water_bytes,
            })
            .collect::<Vec<_>>();
        observed.sort_by(|left, right| left.handle.cmp(&right.handle));
        observed
    }
}

struct ControlledChunkBody {
    bytes: QuarantineBytes,
    offset: usize,
    block_before_second_segment: bool,
    entered: Arc<tokio::sync::Barrier>,
    release: Arc<tokio::sync::Barrier>,
}

impl ChunkBody for ControlledChunkBody {
    fn next_chunk<'a>(
        &'a mut self,
        maximum_bytes: usize,
    ) -> UploadFuture<'a, Result<Option<QuarantineBytes>, UploadError>> {
        Box::pin(async move {
            if self.offset >= self.bytes.len() {
                return Ok(None);
            }
            if self.block_before_second_segment && self.offset > 0 {
                self.block_before_second_segment = false;
                self.entered.wait().await;
                self.release.wait().await;
            }
            let end = self
                .offset
                .saturating_add(maximum_bytes)
                .min(self.bytes.len());
            let bytes = self.bytes.slice(self.offset..end);
            self.offset = end;
            Ok(Some(bytes))
        })
    }
}

struct ControlledQuarantineStore {
    bindings: Mutex<HashMap<String, UploadHandle>>,
    entered: Arc<tokio::sync::Barrier>,
    first_writes: Mutex<BTreeSet<String>>,
    mutation: ResourceWorkloadMutation,
    objects: Mutex<HashMap<String, u64>>,
    release: Arc<tokio::sync::Barrier>,
    tracker: Arc<ServerChunkTracker>,
    unbound: Mutex<Vec<String>>,
}

impl ControlledQuarantineStore {
    fn new(
        mutation: ResourceWorkloadMutation,
        tracker: Arc<ServerChunkTracker>,
        entered: Arc<tokio::sync::Barrier>,
        release: Arc<tokio::sync::Barrier>,
    ) -> Self {
        Self {
            bindings: Mutex::new(HashMap::new()),
            entered,
            first_writes: Mutex::new(BTreeSet::new()),
            mutation,
            objects: Mutex::new(HashMap::new()),
            release,
            tracker,
            unbound: Mutex::new(Vec::new()),
        }
    }

    fn bind_latest(&self, handle: &UploadHandle) -> Result<(), UploadError> {
        let key = lock_unpoisoned(&self.unbound)
            .pop()
            .ok_or_else(|| UploadError::new(UploadErrorKind::ProviderUnavailable))?;
        lock_unpoisoned(&self.bindings).insert(key, handle.clone());
        Ok(())
    }

    fn is_skewed_second(handle: &UploadHandle) -> bool {
        handle.to_string().ends_with("000000000001")
    }

    fn is_skewed_first(handle: &UploadHandle) -> bool {
        handle.to_string().ends_with("000000000000")
    }
}

impl QuarantineStore for ControlledQuarantineStore {
    fn create_exclusive(&self, object: &QuarantineObject) -> QuarantineOperation<()> {
        let key = object.storage_key().to_owned();
        let mut objects = lock_unpoisoned(&self.objects);
        if objects.insert(key.clone(), 0).is_some() {
            return QuarantineOperation::ready(Err(UploadError::new(
                UploadErrorKind::StorageConflict,
            )));
        }
        lock_unpoisoned(&self.unbound).push(key);
        QuarantineOperation::ready(Ok(()))
    }

    fn write_at(
        &self,
        object: &QuarantineObject,
        offset: u64,
        bytes: &[u8],
    ) -> QuarantineOperation<()> {
        let key = object.storage_key().to_owned();
        let handle = match lock_unpoisoned(&self.bindings).get(&key).cloned() {
            Some(handle) => handle,
            None => {
                return QuarantineOperation::ready(Err(UploadError::new(
                    UploadErrorKind::ProviderUnavailable,
                )));
            }
        };
        let end = offset.saturating_add(bytes.len() as u64);
        if let Some(length) = lock_unpoisoned(&self.objects).get_mut(&key) {
            *length = (*length).max(end);
        }
        let first = lock_unpoisoned(&self.first_writes).insert(key);
        if !first {
            return QuarantineOperation::ready(Ok(()));
        }

        if self.mutation == ResourceWorkloadMutation::SkewFirst && Self::is_skewed_second(&handle) {
            self.tracker.provider_started(&handle, 1, bytes.len());
            self.tracker.provider_finished(&handle);
            return QuarantineOperation::ready(Ok(()));
        }

        let extra = usize::from(
            self.mutation == ResourceWorkloadMutation::SkewFirst && Self::is_skewed_first(&handle),
        );
        let retained = (0..extra)
            .map(|_| QuarantineBytes::copy_from_slice(bytes))
            .collect::<Vec<_>>();
        self.tracker.provider_started(
            &handle,
            1 + retained.len(),
            bytes.len().saturating_mul(1 + retained.len()),
        );
        let (operation, completion) = QuarantineOperation::pending();
        let entered = Arc::clone(&self.entered);
        let release = Arc::clone(&self.release);
        let tracker = Arc::clone(&self.tracker);
        tokio::spawn(async move {
            entered.wait().await;
            release.wait().await;
            drop(retained);
            tracker.provider_finished(&handle);
            completion.complete(Ok(()));
        });
        operation
    }

    fn sync(&self, object: &QuarantineObject) -> QuarantineOperation<()> {
        let result = lock_unpoisoned(&self.objects)
            .contains_key(object.storage_key())
            .then_some(())
            .ok_or_else(|| UploadError::new(UploadErrorKind::ProviderUnavailable));
        QuarantineOperation::ready(result)
    }

    fn read_at(
        &self,
        object: &QuarantineObject,
        offset: u64,
        maximum_bytes: usize,
    ) -> QuarantineOperation<QuarantineBytes> {
        let length = lock_unpoisoned(&self.objects)
            .get(object.storage_key())
            .copied()
            .unwrap_or(0);
        let remaining = length.saturating_sub(offset).min(maximum_bytes as u64) as usize;
        QuarantineOperation::ready(Ok(QuarantineBytes::from(vec![0_u8; remaining])))
    }

    fn remove(&self, object: &QuarantineObject) -> QuarantineOperation<RemoveDisposition> {
        let disposition = if lock_unpoisoned(&self.objects)
            .remove(object.storage_key())
            .is_some()
        {
            RemoveDisposition::Removed
        } else {
            RemoveDisposition::AlreadyAbsent
        };
        QuarantineOperation::ready(Ok(disposition))
    }
}

#[derive(Clone)]
struct ResourceTransfer {
    checksum: UploadChecksum,
    grant: TransferGrant,
    handle: UploadHandle,
    idempotency_key: UploadIdempotencyKey,
}

async fn run_server_resource_workload(
    mutation: ResourceWorkloadMutation,
) -> Result<ServerResourceSnapshot, Box<dyn Error>> {
    let active_transfers = if mutation == ResourceWorkloadMutation::ExtraTransfer {
        ACTIVE_TRANSFERS + 1
    } else {
        ACTIVE_TRANSFERS
    };
    let limits = UploadLimits::new(UploadLimitConfig::reference())?;
    let authorization = Arc::new(ControlledUploadAuthorization::new());
    let context = component_support::trusted_context_with_upload_authorization(Arc::clone(
        &authorization,
    )
        as Arc<dyn suprnova_live::upload::UploadAuthorizationPort>);
    let ledger = Arc::new(MemoryUploadLedger::new(limits)?);
    let codec = grant_codec()?;
    let field = ModelField::parse("serial")?;
    let chunk = QuarantineBytes::from(vec![0_u8; CHUNK_BYTES]);
    let chunk_checksum = checksum(&chunk)?;
    let mut transfers = Vec::with_capacity(active_transfers);
    for index in 0..active_transfers {
        let handle = UploadHandle::parse(&format!("018f47c1-2af0-7cc4-b001-{index:012}"))?;
        let authority = TransferGrantScope::new(
            handle.clone(),
            context.mount().component().clone(),
            field.clone(),
            component_support::fixture_host_scope(),
            1,
        );
        let issued = codec.issue(
            TransferGrantRequest::new(authority.clone(), EXPIRES_AT),
            UnixMillis::new(1_000),
        )?;
        ledger.seed(UploadRecord::new(
            authority,
            UploadState::Transferring,
            UploadRevision::new(7),
            UnixMillis::new(1_000),
            EXPIRES_AT,
        )?)?;
        transfers.push(ResourceTransfer {
            checksum: chunk_checksum.clone(),
            grant: TransferGrant::parse(issued.grant().expose_bearer())?,
            handle,
            idempotency_key: UploadIdempotencyKey::parse(&format!("u4-resource-{index}"))?,
        });
    }
    let service_ledger: Arc<dyn UploadLedger> = ledger;
    let service = Arc::new(UploadService::new(service_ledger, codec, limits)?);
    let entered = Arc::new(tokio::sync::Barrier::new(active_transfers + 1));
    let release = Arc::new(tokio::sync::Barrier::new(active_transfers + 1));
    let tracker = Arc::new(ServerChunkTracker::default());
    let store = Arc::new(ControlledQuarantineStore::new(
        mutation,
        Arc::clone(&tracker),
        Arc::clone(&entered),
        Arc::clone(&release),
    ));
    let provider = Arc::new(QuarantinedFileProvider::new(Arc::clone(&store), limits)?);
    for transfer in &transfers {
        provider
            .prepare(PrepareTransfer::new(
                &transfer.handle,
                FILE_BYTES as u64,
                "u4-16.bin",
                NOW,
            ))
            .await?;
        store.bind_latest(&transfer.handle)?;
    }

    let mut tasks = Vec::with_capacity(active_transfers);
    for transfer in transfers.iter().cloned() {
        let provider = Arc::clone(&provider);
        let service = Arc::clone(&service);
        let tracker = Arc::clone(&tracker);
        let entered = Arc::clone(&entered);
        let release = Arc::clone(&release);
        let bytes = chunk.clone();
        tasks.push(tokio::spawn(async move {
            let permit = service
                .transfer_permits()
                .try_acquire()
                .map_err(|_| UploadError::new(UploadErrorKind::ResourceExhausted))?;
            tracker.body_started(&transfer.handle, bytes.len());
            let mut body = ControlledChunkBody {
                bytes,
                offset: 0,
                block_before_second_segment: mutation == ResourceWorkloadMutation::SkewFirst
                    && ControlledQuarantineStore::is_skewed_second(&transfer.handle),
                entered,
                release,
            };
            let result = provider
                .write_chunk(
                    WriteChunk::new(
                        &transfer.handle,
                        0,
                        0,
                        CHUNK_BYTES as u64,
                        &transfer.checksum,
                    ),
                    &mut body,
                )
                .await;
            tracker.body_finished(&transfer.handle);
            drop(permit);
            result.map(|receipt| (transfer, receipt))
        }));
    }

    entered.wait().await;
    let provider_status = provider.retirement_status();
    let chunk_buffers_by_transfer = tracker.snapshot();
    let categories = ServerManagerOwnedCategories {
        active_service_permits: service.transfer_permits().active(),
        provider_active_chunks: provider_status.active_chunks(),
        provider_active_descriptors: provider_status.active_descriptors(),
        provider_active_operations: provider_status.active_operations(),
        provider_control_records: 1,
        provider_owned_transfers: provider_status.owned_transfers(),
        retained_handle_bytes: transfers
            .iter()
            .map(|transfer| transfer.handle.to_string().len())
            .sum(),
        service_control_records: 1,
    };
    let manager_owned_bytes = estimate_server_manager_owned_bytes(categories);
    let live_chunk_buffers = chunk_buffers_by_transfer
        .iter()
        .map(|transfer| transfer.current_total_buffers)
        .sum();
    let retained_chunk_bytes = chunk_buffers_by_transfer
        .iter()
        .map(|transfer| transfer.current_bytes)
        .sum::<usize>();
    let snapshot = ServerResourceSnapshot {
        chunk_buffers_by_transfer,
        live_chunk_buffers,
        manager_owned_bytes,
        manager_owned_categories: categories,
        max_chunks_per_transfer: tracker
            .snapshot()
            .iter()
            .map(|transfer| transfer.total_high_water)
            .max()
            .unwrap_or(0),
        max_concurrent_transfers: categories.active_service_permits,
        max_queue_depth: categories.provider_owned_transfers,
        retained_bytes: retained_chunk_bytes.saturating_add(manager_owned_bytes),
    };
    release.wait().await;

    for task in tasks {
        let (transfer, receipt) = task.await??;
        if receipt.disposition() != ChunkDisposition::Stored {
            return Err(std::io::Error::other("resource provider did not store the chunk").into());
        }
        let transition = UploadTransitionRequest::new(
            transfer.handle.clone(),
            UploadRevision::new(7),
            transfer.idempotency_key,
            UploadTransition::PutChunk(AcceptedChunk::new(
                0,
                CHUNK_BYTES as u64,
                transfer.checksum,
            )?),
        );
        let outcome = service
            .transition(
                &context,
                UploadTransitionAdmission::new(transfer.grant, field.clone(), transition),
                NOW,
            )
            .await?;
        if outcome.disposition() != TransitionDisposition::Applied {
            return Err(
                std::io::Error::other("resource control transition was not applied").into(),
            );
        }
    }
    Ok(snapshot)
}

#[derive(Clone, Copy, Debug, Serialize)]
struct ServerManagerOwnedCategories {
    #[serde(rename = "activeServicePermits")]
    active_service_permits: usize,
    #[serde(rename = "providerActiveChunks")]
    provider_active_chunks: usize,
    #[serde(rename = "providerActiveDescriptors")]
    provider_active_descriptors: usize,
    #[serde(rename = "providerActiveOperations")]
    provider_active_operations: usize,
    #[serde(rename = "providerControlRecords")]
    provider_control_records: usize,
    #[serde(rename = "providerOwnedTransfers")]
    provider_owned_transfers: usize,
    #[serde(rename = "retainedHandleBytes")]
    retained_handle_bytes: usize,
    #[serde(rename = "serviceControlRecords")]
    service_control_records: usize,
}

fn estimate_server_manager_owned_bytes(categories: ServerManagerOwnedCategories) -> usize {
    // Counting model: 512 bytes per production service/provider control record,
    // 256 per provider-owned transfer, 128 per active service permit, provider
    // operation, descriptor, and chunk permit, plus exact UTF-8 handle bytes.
    // Physical body/store payloads are excluded and reported in retainedBytes.
    categories
        .service_control_records
        .saturating_mul(512)
        .saturating_add(categories.provider_control_records.saturating_mul(512))
        .saturating_add(categories.provider_owned_transfers.saturating_mul(256))
        .saturating_add(categories.active_service_permits.saturating_mul(128))
        .saturating_add(categories.provider_active_operations.saturating_mul(128))
        .saturating_add(categories.provider_active_descriptors.saturating_mul(128))
        .saturating_add(categories.provider_active_chunks.saturating_mul(128))
        .saturating_add(categories.retained_handle_bytes)
}

#[derive(Clone, Debug, Serialize)]
struct ServerTransferChunkBuffers {
    #[serde(rename = "bodyHighWater")]
    body_high_water: usize,
    #[serde(rename = "currentBodyBuffers")]
    current_body_buffers: usize,
    #[serde(rename = "currentBytes")]
    current_bytes: usize,
    #[serde(rename = "currentProviderBuffers")]
    current_provider_buffers: usize,
    #[serde(rename = "currentTotalBuffers")]
    current_total_buffers: usize,
    handle: String,
    #[serde(rename = "providerHighWater")]
    provider_high_water: usize,
    #[serde(rename = "totalHighWater")]
    total_high_water: usize,
    #[serde(rename = "totalHighWaterBytes")]
    total_high_water_bytes: usize,
}

#[derive(Clone, Debug)]
struct ServerResourceSnapshot {
    chunk_buffers_by_transfer: Vec<ServerTransferChunkBuffers>,
    live_chunk_buffers: usize,
    manager_owned_bytes: usize,
    manager_owned_categories: ServerManagerOwnedCategories,
    max_chunks_per_transfer: usize,
    max_concurrent_transfers: usize,
    max_queue_depth: usize,
    retained_bytes: usize,
}

impl ServerResourceSnapshot {
    fn exceeds_resource_ceiling(&self) -> bool {
        self.live_chunk_buffers > ACTIVE_TRANSFERS * 2
            || self.max_chunks_per_transfer > 2
            || self.manager_owned_bytes > 512 * 1024
    }
}

#[derive(Serialize)]
struct ServerBudgetResult {
    bounds: ServerBounds,
    environment: EnvironmentEvidence,
    measurements: ServerMeasurements,
    methodology: Methodology,
    workload: Workload,
}

#[derive(Serialize)]
struct ServerBounds {
    #[serde(rename = "maxChunksPerActiveTransfer")]
    max_chunks_per_active_transfer: usize,
    #[serde(rename = "maxControlP95Microseconds")]
    max_control_p95_microseconds: usize,
    #[serde(rename = "maxManagerOwnedBytes")]
    max_manager_owned_bytes: usize,
}

#[derive(Serialize)]
struct ServerMeasurements {
    #[serde(rename = "chunkBuffersByTransfer")]
    chunk_buffers_by_transfer: Vec<ServerTransferChunkBuffers>,
    #[serde(rename = "excludedCalls")]
    excluded_calls: ExcludedCalls,
    #[serde(rename = "liveChunkBuffers")]
    live_chunk_buffers: usize,
    #[serde(rename = "managerOwnedBytes")]
    manager_owned_bytes: usize,
    #[serde(rename = "managerOwnedCategories")]
    manager_owned_categories: ServerManagerOwnedCategories,
    #[serde(rename = "maxChunksPerTransfer")]
    max_chunks_per_transfer: usize,
    #[serde(rename = "maxConcurrentTransfers")]
    max_concurrent_transfers: usize,
    #[serde(rename = "maxQueueDepth")]
    max_queue_depth: usize,
    #[serde(rename = "p50Microseconds")]
    p50_microseconds: f64,
    #[serde(rename = "p95Microseconds")]
    p95_microseconds: f64,
    #[serde(rename = "retainedBytes")]
    retained_bytes: usize,
}

#[derive(Serialize)]
struct Methodology {
    #[serde(rename = "measuredSamples")]
    measured_samples: usize,
    #[serde(rename = "warmupIterations")]
    warmup_iterations: usize,
}

#[derive(Clone, Copy, Serialize)]
struct Workload {
    #[serde(rename = "activeTransfers")]
    active_transfers: usize,
    #[serde(rename = "chunkBytes")]
    chunk_bytes: usize,
    #[serde(rename = "fileBytes")]
    file_bytes: usize,
    files: usize,
}

#[derive(Serialize)]
struct EnvironmentEvidence {
    architecture: &'static str,
    classification: &'static str,
    #[serde(rename = "cpuGovernor")]
    cpu_governor: String,
    #[serde(rename = "cpuModel")]
    cpu_model: String,
    database: &'static str,
    #[serde(rename = "dedicatedVcpusAttested")]
    dedicated_vcpus_attested: bool,
    kernel: String,
    #[serde(rename = "loopbackProviders")]
    loopback_providers: bool,
    #[serde(rename = "memoryBytes")]
    memory_bytes: u64,
    #[serde(rename = "operatingSystem")]
    operating_system: &'static str,
    profile: &'static str,
    #[serde(rename = "qualificationRequirementsMet")]
    qualification_requirements_met: bool,
    rustc: String,
    #[serde(rename = "selectedCpuCount")]
    selected_cpu_count: usize,
    #[serde(rename = "warmFilesystemCache")]
    warm_filesystem_cache: bool,
}

impl EnvironmentEvidence {
    fn collect() -> Self {
        let affinity = read_labeled_value("/proc/self/status", "Cpus_allowed_list")
            .unwrap_or_else(|| "unavailable".to_owned());
        let selected_cpu_count = cpu_list(&affinity).len();
        let memory_bytes = read_labeled_value("/proc/meminfo", "MemTotal")
            .and_then(|value| value.split_whitespace().next()?.parse::<u64>().ok())
            .unwrap_or(0)
            .saturating_mul(1024);
        let cpu_governor = governors(&affinity);
        let dedicated = std::env::var("SUPRNOVA_LIVE_S1_DEDICATED").as_deref() == Ok("1");
        let requirements_met = std::env::consts::OS == "linux"
            && std::env::consts::ARCH == "x86_64"
            && selected_cpu_count == 8
            && memory_bytes >= 16 * 1024 * 1024 * 1024
            && cpu_governor == "performance"
            && dedicated;
        Self {
            architecture: std::env::consts::ARCH,
            classification: if requirements_met {
                "qualified"
            } else {
                "unqualified"
            },
            cpu_governor,
            cpu_model: read_cpu_model().unwrap_or_else(|| "unavailable".to_owned()),
            database: "not_used_by_upload_control_budget",
            dedicated_vcpus_attested: dedicated,
            kernel: command_output("uname", &["-sr"]).unwrap_or_else(|| "unavailable".to_owned()),
            loopback_providers: true,
            memory_bytes,
            operating_system: std::env::consts::OS,
            profile: "S1",
            qualification_requirements_met: requirements_met,
            rustc: command_output("rustc", &["--version"])
                .unwrap_or_else(|| "unavailable".to_owned()),
            selected_cpu_count,
            warm_filesystem_cache: true,
        }
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("upload framework budget failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    if cfg!(debug_assertions) {
        println!("upload framework budget debug contract check only; release timing skipped");
        return Ok(());
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run_async())
}

async fn run_async() -> Result<(), Box<dyn Error>> {
    let mut fixtures = Vec::with_capacity(WARMUP_ITERATIONS + MEASURED_SAMPLES);
    for index in 1..=WARMUP_ITERATIONS + MEASURED_SAMPLES {
        fixtures.push(Fixture::new(index)?);
    }
    let resources = run_server_resource_workload(ResourceWorkloadMutation::None).await?;
    for fixture in fixtures.iter_mut().take(WARMUP_ITERATIONS) {
        black_box(fixture.process_once().await?);
    }
    let mut samples = Vec::with_capacity(MEASURED_SAMPLES);
    for fixture in fixtures.iter_mut().skip(WARMUP_ITERATIONS) {
        let started = Instant::now();
        black_box(fixture.process_once().await?);
        samples.push(started.elapsed().as_secs_f64() * 1_000_000.0);
    }
    samples.sort_by(f64::total_cmp);
    let p50 = percentile(&samples, 0.50);
    let p95 = percentile(&samples, 0.95);
    let environment = EnvironmentEvidence::collect();
    let excluded_calls = fixtures
        .iter()
        .fold(ExcludedCalls::default(), |mut total, fixture| {
            let calls = fixture.excluded.counters.snapshot();
            total.application_validation += calls.application_validation;
            total.body_io += calls.body_io;
            total.provider += calls.provider;
            total.scanner += calls.scanner;
            total
        });
    if excluded_calls != ExcludedCalls::default() {
        return Err(std::io::Error::other("excluded work entered measured control samples").into());
    }
    let result = ServerBudgetResult {
        bounds: ServerBounds {
            max_chunks_per_active_transfer: 2,
            max_control_p95_microseconds: P95_CAP_MICROSECONDS as usize,
            max_manager_owned_bytes: 512 * 1024,
        },
        environment,
        measurements: ServerMeasurements {
            chunk_buffers_by_transfer: resources.chunk_buffers_by_transfer.clone(),
            excluded_calls,
            live_chunk_buffers: resources.live_chunk_buffers,
            manager_owned_bytes: resources.manager_owned_bytes,
            manager_owned_categories: resources.manager_owned_categories,
            max_chunks_per_transfer: resources.max_chunks_per_transfer,
            max_concurrent_transfers: resources.max_concurrent_transfers,
            max_queue_depth: resources.max_queue_depth,
            p50_microseconds: p50,
            p95_microseconds: p95,
            retained_bytes: resources.retained_bytes,
        },
        methodology: Methodology {
            measured_samples: MEASURED_SAMPLES,
            warmup_iterations: WARMUP_ITERATIONS,
        },
        workload: Workload {
            active_transfers: ACTIVE_TRANSFERS,
            chunk_bytes: CHUNK_BYTES,
            file_bytes: FILE_BYTES,
            files: FILES,
        },
    };
    write_result(&result, result_path())?;
    println!(
        "{WORKLOAD} upload control: p50={:.3}us p95={:.3}us environment={}",
        p50, p95, result.environment.classification
    );
    if p95 > P95_CAP_MICROSECONDS {
        return Err(std::io::Error::other("upload control exceeded the 2 ms p95 ceiling").into());
    }
    if resources.exceeds_resource_ceiling() {
        return Err(std::io::Error::other("upload server resource ceiling exceeded").into());
    }
    if std::env::var_os("SUPRNOVA_LIVE_REQUIRE_S1").is_some()
        && !result.environment.qualification_requirements_met
    {
        return Err(
            std::io::Error::other("runner cannot prove the required S1 environment").into(),
        );
    }
    Ok(())
}

fn percentile(samples: &[f64], quantile: f64) -> f64 {
    let index = ((samples.len() as f64 * quantile).ceil() as usize).saturating_sub(1);
    samples[index]
}

fn result_path() -> PathBuf {
    std::env::var_os("SUPRNOVA_LIVE_UPLOAD_SERVER_RESULT").map_or_else(
        || PathBuf::from("benchmarks/local/upload-server-v1.json"),
        PathBuf::from,
    )
}

fn write_result(result: &ServerBudgetResult, path: PathBuf) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = temporary_path(&path);
    fs::write(&temporary, serde_json::to_vec_pretty(result)?)?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut value = path.as_os_str().to_owned();
    value.push(".tmp");
    PathBuf::from(value)
}

fn read_labeled_value(path: &str, label: &str) -> Option<String> {
    fs::read_to_string(path).ok()?.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        (name.trim() == label).then(|| value.trim().to_owned())
    })
}

fn read_cpu_model() -> Option<String> {
    read_labeled_value("/proc/cpuinfo", "model name")
}

fn cpu_list(value: &str) -> BTreeSet<u32> {
    let mut cpus = BTreeSet::new();
    for part in value.split(',') {
        let mut bounds = part.trim().splitn(2, '-');
        let Some(start) = bounds.next().and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };
        let end = bounds
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(start);
        cpus.extend(start..=end);
    }
    cpus
}

fn governors(affinity: &str) -> String {
    let values = cpu_list(affinity)
        .into_iter()
        .filter_map(|cpu| {
            fs::read_to_string(format!(
                "/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_governor"
            ))
            .ok()
            .map(|value| value.trim().to_owned())
        })
        .collect::<BTreeSet<_>>();
    if values.is_empty() {
        "unavailable".to_owned()
    } else {
        values.into_iter().collect::<Vec<_>>().join(",")
    }
}

fn command_output(program: &str, arguments: &[&str]) -> Option<String> {
    let output = Command::new(program).args(arguments).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_owned())
}

#[cfg(test)]
#[allow(
    unused_imports,
    reason = "cargo bench compiles cfg(test) without the integration test harness"
)]
mod integrity_tests {
    use super::*;

    #[tokio::test]
    async fn selected_control_operation_keeps_all_external_ports_at_zero() {
        let mut fixture = Fixture::new(1).expect("fixture");
        fixture.process_once().await.expect("measured control path");
        assert_eq!(
            fixture.excluded.counters.snapshot(),
            ExcludedCalls::default()
        );
    }

    #[tokio::test]
    async fn wrong_operations_reach_every_injected_external_port() {
        let mut body = Fixture::new(2).expect("body fixture");
        body.dispatch(HarnessOperation::BodyIo)
            .await
            .expect("body mutation");
        assert_eq!(body.excluded.counters.snapshot().body_io, 1);

        let mut provider = Fixture::new(3).expect("provider fixture");
        provider
            .dispatch(HarnessOperation::Provider)
            .await
            .expect("provider mutation");
        assert_eq!(provider.excluded.counters.snapshot().provider, 1);

        let mut validation = Fixture::new(4).expect("validation fixture");
        validation
            .dispatch(HarnessOperation::Validation)
            .await
            .expect("validation mutation");
        let calls = validation.excluded.counters.snapshot();
        assert!(calls.provider > 0);
        assert_eq!(calls.scanner, 1);
        assert_eq!(calls.application_validation, 1);
    }

    #[tokio::test]
    async fn resource_measurement_comes_from_live_production_owners() {
        let snapshot = run_server_resource_workload(ResourceWorkloadMutation::None)
            .await
            .expect("four active transfers");

        assert_eq!(snapshot.max_concurrent_transfers, 4);
        assert_eq!(snapshot.live_chunk_buffers, 8);
        assert_eq!(snapshot.max_chunks_per_transfer, 2);
        assert_eq!(snapshot.chunk_buffers_by_transfer.len(), 4);
        assert!(
            snapshot
                .chunk_buffers_by_transfer
                .iter()
                .all(|transfer| transfer.total_high_water == 2)
        );
        assert!(snapshot.chunk_buffers_by_transfer.iter().all(|transfer| {
            transfer.current_body_buffers == 1
                && transfer.current_provider_buffers == 1
                && transfer.current_total_buffers == 2
                && transfer.current_bytes == CHUNK_BYTES + 64 * 1024
                && transfer.total_high_water_bytes == CHUNK_BYTES + 64 * 1024
        }));
        assert_eq!(snapshot.manager_owned_categories.active_service_permits, 4);
        assert_eq!(
            snapshot.manager_owned_categories.provider_active_operations,
            4
        );
        assert_eq!(
            snapshot
                .manager_owned_categories
                .provider_active_descriptors,
            4
        );
        assert_eq!(snapshot.manager_owned_categories.provider_active_chunks, 4);
        assert_eq!(
            snapshot.manager_owned_categories.provider_owned_transfers,
            4
        );
        assert_eq!(snapshot.manager_owned_bytes, 4_240);
        assert_eq!(
            snapshot.retained_bytes,
            ACTIVE_TRANSFERS * (CHUNK_BYTES + 64 * 1024) + 4_240
        );
        assert!(!snapshot.exceeds_resource_ceiling());
    }

    #[tokio::test]
    async fn one_skewed_transfer_fails_even_when_the_document_average_is_two() {
        let snapshot = run_server_resource_workload(ResourceWorkloadMutation::SkewFirst)
            .await
            .expect("skewed active transfers");

        assert_eq!(snapshot.live_chunk_buffers, 8);
        assert_eq!(snapshot.live_chunk_buffers / ACTIVE_TRANSFERS, 2);
        assert_eq!(snapshot.max_chunks_per_transfer, 3);
        assert_eq!(
            snapshot
                .chunk_buffers_by_transfer
                .iter()
                .map(|transfer| transfer.current_total_buffers)
                .collect::<Vec<_>>(),
            vec![3, 1, 2, 2]
        );
        assert_eq!(
            snapshot
                .chunk_buffers_by_transfer
                .iter()
                .map(|transfer| transfer.total_high_water)
                .collect::<Vec<_>>(),
            vec![3, 2, 2, 2]
        );
        assert_eq!(
            snapshot
                .chunk_buffers_by_transfer
                .iter()
                .map(|transfer| transfer.current_bytes)
                .collect::<Vec<_>>(),
            vec![
                CHUNK_BYTES + 2 * 64 * 1024,
                CHUNK_BYTES,
                CHUNK_BYTES + 64 * 1024,
                CHUNK_BYTES + 64 * 1024,
            ]
        );
        assert!(snapshot.exceeds_resource_ceiling());
    }

    #[tokio::test]
    async fn one_added_real_transfer_increases_every_manager_owner_count() {
        let four = run_server_resource_workload(ResourceWorkloadMutation::None)
            .await
            .expect("four active transfers");
        let five = run_server_resource_workload(ResourceWorkloadMutation::ExtraTransfer)
            .await
            .expect("five active transfers");

        assert_eq!(five.max_concurrent_transfers, 5);
        assert_eq!(five.manager_owned_categories.active_service_permits, 5);
        assert_eq!(five.manager_owned_categories.provider_active_operations, 5);
        assert_eq!(five.manager_owned_categories.provider_active_descriptors, 5);
        assert_eq!(five.manager_owned_categories.provider_active_chunks, 5);
        assert_eq!(five.manager_owned_categories.provider_owned_transfers, 5);
        assert!(five.manager_owned_bytes > four.manager_owned_bytes);
        assert!(five.retained_bytes > four.retained_bytes);
        assert!(five.max_queue_depth > four.max_queue_depth);
    }
}
