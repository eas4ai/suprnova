//! Framework-owned HTTP adaptation for the Live upload protocol.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::sync::{Mutex, Weak};

use bytes::Bytes;
use hmac::digest::KeyInit;
use hmac::{Hmac, Mac};
use hyper::Method;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use suprnova_live::identity::{
    BuildId, ComponentName, ContentDigest, IslandSlot, ModelField, RouteIdentity, ScopeFingerprint,
};
use suprnova_live::mount::DocumentMountKey;
use suprnova_live::resource::CancellationFlag;
use suprnova_live::upload::{
    AcceptedChunk, CancelUpload, ChunkBody, ClientUploadMetadata, CompleteUpload,
    DirectPartReference, DirectTransferInstruction, PrepareTransfer, QuarantineBytes, StatusUpload,
    TransferGrant, TransferInstruction, TransitionDisposition, UploadChecksum,
    UploadCreationRequest, UploadError, UploadErrorKind, UploadHandle, UploadIdempotencyKey,
    UploadOperation, UploadProtocolCodec, UploadReacquireRequest, UploadRevision, UploadState,
    UploadTransition, UploadTransitionAdmission, UploadTransitionRequest,
    UploadValidationDisposition, UploadValidationRequest, WriteChunk,
};
use uuid::Uuid;

use crate::{FrameworkError, HttpResponse, Request, Response, RouteBuilder, Router};

impl Router {
    /// Registers an authenticated application-owned upload-reacquisition route.
    ///
    /// The caller chooses the URL outside `/__live` and composes its ordinary
    /// authentication/session/tenant middleware on the returned builder. Live
    /// never installs an application reacquisition URL implicitly.
    pub fn try_live_upload_reacquisition(self, path: &str) -> Result<RouteBuilder, FrameworkError> {
        if path
            .split('/')
            .filter(|segment| *segment == "{handle}")
            .count()
            != 1
        {
            return Err(FrameworkError::internal(
                "A Live upload reacquisition route requires exactly one {handle} parameter",
            ));
        }
        self.try_post(path, reacquire_upload)?
            .with_live_route_metadata(super::context::LiveRouteMetadata::new(
                super::attestation::LiveOperation::Upload,
                super::routes::strict_action_policy(),
            ))
    }
}

/// Exact trusted facts purpose-binding one upload to one finalized island mount.
#[derive(Clone)]
pub(crate) struct UploadMountScopeBinding {
    pub(crate) base_scope: ScopeFingerprint,
    pub(crate) route: RouteIdentity,
    pub(crate) slot: IslandSlot,
    pub(crate) component: ComponentName,
    pub(crate) contract: ContentDigest,
    pub(crate) build: BuildId,
    pub(crate) document_key: DocumentMountKey,
    pub(crate) protocol: u16,
}

impl fmt::Debug for UploadMountScopeBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<UploadMountScopeBinding:redacted>")
    }
}

/// Derives the opaque upload authority scope without changing the v1 grant schema.
pub(crate) fn derive_mount_scope(
    binding: &UploadMountScopeBinding,
) -> Result<ScopeFingerprint, FrameworkError> {
    let mut digest = Sha256::new();
    digest.update(b"suprnova-live/upload-mount-scope/v1\0");
    for part in [
        binding.base_scope.as_bytes(),
        binding.route.as_bytes(),
        binding.slot.as_str().as_bytes(),
        binding.component.as_str().as_bytes(),
        binding.contract.as_bytes(),
        binding.build.as_str().as_bytes(),
        binding.document_key.as_str().as_bytes(),
        &binding.protocol.to_be_bytes(),
    ] {
        let length = u16::try_from(part.len()).map_err(|_| upload_error())?;
        digest.update(length.to_be_bytes());
        digest.update(part);
    }
    let bytes: [u8; 32] = digest.finalize().into();
    ScopeFingerprint::from_bytes(&bytes).map_err(|_| upload_error())
}

type HmacSha256 = Hmac<Sha256>;
const MAX_BODY_SEGMENT_COPY_BYTES: usize = 64 * 1024;

pub(crate) struct UploadBodyBudget {
    permits: Arc<tokio::sync::Semaphore>,
    maximum_bytes: u32,
}

#[derive(Default)]
pub(crate) struct UploadOperationLocks {
    locks: Mutex<HashMap<UploadHandle, Weak<tokio::sync::Mutex<()>>>>,
}

impl UploadOperationLocks {
    fn lock_for(&self, handle: &UploadHandle) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self
            .locks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(handle).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        locks.insert(handle.clone(), Arc::downgrade(&lock));
        lock
    }

    pub(crate) async fn acquire(&self, handle: &UploadHandle) -> tokio::sync::OwnedMutexGuard<()> {
        self.lock_for(handle).lock_owned().await
    }

    pub(crate) fn try_acquire(
        &self,
        handle: &UploadHandle,
    ) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        self.lock_for(handle).try_lock_owned().ok()
    }
}

impl UploadBodyBudget {
    pub(crate) fn new(maximum_bytes: usize) -> Result<Self, FrameworkError> {
        let maximum_bytes = u32::try_from(maximum_bytes).map_err(|_| upload_error())?;
        if maximum_bytes == 0 {
            return Err(upload_error());
        }
        Ok(Self {
            permits: Arc::new(tokio::sync::Semaphore::new(maximum_bytes as usize)),
            maximum_bytes,
        })
    }

    pub(crate) async fn acquire(
        &self,
        bytes: usize,
    ) -> Result<tokio::sync::OwnedSemaphorePermit, UploadError> {
        let bytes = u32::try_from(bytes)
            .ok()
            .filter(|bytes| *bytes > 0)
            .ok_or_else(|| upload_kind(UploadErrorKind::InputTooLarge))?;
        if bytes > self.maximum_bytes {
            return Err(upload_kind(UploadErrorKind::ResourceExhausted));
        }
        Arc::clone(&self.permits)
            .acquire_many_owned(bytes)
            .await
            .map_err(|_| upload_kind(UploadErrorKind::ProviderUnavailable))
    }
}

