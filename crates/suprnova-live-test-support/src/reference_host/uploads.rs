//! Bounded upload routes backed by the production provider contracts.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as SyncMutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use axum::body::Body;
use http_body_util::BodyExt as _;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use suprnova_live::action::{ActionArgumentSchema, AuthorizationRequirement, TransactionPolicy};
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
    AcceptedChunk, ChunkBody, PrepareTransfer, QuarantineBytes, QuarantinedFileProvider,
    ReverseProxyUploadProvider, TransferGrant, TransferGrantCodec, TransferGrantRequest,
    TransferGrantScope, TrustedProviderOrigin, UploadChecksum, UploadCreationRequest, UploadError,
    UploadErrorKind, UploadHandle, UploadIdempotencyKey, UploadProvider, UploadRevision,
    UploadService, UploadState, UploadTransition, UploadTransitionAdmission,
    UploadTransitionRequest, VerifyTransfer, WriteChunk,
};
use suprnova_live::validation::ValidationSelection;
use tokio::sync::{Notify, watch};
use tokio::time::{Duration, timeout};

use crate::{
    ControlledUploadAuthorization, DirectProviderConformanceAdapter, MemoryUploadLedger,
    SyntheticLiveRequestContextBuilder, TokioFileQuarantineStore,
};

use super::faults::ReferenceFaultSchedule;
use super::{ResourceCounter, ResourceLease};

const CREATED_AT: UnixMillis = UnixMillis::new(1_000);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransferMode {
    File,
    Direct,
}

struct StoredUpload {
    handle: UploadHandle,
    field: ModelField,
    expected_bytes: u64,
    received_bytes: u64,
    next_part: u32,
    state: UploadState,
    mode: TransferMode,
    grant: TransferGrant,
    revision: UploadRevision,
    whole_hasher: Sha256,
    active_lease: Option<ResourceLease>,
}

enum UploadSlot {
    Ready(StoredUpload),
    Busy {
        handle: UploadHandle,
        mode: TransferMode,
    },
}

struct UploadSlots {
    records: SyncMutex<HashMap<String, UploadSlot>>,
    retired: AtomicBool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UploadPausePoint {
    Chunk,
    Status,
    Complete,
    Cancel,
    Reacquire,
}

#[derive(Default)]
struct UploadOperationPause {
    target: SyncMutex<Option<UploadPausePoint>>,
    entered: Notify,
    release: Notify,
}

impl UploadOperationPause {
    async fn pause_if_selected(&self, point: UploadPausePoint) {
        if *self
            .target
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            == Some(point)
        {
            self.entered.notify_one();
            self.release.notified().await;
        }
    }

    #[cfg(test)]
    fn select(&self, point: UploadPausePoint) {
        *self
            .target
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(point);
    }

    #[cfg(test)]
    async fn wait_until_entered(&self) {
        self.entered.notified().await;
    }

    #[cfg(test)]
    fn resume(&self) {
        *self
            .target
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;
        self.release.notify_waiters();
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
            records.insert(self.key.clone(), UploadSlot::Ready(upload));
        }
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

pub(super) struct UploadRuntime {
    limits: UploadLimits,
    file: Arc<QuarantinedFileProvider<TokioFileQuarantineStore>>,
    direct: Arc<DirectProviderConformanceAdapter>,
    service: UploadService,
    grants: TransferGrantCodec,
    context: TrustedLiveRequestContext,
    scope: HostScopeFacts,
    uploads: Arc<UploadSlots>,
    service_calls: AtomicUsize,
    fault: ReferenceFaultSchedule,
    fault_applied: AtomicBool,
    shutdown: watch::Receiver<bool>,
    active_uploads: Arc<ResourceCounter>,
    operation_pause: Arc<UploadOperationPause>,
}

impl UploadRuntime {
    pub(super) async fn open(
        root: &std::path::Path,
        fault: ReferenceFaultSchedule,
        shutdown: watch::Receiver<bool>,
        active_uploads: Arc<ResourceCounter>,
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
        let service = UploadService::new(ledger, grant_codec(), limits)
            .map_err(|error| format!("upload service: {error:?}"))?;
        Ok(Self {
            limits,
            file,
            direct,
            service,
            grants: grant_codec(),
            context,
            scope,
            uploads: Arc::new(UploadSlots {
                records: SyncMutex::new(HashMap::new()),
                retired: AtomicBool::new(false),
            }),
            service_calls: AtomicUsize::new(0),
            fault,
            fault_applied: AtomicBool::new(false),
            shutdown,
            active_uploads,
            operation_pause: Arc::new(UploadOperationPause::default()),
        })
    }

