//! Bounded upload routes backed by the production provider contracts.

use std::collections::HashMap;
use std::future::{Future as _, poll_fn};
use std::sync::Arc;
use std::sync::Mutex as SyncMutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::task::Poll;

use axum::body::{Body, Bytes};
use http_body_util::BodyExt as _;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use suprnova_live::action::{
    ActionArgumentSchema, ActionAuthorizationPort, ActionAuthorizationRequest, ActionDispatchFn,
    ActionEntry, ActionError, ActionFuture, ActionTable, AuthorizationDecision,
    AuthorizationRequirement, TransactionPolicy,
};
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::host::{
    HostScopeFacts, MountCatalogBuilder, MountCatalogEntry, MountScopeRequirements, MountSelection,
    PrincipalFingerprint, ScopeRequirement, SessionFingerprint, TenantFingerprint,
    TrustedLiveRequestContext,
};
use suprnova_live::identity::{
    ActionName, BuildId, ComponentName, IslandSlot, KeyId, ModelField, RouteIdentity,
    ScopeFingerprint, UnixMillis, ViewName,
};
use suprnova_live::limits::{UploadLimitConfig, UploadLimits};
use suprnova_live::metadata::{ActionMetadata, ComponentMetadata, ContractVersions, FieldMetadata};
use suprnova_live::registry::{ComponentDescriptor, ComponentRegistryBuilder};
use suprnova_live::snapshot::state::{FieldCategory, FieldSpec, StateCodec, StateSchema};
use suprnova_live::snapshot::{ComponentContract, ExpectedSeedV1, SnapshotSchemaSet};
use suprnova_live::upload::{
    AcceptedChunk, ChunkBody, ChunkDisposition, ClientUploadMetadata, DetectedUploadType,
    DirectTransferInstruction, DirectUploadProvider, DurableUpload, DurableUploadId,
    FailedFinalize, FinalizeRequest, FinalizeToken, FinalizeUploadRequest, PrepareTransfer,
    PreparedFinalize, QuarantineBytes, QuarantinedFileProvider, ReportDirectPart,
    ReverseProxyUploadProvider, ScanDisposition, ScanFailurePolicy, ScanInput, TransferGrant,
    TransferGrantCodec, TransferGrantRequest, TransferGrantScope, TrustedProviderOrigin,
    UploadChecksum, UploadCreationRequest, UploadError, UploadErrorKind, UploadFieldPolicy,
    UploadFinalizationService, UploadFinalizer, UploadFuture, UploadHandle, UploadIdempotencyKey,
    UploadInspection, UploadProvider, UploadRejectionReason, UploadReplacementPolicy,
    UploadRevision, UploadScanPolicy, UploadScanner, UploadService, UploadState, UploadTransition,
    UploadTransitionAdmission, UploadTransitionRequest, UploadValidationDisposition,
    UploadValidationRequest, UploadValidationService, UploadValidationStore, ValidatedUpload,
    ValidationStoreDisposition, VerifyTransfer, WriteChunk,
};
use suprnova_live::validation::ValidationSelection;
use tokio::sync::{Mutex as AsyncMutex, Notify, watch};
use tokio::time::{Duration, timeout};

use crate::{
    ControlledUploadAuthorization, DirectProviderConformanceAdapter, MemoryUploadLedger,
    SyntheticLiveRequestContextBuilder, TokioFileQuarantineStore,
};

use super::faults::ReferenceFaultSchedule;
use super::{ResourceCounter, ResourceLease};

const CREATED_AT: UnixMillis = UnixMillis::new(1_000);
const UPLOAD_PAUSE_DEADLINE_STEPS: u64 = 1;
const MAX_BROWSER_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransferMode {
    File,
    Direct,
}

struct StoredUpload {
    handle: UploadHandle,
    field: ModelField,
    client: Option<ClientUploadMetadata>,
    policy: Option<UploadFieldPolicy>,
    expected_bytes: u64,
    received_bytes: u64,
    next_part: u32,
    state: UploadState,
    mode: TransferMode,
    grant: TransferGrant,
    revision: UploadRevision,
    whole_hasher: Sha256,
    direct_instruction: Option<DirectTransferInstruction>,
    direct_checksum: Option<UploadChecksum>,
    active_lease: Option<ResourceLease>,
}

enum UploadSlot {
    Ready(Box<StoredUpload>),
    Busy {
        handle: UploadHandle,
        mode: TransferMode,
    },
}

struct ReferenceActionAuthorization;

impl ActionAuthorizationPort for ReferenceActionAuthorization {
    fn authorize<'a>(
        &'a self,
        _request: ActionAuthorizationRequest<'a>,
    ) -> ActionFuture<'a, Result<AuthorizationDecision, ActionError>> {
        Box::pin(async { Ok(AuthorizationDecision::Allow) })
    }
}

struct UploadSlots {
    records: SyncMutex<HashMap<String, UploadSlot>>,
    retired: AtomicBool,
    changed: Notify,
}

struct UploadChunkRejection {
    handle: String,
    operation_generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UploadPausePoint {
    Chunk,
    Status,
    Complete,
    Cancel,
    Reacquire,
    Finalize,
    Expire,
}

struct UploadOperationPause {
    state: SyncMutex<UploadPauseState>,
    entered: Notify,
    changed: Notify,
    timers: Arc<ResourceCounter>,
}

struct UploadPauseState {
    selected: Option<UploadPauseSelection>,
    active: Option<UploadPauseSelection>,
    clock_step: u64,
    next_generation: u64,
    last_retired_generation: u64,
    retired: bool,
}

struct UploadPauseSelection {
    point: UploadPausePoint,
    handle: String,
    operation_generation: u64,
    control_generation: u64,
    deadline_step: u64,
    _timer_lease: ResourceLease,
}

struct ActivePauseGuard {
    pause: std::sync::Weak<UploadOperationPause>,
    generation: u64,
}

impl Drop for ActivePauseGuard {
    fn drop(&mut self) {
        if let Some(pause) = self.pause.upgrade() {
            pause.expire(self.generation);
        }
    }
}

impl UploadOperationPause {
    fn new(timers: Arc<ResourceCounter>) -> Self {
        Self {
            state: SyncMutex::new(UploadPauseState {
                selected: None,
                active: None,
                clock_step: 0,
                next_generation: 0,
                last_retired_generation: 0,
                retired: false,
            }),
            entered: Notify::new(),
            changed: Notify::new(),
            timers,
        }
    }

    fn select(
        self: &Arc<Self>,
        point: UploadPausePoint,
        handle: &str,
        operation_generation: u64,
    ) -> Result<u64, &'static str> {
        if handle.is_empty() || operation_generation == 0 {
            return Err("upload_pause_scope_invalid");
        }
        let timer_lease = self.timers.acquire().ok_or("upload_pause_retired")?;
        let generation = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.retired {
                return Err("upload_pause_retired");
            }
            if state.selected.is_some() || state.active.is_some() {
                return Err("upload_pause_in_use");
            }
            let generation = state
                .next_generation
                .checked_add(1)
                .filter(|generation| *generation <= MAX_BROWSER_SAFE_INTEGER)
                .ok_or("upload_pause_capacity_exceeded")?;
            let deadline_step = state
                .clock_step
                .checked_add(UPLOAD_PAUSE_DEADLINE_STEPS)
                .filter(|deadline| *deadline <= MAX_BROWSER_SAFE_INTEGER)
                .ok_or("upload_pause_capacity_exceeded")?;
            state.next_generation = generation;
            state.selected = Some(UploadPauseSelection {
                point,
                handle: handle.to_owned(),
                operation_generation,
                control_generation: generation,
                deadline_step,
                _timer_lease: timer_lease,
            });
            generation
        };
        Ok(generation)
    }

    fn advance_clock(&self) -> Result<(), &'static str> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.retired {
            return Err("upload_pause_retired");
        }
        if state.selected.is_none() && state.active.is_none() {
            return Err("upload_pause_clock_idle");
        }
        state.clock_step = state
            .clock_step
            .checked_add(UPLOAD_PAUSE_DEADLINE_STEPS)
            .filter(|step| *step <= MAX_BROWSER_SAFE_INTEGER)
            .ok_or("upload_pause_capacity_exceeded")?;
        let current_step = state.clock_step;
        let selected = state
            .selected
            .as_ref()
            .is_some_and(|selection| selection.deadline_step <= current_step);
        let active = state
            .active
            .as_ref()
            .is_some_and(|selection| selection.deadline_step <= current_step);
        let retired_generation = state
            .selected
            .as_ref()
            .filter(|_| selected)
            .or_else(|| state.active.as_ref().filter(|_| active))
            .map_or(0, |selection| selection.control_generation);
        if selected {
            drop(state.selected.take());
        }
        if active {
            drop(state.active.take());
        }
        state.last_retired_generation = state.last_retired_generation.max(retired_generation);
        drop(state);
        self.changed.notify_waiters();
        Ok(())
    }

    async fn pause_if_selected(
        self: &Arc<Self>,
        point: UploadPausePoint,
        handle: &str,
        operation_generation: u64,
    ) {
        let generation = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            let selected = state.selected.as_ref().filter(|selected| {
                selected.point == point
                    && selected.handle == handle
                    && selected.operation_generation == operation_generation
            });
            let Some(generation) = selected.map(|selected| selected.control_generation) else {
                return;
            };
            state.active = state.selected.take();
            generation
        };
        let guard = ActivePauseGuard {
            pause: Arc::downgrade(self),
            generation,
        };
        self.entered.notify_waiters();
        loop {
            let changed = self.changed.notified();
            if !self.is_active(generation) {
                break;
            }
            changed.await;
        }
        drop(guard);
    }

    fn resume(&self, generation: u64) -> Result<(), &'static str> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let selected = state
            .selected
            .as_ref()
            .is_some_and(|selected| selected.control_generation == generation);
        let active = state
            .active
            .as_ref()
            .is_some_and(|active| active.control_generation == generation);
        if !selected && !active {
            return Err(if generation <= state.last_retired_generation {
                "upload_pause_generation_stale"
            } else {
                "upload_pause_generation_invalid"
            });
        }
        if selected {
            drop(state.selected.take());
        }
        if active {
            drop(state.active.take());
        }
        state.last_retired_generation = state.last_retired_generation.max(generation);
        drop(state);
        self.changed.notify_waiters();
        Ok(())
    }

    fn expire(&self, generation: u64) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let selected = state
            .selected
            .as_ref()
            .is_some_and(|selected| selected.control_generation == generation);
        let active = state
            .active
            .as_ref()
            .is_some_and(|active| active.control_generation == generation);
        if selected {
            drop(state.selected.take());
        }
        if active {
            drop(state.active.take());
        }
        if selected || active {
            state.last_retired_generation = state.last_retired_generation.max(generation);
        }
        drop(state);
        self.changed.notify_waiters();
    }

    fn is_active(&self, generation: u64) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .active
            .as_ref()
            .is_some_and(|active| active.control_generation == generation)
    }

    fn active_count(&self) -> usize {
        usize::from(
            self.state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .active
                .is_some(),
        )
    }

    fn has_authority(&self) -> bool {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.selected.is_some() || state.active.is_some()
    }

    async fn wait_until_entered(&self, generation: u64) {
        loop {
            let entered = self.entered.notified();
            if self.is_active(generation) {
                return;
            }
            entered.await;
        }
    }

    fn retire(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.retired = true;
        if let Some(selected) = state.selected.take() {
            state.last_retired_generation = state
                .last_retired_generation
                .max(selected.control_generation);
        }
        if let Some(active) = state.active.take() {
            state.last_retired_generation =
                state.last_retired_generation.max(active.control_generation);
        }
        drop(state);
        self.changed.notify_waiters();
    }
}