const UPLOAD_HANDLE_KEY_PURPOSE: &[u8] = b"suprnova-live/upload-handle-key/v1\0";
const UPLOAD_HANDLE_PURPOSE: &[u8] = b"suprnova-live/upload-handle/v1\0";

/// Derives one opaque deterministic upload handle from trusted authority and retry identity.
pub(crate) fn derive_upload_handle(
    root_key: &[u8],
    scope: &ScopeFingerprint,
    field: &ModelField,
    idempotency_key: &UploadIdempotencyKey,
) -> Result<UploadHandle, FrameworkError> {
    let mut key_mac = HmacSha256::new_from_slice(root_key).map_err(|_| upload_error())?;
    key_mac.update(UPLOAD_HANDLE_KEY_PURPOSE);
    let derived_key = key_mac.finalize().into_bytes();

    let mut handle_mac = HmacSha256::new_from_slice(&derived_key).map_err(|_| upload_error())?;
    handle_mac.update(UPLOAD_HANDLE_PURPOSE);
    for part in [
        scope.as_bytes(),
        field.as_str().as_bytes(),
        idempotency_key.as_str().as_bytes(),
    ] {
        let length = u16::try_from(part.len()).map_err(|_| upload_error())?;
        handle_mac.update(&length.to_be_bytes());
        handle_mac.update(part);
    }
    let digest = handle_mac.finalize().into_bytes();
    let mut bytes: [u8; 16] = digest[..16].try_into().expect("fixed HMAC digest");
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    UploadHandle::parse(&Uuid::from_bytes(bytes).hyphenated().to_string())
        .map_err(|_| upload_error())
}

/// Derives the active handle followed by accepted rotation-window candidates.
pub(crate) fn derive_upload_handle_candidates(
    current_key: &[u8],
    previous_keys: &[&[u8]],
    scope: &ScopeFingerprint,
    field: &ModelField,
    idempotency_key: &UploadIdempotencyKey,
) -> Result<Vec<UploadHandle>, FrameworkError> {
    let mut handles = Vec::with_capacity(previous_keys.len().saturating_add(1));
    for key in std::iter::once(current_key).chain(previous_keys.iter().copied()) {
        let handle = derive_upload_handle(key, scope, field, idempotency_key)?;
        if !handles.contains(&handle) {
            handles.push(handle);
        }
    }
    Ok(handles)
}

fn upload_error() -> FrameworkError {
    FrameworkError::internal("Live upload request was rejected")
}

pub(crate) async fn handle(request: Request) -> Response {
    if request.method() != Method::POST {
        return Ok(closed_response(405).header("Allow", "POST"));
    }
    if request.header("x-suprnova-live") != Some("upload-v1") {
        return Ok(closed_response(400));
    }
    let control = match request.header("content-type") {
        Some("application/json") => true,
        Some("application/octet-stream") => false,
        _ => return Ok(closed_response(415)),
    };
    let runtime = match super::runtime::LiveRuntime::bind() {
        Ok(runtime) => runtime,
        Err(_) => return Ok(semantic_error(UploadErrorKind::ProviderUnavailable)),
    };
    if runtime.ensure_upload_cleanup_runner().is_err() {
        return Ok(semantic_error(UploadErrorKind::ProviderUnavailable));
    }
    let response = if control {
        let request = match request.buffer_body(16 * 1024).await {
            Ok(request) => request,
            Err(error) if error.status_code() == 413 => return Ok(closed_response(413)),
            Err(_) => return Ok(closed_response(500)),
        };
        let Some(body) = request.cached_body() else {
            return Ok(closed_response(500));
        };
        if !matches!(
            serde_json::from_slice(body),
            Ok(serde_json::Value::Object(_))
        ) {
            return Ok(closed_response(400));
        }
        dispatch_control(&runtime, &request, body).await
    } else {
        dispatch_chunk(&runtime, request).await
    };

    Ok(match response {
        Ok(response) => response,
        Err(error) => semantic_error(error.kind()),
    })
}

async fn reacquire_upload(request: Request) -> Response {
    let response = reacquire_upload_inner(request).await;
    Ok(match response {
        Ok(response) => response,
        Err(error) => semantic_error(error.kind()),
    })
}