    pub(super) async fn create(&self, request: CreateUploadRequest) -> Result<Value, UploadError> {
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
        let instruction = plan
            .instructions()
            .find_map(|instruction| instruction.as_direct())
            .map(|instruction| {
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
            });
        let stored = StoredUpload {
            handle: handle.clone(),
            field,
            expected_bytes: request.expected_bytes,
            received_bytes: 0,
            next_part: 0,
            state,
            mode,
            grant: grant.clone(),
            revision,
            whole_hasher: Sha256::new(),
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
                uploads.insert(encoded.clone(), UploadSlot::Ready(stored));
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
        self.operation_pause
            .pause_if_selected(UploadPausePoint::Chunk)
            .await;
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

    pub(super) async fn status(&self, handle: &str, grant: &str) -> Result<Value, UploadError> {
        self.record_call();
        let operation = self.take_upload(handle)?;
        self.operation_pause
            .pause_if_selected(UploadPausePoint::Status)
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
    ) -> Result<Value, UploadError> {
        self.record_call();
        let mut operation = self.take_upload(handle)?;
        self.operation_pause
            .pause_if_selected(UploadPausePoint::Complete)
            .await;
        let result = self
            .complete_inner(operation.upload_mut(), &request.grant)
            .await;
        result.map(|()| status_json(operation.upload()))
    }

    async fn complete_inner(
        &self,
        upload: &mut StoredUpload,
        grant: &str,
    ) -> Result<(), UploadError> {
        self.require_current_grant(upload, grant)?;
        if upload.mode != TransferMode::File || upload.received_bytes != upload.expected_bytes {
            return Err(UploadError::new(UploadErrorKind::UploadConflict));
        }
        if upload.state == UploadState::Ready {
            drop(upload.active_lease.take());
            return Ok(());
        }
        if upload.state != UploadState::Transferring && upload.state != UploadState::Verifying {
            return Err(UploadError::new(UploadErrorKind::UploadConflict));
        }
        let checksum = UploadChecksum::parse(&hex_digest(upload.whole_hasher.clone().finalize()))?;
        if upload.state == UploadState::Transferring {
            self.transition(upload, UploadTransition::Complete, "complete")
                .await?;
        }
        self.file
            .verify(VerifyTransfer::new(&upload.handle, &checksum))
            .await?;
        self.transition(upload, UploadTransition::Accept, "accept")
            .await?;
        drop(upload.active_lease.take());
        Ok(())
    }

    pub(super) async fn cancel(&self, handle: &str, grant: &str) -> Result<Value, UploadError> {
        self.record_call();
        let mut operation = self.take_upload(handle)?;
        self.operation_pause
            .pause_if_selected(UploadPausePoint::Cancel)
            .await;
        self.require_current_grant(operation.upload(), grant)?;
        let result = self.cancel_inner(operation.upload_mut()).await;
        result.map(|()| status_json(operation.upload()))
    }

    async fn cancel_inner(&self, upload: &mut StoredUpload) -> Result<(), UploadError> {
        match upload.mode {
            TransferMode::File => self.file.cancel(&upload.handle).await?,
            TransferMode::Direct => self.direct.cancel(&upload.handle).await?,
        }
        self.transition(upload, UploadTransition::Cancel, "cancel")
            .await?;
        drop(upload.active_lease.take());
        Ok(())
    }

    pub(super) async fn reacquire(&self, handle: &str, grant: &str) -> Result<Value, UploadError> {
        self.record_call();
        let mut operation = self.take_upload(handle)?;
        self.operation_pause
            .pause_if_selected(UploadPausePoint::Reacquire)
            .await;
        self.require_current_grant(operation.upload(), grant)?;
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
        let issued = self.grants.issue(
            TransferGrantRequest::new(scope, UnixMillis::new(61_000)),
            CREATED_AT,
        )?;
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

    pub(super) async fn retire(&self) -> Result<(), String> {
        self.uploads.retired.store(true, Ordering::Release);
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
        self.file.retire();
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
                UploadSlot::Ready(upload) => Some(upload),
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

fn idempotency(value: &str) -> Result<UploadIdempotencyKey, UploadError> {
    UploadIdempotencyKey::parse(value)
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
            vec![
                ActionMetadata::new_with_contract(
                    ActionName::parse("refresh").expect("static action identity"),
                    1,
                    ActionArgumentSchema::empty(),
                    AuthorizationRequirement::Current,
                    ValidationSelection::ComponentAndArguments,
                    TransactionPolicy::None,
                )
                .expect("static action metadata"),
            ],
        )
        .expect("static component metadata")
    })
}

fn deterministic_bytes<const LENGTH: usize>(start: u8) -> [u8; LENGTH] {
    std::array::from_fn(|index| start.wrapping_add(index as u8))
}

struct AxumChunkBody {
    body: Body,
    hasher: Sha256,
    interrupt_after_first: bool,
    yielded_chunks: usize,
    shutdown: watch::Receiver<bool>,
}

impl AxumChunkBody {
    fn new(
        body: Body,
        hasher: Sha256,
        interrupt_after_first: bool,
        shutdown: watch::Receiver<bool>,
    ) -> Self {
        Self {
            body,
            hasher,
            interrupt_after_first,
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
                    if data.len() > maximum_bytes {
                        return Err(UploadError::new(UploadErrorKind::InputTooLarge));
                    }
                    self.hasher.update(&data);
                    self.yielded_chunks = self.yielded_chunks.saturating_add(1);
                    return Ok(Some(QuarantineBytes::copy_from_slice(&data)));
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
        let runtime = Arc::new(
            UploadRuntime::open(
                &root,
                ReferenceFaultSchedule::None,
                shutdown,
                Arc::clone(&active),
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
            runtime.operation_pause.select(point);
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
                    UploadPausePoint::Complete => {
                        operation_runtime
                            .complete(
                                &operation_handle,
                                CompleteUploadRequest {
                                    grant: operation_grant,
                                },
                            )
                            .await
                    }
                    UploadPausePoint::Cancel => {
                        operation_runtime
                            .cancel(&operation_handle, &operation_grant)
                            .await
                    }
                    UploadPausePoint::Reacquire => {
                        operation_runtime
                            .reacquire(&operation_handle, &operation_grant)
                            .await
                    }
                }
            });
            timeout(
                Duration::from_secs(1),
                runtime.operation_pause.wait_until_entered(),
            )
            .await
            .expect("operation reached cancellation point");
            task.abort();
            assert!(task.await.expect_err("operation aborted").is_cancelled());
            runtime.operation_pause.resume();

            let coherent = runtime
                .status(&handle, &grant)
                .await
                .expect("successor request sees restored upload");
            assert_eq!(coherent["handle"], handle);
            assert_eq!(coherent["state"], "created");
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
}