struct UploadOperation {
    slots: Arc<UploadSlots>,
    key: String,
    upload: Option<StoredUpload>,
}

impl UploadOperation {
    fn upload(&self) -> &StoredUpload {
        self.upload.as_ref().expect("active upload operation")
    }

    fn upload_mut(&mut self) -> &mut StoredUpload {
        self.upload.as_mut().expect("active upload operation")
    }

    fn pause_scope(&self) -> (String, u64) {
        (
            self.upload().handle.to_string(),
            self.upload().revision.get(),
        )
    }
}

impl Drop for UploadOperation {
    fn drop(&mut self) {
        let Some(upload) = self.upload.take() else {
            return;
        };
        let mut records = self
            .slots
            .records
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !self.slots.retired.load(Ordering::Acquire) {
            records.insert(self.key.clone(), UploadSlot::Ready(Box::new(upload)));
        }
        drop(records);
        self.slots.changed.notify_waiters();
    }
}

#[derive(Deserialize)]
pub(super) struct CreateUploadRequest {
    field: String,
    filename: String,
    content_type: String,
    expected_bytes: u64,
    mode: String,
}

#[derive(Deserialize)]
pub(super) struct CompleteUploadRequest {
    grant: String,
}

pub(super) enum CompleteUploadOutcome {
    Ready(Value),
    Rejected {
        reason: UploadRejectionReason,
        status: Value,
    },
    Retry {
        reason: UploadRejectionReason,
        status: Value,
    },
}

impl CompleteUploadOutcome {
    #[cfg(test)]
    fn status(&self) -> &Value {
        match self {
            Self::Ready(status) | Self::Rejected { status, .. } | Self::Retry { status, .. } => {
                status
            }
        }
    }
}

#[derive(Deserialize)]
pub(super) struct FinalizeUploadRequestBody {
    pub(super) handle: String,
    pub(super) ready_revision: u64,
}

#[derive(Debug, Serialize)]
pub(super) struct UploadRaceOutcome {
    pub(super) disposition: &'static str,
    pub(super) terminal_state: &'static str,
    pub(super) accepted_outcomes: usize,
    pub(super) active_uploads: usize,
}

#[derive(Default)]
struct ReferenceValidationStore {
    evidence: SyncMutex<HashMap<String, ValidatedUpload>>,
}

impl UploadValidationStore for ReferenceValidationStore {
    fn put<'a>(
        &'a self,
        evidence: ValidatedUpload,
    ) -> UploadFuture<'a, Result<ValidationStoreDisposition, UploadError>> {
        Box::pin(async move {
            let key = evidence.handle().to_string();
            let mut stored = self
                .evidence
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            match stored.get(&key) {
                Some(existing) if existing == &evidence => {
                    Ok(ValidationStoreDisposition::ExistingOutcome)
                }
                Some(_) => Err(UploadError::new(UploadErrorKind::UploadConflict)),
                None => {
                    stored.insert(key, evidence);
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
            Ok(self
                .evidence
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .get(&upload.to_string())
                .cloned())
        })
    }

    fn remove<'a>(&'a self, upload: &'a UploadHandle) -> UploadFuture<'a, Result<(), UploadError>> {
        Box::pin(async move {
            self.evidence
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .remove(&upload.to_string());
            Ok(())
        })
    }
}

struct ReferenceFinalizer {
    durable: SyncMutex<HashMap<String, DurableUpload>>,
    ledger: Arc<MemoryUploadLedger>,
    operation_pause: Arc<UploadOperationPause>,
    fault: SyncMutex<Option<ArmedReferenceFinalizeFault>>,
    commit_calls: AtomicUsize,
    compensation_calls: AtomicUsize,
    reconciliation_calls: AtomicUsize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReferenceFinalizeFault {
    CommitUnavailable,
    LedgerAfterDurableCommit,
}

struct ArmedReferenceFinalizeFault {
    fault: ReferenceFinalizeFault,
    handle: UploadHandle,
    ready_revision: UploadRevision,
}

impl ArmedReferenceFinalizeFault {
    fn targets(&self, prepared: &PreparedFinalize) -> bool {
        self.handle == *prepared.handle() && self.ready_revision == prepared.ready_revision()
    }
}

#[derive(Default)]
struct ReferenceScanner {
    timeouts: SyncMutex<HashMap<String, UploadRevision>>,
    scan_calls: AtomicUsize,
}

impl UploadScanner for ReferenceScanner {
    fn scan<'a>(
        &'a self,
        input: ScanInput<'a>,
    ) -> UploadFuture<'a, Result<ScanDisposition, UploadError>> {
        Box::pin(async move {
            self.scan_calls.fetch_add(1, Ordering::SeqCst);
            let prefix = input.content().read(0, 12).await?;
            if prefix.len() > 12 || input.deadline() <= input.started_at() {
                return Err(UploadError::new(UploadErrorKind::InvalidField));
            }
            let handle = input.upload().handle().to_string();
            let timed_out = self
                .timeouts
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .remove(&handle)
                .is_some();
            Ok(if timed_out {
                ScanDisposition::TimedOut
            } else {
                ScanDisposition::Clean
            })
        })
    }
}

impl UploadFinalizer for ReferenceFinalizer {
    fn prepare<'a>(
        &'a self,
        request: FinalizeRequest<'a>,
    ) -> UploadFuture<'a, Result<PreparedFinalize, UploadError>> {
        Box::pin(async move {
            self.operation_pause
                .pause_if_selected(
                    UploadPausePoint::Finalize,
                    &request.evidence().handle().to_string(),
                    request.evidence().ready_revision().get(),
                )
                .await;
            let token = FinalizeToken::parse(&format!("prepared-{}", request.evidence().handle()))?;
            Ok(PreparedFinalize::new(&request, token))
        })
    }

    fn commit<'a>(
        &'a self,
        prepared: PreparedFinalize,
    ) -> UploadFuture<'a, Result<DurableUpload, UploadError>> {
        Box::pin(async move {
            self.commit_calls.fetch_add(1, Ordering::SeqCst);
            let fault = {
                let mut armed = self.fault.lock().unwrap_or_else(|error| error.into_inner());
                if armed
                    .as_ref()
                    .is_some_and(|candidate| candidate.targets(&prepared))
                {
                    armed.take().map(|candidate| candidate.fault)
                } else {
                    None
                }
            };
            if fault == Some(ReferenceFinalizeFault::CommitUnavailable) {
                return Err(UploadError::new(UploadErrorKind::ProviderUnavailable));
            }
            let durable = DurableUpload::new(
                &prepared,
                DurableUploadId::parse(&format!("durable-{}", prepared.handle()))?,
            );
            let key = durable.handle().to_string();
            let mut stored = self
                .durable
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let outcome = match stored.get(&key) {
                Some(existing) if existing == &durable => Ok(existing.clone()),
                Some(_) => Err(UploadError::new(UploadErrorKind::UploadConflict)),
                None => {
                    stored.insert(key, durable.clone());
                    Ok(durable)
                }
            }?;
            drop(stored);
            if fault == Some(ReferenceFinalizeFault::LedgerAfterDurableCommit) {
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
            Ok(())
        })
    }

    fn reconcile<'a>(
        &'a self,
        request: FinalizeRequest<'a>,
    ) -> UploadFuture<'a, Result<Option<DurableUpload>, UploadError>> {
        Box::pin(async move {
            self.reconciliation_calls.fetch_add(1, Ordering::SeqCst);
            Ok(self
                .durable
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .get(&request.evidence().handle().to_string())
                .cloned())
        })
    }
}

pub(super) struct UploadRuntime {
    limits: UploadLimits,
    file: Arc<QuarantinedFileProvider<TokioFileQuarantineStore>>,
    direct: Arc<DirectProviderConformanceAdapter>,
    ledger: Arc<MemoryUploadLedger>,
    service: Arc<UploadService>,
    validation: Arc<ReferenceValidationStore>,
    validation_service: UploadValidationService,
    scanner: Arc<ReferenceScanner>,
    finalization: UploadFinalizationService,
    finalizer: Arc<ReferenceFinalizer>,
    grants: TransferGrantCodec,
    context: TrustedLiveRequestContext,
    scope: HostScopeFacts,
    uploads: Arc<UploadSlots>,
    service_calls: AtomicUsize,
    grant_sequence: AtomicU64,
    fault: ReferenceFaultSchedule,
    fault_applied: AtomicBool,
    shutdown: watch::Receiver<bool>,
    active_uploads: Arc<ResourceCounter>,
    creation_window_gate: AsyncMutex<()>,
    operation_pause: Arc<UploadOperationPause>,
    chunk_rejection: SyncMutex<Option<UploadChunkRejection>>,
}