async fn reacquire_upload_inner(request: Request) -> Result<HttpResponse, UploadError> {
    if request.method() != Method::POST {
        return Err(upload_kind(UploadErrorKind::UnsupportedOperation));
    }
    if request.header("content-type") != Some("application/json") {
        return Err(upload_kind(UploadErrorKind::InvalidField));
    }
    let route_handle = request
        .param("handle")
        .map_err(|_| upload_kind(UploadErrorKind::MissingField))
        .and_then(UploadHandle::parse)?;
    let request = request.buffer_body(16 * 1024).await.map_err(|error| {
        if error.status_code() == 413 {
            upload_kind(UploadErrorKind::InputTooLarge)
        } else {
            upload_kind(UploadErrorKind::BodyInterrupted)
        }
    })?;
    let body = request
        .cached_body()
        .ok_or_else(|| upload_kind(UploadErrorKind::BodyInterrupted))?;
    let operation = UploadProtocolCodec::v1().decode(body)?;
    let UploadOperation::Reacquire(reacquire) = operation else {
        return Err(upload_kind(UploadErrorKind::UnsupportedOperation));
    };
    if reacquire.handle() != &route_handle {
        return Err(upload_kind(UploadErrorKind::ScopeMismatch));
    }

    let runtime = super::runtime::LiveRuntime::bind()
        .map_err(|_| upload_kind(UploadErrorKind::ProviderUnavailable))?;
    let now = upload_now(&runtime)?;
    let (context, record) = runtime
        .resolve_upload_request_context(&request, &route_handle)
        .await
        .map_err(|_| upload_kind(UploadErrorKind::ScopeMismatch))?;
    let grant_expires_at = suprnova_live::identity::UnixMillis::new(
        now.get()
            .checked_add(60_000)
            .ok_or_else(|| upload_kind(UploadErrorKind::ResourceExhausted))?
            .min(record.expires_at().get()),
    );
    let outcome = runtime
        .upload_authority()
        .reacquire(
            &context,
            UploadReacquireRequest::new(
                route_handle.clone(),
                record.authority().field().clone(),
                grant_expires_at,
            ),
            now,
        )
        .await?;
    let metadata = runtime
        .upload_reverse_proxy_adapter()
        .create_metadata(&route_handle, now)?;
    let progress = runtime.upload_provider_adapter().progress(&route_handle)?;
    if progress.expected_bytes != metadata.expected_bytes()
        || progress.committed_bytes > metadata.expected_bytes()
    {
        return Err(upload_kind(UploadErrorKind::UploadConflict));
    }
    Ok(json_response(
        200,
        serde_json::json!({
            "fileIdentity": {
                "lastModified": metadata.last_modified(),
                "name": metadata.client().display_name(),
                "size": metadata.expected_bytes(),
                "type": metadata.client().claimed_media_type().unwrap_or(""),
            },
            "grant": outcome.grant().expose_bearer(),
            "nextChunkIndex": progress.next_chunk_index,
            "revision": outcome.record().revision().get().to_string(),
            "state": browser_upload_state(outcome.record().state()),
            "uploadedBytes": progress.committed_bytes,
        }),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserCreateUpload {
    protocol_version: u16,
    operation: String,
    field: String,
    file: BrowserUploadFile,
    idempotency_key: String,
    island: BrowserUploadIsland,
    #[serde(default)]
    mode: BrowserUploadMode,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BrowserUploadMode {
    #[default]
    #[serde(alias = "file")]
    ReverseProxy,
    Direct,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserReportDirectPart {
    protocol_version: u16,
    operation: String,
    handle: String,
    expected_revision: String,
    idempotency_key: String,
    part: u32,
    reference: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserUploadFile {
    #[serde(rename = "lastModified")]
    last_modified: u64,
    name: String,
    size: u64,
    #[serde(rename = "type")]
    claimed_media_type: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserUploadIsland {
    component: String,
    #[serde(rename = "documentKey")]
    document_key: String,
    slot: String,
}

async fn dispatch_control(
    runtime: &super::runtime::LiveRuntime,
    request: &Request,
    body: &[u8],
) -> Result<HttpResponse, UploadError> {
    let operation = serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("operation")?.as_str().map(str::to_owned))
        .ok_or_else(|| upload_kind(UploadErrorKind::InvalidField))?;
    if operation == "create" {
        let create = serde_json::from_slice::<BrowserCreateUpload>(body)
            .map_err(|_| upload_kind(UploadErrorKind::InvalidField))?;
        return create_upload(runtime, request, create).await;
    }
    if operation == "report_direct_part" {
        let report = serde_json::from_slice::<BrowserReportDirectPart>(body)
            .map_err(|_| upload_kind(UploadErrorKind::InvalidField))?;
        let grant = parse_grant(request)?;
        return report_direct_part(runtime, request, grant, report).await;
    }
    let operation = UploadProtocolCodec::v1().decode(body)?;
    let grant = parse_grant(request)?;
    match operation {
        UploadOperation::Status(status) => status_upload(runtime, request, grant, status).await,
        UploadOperation::Complete(complete) => {
            complete_upload(runtime, request, grant, complete).await
        }
        UploadOperation::Cancel(cancel) => cancel_upload(runtime, request, grant, cancel).await,
        UploadOperation::Create(_)
        | UploadOperation::PutChunk(_)
        | UploadOperation::Reacquire(_) => Err(upload_kind(UploadErrorKind::UnsupportedOperation)),
    }
}

async fn create_upload(
    runtime: &super::runtime::LiveRuntime,
    request: &Request,
    create: BrowserCreateUpload,
) -> Result<HttpResponse, UploadError> {
    if create.protocol_version != 1 || create.operation != "create" {
        return Err(upload_kind(UploadErrorKind::UnsupportedProtocol));
    }
    let field =
        ModelField::parse(&create.field).map_err(|_| upload_kind(UploadErrorKind::InvalidField))?;
    let idempotency_key = UploadIdempotencyKey::parse(&create.idempotency_key)?;
    let context = runtime
        .validate_upload_request_context(
            request,
            &create.island.component,
            &create.island.slot,
            &create.island.document_key,
        )
        .map_err(|_| upload_kind(UploadErrorKind::ScopeMismatch))?;
    let component = context.mount().component().clone();
    let policy = runtime
        .upload_policy(&component, &field)
        .map_err(|_| upload_kind(UploadErrorKind::UnknownField))?;
    if create.file.size == 0
        || create.file.size > policy.maximum_file_bytes()
        || create.file.size > runtime.upload_limits().max_file_bytes()
    {
        return Err(upload_kind(UploadErrorKind::InputTooLarge));
    }
    let client =
        ClientUploadMetadata::new(&create.file.name, create.file.claimed_media_type.as_deref())?;
    let candidates = runtime
        .derive_upload_handle_candidates(context.scope(), &field, &idempotency_key)
        .map_err(|_| upload_kind(UploadErrorKind::ProviderUnavailable))?;
    runtime
        .authorize_upload_create(&component, &field)
        .await
        .map_err(|_| upload_kind(UploadErrorKind::AuthorizationDenied))?;
    let now = upload_now(runtime)?;
    let (handle, expires_at) =
        select_create_handle(runtime, &context, &field, &candidates, now).await?;
    let _operation = runtime.upload_operation_locks().acquire(&handle).await;
    let metadata = super::ports::upload_provider::UploadCreateMetadata::new(
        client,
        create.file.size,
        create.file.last_modified,
        expires_at,
        context.scope().clone(),
    );
    let memo_disposition = runtime
        .upload_reverse_proxy_adapter()
        .bind_create_metadata(handle.clone(), metadata, now)?;
    let outcome = runtime
        .upload_authority()
        .create(
            &context,
            UploadCreationRequest::new(
                handle.clone(),
                field.clone(),
                idempotency_key.clone(),
                expires_at,
                create.file.size,
                policy.clone(),
            ),
            now,
        )
        .await;
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(error) => {
            if memo_disposition
                == super::ports::upload_provider::UploadCreateMetadataDisposition::Inserted
                && runtime
                    .upload_ledger()
                    .load(&handle)
                    .await
                    .ok()
                    .flatten()
                    .is_none()
            {
                runtime
                    .upload_reverse_proxy_adapter()
                    .remove_create_metadata(&handle);
            }
            return Err(error);
        }
    };
    let prepare = PrepareTransfer::new(&handle, create.file.size, &create.file.name, now);
    let plan = match create.mode {
        BrowserUploadMode::ReverseProxy => {
            runtime
                .upload_provider_adapter()
                .prepare_reverse(prepare, context.scope())
                .await
        }
        BrowserUploadMode::Direct => {
            runtime
                .upload_provider_adapter()
                .prepare_direct(prepare, context.scope())
                .await
        }
    };
    let plan = match plan {
        Ok(plan) => plan,
        Err(error) => {
            if let Ok(failure_key) = phase_idempotency(&idempotency_key, "prepare-failed") {
                let _ = transition(
                    runtime,
                    &context,
                    outcome.grant(),
                    &field,
                    &handle,
                    outcome.record().revision(),
                    failure_key,
                    UploadTransition::Fail,
                    now,
                )
                .await;
            }
            if memo_disposition
                == super::ports::upload_provider::UploadCreateMetadataDisposition::Inserted
            {
                runtime
                    .upload_reverse_proxy_adapter()
                    .remove_create_metadata(&handle);
            }
            let _ = runtime.upload_provider().cancel(&handle).await;
            let _ = runtime.wake_upload_cleanup();
            return Err(error);
        }
    };
    let _ = runtime.wake_upload_cleanup();
    let instructions = plan
        .instructions()
        .map(transfer_instruction_json)
        .collect::<Vec<_>>();
    let mut response = serde_json::json!({
        "grant": outcome.grant().expose_bearer(),
        "handle": outcome.record().authority().handle().to_string(),
        "instructions": instructions,
        "maximumChunkBytes": plan.maximum_chunk_bytes(),
        "mode": match create.mode {
            BrowserUploadMode::ReverseProxy => "reverse_proxy",
            BrowserUploadMode::Direct => "direct",
        },
        "revision": outcome.record().revision().get().to_string(),
        "state": browser_upload_state(outcome.record().state()),
    });
    if instructions.len() == 1 {
        response["instruction"] = instructions[0].clone();
    }
    Ok(json_response(201, response))
}

fn transfer_instruction_json(instruction: &TransferInstruction) -> serde_json::Value {
    match instruction {
        TransferInstruction::ReverseProxy { maximum_bytes } => serde_json::json!({
            "maximum_bytes": maximum_bytes,
            "mode": "reverse_proxy",
        }),
        TransferInstruction::Direct(instruction) => direct_instruction_json(instruction),
    }
}

fn direct_instruction_json(instruction: &DirectTransferInstruction) -> serde_json::Value {
    serde_json::json!({
        "expires_at": instruction.expires_at().get(),
        "headers": instruction.required_headers().iter().map(|(name, value)| {
            (name.as_str().to_owned(), serde_json::Value::String(value.to_owned()))
        }).collect::<serde_json::Map<_, _>>(),
        "maximum_bytes": instruction.maximum_bytes(),
        "method": instruction.method().as_str(),
        "offset": instruction.part().offset(),
        "part": instruction.part().index(),
        "reference": instruction.reference().as_str(),
        "url": instruction.endpoint().as_str(),
    })
}

async fn select_create_handle(
    runtime: &super::runtime::LiveRuntime,
    context: &suprnova_live::host::TrustedLiveRequestContext,
    field: &ModelField,
    candidates: &[UploadHandle],
    now: suprnova_live::identity::UnixMillis,
) -> Result<(UploadHandle, suprnova_live::identity::UnixMillis), UploadError> {
    let mut found = None;
    for candidate in candidates {
        let Some(record) = runtime.upload_ledger().load(candidate).await? else {
            continue;
        };
        let exact_authority = record.authority().component() == context.mount().component()
            && record.authority().field() == field
            && record.authority().host_scope().scope() == context.scope()
            && record.authority().upload_protocol() == 1;
        if !exact_authority || found.is_some() {
            return Err(upload_kind(UploadErrorKind::UploadConflict));
        }
        found = Some((candidate.clone(), record.expires_at()));
    }
    if let Some(found) = found {
        return Ok(found);
    }
    let handle = candidates
        .first()
        .cloned()
        .ok_or_else(|| upload_kind(UploadErrorKind::ProviderUnavailable))?;
    let expires_at = suprnova_live::identity::UnixMillis::new(
        now.get()
            .checked_add(runtime.upload_limits().max_age_ms())
            .ok_or_else(|| upload_kind(UploadErrorKind::UploadExpired))?,
    );
    Ok((handle, expires_at))
}

async fn status_upload(
    runtime: &super::runtime::LiveRuntime,
    request: &Request,
    grant: TransferGrant,
    status: StatusUpload,
) -> Result<HttpResponse, UploadError> {
    let now = upload_now(runtime)?;
    let (context, record) = runtime
        .resolve_upload_request_context(request, status.handle())
        .await
        .map_err(|_| upload_kind(UploadErrorKind::ScopeMismatch))?;
    let field = record.authority().field().clone();
    let record = runtime
        .upload_authority()
        .status(&context, grant, field, status.handle().clone(), now)
        .await?;
    runtime
        .upload_reverse_proxy_adapter()
        .create_metadata(status.handle(), now)?;
    let progress = runtime
        .upload_provider_adapter()
        .progress(status.handle())?;
    Ok(json_response(
        200,
        serde_json::json!({
            "committedBytes": progress.committed_bytes,
            "expectedBytes": progress.expected_bytes,
            "nextChunkIndex": progress.next_chunk_index,
            "revision": record.revision().get().to_string(),
            "state": browser_upload_state(record.state()),
        }),
    ))
}

async fn complete_upload(
    runtime: &super::runtime::LiveRuntime,
    request: &Request,
    grant: TransferGrant,
    complete: CompleteUpload,
) -> Result<HttpResponse, UploadError> {
    let _operation = runtime
        .upload_operation_locks()
        .acquire(complete.handle())
        .await;
    let now = upload_now(runtime)?;
    let (context, record) = runtime
        .resolve_upload_request_context(request, complete.handle())
        .await
        .map_err(|_| upload_kind(UploadErrorKind::ScopeMismatch))?;
    let field = record.authority().field().clone();
    let metadata = runtime
        .upload_reverse_proxy_adapter()
        .create_metadata(complete.handle(), now)?;
    let progress = runtime
        .upload_provider_adapter()
        .progress(complete.handle())?;
    if progress.expected_bytes != metadata.expected_bytes()
        || progress.committed_bytes != metadata.expected_bytes()
    {
        return Err(upload_kind(UploadErrorKind::IncompleteTransfer));
    }
    let verifying = runtime
        .upload_authority()
        .transition(
            &context,
            UploadTransitionAdmission::new(
                grant,
                field.clone(),
                UploadTransitionRequest::new(
                    complete.handle().clone(),
                    complete.expected_revision(),
                    complete.idempotency_key().clone(),
                    UploadTransition::Complete,
                ),
            ),
            now,
        )
        .await?;
    if verifying.disposition() == TransitionDisposition::ExistingOutcome {
        let current = runtime
            .upload_ledger()
            .load(complete.handle())
            .await?
            .ok_or_else(|| upload_kind(UploadErrorKind::UploadConflict))?;
        let post_validation_revision = verifying
            .revision()
            .get()
            .checked_add(1)
            .ok_or_else(|| upload_kind(UploadErrorKind::RevisionExhausted))?;
        if current.revision().get() == post_validation_revision {
            let status = match current.state() {
                UploadState::Ready => 200,
                UploadState::Rejected => 422,
                _ => return Err(upload_kind(UploadErrorKind::UploadConflict)),
            };
            return Ok(json_response(
                status,
                serde_json::json!({
                    "revision": current.revision().get().to_string(),
                    "state": browser_upload_state(current.state()),
                }),
            ));
        }
        if current.revision() != verifying.revision() || current.state() != UploadState::Verifying {
            return Err(upload_kind(UploadErrorKind::UploadConflict));
        }
    }
    let policy = runtime
        .upload_policy(context.mount().component(), &field)
        .map_err(|_| upload_kind(UploadErrorKind::UnknownField))?;
    let validation_key = phase_idempotency(complete.idempotency_key(), "validate")?;
    let validation = runtime
        .upload_validation()
        .validate(
            &context,
            UploadValidationRequest::new(
                complete.handle().clone(),
                field,
                verifying.revision(),
                validation_key,
                metadata.client().clone(),
                metadata.expected_bytes(),
                complete.whole_checksum().clone(),
                policy,
            ),
            now,
        )
        .await?;
    if validation.disposition() == UploadValidationDisposition::Retry {
        return Ok(json_response(
            503,
            serde_json::json!({
                "revision": verifying.revision().get().to_string(),
                "state": browser_upload_state(verifying.state()),
            }),
        ));
    }
    let transition = validation
        .transition()
        .ok_or_else(|| upload_kind(UploadErrorKind::ValidationEvidenceUnavailable))?;
    let status = if validation.disposition() == UploadValidationDisposition::Ready {
        200
    } else {
        let _ = runtime.wake_upload_cleanup();
        422
    };
    Ok(json_response(
        status,
        serde_json::json!({
            "revision": transition.revision().get().to_string(),
            "state": browser_upload_state(transition.state()),
        }),
    ))
}

async fn report_direct_part(
    runtime: &super::runtime::LiveRuntime,
    request: &Request,
    grant: TransferGrant,
    report: BrowserReportDirectPart,
) -> Result<HttpResponse, UploadError> {
    if report.protocol_version != 1 || report.operation != "report_direct_part" {
        return Err(upload_kind(UploadErrorKind::UnsupportedProtocol));
    }
    let handle = UploadHandle::parse(&report.handle)?;
    let expected_revision = UploadRevision::parse(&report.expected_revision)?;
    let idempotency_key = UploadIdempotencyKey::parse(&report.idempotency_key)?;
    let reference = DirectPartReference::parse(&report.reference)?;
    let _operation = runtime.upload_operation_locks().acquire(&handle).await;
    let now = upload_now(runtime)?;
    let (context, record) = runtime
        .resolve_upload_request_context(request, &handle)
        .await
        .map_err(|_| upload_kind(UploadErrorKind::ScopeMismatch))?;
    if runtime.upload_provider_adapter().mode(&handle)?
        != super::ports::upload_provider::UploadProviderMode::Direct
    {
        return Err(upload_kind(UploadErrorKind::UnsupportedOperation));
    }
    let field = record.authority().field().clone();
    runtime
        .upload_authority()
        .status(&context, grant.clone(), field.clone(), handle.clone(), now)
        .await?;
    let part = runtime
        .upload_provider_adapter()
        .direct_part(&handle, report.part, &reference)?;
    let checksum = direct_part_checksum(&handle, &part, &reference)?;

    let mut transition_revision = expected_revision;
    if part.index() == 0 {
        let queued = transition(
            runtime,
            &context,
            &grant,
            &field,
            &handle,
            transition_revision,
            phase_idempotency(&idempotency_key, "queue")?,
            UploadTransition::Queue,
            now,
        )
        .await?;
        transition_revision = queued.revision();
        let begun = transition(
            runtime,
            &context,
            &grant,
            &field,
            &handle,
            transition_revision,
            phase_idempotency(&idempotency_key, "begin")?,
            UploadTransition::BeginTransfer,
            now,
        )
        .await?;
        transition_revision = begun.revision();
    }

    let accepted_chunk = AcceptedChunk::new(part.index(), part.bytes(), checksum.clone())?;
    let current = runtime
        .upload_authority()
        .status(&context, grant.clone(), field.clone(), handle.clone(), now)
        .await?;
    if current.revision() != transition_revision {
        let replay = transition(
            runtime,
            &context,
            &grant,
            &field,
            &handle,
            transition_revision,
            idempotency_key,
            UploadTransition::PutChunk(accepted_chunk),
            now,
        )
        .await?;
        let progress = runtime.upload_provider_adapter().progress(&handle)?;
        let end = part
            .offset()
            .checked_add(part.bytes())
            .ok_or_else(|| upload_kind(UploadErrorKind::InputTooLarge))?;
        if progress.next_chunk_index <= part.index() || progress.committed_bytes < end {
            return Err(upload_kind(UploadErrorKind::ReconciliationRequired));
        }
        return Ok(direct_part_response(&replay, progress, None));
    }

    let progress = runtime.upload_provider_adapter().progress(&handle)?;
    if progress.next_chunk_index != part.index()
        || progress.committed_bytes != part.offset()
        || progress.committed_bytes >= progress.expected_bytes
    {
        return Err(upload_kind(UploadErrorKind::UploadConflict));
    }
    let receipt = runtime
        .upload_provider_adapter()
        .report_direct_part(&handle, part.index(), reference, now)
        .await?;
    let accepted = transition(
        runtime,
        &context,
        &grant,
        &field,
        &handle,
        transition_revision,
        idempotency_key,
        UploadTransition::PutChunk(AcceptedChunk::new(part.index(), receipt.bytes(), checksum)?),
        now,
    )
    .await?;
    let progress = runtime.upload_provider_adapter().progress(&handle)?;
    Ok(direct_part_response(&accepted, progress, Some(&receipt)))
}

fn direct_part_checksum(
    handle: &UploadHandle,
    part: &suprnova_live::upload::UploadPart,
    reference: &DirectPartReference,
) -> Result<UploadChecksum, UploadError> {
    let mut digest = Sha256::new();
    digest.update(b"suprnova-live/direct-part-receipt/v1\0");
    digest.update(handle.to_string().as_bytes());
    digest.update(part.index().to_be_bytes());
    digest.update(part.offset().to_be_bytes());
    digest.update(part.bytes().to_be_bytes());
    digest.update(reference.as_str().as_bytes());
    UploadChecksum::parse(&hex::encode(digest.finalize()))
}

fn direct_part_response(
    accepted: &suprnova_live::upload::TransitionOutcome,
    progress: super::ports::upload_provider::UploadTransferProgress,
    receipt: Option<&suprnova_live::upload::ChunkReceipt>,
) -> HttpResponse {
    let mut response = serde_json::json!({
        "nextChunkIndex": progress.next_chunk_index,
        "revision": accepted.revision().get().to_string(),
        "state": browser_upload_state(accepted.state()),
        "uploadedBytes": progress.committed_bytes,
    });
    if let Some(receipt) = receipt {
        response["receipt"] = serde_json::json!({
            "bytes": receipt.bytes(),
            "offset": receipt.offset(),
            "part": receipt.index(),
        });
        if let Some(instruction) = receipt
            .next_instruction()
            .and_then(TransferInstruction::as_direct)
        {
            response["instruction"] = direct_instruction_json(instruction);
        }
    }
    json_response(200, response)
}

async fn cancel_upload(
    runtime: &super::runtime::LiveRuntime,
    request: &Request,
    grant: TransferGrant,
    cancel: CancelUpload,
) -> Result<HttpResponse, UploadError> {
    let _operation = runtime
        .upload_operation_locks()
        .acquire(cancel.handle())
        .await;
    let now = upload_now(runtime)?;
    let (context, record) = runtime
        .resolve_upload_request_context(request, cancel.handle())
        .await
        .map_err(|_| upload_kind(UploadErrorKind::ScopeMismatch))?;
    let transition = runtime
        .upload_authority()
        .transition(
            &context,
            UploadTransitionAdmission::new(
                grant,
                record.authority().field().clone(),
                UploadTransitionRequest::new(
                    cancel.handle().clone(),
                    cancel.expected_revision(),
                    cancel.idempotency_key().clone(),
                    UploadTransition::Cancel,
                ),
            ),
            now,
        )
        .await?;
    runtime.upload_provider().cancel(cancel.handle()).await?;
    runtime
        .upload_reverse_proxy_adapter()
        .remove_create_metadata(cancel.handle());
    let _ = runtime.wake_upload_cleanup();
    Ok(json_response(
        200,
        serde_json::json!({
            "revision": transition.revision().get().to_string(),
            "state": browser_upload_state(transition.state()),
        }),
    ))
}

async fn dispatch_chunk(
    runtime: &super::runtime::LiveRuntime,
    request: Request,
) -> Result<HttpResponse, UploadError> {
    if request.header("x-suprnova-upload-operation") != Some("put_chunk") {
        return Err(upload_kind(UploadErrorKind::UnsupportedOperation));
    }
    let grant = parse_grant(&request)?;
    let handle = parse_header(&request, "x-suprnova-upload-handle", UploadHandle::parse)?;
    let _operation = runtime.upload_operation_locks().acquire(&handle).await;
    let expected_revision = parse_header(
        &request,
        "x-suprnova-upload-revision",
        UploadRevision::parse,
    )?;
    let idempotency_key = parse_header(
        &request,
        "x-suprnova-upload-idempotency",
        UploadIdempotencyKey::parse,
    )?;
    let chunk_index = request
        .header("x-suprnova-upload-chunk")
        .ok_or_else(|| upload_kind(UploadErrorKind::MissingField))?
        .parse::<u32>()
        .map_err(|_| upload_kind(UploadErrorKind::InvalidField))?;
    let offset = request
        .header("x-suprnova-upload-offset")
        .ok_or_else(|| upload_kind(UploadErrorKind::MissingField))?
        .parse::<u64>()
        .map_err(|_| upload_kind(UploadErrorKind::InvalidField))?;
    let checksum = parse_header(
        &request,
        "x-suprnova-upload-checksum",
        UploadChecksum::parse,
    )?;
    let declared_bytes = request
        .header("content-length")
        .ok_or_else(|| upload_kind(UploadErrorKind::MissingField))?
        .parse::<usize>()
        .map_err(|_| upload_kind(UploadErrorKind::InvalidField))?;
    let now = upload_now(runtime)?;
    let (context, record) = runtime
        .resolve_upload_request_context(&request, &handle)
        .await
        .map_err(|_| upload_kind(UploadErrorKind::ScopeMismatch))?;
    if runtime.upload_provider_adapter().mode(&handle)?
        != super::ports::upload_provider::UploadProviderMode::ReverseProxy
    {
        return Err(upload_kind(UploadErrorKind::UnsupportedOperation));
    }
    let field = record.authority().field().clone();
    let _authorized = runtime
        .upload_authority()
        .status(&context, grant.clone(), field.clone(), handle.clone(), now)
        .await?;
    let limits = runtime.upload_limits();
    if declared_bytes == 0 || declared_bytes > limits.max_chunk_bytes() {
        return Err(upload_kind(UploadErrorKind::InputTooLarge));
    }
    let budgeted_bytes = declared_bytes
        .checked_add(declared_bytes.min(MAX_BODY_SEGMENT_COPY_BYTES))
        .ok_or_else(|| upload_kind(UploadErrorKind::ResourceExhausted))?;
    let _body_budget = runtime.upload_body_budget().acquire(budgeted_bytes).await?;
    let cancellation = request.live_cancellation().unwrap_or_default();
    let request = request
        .buffer_body(limits.max_chunk_bytes())
        .await
        .map_err(|error| {
            if error.status_code() == 413 {
                upload_kind(UploadErrorKind::InputTooLarge)
            } else {
                upload_kind(UploadErrorKind::BodyInterrupted)
            }
        })?;
    let bytes = request
        .cached_body()
        .cloned()
        .ok_or_else(|| upload_kind(UploadErrorKind::BodyInterrupted))?;
    if bytes.is_empty() || bytes.len() > limits.max_chunk_bytes() {
        return Err(upload_kind(UploadErrorKind::InputTooLarge));
    }
    if bytes.len() != declared_bytes {
        return Err(upload_kind(UploadErrorKind::IncompleteTransfer));
    }
    if hex::encode(Sha256::digest(&bytes)) != checksum.as_str() {
        return Err(upload_kind(UploadErrorKind::ChecksumMismatch));
    }
    let mut transition_revision = expected_revision;
    if chunk_index == 0 {
        let queued = transition(
            runtime,
            &context,
            &grant,
            &field,
            &handle,
            transition_revision,
            phase_idempotency(&idempotency_key, "queue")?,
            UploadTransition::Queue,
            now,
        )
        .await?;
        transition_revision = queued.revision();
        let begun = transition(
            runtime,
            &context,
            &grant,
            &field,
            &handle,
            transition_revision,
            phase_idempotency(&idempotency_key, "begin")?,
            UploadTransition::BeginTransfer,
            now,
        )
        .await?;
        transition_revision = begun.revision();
    }
    let size =
        u64::try_from(bytes.len()).map_err(|_| upload_kind(UploadErrorKind::InputTooLarge))?;
    let accepted_chunk = AcceptedChunk::new(chunk_index, size, checksum.clone())?;
    let current = runtime
        .upload_authority()
        .status(&context, grant.clone(), field.clone(), handle.clone(), now)
        .await?;
    if current.revision() != transition_revision {
        let replay = transition(
            runtime,
            &context,
            &grant,
            &field,
            &handle,
            transition_revision,
            idempotency_key,
            UploadTransition::PutChunk(accepted_chunk),
            now,
        )
        .await?;
        let progress = runtime.upload_provider_adapter().progress(&handle)?;
        let end = offset
            .checked_add(size)
            .ok_or_else(|| upload_kind(UploadErrorKind::InputTooLarge))?;
        if progress.next_chunk_index <= chunk_index || progress.committed_bytes < end {
            return Err(upload_kind(UploadErrorKind::ReconciliationRequired));
        }
        return Ok(chunk_response(&replay));
    }
    let progress = runtime.upload_provider_adapter().progress(&handle)?;
    if progress.next_chunk_index != chunk_index
        || progress.committed_bytes != offset
        || progress.committed_bytes >= progress.expected_bytes
    {
        return Err(upload_kind(UploadErrorKind::UploadConflict));
    }
    let mut body = BufferedChunkBody::new(bytes, cancellation);
    let receipt = runtime
        .upload_reverse_proxy()
        .write_chunk(
            WriteChunk::new(&handle, chunk_index, offset, size, &checksum),
            &mut body,
        )
        .await?;
    let accepted = transition(
        runtime,
        &context,
        &grant,
        &field,
        &handle,
        transition_revision,
        idempotency_key,
        UploadTransition::PutChunk(AcceptedChunk::new(chunk_index, receipt.bytes(), checksum)?),
        now,
    )
    .await?;
    Ok(chunk_response(&accepted))
}

fn chunk_response(accepted: &suprnova_live::upload::TransitionOutcome) -> HttpResponse {
    json_response(
        200,
        serde_json::json!({
            "revision": accepted.revision().get().to_string(),
            "state": browser_upload_state(accepted.state()),
        }),
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "one engine transition keeps every authority and conditional fact explicit"
)]
async fn transition(
    runtime: &super::runtime::LiveRuntime,
    context: &suprnova_live::host::TrustedLiveRequestContext,
    grant: &TransferGrant,
    field: &ModelField,
    handle: &UploadHandle,
    revision: UploadRevision,
    idempotency_key: UploadIdempotencyKey,
    operation: UploadTransition,
    now: suprnova_live::identity::UnixMillis,
) -> Result<suprnova_live::upload::TransitionOutcome, UploadError> {
    runtime
        .upload_authority()
        .transition(
            context,
            UploadTransitionAdmission::new(
                grant.clone(),
                field.clone(),
                UploadTransitionRequest::new(handle.clone(), revision, idempotency_key, operation),
            ),
            now,
        )
        .await
}

struct BufferedChunkBody {
    bytes: Bytes,
    cancellation: CancellationFlag,
}

impl BufferedChunkBody {
    fn new(bytes: Bytes, cancellation: CancellationFlag) -> Self {
        Self {
            bytes,
            cancellation,
        }
    }
}

impl ChunkBody for BufferedChunkBody {
    fn next_chunk<'a>(
        &'a mut self,
        maximum_bytes: usize,
    ) -> suprnova_live::upload::UploadFuture<'a, Result<Option<QuarantineBytes>, UploadError>> {
        Box::pin(async move {
            if self.cancellation.is_canceled() {
                return Err(upload_kind(UploadErrorKind::TransferCanceled));
            }
            if self.bytes.is_empty() {
                return Ok(None);
            }
            let take = self.bytes.len().min(maximum_bytes);
            let bytes = self.bytes.split_to(take);
            Ok(Some(QuarantineBytes::copy_from_slice(&bytes)))
        })
    }
}

fn phase_idempotency(
    logical: &UploadIdempotencyKey,
    phase: &str,
) -> Result<UploadIdempotencyKey, UploadError> {
    let mut digest = Sha256::new();
    digest.update(b"suprnova-live/upload-host-phase/v1\0");
    digest.update(phase.as_bytes());
    digest.update([0]);
    digest.update(logical.as_str().as_bytes());
    UploadIdempotencyKey::parse(&format!("host:{phase}:{}", hex::encode(digest.finalize())))
}

fn parse_grant(request: &Request) -> Result<TransferGrant, UploadError> {
    request
        .header("authorization")
        .and_then(|value| value.strip_prefix("SuprnovaUpload "))
        .ok_or_else(|| upload_kind(UploadErrorKind::InvalidGrantEncoding))
        .and_then(TransferGrant::parse)
}

fn parse_header<T>(
    request: &Request,
    name: &str,
    parse: impl FnOnce(&str) -> Result<T, UploadError>,
) -> Result<T, UploadError> {
    request
        .header(name)
        .ok_or_else(|| upload_kind(UploadErrorKind::MissingField))
        .and_then(parse)
}

fn upload_now(
    runtime: &super::runtime::LiveRuntime,
) -> Result<suprnova_live::identity::UnixMillis, UploadError> {
    runtime
        .upload_now()
        .map_err(|_| upload_kind(UploadErrorKind::ProviderUnavailable))
}

fn upload_kind(kind: UploadErrorKind) -> UploadError {
    UploadError::new(kind)
}

fn browser_upload_state(state: UploadState) -> &'static str {
    match state {
        UploadState::Created => "queued",
        _ => state.as_str(),
    }
}

fn json_response(status: u16, body: serde_json::Value) -> HttpResponse {
    HttpResponse::json(body)
        .header("Cache-Control", "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .header("Referrer-Policy", "no-referrer")
        .status(status)
}

fn semantic_error(kind: UploadErrorKind) -> HttpResponse {
    let status = match kind {
        UploadErrorKind::AuthorizationDenied | UploadErrorKind::ScopeMismatch => 403,
        UploadErrorKind::InputTooLarge => 413,
        UploadErrorKind::UploadExpired | UploadErrorKind::GrantExpired => 410,
        UploadErrorKind::ProviderUnavailable
        | UploadErrorKind::AuthorizationUnavailable
        | UploadErrorKind::LedgerUnavailable
        | UploadErrorKind::ResourceExhausted => 503,
        UploadErrorKind::ValidationEvidenceUnavailable | UploadErrorKind::UploadConflict => 409,
        UploadErrorKind::IncompleteTransfer | UploadErrorKind::ChecksumMismatch => 422,
        _ => 400,
    };
    let mut body = serde_json::json!({ "error": kind.as_str() });
    if kind == UploadErrorKind::ValidationEvidenceUnavailable {
        body["recovery"] = serde_json::Value::String("refresh_or_start_new_upload".to_owned());
    }
    json_response(status, body)
}

fn closed_response(status: u16) -> HttpResponse {
    HttpResponse::new()
        .header("Cache-Control", "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .header("Referrer-Policy", "no-referrer")
        .header(
            "Content-Security-Policy",
            "default-src 'none'; frame-ancestors 'none'",
        )
        .header("Content-Length", "0")
        .status(status)
}

#[cfg(test)]
mod tests {
    use super::{UploadBodyBudget, UploadErrorKind};

    #[tokio::test]
    async fn body_budget_rejects_an_impossible_permit_request_instead_of_waiting_forever() {
        let budget = UploadBodyBudget::new(4).expect("finite upload body budget");
        let error = budget
            .acquire(5)
            .await
            .expect_err("a request above the fixed budget must fail immediately");

        assert_eq!(error.kind(), UploadErrorKind::ResourceExhausted);
    }
}