impl UploadRuntime {
    pub(super) async fn open(
        root: &std::path::Path,
        fault: ReferenceFaultSchedule,
        shutdown: watch::Receiver<bool>,
        active_uploads: Arc<ResourceCounter>,
        open_timers: Arc<ResourceCounter>,
    ) -> Result<Self, String> {
        let limits = UploadLimits::new(UploadLimitConfig::reference())
            .map_err(|error| format!("upload limits: {error:?}"))?;
        let store = Arc::new(
            TokioFileQuarantineStore::open(
                root,
                limits.max_pending_per_scope(),
                limits.max_chunk_bytes(),
            )
            .await
            .map_err(|error| format!("quarantine store: {error:?}"))?,
        );
        let file = Arc::new(
            QuarantinedFileProvider::new(store, limits)
                .map_err(|error| format!("file provider: {error:?}"))?,
        );
        let origin = TrustedProviderOrigin::parse("https://uploads.example.test")
            .map_err(|error| format!("direct origin: {error:?}"))?;
        let direct = Arc::new(
            DirectProviderConformanceAdapter::new(limits, origin)
                .map_err(|error| format!("direct provider: {error:?}"))?,
        );
        let authorization = Arc::new(ControlledUploadAuthorization::new());
        let (context, scope) = reference_upload_context(authorization)
            .map_err(|error| format!("upload context: {error}"))?;
        let ledger = Arc::new(
            MemoryUploadLedger::new(limits).map_err(|error| format!("upload ledger: {error:?}"))?,
        );
        let service_ledger: Arc<dyn suprnova_live::upload::UploadLedger> = ledger.clone();
        let service = Arc::new(
            UploadService::new(service_ledger, grant_codec(), limits)
                .map_err(|error| format!("upload service: {error:?}"))?,
        );
        let validation = Arc::new(ReferenceValidationStore::default());
        let scanner = Arc::new(ReferenceScanner::default());
        let validation_service = UploadValidationService::new(
            Arc::clone(&service),
            file.clone(),
            validation.clone(),
            Some(scanner.clone()),
            None,
            limits,
        )
        .map_err(|error| format!("upload validation: {error:?}"))?;
        let operation_pause = Arc::new(UploadOperationPause::new(open_timers));
        let finalizer = Arc::new(ReferenceFinalizer {
            durable: SyncMutex::new(HashMap::new()),
            ledger: Arc::clone(&ledger),
            operation_pause: Arc::clone(&operation_pause),
            fault: SyncMutex::new(None),
            commit_calls: AtomicUsize::new(0),
            compensation_calls: AtomicUsize::new(0),
            reconciliation_calls: AtomicUsize::new(0),
        });
        let finalization = UploadFinalizationService::new(
            Arc::clone(&service),
            validation.clone(),
            finalizer.clone(),
        );
        Ok(Self {
            limits,
            file,
            direct,
            ledger,
            service,
            validation,
            validation_service,
            scanner,
            finalization,
            finalizer,
            grants: grant_codec(),
            context,
            scope,
            uploads: Arc::new(UploadSlots {
                records: SyncMutex::new(HashMap::new()),
                retired: AtomicBool::new(false),
                changed: Notify::new(),
            }),
            service_calls: AtomicUsize::new(0),
            grant_sequence: AtomicU64::new(0),
            fault,
            fault_applied: AtomicBool::new(false),
            shutdown,
            active_uploads,
            creation_window_gate: AsyncMutex::new(()),
            operation_pause,
            chunk_rejection: SyncMutex::new(None),
        })
    }

    pub(super) async fn create(&self, request: CreateUploadRequest) -> Result<Value, UploadError> {
        let _creation_window_guard = self.creation_window_gate.lock().await;
        self.record_call();
        if request.field.is_empty()
            || request.field.len() > 64
            || request.filename.is_empty()
            || request.filename.len() > 1_024
            || request.content_type.is_empty()
            || request.content_type.len() > 255
            || request.expected_bytes > self.limits.max_file_bytes()
        {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        }
        let mode = match request.mode.as_str() {
            "file" => TransferMode::File,
            "direct" => TransferMode::Direct,
            _ => return Err(UploadError::new(UploadErrorKind::InvalidField)),
        };
        let active_lease = self
            .active_uploads
            .acquire()
            .ok_or_else(|| UploadError::new(UploadErrorKind::UploadConflict))?;
        let mut handle_bytes = [0_u8; 16];
        getrandom::fill(&mut handle_bytes)
            .map_err(|_| UploadError::new(UploadErrorKind::RandomUnavailable))?;
        handle_bytes[6] = (handle_bytes[6] & 0x0f) | 0x40;
        handle_bytes[8] = (handle_bytes[8] & 0x3f) | 0x80;
        let encoded = format!(
            "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            handle_bytes[0],
            handle_bytes[1],
            handle_bytes[2],
            handle_bytes[3],
            handle_bytes[4],
            handle_bytes[5],
            handle_bytes[6],
            handle_bytes[7],
            handle_bytes[8],
            handle_bytes[9],
            handle_bytes[10],
            handle_bytes[11],
            handle_bytes[12],
            handle_bytes[13],
            handle_bytes[14],
            handle_bytes[15],
        );
        let handle = UploadHandle::parse(&encoded)?;
        let field = ModelField::parse(&request.field)
            .map_err(|_| UploadError::new(UploadErrorKind::InvalidField))?;
        let client = ClientUploadMetadata::new(&request.filename, Some(&request.content_type)).ok();
        let policy = reference_upload_policy(&request.filename, &request.content_type).ok();
        let created = self
            .service
            .create(
                &self.context,
                UploadCreationRequest::new(
                    handle.clone(),
                    field.clone(),
                    idempotency(&format!("create-{}", &encoded[..12]))?,
                    UnixMillis::new(60_000),
                ),
                CREATED_AT,
            )
            .await?;
        let grant = created.grant().clone();
        let state = created.record().state();
        let revision = created.record().revision();
        let plan = match mode {
            TransferMode::File => {
                self.file
                    .prepare(PrepareTransfer::new(
                        &handle,
                        request.expected_bytes,
                        &request.filename,
                        CREATED_AT,
                    ))
                    .await?
            }
            TransferMode::Direct => {
                self.direct
                    .prepare(PrepareTransfer::new(
                        &handle,
                        request.expected_bytes,
                        &request.filename,
                        CREATED_AT,
                    ))
                    .await?
            }
        };
        let direct_instruction = plan
            .instructions()
            .find_map(|instruction| instruction.as_direct())
            .cloned();
        let instruction = direct_instruction.as_ref().map(direct_instruction_json);
        let stored = StoredUpload {
            handle: handle.clone(),
            field,
            client,
            policy,
            expected_bytes: request.expected_bytes,
            received_bytes: 0,
            next_part: 0,
            state,
            mode,
            grant: grant.clone(),
            revision,
            whole_hasher: Sha256::new(),
            direct_instruction,
            direct_checksum: None,
            active_lease: Some(active_lease),
        };
        let insert_conflict = {
            let mut uploads = self
                .uploads
                .records
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if self.uploads.retired.load(Ordering::Acquire) || uploads.contains_key(&encoded) {
                true
            } else {
                uploads.insert(encoded.clone(), UploadSlot::Ready(Box::new(stored)));
                false
            }
        };
        if insert_conflict {
            match mode {
                TransferMode::File => self.file.cancel(&handle).await?,
                TransferMode::Direct => self.direct.cancel(&handle).await?,
            }
            return Err(UploadError::new(UploadErrorKind::UploadConflict));
        }
        let mut response = json!({
            "handle": encoded,
            "grant": grant.expose_bearer(),
            "state": state.as_str(),
            "revision": revision.get(),
        });
        if let Some(instruction) = instruction {
            response["instruction"] = instruction;
        }
        Ok(response)
    }

    pub(super) async fn write_chunk(
        &self,
        handle: &str,
        part: u32,
        grant: &str,
        checksum: &str,
        content_length: Option<u64>,
        body: Body,
    ) -> Result<Value, UploadError> {
        self.record_call();
        let mut operation = self.take_upload(handle)?;
        let (pause_handle, pause_generation) = operation.pause_scope();
        self.operation_pause
            .pause_if_selected(UploadPausePoint::Chunk, &pause_handle, pause_generation)
            .await;
        if self.consume_chunk_rejection(&pause_handle, pause_generation) {
            return Err(UploadError::new(UploadErrorKind::InputTooLarge));
        }
        let result = self
            .write_chunk_inner(
                operation.upload_mut(),
                part,
                grant,
                checksum,
                content_length,
                body,
            )
            .await;
        result.map(|()| status_json(operation.upload()))
    }

    async fn write_chunk_inner(
        &self,
        upload: &mut StoredUpload,
        part: u32,
        grant: &str,
        checksum: &str,
        content_length: Option<u64>,
        body: Body,
    ) -> Result<(), UploadError> {
        self.require_current_grant(upload, grant)?;
        if upload.mode != TransferMode::File
            || upload.state.is_terminal()
            || part != upload.next_part
        {
            return Err(UploadError::new(UploadErrorKind::UploadConflict));
        }
        if upload.state == UploadState::Created {
            self.transition(upload, UploadTransition::Queue, "queue")
                .await?;
            self.transition(upload, UploadTransition::BeginTransfer, "begin-transfer")
                .await?;
        }
        if upload.state != UploadState::Transferring {
            return Err(UploadError::new(UploadErrorKind::InvalidTransition));
        }
        let remaining = upload.expected_bytes.saturating_sub(upload.received_bytes);
        let size = content_length.unwrap_or(remaining);
        if size == 0 || size > remaining || size > self.limits.max_chunk_bytes() as u64 {
            return Err(UploadError::new(UploadErrorKind::InputTooLarge));
        }
        let checksum = UploadChecksum::parse(checksum)?;
        let interrupt_after_first = self.fault == ReferenceFaultSchedule::UploadBodyInterruptedOnce
            && !self.fault_applied.swap(true, Ordering::SeqCst);
        let mut chunks = AxumChunkBody::new(
            body,
            upload.whole_hasher.clone(),
            interrupt_after_first,
            usize::try_from(size).map_err(|_| UploadError::new(UploadErrorKind::InputTooLarge))?,
            self.shutdown.clone(),
        );
        let receipt = self
            .file
            .write_chunk(
                WriteChunk::new(&upload.handle, part, upload.received_bytes, size, &checksum),
                &mut chunks,
            )
            .await?;
        upload.whole_hasher = chunks.into_hasher();
        upload.received_bytes = upload
            .received_bytes
            .checked_add(receipt.bytes())
            .ok_or_else(|| UploadError::new(UploadErrorKind::InputTooLarge))?;
        upload.next_part += 1;
        self.transition(
            upload,
            UploadTransition::PutChunk(AcceptedChunk::new(
                part,
                receipt.bytes(),
                checksum.clone(),
            )?),
            &format!("put-{part}"),
        )
        .await?;
        Ok(())
    }

    pub(super) fn store_direct_part(&self, handle: &str, bytes: &[u8]) -> Result<(), UploadError> {
        self.record_call();
        let mut operation = self.take_upload(handle)?;
        let upload = operation.upload_mut();
        let instruction = upload
            .direct_instruction
            .as_ref()
            .filter(|_| upload.mode == TransferMode::Direct && !upload.state.is_terminal())
            .ok_or_else(|| UploadError::new(UploadErrorKind::UploadConflict))?;
        let disposition = self
            .direct
            .store_part_for_test(instruction, bytes, CREATED_AT)?;
        if disposition == ChunkDisposition::Stored {
            upload.whole_hasher.update(bytes);
            upload.direct_checksum =
                Some(UploadChecksum::parse(&hex_digest(Sha256::digest(bytes)))?);
        }
        Ok(())
    }

    pub(super) fn store_direct_capability(
        &self,
        endpoint: &str,
        part: u32,
        reference: &str,
        required_part_header: &str,
        bytes: &[u8],
    ) -> Result<(), UploadError> {
        self.record_call();
        let handle = {
            let records = self
                .uploads
                .records
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            records.iter().find_map(|(handle, slot)| match slot {
                UploadSlot::Ready(upload)
                    if upload
                        .direct_instruction
                        .as_ref()
                        .is_some_and(|instruction| instruction.endpoint().as_str() == endpoint) =>
                {
                    Some(handle.clone())
                }
                UploadSlot::Ready(_) | UploadSlot::Busy { .. } => None,
            })
        }
        .ok_or_else(|| UploadError::new(UploadErrorKind::UploadConflict))?;
        let mut operation = self.take_upload(&handle)?;
        let upload = operation.upload_mut();
        let instruction = upload
            .direct_instruction
            .as_ref()
            .filter(|instruction| {
                upload.mode == TransferMode::Direct
                    && !upload.state.is_terminal()
                    && instruction.part().index() == part
                    && instruction.reference().as_str() == reference
                    && instruction.required_headers().iter().any(|(name, value)| {
                        name.as_str() == "x-suprnova-part" && value == required_part_header
                    })
            })
            .ok_or_else(|| UploadError::new(UploadErrorKind::ScopeMismatch))?;
        let disposition = self
            .direct
            .store_part_for_test(instruction, bytes, CREATED_AT)?;
        if disposition == ChunkDisposition::Stored {
            upload.whole_hasher.update(bytes);
            upload.direct_checksum =
                Some(UploadChecksum::parse(&hex_digest(Sha256::digest(bytes)))?);
        }
        Ok(())
    }

    pub(super) async fn report_direct_part(
        &self,
        handle: &str,
        request: CompleteUploadRequest,
    ) -> Result<Value, UploadError> {
        self.record_call();
        let mut operation = self.take_upload(handle)?;
        self.require_current_grant(operation.upload(), &request.grant)?;
        let upload = operation.upload_mut();
        let instruction = upload
            .direct_instruction
            .clone()
            .filter(|_| upload.mode == TransferMode::Direct && !upload.state.is_terminal())
            .ok_or_else(|| UploadError::new(UploadErrorKind::UploadConflict))?;
        let checksum = upload
            .direct_checksum
            .clone()
            .ok_or_else(|| UploadError::new(UploadErrorKind::IncompleteTransfer))?;
        let receipt = self
            .direct
            .report_part(ReportDirectPart::new(
                &upload.handle,
                instruction.part().clone(),
                instruction.reference().clone(),
                CREATED_AT,
            ))
            .await?;
        if upload.state == UploadState::Created {
            self.transition(upload, UploadTransition::Queue, "direct-queue")
                .await?;
            self.transition(
                upload,
                UploadTransition::BeginTransfer,
                "direct-begin-transfer",
            )
            .await?;
        }
        if receipt.index() != upload.next_part || receipt.offset() != upload.received_bytes {
            return Err(UploadError::new(UploadErrorKind::UploadConflict));
        }
        upload.received_bytes = upload
            .received_bytes
            .checked_add(receipt.bytes())
            .ok_or_else(|| UploadError::new(UploadErrorKind::InputTooLarge))?;
        upload.next_part = upload
            .next_part
            .checked_add(1)
            .ok_or_else(|| UploadError::new(UploadErrorKind::ResourceExhausted))?;
        self.transition(
            upload,
            UploadTransition::PutChunk(AcceptedChunk::new(
                receipt.index(),
                receipt.bytes(),
                checksum,
            )?),
            &format!("direct-put-{}", receipt.index()),
        )
        .await?;
        upload.direct_instruction = receipt
            .next_instruction()
            .and_then(|next| next.as_direct())
            .cloned();
        upload.direct_checksum = None;
        let mut response = status_json(upload);
        response["receipt"] = json!({
            "part": receipt.index(),
            "offset": receipt.offset(),
            "bytes": receipt.bytes(),
            "disposition": match receipt.disposition() {
                ChunkDisposition::Stored => "stored",
                ChunkDisposition::ExistingOutcome => "existing_outcome",
            },
        });
        if let Some(instruction) = upload.direct_instruction.as_ref() {
            response["instruction"] = direct_instruction_json(instruction);
        }
        Ok(response)
    }

    pub(super) async fn status(&self, handle: &str, grant: &str) -> Result<Value, UploadError> {
        self.record_call();
        let operation = self.take_upload(handle)?;
        let (pause_handle, pause_generation) = operation.pause_scope();
        self.operation_pause
            .pause_if_selected(UploadPausePoint::Status, &pause_handle, pause_generation)
            .await;
        let presented = self.require_current_grant(operation.upload(), grant)?;
        self.service
            .status(
                &self.context,
                presented,
                operation.upload().field.clone(),
                operation.upload().handle.clone(),
                CREATED_AT,
            )
            .await
            .map(|_| status_json(operation.upload()))
    }

    pub(super) async fn complete(
        &self,
        handle: &str,
        request: CompleteUploadRequest,
    ) -> Result<CompleteUploadOutcome, UploadError> {
        self.record_call();
        let mut operation = self.take_upload(handle)?;
        let (pause_handle, pause_generation) = operation.pause_scope();
        self.operation_pause
            .pause_if_selected(UploadPausePoint::Complete, &pause_handle, pause_generation)
            .await;
        self.complete_inner(operation.upload_mut(), &request.grant)
            .await
    }

    async fn complete_inner(
        &self,
        upload: &mut StoredUpload,
        grant: &str,
    ) -> Result<CompleteUploadOutcome, UploadError> {
        self.require_current_grant(upload, grant)?;
        if upload.received_bytes != upload.expected_bytes {
            return Err(UploadError::new(UploadErrorKind::UploadConflict));
        }
        if upload.state == UploadState::Ready {
            drop(upload.active_lease.take());
            return Ok(CompleteUploadOutcome::Ready(status_json(upload)));
        }
        if upload.state != UploadState::Transferring && upload.state != UploadState::Verifying {
            return Err(UploadError::new(UploadErrorKind::UploadConflict));
        }
        let checksum = UploadChecksum::parse(&hex_digest(upload.whole_hasher.clone().finalize()))?;
        if upload.state == UploadState::Transferring {
            self.transition(upload, UploadTransition::Complete, "complete")
                .await?;
        }
        let client = upload
            .client
            .clone()
            .ok_or_else(|| UploadError::new(UploadErrorKind::InvalidField))?;
        let policy = upload
            .policy
            .clone()
            .ok_or_else(|| UploadError::new(UploadErrorKind::InvalidField))?;
        let outcome = match upload.mode {
            TransferMode::File => {
                self.validation_service
                    .validate(
                        &self.context,
                        UploadValidationRequest::new(
                            upload.handle.clone(),
                            upload.field.clone(),
                            upload.revision,
                            idempotency("validate")?,
                            client,
                            upload.received_bytes,
                            checksum,
                            policy,
                        ),
                        CREATED_AT,
                    )
                    .await?
            }
            TransferMode::Direct => {
                self.direct
                    .verify(VerifyTransfer::new(&upload.handle, &checksum))
                    .await?;
                self.transition(upload, UploadTransition::Accept, "accept")
                    .await?;
                let authority = TransferGrantScope::new(
                    upload.handle.clone(),
                    self.context.mount().component().clone(),
                    upload.field.clone(),
                    self.scope.clone(),
                    1,
                );
                let inspection = UploadInspection::from_store(
                    upload.handle.clone(),
                    client,
                    DetectedUploadType::Unknown,
                    None,
                    upload.received_bytes,
                    checksum,
                    None,
                    CREATED_AT,
                )?;
                self.validation
                    .put(ValidatedUpload::from_store(
                        authority,
                        upload.revision,
                        policy.contract_digest().clone(),
                        inspection,
                    )?)
                    .await?;
                drop(upload.active_lease.take());
                return Ok(CompleteUploadOutcome::Ready(status_json(upload)));
            }
        };
        if let Some(transition) = outcome.transition() {
            upload.state = transition.state();
            upload.revision = transition.revision();
        }
        let reason = outcome.reason();
        match outcome.disposition() {
            UploadValidationDisposition::Ready => {
                drop(upload.active_lease.take());
                Ok(CompleteUploadOutcome::Ready(status_json(upload)))
            }
            UploadValidationDisposition::Rejected => {
                drop(upload.active_lease.take());
                Ok(CompleteUploadOutcome::Rejected {
                    reason: reason
                        .ok_or_else(|| UploadError::new(UploadErrorKind::InvalidField))?,
                    status: status_json(upload),
                })
            }
            UploadValidationDisposition::Retry => Ok(CompleteUploadOutcome::Retry {
                reason: reason.ok_or_else(|| UploadError::new(UploadErrorKind::InvalidField))?,
                status: status_json(upload),
            }),
        }
    }

    pub(super) async fn finalize(
        &self,
        request: FinalizeUploadRequestBody,
    ) -> Result<Value, UploadError> {
        self.record_call();
        let mut operation = self.take_upload(&request.handle)?;
        let upload = operation.upload_mut();
        let policy = upload
            .policy
            .clone()
            .ok_or_else(|| UploadError::new(UploadErrorKind::ValidationEvidenceUnavailable))?;
        let action = ActionTable::new(vec![ActionEntry::new(
            reference_action_metadata(),
            unused_action_dispatcher(),
        )])
        .map_err(|_| UploadError::new(UploadErrorKind::AuthorizationDenied))?
        .authorize(
            self.context.mount().component(),
            self.context.capabilities(),
            policy.finalize_action(),
        )
        .await
        .map_err(|_| UploadError::new(UploadErrorKind::AuthorizationDenied))?;
        let outcome = self
            .finalization
            .finalize(
                &self.context,
                FinalizeUploadRequest::new(
                    upload.handle.clone(),
                    upload.field.clone(),
                    UploadRevision::new(request.ready_revision),
                    idempotency(&format!("finalize-{}", &request.handle[..12]))?,
                    action,
                    policy,
                ),
                CREATED_AT,
            )
            .await?;
        upload.state = UploadState::Finalized;
        upload.revision = outcome.revision();
        Ok(status_json(upload))
    }

    pub(super) async fn cancel(&self, handle: &str, grant: &str) -> Result<Value, UploadError> {
        self.record_call();
        let mut operation = self.take_upload_after_inflight(handle).await?;
        let (pause_handle, pause_generation) = operation.pause_scope();
        self.operation_pause
            .pause_if_selected(UploadPausePoint::Cancel, &pause_handle, pause_generation)
            .await;
        self.require_current_grant(operation.upload(), grant)?;
        let result = self.cancel_inner(operation.upload_mut()).await;
        result.map(|()| status_json(operation.upload()))
    }

    async fn take_upload_after_inflight(
        &self,
        handle: &str,
    ) -> Result<UploadOperation, UploadError> {
        let mut shutdown = self.shutdown.clone();
        loop {
            let changed = self.uploads.changed.notified();
            let upload = {
                let mut uploads = self
                    .uploads
                    .records
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if self.uploads.retired.load(Ordering::Acquire) {
                    return Err(UploadError::new(UploadErrorKind::UploadConflict));
                }
                match uploads.remove(handle) {
                    Some(UploadSlot::Ready(upload)) => {
                        let upload = *upload;
                        uploads.insert(
                            handle.to_owned(),
                            UploadSlot::Busy {
                                handle: upload.handle.clone(),
                                mode: upload.mode,
                            },
                        );
                        Some(upload)
                    }
                    Some(busy @ UploadSlot::Busy { .. }) => {
                        uploads.insert(handle.to_owned(), busy);
                        None
                    }
                    None => return Err(UploadError::new(UploadErrorKind::UploadConflict)),
                }
            };
            if let Some(upload) = upload {
                return Ok(UploadOperation {
                    slots: Arc::clone(&self.uploads),
                    key: handle.to_owned(),
                    upload: Some(upload),
                });
            }
            tokio::select! {
                () = changed => {}
                result = shutdown.changed() => {
                    if result.is_err() || *shutdown.borrow() {
                        return Err(UploadError::new(UploadErrorKind::UploadConflict));
                    }
                }
            }
        }
    }

    async fn cancel_inner(&self, upload: &mut StoredUpload) -> Result<(), UploadError> {
        self.transition(upload, UploadTransition::Cancel, "cancel")
            .await
            .map_err(normalize_terminal_conflict)?;
        match upload.mode {
            TransferMode::File => self.file.cancel(&upload.handle).await?,
            TransferMode::Direct => self.direct.cancel(&upload.handle).await?,
        }
        drop(upload.active_lease.take());
        Ok(())
    }

    async fn expire(&self, handle: &str) -> Result<Value, UploadError> {
        self.record_call();
        let mut operation = self.take_upload_after_inflight(handle).await?;
        let (pause_handle, pause_generation) = operation.pause_scope();
        self.operation_pause
            .pause_if_selected(UploadPausePoint::Expire, &pause_handle, pause_generation)
            .await;
        let upload = operation.upload_mut();
        self.transition(upload, UploadTransition::Expire, "expire")
            .await
            .map_err(normalize_terminal_conflict)?;
        match upload.mode {
            TransferMode::File => self.file.expire(&upload.handle).await?,
            TransferMode::Direct => self.direct.expire(&upload.handle).await?,
        }
        drop(upload.active_lease.take());
        Ok(status_json(upload))
    }

    pub(super) async fn adversarial_race(
        &self,
        handle: &str,
        ready_revision: u64,
        case: &str,
    ) -> Result<UploadRaceOutcome, UploadError> {
        let (terminal_is_cancel, terminal_wins, terminal_state) = match case {
            "cancel-finalize-cancel-wins" => (true, true, "canceled"),
            "cancel-finalize-finalize-wins" => (true, false, "finalized"),
            "expire-finalize-expire-wins" => (false, true, "expired"),
            "expire-finalize-finalize-wins" => (false, false, "finalized"),
            _ => return Err(UploadError::new(UploadErrorKind::InvalidField)),
        };
        let grant = {
            let records = self
                .uploads
                .records
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let Some(UploadSlot::Ready(upload)) = records.get(handle) else {
                return Ok(UploadRaceOutcome {
                    disposition: "upload_missing",
                    terminal_state,
                    accepted_outcomes: 0,
                    active_uploads: self.active_uploads.current(),
                });
            };
            if upload.state != UploadState::Ready || upload.revision.get() != ready_revision {
                return Ok(UploadRaceOutcome {
                    disposition: "upload_not_ready",
                    terminal_state,
                    accepted_outcomes: 0,
                    active_uploads: self.active_uploads.current(),
                });
            }
            upload.grant.expose_bearer().to_owned()
        };
        let pause_point = if terminal_wins {
            if terminal_is_cancel {
                UploadPausePoint::Cancel
            } else {
                UploadPausePoint::Expire
            }
        } else {
            UploadPausePoint::Finalize
        };
        let pause_generation =
            match self
                .operation_pause
                .select(pause_point, handle, ready_revision)
            {
                Ok(generation) => generation,
                Err(_) => {
                    return Ok(UploadRaceOutcome {
                        disposition: "pause_select_failed",
                        terminal_state,
                        accepted_outcomes: 0,
                        active_uploads: self.active_uploads.current(),
                    });
                }
            };

        let finalize = self.finalize(FinalizeUploadRequestBody {
            handle: handle.to_owned(),
            ready_revision,
        });
        let terminal = async {
            if terminal_is_cancel {
                self.cancel(handle, &grant).await
            } else {
                self.expire(handle).await
            }
        };
        tokio::pin!(finalize);
        tokio::pin!(terminal);

        let (winner, loser) = if terminal_wins {
            tokio::select! {
                () = self.operation_pause.wait_until_entered(pause_generation) => {}
                result = &mut terminal => return Ok(UploadRaceOutcome {
                    disposition: if result.is_ok() { "race_did_not_pause" } else { "terminal_failed_before_pause" },
                    terminal_state,
                    accepted_outcomes: 0,
                    active_uploads: self.active_uploads.current(),
                }),
            }
            let loser = finalize.await;
            if self.operation_pause.resume(pause_generation).is_err() {
                return Ok(UploadRaceOutcome {
                    disposition: "pause_resume_failed",
                    terminal_state,
                    accepted_outcomes: 0,
                    active_uploads: self.active_uploads.current(),
                });
            }
            let winner = terminal.await;
            (winner, loser)
        } else {
            tokio::select! {
                () = self.operation_pause.wait_until_entered(pause_generation) => {}
                result = &mut finalize => return Ok(UploadRaceOutcome {
                    disposition: if result.is_ok() { "race_did_not_pause" } else { "finalize_failed_before_pause" },
                    terminal_state,
                    accepted_outcomes: 0,
                    active_uploads: self.active_uploads.current(),
                }),
            }
            let terminal_pending =
                poll_fn(|task| Poll::Ready(matches!(terminal.as_mut().poll(task), Poll::Pending)))
                    .await;
            if !terminal_pending {
                return Ok(UploadRaceOutcome {
                    disposition: "terminal_not_pending",
                    terminal_state,
                    accepted_outcomes: 0,
                    active_uploads: self.active_uploads.current(),
                });
            }
            if self.operation_pause.resume(pause_generation).is_err() {
                return Ok(UploadRaceOutcome {
                    disposition: "pause_resume_failed",
                    terminal_state,
                    accepted_outcomes: 0,
                    active_uploads: self.active_uploads.current(),
                });
            }
            let winner = finalize.await;
            let loser = terminal.await;
            (winner, loser)
        };
        if let Err(error) = winner {
            return Ok(UploadRaceOutcome {
                disposition: error.kind().as_str(),
                terminal_state,
                accepted_outcomes: 0,
                active_uploads: self.active_uploads.current(),
            });
        }
        let loser = match loser {
            Err(error) => error,
            Ok(_) => {
                return Ok(UploadRaceOutcome {
                    disposition: "multiple_outcomes_accepted",
                    terminal_state,
                    accepted_outcomes: 2,
                    active_uploads: self.active_uploads.current(),
                });
            }
        };
        if loser.kind() != UploadErrorKind::UploadConflict {
            return Ok(UploadRaceOutcome {
                disposition: loser.kind().as_str(),
                terminal_state,
                accepted_outcomes: 1,
                active_uploads: self.active_uploads.current(),
            });
        }
        Ok(UploadRaceOutcome {
            disposition: "upload_conflict",
            terminal_state,
            accepted_outcomes: 1,
            active_uploads: self.active_uploads.current(),
        })
    }

    pub(super) async fn reacquire(&self, handle: &str) -> Result<Value, UploadError> {
        self.record_call();
        let mut operation = self.take_upload(handle)?;
        let (pause_handle, pause_generation) = operation.pause_scope();
        self.operation_pause
            .pause_if_selected(UploadPausePoint::Reacquire, &pause_handle, pause_generation)
            .await;
        self.reacquire_inner(operation.upload_mut()).await
    }

    async fn reacquire_inner(&self, upload: &mut StoredUpload) -> Result<Value, UploadError> {
        let current = self
            .service
            .status(
                &self.context,
                upload.grant.clone(),
                upload.field.clone(),
                upload.handle.clone(),
                CREATED_AT,
            )
            .await?;
        upload.state = current.state();
        upload.revision = current.revision();
        let scope = TransferGrantScope::new(
            upload.handle.clone(),
            self.context.mount().component().clone(),
            upload.field.clone(),
            self.scope.clone(),
            1,
        );
        let sequence = self.grant_sequence.fetch_add(1, Ordering::SeqCst);
        let expires_at = 61_000_u64
            .checked_add(sequence)
            .map(UnixMillis::new)
            .ok_or_else(|| UploadError::new(UploadErrorKind::ResourceExhausted))?;
        let issued = self
            .grants
            .issue(TransferGrantRequest::new(scope, expires_at), CREATED_AT)?;
        upload.grant = issued.grant().clone();
        let mut response = status_json(upload);
        response["grant"] = Value::String(upload.grant.expose_bearer().to_owned());
        Ok(response)
    }

    pub(super) fn service_calls(&self) -> usize {
        self.service_calls.load(Ordering::SeqCst)
    }

    pub(super) fn fault_count(&self) -> usize {
        usize::from(self.fault_applied.load(Ordering::SeqCst))
    }

    pub(super) fn arm_scan_timeout(
        &self,
        handle: &str,
        operation_generation: u64,
    ) -> Result<(), &'static str> {
        let mut records = self
            .uploads
            .records
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let upload = records.get_mut(handle).and_then(|slot| match slot {
            UploadSlot::Ready(upload) => Some(upload.as_mut()),
            UploadSlot::Busy { .. } => None,
        });
        let Some(upload) = upload else {
            return Err("upload_scan_scope_invalid");
        };
        if upload.revision.get() != operation_generation
            || upload.state != UploadState::Created
            || upload.mode != TransferMode::File
        {
            return Err("upload_scan_scope_invalid");
        }
        let client = upload.client.as_ref().ok_or("upload_scan_scope_invalid")?;
        upload.policy = Some(
            reference_upload_policy_with_scan(
                client.display_name(),
                client
                    .claimed_media_type()
                    .unwrap_or("application/octet-stream"),
                UploadScanPolicy::Required {
                    on_timeout: ScanFailurePolicy::Retry,
                    on_unavailable: ScanFailurePolicy::Reject,
                },
            )
            .map_err(|_| "upload_scan_scope_invalid")?,
        );
        let mut timeouts = self
            .scanner
            .timeouts
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if timeouts.contains_key(handle) {
            return Err("upload_scan_fault_in_use");
        }
        timeouts.insert(handle.to_owned(), UploadRevision::new(operation_generation));
        Ok(())
    }

    pub(super) fn arm_finalize_fault(
        &self,
        handle: &str,
        operation_generation: u64,
        fault: &'static str,
    ) -> Result<(), &'static str> {
        let records = self
            .uploads
            .records
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let current = records.get(handle).and_then(|slot| match slot {
            UploadSlot::Ready(upload) if upload.state == UploadState::Ready => {
                Some(upload.revision.get())
            }
            UploadSlot::Ready(_) | UploadSlot::Busy { .. } => None,
        });
        if current != Some(operation_generation) {
            return Err("upload_finalize_fault_scope_invalid");
        }
        let selected = match fault {
            "commit-unavailable" => ReferenceFinalizeFault::CommitUnavailable,
            "ledger-after-commit" => ReferenceFinalizeFault::LedgerAfterDurableCommit,
            _ => return Err("upload_finalize_fault_invalid"),
        };
        let mut current = self
            .finalizer
            .fault
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if current.is_some() {
            return Err("upload_finalize_fault_in_use");
        }
        *current = Some(ArmedReferenceFinalizeFault {
            fault: selected,
            handle: UploadHandle::parse(handle)
                .map_err(|_| "upload_finalize_fault_scope_invalid")?,
            ready_revision: UploadRevision::new(operation_generation),
        });
        drop(records);
        Ok(())
    }

    pub(super) fn validation_scan_calls(&self) -> usize {
        self.scanner.scan_calls.load(Ordering::SeqCst)
    }

    pub(super) fn finalizer_counts(&self) -> (usize, usize, usize) {
        (
            self.finalizer.commit_calls.load(Ordering::SeqCst),
            self.finalizer.compensation_calls.load(Ordering::SeqCst),
            self.finalizer.reconciliation_calls.load(Ordering::SeqCst),
        )
    }

    pub(super) async fn pause_chunk(
        &self,
        handle: &str,
        operation_generation: u64,
    ) -> Result<u64, &'static str> {
        let _creation_window_guard = self.creation_window_gate.lock().await;
        let records = self
            .uploads
            .records
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let current = records.get(handle).and_then(|slot| match slot {
            UploadSlot::Ready(upload)
                if upload.mode == TransferMode::File && !upload.state.is_terminal() =>
            {
                Some(upload.revision.get())
            }
            UploadSlot::Ready(_) | UploadSlot::Busy { .. } => None,
        });
        if current != Some(operation_generation) {
            return Err("upload_pause_scope_invalid");
        }
        drop(records);
        self.operation_pause
            .select(UploadPausePoint::Chunk, handle, operation_generation)
    }

    pub(super) fn resume_chunk(&self, generation: u64) -> Result<(), &'static str> {
        self.operation_pause.resume(generation)
    }

    pub(super) async fn pause_finalize(
        &self,
        handle: &str,
        operation_generation: u64,
    ) -> Result<u64, &'static str> {
        let _creation_window_guard = self.creation_window_gate.lock().await;
        let records = self
            .uploads
            .records
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let current = records.get(handle).and_then(|slot| match slot {
            UploadSlot::Ready(upload) if upload.state == UploadState::Ready => {
                Some(upload.revision.get())
            }
            UploadSlot::Ready(_) | UploadSlot::Busy { .. } => None,
        });
        if current != Some(operation_generation) {
            return Err("upload_pause_scope_invalid");
        }
        drop(records);
        self.operation_pause
            .select(UploadPausePoint::Finalize, handle, operation_generation)
    }

    pub(super) fn reject_chunk_once(
        &self,
        handle: &str,
        operation_generation: u64,
    ) -> Result<(), &'static str> {
        let records = self
            .uploads
            .records
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let current = records.get(handle).and_then(|slot| match slot {
            UploadSlot::Ready(upload)
                if upload.mode == TransferMode::File && !upload.state.is_terminal() =>
            {
                Some(upload.revision.get())
            }
            UploadSlot::Ready(_) | UploadSlot::Busy { .. } => None,
        });
        if current != Some(operation_generation) {
            return Err("upload_rejection_scope_invalid");
        }
        let mut selected = self
            .chunk_rejection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if selected.is_some() {
            return Err("upload_rejection_in_use");
        }
        *selected = Some(UploadChunkRejection {
            handle: handle.to_owned(),
            operation_generation,
        });
        Ok(())
    }

    fn consume_chunk_rejection(&self, handle: &str, operation_generation: u64) -> bool {
        let mut selected = self
            .chunk_rejection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if selected.as_ref().is_some_and(|selection| {
            selection.handle == handle && selection.operation_generation == operation_generation
        }) {
            *selected = None;
            true
        } else {
            false
        }
    }

    pub(super) fn paused_operations(&self) -> usize {
        self.operation_pause.active_count()
    }

    pub(super) async fn wait_until_operation_paused(&self, generation: u64) {
        self.operation_pause.wait_until_entered(generation).await;
    }

    pub(super) fn advance_pause_clock(&self) -> Result<(), &'static str> {
        self.operation_pause.advance_clock()
    }

    pub(super) async fn reset_creation_window(&self) -> Result<(), &'static str> {
        let _creation_window_guard = self.creation_window_gate.lock().await;
        let uploads = self
            .uploads
            .records
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let slots_are_terminal = !uploads.values().any(|slot| match slot {
            UploadSlot::Ready(upload) => !upload.state.is_terminal(),
            UploadSlot::Busy { .. } => true,
        });
        if !slots_are_terminal || self.operation_pause.has_authority() {
            return Err("upload_window_not_quiescent");
        }
        if !self.ledger.reset_creation_window_if_all_terminal() {
            return Err("upload_window_not_quiescent");
        }
        Ok(())
    }

    pub(super) async fn retire(&self) -> Result<(), String> {
        self.operation_pause.retire();
        *self
            .chunk_rejection
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;
        self.uploads.retired.store(true, Ordering::Release);
        self.uploads.changed.notify_waiters();
        let pending = {
            let mut uploads = self
                .uploads
                .records
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            uploads
                .drain()
                .map(|(_, slot)| match slot {
                    UploadSlot::Ready(upload) => (upload.handle, upload.mode),
                    UploadSlot::Busy { handle, mode } => (handle, mode),
                })
                .collect::<Vec<_>>()
        };
        let mut first_error = None;
        for (handle, mode) in pending {
            let cleanup = async {
                match mode {
                    TransferMode::File => self.file.cancel(&handle).await,
                    TransferMode::Direct => self.direct.cancel(&handle).await,
                }
            };
            match timeout(Duration::from_millis(500), cleanup).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) if first_error.is_none() => {
                    first_error = Some(format!("upload quarantine cleanup: {error:?}"));
                }
                Err(_) if first_error.is_none() => {
                    first_error = Some("upload quarantine cleanup timed out".to_owned());
                }
                Ok(Err(_)) | Err(_) => {}
            }
        }
        match timeout(Duration::from_millis(500), self.file.retire_and_cleanup()).await {
            Ok(Ok(_)) => {}
            Ok(Err(error)) if first_error.is_none() => {
                first_error = Some(format!("upload provider retirement: {error:?}"));
            }
            Err(_) if first_error.is_none() => {
                let status = self.file.retirement_status();
                first_error = Some(format!(
                    "upload provider retirement timed out: active_operations={}, owned_transfers={}, active_descriptors={}, active_chunks={}",
                    status.active_operations(),
                    status.owned_transfers(),
                    status.active_descriptors(),
                    status.active_chunks(),
                ));
            }
            Ok(Err(_)) | Err(_) => {}
        }
        first_error.map_or(Ok(()), Err)
    }

    fn record_call(&self) {
        self.service_calls.fetch_add(1, Ordering::SeqCst);
    }

    fn take_upload(&self, handle: &str) -> Result<UploadOperation, UploadError> {
        let mut uploads = self
            .uploads
            .records
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.uploads.retired.load(Ordering::Acquire) {
            return Err(UploadError::new(UploadErrorKind::UploadConflict));
        }
        let upload = uploads
            .remove(handle)
            .and_then(|slot| match slot {
                UploadSlot::Ready(upload) => Some(*upload),
                busy @ UploadSlot::Busy { .. } => {
                    uploads.insert(handle.to_owned(), busy);
                    None
                }
            })
            .ok_or_else(|| UploadError::new(UploadErrorKind::UploadConflict))?;
        uploads.insert(
            handle.to_owned(),
            UploadSlot::Busy {
                handle: upload.handle.clone(),
                mode: upload.mode,
            },
        );
        Ok(UploadOperation {
            slots: Arc::clone(&self.uploads),
            key: handle.to_owned(),
            upload: Some(upload),
        })
    }

    fn require_current_grant(
        &self,
        upload: &StoredUpload,
        bearer: &str,
    ) -> Result<TransferGrant, UploadError> {
        let presented = TransferGrant::parse(bearer)?;
        if upload.grant.expose_bearer() != presented.expose_bearer() {
            return Err(UploadError::new(UploadErrorKind::InvalidGrant));
        }
        let expected = TransferGrantScope::new(
            upload.handle.clone(),
            self.context.mount().component().clone(),
            upload.field.clone(),
            self.scope.clone(),
            1,
        );
        self.grants.verify(&presented, &expected, CREATED_AT)?;
        Ok(presented)
    }

    async fn transition(
        &self,
        upload: &mut StoredUpload,
        operation: UploadTransition,
        key: &str,
    ) -> Result<(), UploadError> {
        let outcome = self
            .service
            .transition(
                &self.context,
                UploadTransitionAdmission::new(
                    upload.grant.clone(),
                    upload.field.clone(),
                    UploadTransitionRequest::new(
                        upload.handle.clone(),
                        upload.revision,
                        idempotency(key)?,
                        operation,
                    ),
                ),
                CREATED_AT,
            )
            .await?;
        upload.state = outcome.state();
        upload.revision = outcome.revision();
        Ok(())
    }
}

fn normalize_terminal_conflict(error: UploadError) -> UploadError {
    if error.kind() == UploadErrorKind::InvalidTransition {
        UploadError::new(UploadErrorKind::UploadConflict)
    } else {
        error
    }
}

fn status_json(upload: &StoredUpload) -> Value {
    json!({
        "handle": upload.handle.to_string(),
        "field": upload.field.as_str(),
        "state": upload.state.as_str(),
        "expected_bytes": upload.expected_bytes,
        "received_bytes": upload.received_bytes,
        "next_part": upload.next_part,
        "revision": upload.revision.get(),
    })
}

fn direct_instruction_json(instruction: &DirectTransferInstruction) -> Value {
    json!({
        "method": instruction.method().as_str(),
        "url": instruction.endpoint().as_str(),
        "headers": instruction.required_headers().iter().map(|(name, value)| {
            (name.as_str().to_owned(), Value::String(value.to_owned()))
        }).collect::<serde_json::Map<_, _>>(),
        "part": instruction.part().index(),
        "offset": instruction.part().offset(),
        "maximum_bytes": instruction.maximum_bytes(),
        "expires_at": instruction.expires_at().get(),
        "reference": instruction.reference().as_str(),
    })
}

fn idempotency(value: &str) -> Result<UploadIdempotencyKey, UploadError> {
    UploadIdempotencyKey::parse(value)
}

fn reference_upload_policy(
    filename: &str,
    content_type: &str,
) -> Result<UploadFieldPolicy, UploadError> {
    reference_upload_policy_with_scan(filename, content_type, UploadScanPolicy::Disabled)
}

fn reference_upload_policy_with_scan(
    _filename: &str,
    _content_type: &str,
    scan: UploadScanPolicy,
) -> Result<UploadFieldPolicy, UploadError> {
    UploadFieldPolicy::new_with_accepted_types(
        16,
        UploadLimitConfig::reference().max_file_bytes,
        UploadReplacementPolicy::RetirePrevious,
        Vec::new(),
        None,
        scan,
        ActionName::parse("refresh")
            .map_err(|_| UploadError::new(UploadErrorKind::InvalidField))?,
    )
}

fn unused_action_dispatcher() -> ActionDispatchFn {
    |_target, _authorized, _arguments| Box::pin(std::future::pending())
}

fn grant_codec() -> TransferGrantCodec {
    let key = KeyRecord::new(
        KeyId::parse("reference-upload-key").expect("static key identity"),
        RootKey::new(b"reference-upload-root-key-000000".to_vec()).expect("static root key"),
        UnixMillis::new(0),
        UnixMillis::new(100_000),
        UnixMillis::new(200_000),
    )
    .expect("static key window");
    TransferGrantCodec::new(SnapshotKeyRing::new(key, Vec::new()).expect("static key ring"))
}

fn reference_upload_context(
    authorization: Arc<ControlledUploadAuthorization>,
) -> Result<(TrustedLiveRequestContext, HostScopeFacts), String> {
    let metadata = reference_metadata();
    let descriptor = ComponentDescriptor::new(metadata.clone());
    let contract = ComponentContract::new(
        metadata.identity().clone(),
        descriptor.contract_digest().clone(),
        1,
        1,
        1,
    )
    .map_err(|error| format!("component contract: {error:?}"))?;
    let registry = ComponentRegistryBuilder::new()
        .register(descriptor)
        .map_err(|error| format!("component registry: {error:?}"))?
        .build();
    let route = RouteIdentity::from_bytes(&deterministic_bytes::<32>(0x30))
        .map_err(|error| format!("route identity: {error:?}"))?;
    let slot =
        IslandSlot::parse("reference-upload").map_err(|error| format!("island slot: {error:?}"))?;
    let schemas = SnapshotSchemaSet::new(
        StateSchema::new(
            1,
            vec![
                FieldSpec::new("serial", StateCodec::Json, FieldCategory::State, true)
                    .map_err(|error| format!("state field: {error:?}"))?,
            ],
        )
        .map_err(|error| format!("state schema: {error:?}"))?,
        StateSchema::new(1, vec![]).map_err(|error| format!("memo schema: {error:?}"))?,
        StateSchema::new(1, vec![]).map_err(|error| format!("mount schema: {error:?}"))?,
    )
    .map_err(|error| format!("snapshot schemas: {error:?}"))?;
    let catalog = MountCatalogBuilder::new()
        .register(
            &registry,
            MountCatalogEntry::new(
                ExpectedSeedV1::new(
                    contract,
                    BuildId::parse("reference-upload-build")
                        .map_err(|error| format!("build identity: {error:?}"))?,
                    route.clone(),
                    slot.clone(),
                    schemas,
                ),
                MountScopeRequirements::new(
                    ScopeRequirement::Required,
                    ScopeRequirement::Required,
                    ScopeRequirement::Required,
                ),
            ),
        )
        .map_err(|error| format!("mount catalog: {error:?}"))?
        .build();
    let scope = HostScopeFacts::new(
        ScopeFingerprint::from_bytes(&deterministic_bytes::<32>(0x40))
            .map_err(|error| format!("scope identity: {error:?}"))?,
        Some(
            SessionFingerprint::from_bytes(&deterministic_bytes::<32>(0x41))
                .map_err(|error| format!("session identity: {error:?}"))?,
        ),
        Some(
            PrincipalFingerprint::from_bytes(&deterministic_bytes::<32>(0x42))
                .map_err(|error| format!("principal identity: {error:?}"))?,
        ),
        Some(
            TenantFingerprint::from_bytes(&deterministic_bytes::<32>(0x43))
                .map_err(|error| format!("tenant identity: {error:?}"))?,
        ),
    );
    let context = SyntheticLiveRequestContextBuilder::new(
        catalog,
        MountSelection::new(
            route,
            slot,
            metadata.identity().clone(),
            metadata.contract_digest().clone(),
            1,
        ),
        scope.clone(),
        CREATED_AT,
        UnixMillis::new(60_000),
    )
    .with_action_authorization(Arc::new(ReferenceActionAuthorization))
    .with_upload_authorization(authorization)
    .build()
    .map_err(|error| format!("trusted upload context: {error:?}"))?;
    Ok((context, scope))
}

fn reference_metadata() -> &'static ComponentMetadata {
    static METADATA: OnceLock<ComponentMetadata> = OnceLock::new();
    METADATA.get_or_init(|| {
        ComponentMetadata::new(
            ComponentName::parse("reference.uploads").expect("static component identity"),
            ViewName::parse("reference/uploads.html").expect("static view identity"),
            ContractVersions::new(1, 1, 1, 1, 1).expect("static contract versions"),
            vec![FieldMetadata::new(
                ModelField::parse("serial").expect("static field identity"),
                FieldCategory::State,
                StateCodec::Json,
                true,
            )],
            vec![reference_action_metadata()],
        )
        .expect("static component metadata")
    })
}

fn reference_action_metadata() -> ActionMetadata {
    ActionMetadata::new_with_contract(
        ActionName::parse("refresh").expect("static action identity"),
        1,
        ActionArgumentSchema::empty(),
        AuthorizationRequirement::Current,
        ValidationSelection::ComponentAndArguments,
        TransactionPolicy::None,
    )
    .expect("static action metadata")
}

fn deterministic_bytes<const LENGTH: usize>(start: u8) -> [u8; LENGTH] {
    std::array::from_fn(|index| start.wrapping_add(index as u8))
}

struct AxumChunkBody {
    body: Body,
    hasher: Sha256,
    interrupt_after_first: bool,
    maximum_body_bytes: usize,
    pending: Option<Bytes>,
    yielded_chunks: usize,
    shutdown: watch::Receiver<bool>,
}

impl AxumChunkBody {
    fn new(
        body: Body,
        hasher: Sha256,
        interrupt_after_first: bool,
        maximum_body_bytes: usize,
        shutdown: watch::Receiver<bool>,
    ) -> Self {
        Self {
            body,
            hasher,
            interrupt_after_first,
            maximum_body_bytes,
            pending: None,
            yielded_chunks: 0,
            shutdown,
        }
    }

    fn into_hasher(self) -> Sha256 {
        self.hasher
    }
}

impl ChunkBody for AxumChunkBody {
    fn next_chunk<'a>(
        &'a mut self,
        maximum_bytes: usize,
    ) -> suprnova_live::upload::UploadFuture<'a, Result<Option<QuarantineBytes>, UploadError>> {
        Box::pin(async move {
            if self.interrupt_after_first && self.yielded_chunks > 0 {
                return Err(UploadError::new(UploadErrorKind::BodyInterrupted));
            }
            if *self.shutdown.borrow() {
                return Err(UploadError::new(UploadErrorKind::BodyInterrupted));
            }
            loop {
                if let Some(mut pending) = self.pending.take() {
                    let selected = pending.split_to(pending.len().min(maximum_bytes));
                    if !pending.is_empty() {
                        self.pending = Some(pending);
                    }
                    self.hasher.update(&selected);
                    self.yielded_chunks = self.yielded_chunks.saturating_add(1);
                    return Ok(Some(QuarantineBytes::copy_from_slice(&selected)));
                }
                let frame = tokio::select! {
                    biased;
                    changed = self.shutdown.changed() => {
                        let _ = changed;
                        return Err(UploadError::new(UploadErrorKind::BodyInterrupted));
                    }
                    frame = self.body.frame() => frame,
                };
                let Some(frame) = frame else {
                    return Ok(None);
                };
                let frame =
                    frame.map_err(|_| UploadError::new(UploadErrorKind::BodyInterrupted))?;
                if let Ok(data) = frame.into_data() {
                    if data.is_empty() {
                        continue;
                    }
                    if data.len() > self.maximum_body_bytes {
                        return Err(UploadError::new(UploadErrorKind::InputTooLarge));
                    }
                    self.pending = Some(data);
                    continue;
                }
            }
        })
    }
}

fn hex_digest(digest: impl AsRef<[u8]>) -> String {
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn one_axum_frame_is_split_at_the_provider_pull_boundary_without_retained_excess() {
        const PROVIDER_PULL_BYTES: usize = 256 * 1024;
        let expected = (0..=PROVIDER_PULL_BYTES)
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>();
        let expected_digest = Sha256::digest(&expected);
        let (_shutdown, shutdown) = watch::channel(false);
        let mut body = AxumChunkBody::new(
            Body::from(expected.clone()),
            Sha256::new(),
            false,
            expected.len(),
            shutdown,
        );

        let first = body
            .next_chunk(PROVIDER_PULL_BYTES)
            .await
            .expect("first provider pull")
            .expect("first bounded body fragment");
        assert_eq!(first.len(), PROVIDER_PULL_BYTES);
        let second = body
            .next_chunk(PROVIDER_PULL_BYTES)
            .await
            .expect("second provider pull")
            .expect("one-byte body remainder");
        assert_eq!(second.len(), 1);
        assert!(
            body.next_chunk(1)
                .await
                .expect("excess-body probe")
                .is_none(),
            "provider observed bytes beyond the declared frame"
        );
        assert!(body.pending.is_none(), "Axum frame remainder was retained");
        assert_eq!(body.yielded_chunks, 2);

        let mut delivered = Vec::with_capacity(expected.len());
        delivered.extend_from_slice(first.as_ref());
        delivered.extend_from_slice(second.as_ref());
        assert_eq!(delivered, expected);
        assert_eq!(
            body.into_hasher().finalize().as_slice(),
            expected_digest.as_slice()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn every_detached_upload_operation_restores_state_when_its_future_is_aborted() {
        let mut random = [0_u8; 8];
        getrandom::fill(&mut random).expect("test root entropy");
        let root = std::env::temp_dir().join(format!(
            "suprnova-live-upload-abort-{}",
            u64::from_le_bytes(random)
        ));
        tokio::fs::create_dir_all(&root).await.expect("test root");
        let (_shutdown, shutdown) = watch::channel(false);
        let active = Arc::new(ResourceCounter::default());
        let timers = Arc::new(ResourceCounter::default());
        let runtime = Arc::new(
            UploadRuntime::open(
                &root,
                ReferenceFaultSchedule::None,
                shutdown,
                Arc::clone(&active),
                Arc::clone(&timers),
            )
            .await
            .expect("upload runtime"),
        );

        for point in [
            UploadPausePoint::Chunk,
            UploadPausePoint::Status,
            UploadPausePoint::Complete,
            UploadPausePoint::Cancel,
            UploadPausePoint::Reacquire,
            UploadPausePoint::Finalize,
            UploadPausePoint::Expire,
        ] {
            let created = runtime
                .create(CreateUploadRequest {
                    field: "serial".to_owned(),
                    filename: format!("{point:?}.bin"),
                    content_type: "application/octet-stream".to_owned(),
                    expected_bytes: 8,
                    mode: "file".to_owned(),
                })
                .await
                .expect("created upload");
            let handle = created["handle"].as_str().expect("handle").to_owned();
            let grant = created["grant"].as_str().expect("grant").to_owned();
            let revision = if matches!(point, UploadPausePoint::Finalize | UploadPausePoint::Expire)
            {
                let checksum = hex_digest(Sha256::digest(b"abcdefgh"));
                runtime
                    .write_chunk(
                        &handle,
                        0,
                        &grant,
                        &checksum,
                        Some(8),
                        Body::from("abcdefgh"),
                    )
                    .await
                    .expect("finalization upload chunk");
                runtime
                    .complete(
                        &handle,
                        CompleteUploadRequest {
                            grant: grant.clone(),
                        },
                    )
                    .await
                    .expect("finalization upload ready")
                    .status()["revision"]
                    .as_u64()
                    .expect("ready revision")
            } else {
                created["revision"].as_u64().expect("revision")
            };
            let pause_generation = runtime
                .operation_pause
                .select(point, &handle, revision)
                .expect("scoped pause");
            let operation_runtime = Arc::clone(&runtime);
            let operation_handle = handle.clone();
            let operation_grant = grant.clone();
            let task = tokio::spawn(async move {
                match point {
                    UploadPausePoint::Chunk => {
                        let checksum = hex_digest(Sha256::digest(b"abcdefgh"));
                        operation_runtime
                            .write_chunk(
                                &operation_handle,
                                0,
                                &operation_grant,
                                &checksum,
                                Some(8),
                                Body::from("abcdefgh"),
                            )
                            .await
                    }
                    UploadPausePoint::Status => {
                        operation_runtime
                            .status(&operation_handle, &operation_grant)
                            .await
                    }
                    UploadPausePoint::Complete => operation_runtime
                        .complete(
                            &operation_handle,
                            CompleteUploadRequest {
                                grant: operation_grant,
                            },
                        )
                        .await
                        .map(|outcome| outcome.status().clone()),
                    UploadPausePoint::Cancel => {
                        operation_runtime
                            .cancel(&operation_handle, &operation_grant)
                            .await
                    }
                    UploadPausePoint::Reacquire => {
                        operation_runtime.reacquire(&operation_handle).await
                    }
                    UploadPausePoint::Finalize => {
                        operation_runtime
                            .finalize(FinalizeUploadRequestBody {
                                handle: operation_handle,
                                ready_revision: revision,
                            })
                            .await
                    }
                    UploadPausePoint::Expire => operation_runtime.expire(&operation_handle).await,
                }
            });
            timeout(
                Duration::from_secs(1),
                runtime.operation_pause.wait_until_entered(pause_generation),
            )
            .await
            .expect("operation reached cancellation point");
            task.abort();
            assert!(task.await.expect_err("operation aborted").is_cancelled());
            assert_eq!(
                runtime.operation_pause.resume(pause_generation),
                Err("upload_pause_generation_stale")
            );
            if matches!(point, UploadPausePoint::Finalize | UploadPausePoint::Expire) {
                let parsed_handle = UploadHandle::parse(&handle).expect("stored upload handle");
                let authoritative = suprnova_live::upload::UploadLedger::load(
                    runtime.ledger.as_ref(),
                    &parsed_handle,
                )
                .await
                .expect("authoritative finalizing upload")
                .expect("retained finalizing upload");
                assert_eq!(
                    authoritative.state(),
                    if point == UploadPausePoint::Finalize {
                        UploadState::Finalizing
                    } else {
                        UploadState::Ready
                    }
                );
                assert_eq!(
                    runtime.reset_creation_window().await,
                    Err("upload_window_not_quiescent")
                );
            }

            let coherent = runtime
                .status(&handle, &grant)
                .await
                .expect("successor request sees restored upload");
            assert_eq!(coherent["handle"], handle);
            assert_eq!(
                coherent["state"],
                if matches!(point, UploadPausePoint::Finalize | UploadPausePoint::Expire) {
                    "ready"
                } else {
                    "created"
                }
            );
        }

        runtime.retire().await.expect("runtime retires");
        assert_eq!(active.current(), 0);
        let mut entries = tokio::fs::read_dir(&root).await.expect("quarantine root");
        assert!(
            entries
                .next_entry()
                .await
                .expect("quarantine entry")
                .is_none()
        );
        tokio::fs::remove_dir(&root)
            .await
            .expect("remove empty test root");
    }

    #[tokio::test]
    async fn upload_pause_is_exact_generation_bounded_and_auto_retires() {
        let mut random = [0_u8; 8];
        getrandom::fill(&mut random).expect("test root entropy");
        let root = std::env::temp_dir().join(format!(
            "suprnova-live-upload-pause-{}",
            u64::from_le_bytes(random)
        ));
        tokio::fs::create_dir_all(&root).await.expect("test root");
        let (_shutdown, shutdown) = watch::channel(false);
        let active = Arc::new(ResourceCounter::default());
        let timers = Arc::new(ResourceCounter::default());
        let runtime = Arc::new(
            UploadRuntime::open(
                &root,
                ReferenceFaultSchedule::None,
                shutdown,
                Arc::clone(&active),
                Arc::clone(&timers),
            )
            .await
            .expect("upload runtime"),
        );
        let created = runtime
            .create(CreateUploadRequest {
                field: "serial".to_owned(),
                filename: "pause.bin".to_owned(),
                content_type: "application/octet-stream".to_owned(),
                expected_bytes: 8,
                mode: "file".to_owned(),
            })
            .await
            .expect("created upload");
        let handle = created["handle"].as_str().expect("handle").to_owned();
        let revision = created["revision"].as_u64().expect("revision");
        let generation = runtime
            .pause_chunk(&handle, revision)
            .await
            .expect("scoped pause generation");
        assert_eq!(timers.current(), 1);
        assert_eq!(
            runtime.resume_chunk(generation + 1),
            Err("upload_pause_generation_invalid")
        );
        assert_eq!(
            runtime.pause_chunk("wrong-handle", revision).await,
            Err("upload_pause_scope_invalid")
        );
        runtime
            .advance_pause_clock()
            .expect("controlled deadline retires selected pause");
        assert_eq!(
            runtime.resume_chunk(generation),
            Err("upload_pause_generation_stale")
        );
        assert_eq!(runtime.paused_operations(), 0);
        assert_eq!(timers.current(), 0);

        let generation = runtime
            .pause_chunk(&handle, revision)
            .await
            .expect("second scoped pause generation");
        let operation_runtime = Arc::clone(&runtime);
        let operation_handle = handle.clone();
        let grant = created["grant"].as_str().expect("grant").to_owned();
        let operation = tokio::spawn(async move {
            let checksum = hex_digest(Sha256::digest(b"abcdefgh"));
            operation_runtime
                .write_chunk(
                    &operation_handle,
                    0,
                    &grant,
                    &checksum,
                    Some(8),
                    Body::from("abcdefgh"),
                )
                .await
        });
        runtime.operation_pause.wait_until_entered(generation).await;
        assert_eq!(runtime.paused_operations(), 1);
        assert_eq!(timers.current(), 1);
        runtime
            .advance_pause_clock()
            .expect("controlled deadline retires active pause");
        let result = operation
            .await
            .expect("timed operation")
            .expect("chunk result");
        assert_eq!(result["state"], "transferring");
        assert_eq!(runtime.paused_operations(), 0);
        assert_eq!(timers.current(), 0);
        assert_eq!(
            runtime.resume_chunk(generation),
            Err("upload_pause_generation_stale")
        );

        let current_revision = result["revision"]
            .as_u64()
            .expect("current upload revision");
        for _ in 0..32 {
            let generation = runtime
                .pause_chunk(&handle, current_revision)
                .await
                .expect("repeated scoped pause");
            assert_eq!(timers.current(), 1);
            runtime
                .resume_chunk(generation)
                .expect("repeated pause resumes");
            assert_eq!(runtime.paused_operations(), 0);
            assert!(!runtime.operation_pause.has_authority());
            assert_eq!(timers.current(), 0);
        }
        runtime
            .pause_chunk(&handle, current_revision)
            .await
            .expect("shutdown-owned pause");
        assert!(runtime.operation_pause.has_authority());
        assert_eq!(timers.current(), 1);
        runtime.retire().await.expect("runtime retires");
        assert!(!runtime.operation_pause.has_authority());
        assert_eq!(timers.current(), 0);
        assert_eq!(active.current(), 0);
        tokio::fs::remove_dir_all(&root)
            .await
            .expect("remove test root");
    }
}
