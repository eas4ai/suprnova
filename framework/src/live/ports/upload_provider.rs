//! Bounded asynchronous quarantine I/O and explicit upload-provider adapters.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;

use suprnova_live::identity::ScopeFingerprint;
use suprnova_live::identity::UnixMillis;
use suprnova_live::limits::UploadLimits;
use suprnova_live::resource::PermitPool;
use suprnova_live::upload::{
    ChunkBody, ChunkDisposition, ChunkReceipt, ClientUploadMetadata, DirectPartReference,
    DirectTransferInstruction, DirectUploadProvider, IntegrityEvidence, PrepareTransfer,
    QuarantineBytes, QuarantineObject, QuarantineOperation, QuarantineStore, ReadUpload,
    RemoveDisposition, ReportDirectPart, ReverseProxyUploadProvider, TransferInstruction,
    TransferPlan, UploadError, UploadErrorKind, UploadFuture, UploadHandle, UploadProvider,
    VerifyTransfer, WriteChunk,
};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

const MAX_STORE_OPERATION_BYTES: usize = 64 * 1024 * 1024;
const CREATE_METADATA_ENTRY_OVERHEAD_BYTES: usize = 128;

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct UploadCreateMetadata {
    client: ClientUploadMetadata,
    expected_bytes: u64,
    last_modified: u64,
    expires_at: UnixMillis,
    scope: ScopeFingerprint,
}

impl UploadCreateMetadata {
    pub(crate) fn new(
        client: ClientUploadMetadata,
        expected_bytes: u64,
        last_modified: u64,
        expires_at: UnixMillis,
        scope: ScopeFingerprint,
    ) -> Self {
        Self {
            client,
            expected_bytes,
            last_modified,
            expires_at,
            scope,
        }
    }

    pub(crate) const fn client(&self) -> &ClientUploadMetadata {
        &self.client
    }

    pub(crate) const fn expected_bytes(&self) -> u64 {
        self.expected_bytes
    }

    pub(crate) const fn last_modified(&self) -> u64 {
        self.last_modified
    }

    fn retained_bytes(&self) -> Result<usize, UploadError> {
        self.client
            .display_name()
            .len()
            .checked_add(self.client.claimed_media_type().map_or(0, str::len))
            .and_then(|bytes| bytes.checked_add(CREATE_METADATA_ENTRY_OVERHEAD_BYTES))
            .ok_or_else(|| UploadError::new(UploadErrorKind::ResourceExhausted))
    }
}

impl fmt::Debug for UploadCreateMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<UploadCreateMetadata:redacted>")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UploadCreateMetadataDisposition {
    Inserted,
    ExistingOutcome,
}

struct UploadCreateMetadataState {
    entries: HashMap<UploadHandle, UploadCreateMetadata>,
    retained_bytes: usize,
    scopes: HashMap<ScopeFingerprint, MetadataUsage>,
}

#[derive(Clone, Copy, Default)]
struct MetadataUsage {
    entries: usize,
    retained_bytes: usize,
}

pub(crate) struct UploadCreateMetadataMemo {
    maximum_entries: usize,
    maximum_bytes: usize,
    maximum_entries_per_scope: usize,
    maximum_bytes_per_scope: usize,
    maximum_ttl_ms: u64,
    state: Mutex<UploadCreateMetadataState>,
}

impl UploadCreateMetadataMemo {
    pub(crate) fn new(
        maximum_entries: usize,
        maximum_bytes: usize,
        maximum_entries_per_scope: usize,
        maximum_bytes_per_scope: usize,
        maximum_ttl_ms: u64,
    ) -> Result<Self, UploadError> {
        if maximum_entries == 0
            || maximum_bytes == 0
            || maximum_entries_per_scope == 0
            || maximum_bytes_per_scope == 0
            || maximum_entries < maximum_entries_per_scope
            || maximum_bytes < maximum_bytes_per_scope
            || maximum_ttl_ms == 0
        {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        }
        Ok(Self {
            maximum_entries,
            maximum_bytes,
            maximum_entries_per_scope,
            maximum_bytes_per_scope,
            maximum_ttl_ms,
            state: Mutex::new(UploadCreateMetadataState {
                entries: HashMap::new(),
                retained_bytes: 0,
                scopes: HashMap::new(),
            }),
        })
    }

    pub(crate) fn bind(
        &self,
        handle: UploadHandle,
        metadata: UploadCreateMetadata,
        now: UnixMillis,
    ) -> Result<UploadCreateMetadataDisposition, UploadError> {
        if metadata.expires_at <= now
            || metadata.expires_at.get().saturating_sub(now.get()) > self.maximum_ttl_ms
        {
            return Err(UploadError::new(UploadErrorKind::UploadExpired));
        }
        let retained = metadata.retained_bytes()?;
        let mut state = lock_metadata(&self.state);
        prune_expired(&mut state, now);
        if let Some(existing) = state.entries.get(&handle) {
            return if existing == &metadata {
                Ok(UploadCreateMetadataDisposition::ExistingOutcome)
            } else {
                Err(UploadError::new(UploadErrorKind::UploadConflict))
            };
        }
        let scope_usage = state
            .scopes
            .get(&metadata.scope)
            .copied()
            .unwrap_or_default();
        if state.entries.len() >= self.maximum_entries
            || state
                .retained_bytes
                .checked_add(retained)
                .is_none_or(|bytes| bytes > self.maximum_bytes)
            || scope_usage.entries >= self.maximum_entries_per_scope
            || scope_usage
                .retained_bytes
                .checked_add(retained)
                .is_none_or(|bytes| bytes > self.maximum_bytes_per_scope)
        {
            return Err(UploadError::new(UploadErrorKind::ResourceExhausted));
        }
        state.retained_bytes += retained;
        let usage = state.scopes.entry(metadata.scope.clone()).or_default();
        usage.entries += 1;
        usage.retained_bytes += retained;
        state.entries.insert(handle, metadata);
        Ok(UploadCreateMetadataDisposition::Inserted)
    }

    pub(crate) fn load(
        &self,
        handle: &UploadHandle,
        now: UnixMillis,
    ) -> Result<UploadCreateMetadata, UploadError> {
        let mut state = lock_metadata(&self.state);
        prune_expired(&mut state, now);
        state
            .entries
            .get(handle)
            .cloned()
            .ok_or_else(|| UploadError::new(UploadErrorKind::ValidationEvidenceUnavailable))
    }

    pub(crate) fn remove(&self, handle: &UploadHandle) {
        let mut state = lock_metadata(&self.state);
        remove_metadata_entry(&mut state, handle);
    }
}

impl fmt::Debug for UploadCreateMetadataMemo {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<UploadCreateMetadataMemo:redacted>")
    }
}

fn lock_metadata(
    state: &Mutex<UploadCreateMetadataState>,
) -> MutexGuard<'_, UploadCreateMetadataState> {
    state.lock().unwrap_or_else(|error| error.into_inner())
}

fn prune_expired(state: &mut UploadCreateMetadataState, now: UnixMillis) {
    let expired = state
        .entries
        .iter()
        .filter_map(|(handle, metadata)| (metadata.expires_at <= now).then_some(handle.clone()))
        .collect::<Vec<_>>();
    for handle in expired {
        remove_metadata_entry(state, &handle);
    }
}

fn remove_metadata_entry(state: &mut UploadCreateMetadataState, handle: &UploadHandle) {
    let Some(metadata) = state.entries.remove(handle) else {
        return;
    };
    let retained = metadata.retained_bytes().unwrap_or(0);
    state.retained_bytes = state.retained_bytes.saturating_sub(retained);
    let remove_scope = state.scopes.get_mut(&metadata.scope).is_some_and(|usage| {
        usage.entries = usage.entries.saturating_sub(1);
        usage.retained_bytes = usage.retained_bytes.saturating_sub(retained);
        usage.entries == 0
    });
    if remove_scope {
        state.scopes.remove(&metadata.scope);
    }
}

pub(crate) struct SuprnovaQuarantineStore {
    root: PathBuf,
    _temporary_root: Option<tempfile::TempDir>,
    descriptors: PermitPool,
    maximum_operation_bytes: usize,
}

impl SuprnovaQuarantineStore {
    pub(crate) fn open(
        root: impl AsRef<Path>,
        maximum_descriptors: usize,
        maximum_operation_bytes: usize,
    ) -> Result<Self, UploadError> {
        if maximum_operation_bytes == 0 || maximum_operation_bytes > MAX_STORE_OPERATION_BYTES {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        }
        create_private_directory(root.as_ref()).map_err(provider_error)?;
        let root = std::fs::canonicalize(root.as_ref()).map_err(provider_error)?;
        if !root.is_dir() {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        }
        restrict_directory_permissions(&root).map_err(provider_error)?;
        let descriptors = PermitPool::new(maximum_descriptors)
            .map_err(|_| UploadError::new(UploadErrorKind::InvalidField))?;
        Ok(Self {
            root,
            _temporary_root: None,
            descriptors,
            maximum_operation_bytes,
        })
    }

    pub(crate) fn temporary(
        maximum_descriptors: usize,
        maximum_operation_bytes: usize,
    ) -> Result<Self, UploadError> {
        let temporary_root = tempfile::Builder::new()
            .prefix("suprnova-live-quarantine-")
            .tempdir()
            .map_err(provider_error)?;
        let mut store = Self::open(
            temporary_root.path(),
            maximum_descriptors,
            maximum_operation_bytes,
        )?;
        store._temporary_root = Some(temporary_root);
        Ok(store)
    }

    fn path_for(&self, object: &QuarantineObject) -> PathBuf {
        self.root.join(object.storage_key())
    }

    fn descriptor(&self) -> Result<suprnova_live::resource::Permit, UploadError> {
        self.descriptors
            .try_acquire()
            .map_err(|_| UploadError::new(UploadErrorKind::ResourceExhausted))
    }
}

impl QuarantineStore for SuprnovaQuarantineStore {
    fn create_exclusive(&self, object: &QuarantineObject) -> QuarantineOperation<()> {
        let descriptor = match self.descriptor() {
            Ok(descriptor) => descriptor,
            Err(error) => return QuarantineOperation::ready(Err(error)),
        };
        let path = self.path_for(object);
        spawn_operation(async move {
            let _descriptor = descriptor;
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
            match options.open(path).await {
                Ok(_) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    Err(UploadError::new(UploadErrorKind::StorageConflict))
                }
                Err(error) => Err(provider_error(error)),
            }
        })
    }

    fn write_at(
        &self,
        object: &QuarantineObject,
        offset: u64,
        bytes: &[u8],
    ) -> QuarantineOperation<()> {
        if bytes.is_empty() || bytes.len() > self.maximum_operation_bytes {
            return QuarantineOperation::ready(Err(UploadError::new(
                UploadErrorKind::InputTooLarge,
            )));
        }
        let descriptor = match self.descriptor() {
            Ok(descriptor) => descriptor,
            Err(error) => return QuarantineOperation::ready(Err(error)),
        };
        let path = self.path_for(object);
        let bytes = QuarantineBytes::copy_from_slice(bytes);
        spawn_operation(async move {
            let _descriptor = descriptor;
            let mut file = OpenOptions::new()
                .write(true)
                .open(path)
                .await
                .map_err(provider_error)?;
            file.seek(std::io::SeekFrom::Start(offset))
                .await
                .map_err(provider_error)?;
            file.write_all(bytes.as_ref()).await.map_err(provider_error)
        })
    }

    fn sync(&self, object: &QuarantineObject) -> QuarantineOperation<()> {
        let descriptor = match self.descriptor() {
            Ok(descriptor) => descriptor,
            Err(error) => return QuarantineOperation::ready(Err(error)),
        };
        let path = self.path_for(object);
        spawn_operation(async move {
            let _descriptor = descriptor;
            let file = OpenOptions::new()
                .write(true)
                .open(path)
                .await
                .map_err(provider_error)?;
            file.sync_data().await.map_err(provider_error)
        })
    }

    fn read_at(
        &self,
        object: &QuarantineObject,
        offset: u64,
        maximum_bytes: usize,
    ) -> QuarantineOperation<QuarantineBytes> {
        if maximum_bytes == 0 || maximum_bytes > self.maximum_operation_bytes {
            return QuarantineOperation::ready(Err(UploadError::new(
                UploadErrorKind::InputTooLarge,
            )));
        }
        let descriptor = match self.descriptor() {
            Ok(descriptor) => descriptor,
            Err(error) => return QuarantineOperation::ready(Err(error)),
        };
        let path = self.path_for(object);
        spawn_operation(async move {
            let _descriptor = descriptor;
            let mut file = File::open(path).await.map_err(provider_error)?;
            file.seek(std::io::SeekFrom::Start(offset))
                .await
                .map_err(provider_error)?;
            let mut bytes = vec![0_u8; maximum_bytes];
            let read = file.read(&mut bytes).await.map_err(provider_error)?;
            bytes.truncate(read);
            Ok(QuarantineBytes::from(bytes))
        })
    }

    fn remove(&self, object: &QuarantineObject) -> QuarantineOperation<RemoveDisposition> {
        let descriptor = match self.descriptor() {
            Ok(descriptor) => descriptor,
            Err(error) => return QuarantineOperation::ready(Err(error)),
        };
        let path = self.path_for(object);
        spawn_operation(async move {
            let _descriptor = descriptor;
            match tokio::fs::remove_file(path).await {
                Ok(()) => Ok(RemoveDisposition::Removed),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    Ok(RemoveDisposition::AlreadyAbsent)
                }
                Err(error) => Err(provider_error(error)),
            }
        })
    }
}

fn create_private_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;

        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true).mode(0o700).create(path)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(path)
    }
}

fn restrict_directory_permissions(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

pub(crate) struct SuprnovaReverseProxyUploadProvider {
    inner: suprnova_live::upload::QuarantinedFileProvider<SuprnovaQuarantineStore>,
    create_metadata: UploadCreateMetadataMemo,
}

impl SuprnovaReverseProxyUploadProvider {
    pub(crate) fn new(
        store: Arc<SuprnovaQuarantineStore>,
        limits: UploadLimits,
    ) -> Result<Self, UploadError> {
        let maximum_metadata_bytes = limits
            .max_pending_per_scope()
            .checked_mul(1_408)
            .ok_or_else(|| UploadError::new(UploadErrorKind::ResourceExhausted))?;
        let maximum_global_entries = limits
            .max_pending_per_scope()
            .checked_mul(limits.max_concurrent_transfers())
            .ok_or_else(|| UploadError::new(UploadErrorKind::ResourceExhausted))?;
        let maximum_global_bytes = maximum_metadata_bytes
            .checked_mul(limits.max_concurrent_transfers())
            .ok_or_else(|| UploadError::new(UploadErrorKind::ResourceExhausted))?;
        Ok(Self {
            inner: suprnova_live::upload::QuarantinedFileProvider::new(store, limits)?,
            create_metadata: UploadCreateMetadataMemo::new(
                maximum_global_entries,
                maximum_global_bytes,
                limits.max_pending_per_scope(),
                maximum_metadata_bytes,
                limits.max_age_ms(),
            )?,
        })
    }

    pub(crate) fn bind_create_metadata(
        &self,
        handle: UploadHandle,
        metadata: UploadCreateMetadata,
        now: UnixMillis,
    ) -> Result<UploadCreateMetadataDisposition, UploadError> {
        self.create_metadata.bind(handle, metadata, now)
    }

    pub(crate) fn create_metadata(
        &self,
        handle: &UploadHandle,
        now: UnixMillis,
    ) -> Result<UploadCreateMetadata, UploadError> {
        self.create_metadata.load(handle, now)
    }

    pub(crate) fn remove_create_metadata(&self, handle: &UploadHandle) {
        self.create_metadata.remove(handle);
    }

    pub(crate) fn progress(
        &self,
        handle: &UploadHandle,
    ) -> Result<UploadTransferProgress, UploadError> {
        let checkpoint = self.inner.checkpoint(handle)?;
        let next_chunk_index = u32::try_from(checkpoint.chunks().count())
            .map_err(|_| UploadError::new(UploadErrorKind::ResourceExhausted))?;
        Ok(UploadTransferProgress {
            expected_bytes: checkpoint.expected_bytes(),
            committed_bytes: checkpoint.committed_bytes(),
            next_chunk_index,
        })
    }
}

#[derive(Clone, Copy)]
pub(crate) struct UploadTransferProgress {
    pub(crate) expected_bytes: u64,
    pub(crate) committed_bytes: u64,
    pub(crate) next_chunk_index: u32,
}

impl UploadProvider for SuprnovaReverseProxyUploadProvider {
    fn prepare<'a>(
        &'a self,
        request: PrepareTransfer<'a>,
    ) -> UploadFuture<'a, Result<TransferPlan, UploadError>> {
        self.inner.prepare(request)
    }

    fn verify<'a>(
        &'a self,
        request: VerifyTransfer<'a>,
    ) -> UploadFuture<'a, Result<IntegrityEvidence, UploadError>> {
        self.inner.verify(request)
    }

    fn read<'a>(
        &'a self,
        request: ReadUpload<'a>,
    ) -> UploadFuture<'a, Result<QuarantineBytes, UploadError>> {
        self.inner.read(request)
    }

    fn cancel<'a>(&'a self, handle: &'a UploadHandle) -> UploadFuture<'a, Result<(), UploadError>> {
        Box::pin(async move {
            self.inner.cancel(handle).await?;
            self.create_metadata.remove(handle);
            Ok(())
        })
    }

    fn cleanup<'a>(
        &'a self,
        handle: &'a UploadHandle,
    ) -> UploadFuture<'a, Result<(), UploadError>> {
        Box::pin(async move {
            self.inner.cleanup(handle).await?;
            self.create_metadata.remove(handle);
            Ok(())
        })
    }
}

impl ReverseProxyUploadProvider for SuprnovaReverseProxyUploadProvider {
    fn write_chunk<'a>(
        &'a self,
        request: WriteChunk<'a>,
        body: &'a mut dyn ChunkBody,
    ) -> UploadFuture<'a, Result<ChunkReceipt, UploadError>> {
        self.inner.write_chunk(request, body)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UploadProviderMode {
    ReverseProxy,
    Direct,
}

struct DirectProgress {
    expected_bytes: u64,
    committed_bytes: u64,
    next_part: u32,
    instructions: HashMap<u32, DirectTransferInstruction>,
}

struct ProviderBinding {
    scope: ScopeFingerprint,
    mode: UploadProviderMode,
    direct: Option<DirectProgress>,
}

#[derive(Default)]
struct ProviderRouterState {
    bindings: HashMap<UploadHandle, ProviderBinding>,
    scope_entries: HashMap<ScopeFingerprint, usize>,
}

/// Routes provider-neutral lifecycle operations to the mode bound at create.
pub(crate) struct SuprnovaUploadProviderRouter {
    reverse: Arc<SuprnovaReverseProxyUploadProvider>,
    direct: Arc<dyn DirectUploadProvider>,
    maximum_entries: usize,
    maximum_entries_per_scope: usize,
    state: Mutex<ProviderRouterState>,
}

impl SuprnovaUploadProviderRouter {
    pub(crate) fn new(
        reverse: Arc<SuprnovaReverseProxyUploadProvider>,
        direct: Arc<dyn DirectUploadProvider>,
        limits: UploadLimits,
    ) -> Result<Self, UploadError> {
        let maximum_entries = limits
            .max_pending_per_scope()
            .checked_mul(limits.max_concurrent_transfers())
            .ok_or_else(|| UploadError::new(UploadErrorKind::ResourceExhausted))?;
        if maximum_entries == 0 || limits.max_pending_per_scope() == 0 {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        }
        Ok(Self {
            reverse,
            direct,
            maximum_entries,
            maximum_entries_per_scope: limits.max_pending_per_scope(),
            state: Mutex::new(ProviderRouterState::default()),
        })
    }

    pub(crate) async fn prepare_reverse(
        &self,
        request: PrepareTransfer<'_>,
        scope: &ScopeFingerprint,
    ) -> Result<TransferPlan, UploadError> {
        self.bind(
            request.handle(),
            scope,
            UploadProviderMode::ReverseProxy,
            request.expected_bytes(),
        )?;
        let plan = self.reverse.prepare(request).await?;
        if plan.handle() != request.handle()
            || plan
                .instructions()
                .any(|instruction| !matches!(instruction, TransferInstruction::ReverseProxy { .. }))
        {
            return Err(UploadError::new(UploadErrorKind::ProviderUnavailable));
        }
        Ok(plan)
    }

    pub(crate) async fn prepare_direct(
        &self,
        request: PrepareTransfer<'_>,
        scope: &ScopeFingerprint,
    ) -> Result<TransferPlan, UploadError> {
        self.bind(
            request.handle(),
            scope,
            UploadProviderMode::Direct,
            request.expected_bytes(),
        )?;
        let plan = self.direct.prepare(request).await?;
        if plan.handle() != request.handle() {
            return Err(UploadError::new(UploadErrorKind::ProviderUnavailable));
        }
        let instructions = plan
            .instructions()
            .map(|instruction| {
                instruction
                    .as_direct()
                    .cloned()
                    .ok_or_else(|| UploadError::new(UploadErrorKind::ProviderUnavailable))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if instructions.is_empty() {
            return Err(UploadError::new(UploadErrorKind::ProviderUnavailable));
        }
        let mut state = lock(&self.state);
        let binding = state
            .bindings
            .get_mut(request.handle())
            .filter(|binding| binding.mode == UploadProviderMode::Direct)
            .ok_or_else(|| UploadError::new(UploadErrorKind::ProviderUnavailable))?;
        let instruction_map = instructions
            .into_iter()
            .map(|instruction| (instruction.part().index(), instruction))
            .collect::<HashMap<_, _>>();
        if instruction_map.len() != plan.instructions().len() {
            return Err(UploadError::new(UploadErrorKind::ProviderUnavailable));
        }
        if binding
            .direct
            .as_ref()
            .is_some_and(|progress| !progress.instructions.is_empty())
        {
            return Ok(plan);
        }
        binding.direct = Some(DirectProgress {
            expected_bytes: request.expected_bytes(),
            committed_bytes: 0,
            next_part: 0,
            instructions: instruction_map,
        });
        Ok(plan)
    }

    pub(crate) fn mode(&self, handle: &UploadHandle) -> Result<UploadProviderMode, UploadError> {
        lock(&self.state)
            .bindings
            .get(handle)
            .map(|binding| binding.mode)
            .ok_or_else(|| UploadError::new(UploadErrorKind::UploadConflict))
    }

    pub(crate) fn progress(
        &self,
        handle: &UploadHandle,
    ) -> Result<UploadTransferProgress, UploadError> {
        let state = lock(&self.state);
        let binding = state
            .bindings
            .get(handle)
            .ok_or_else(|| UploadError::new(UploadErrorKind::UploadConflict))?;
        match &binding.direct {
            Some(progress) if binding.mode == UploadProviderMode::Direct => {
                Ok(UploadTransferProgress {
                    expected_bytes: progress.expected_bytes,
                    committed_bytes: progress.committed_bytes,
                    next_chunk_index: progress.next_part,
                })
            }
            None if binding.mode == UploadProviderMode::ReverseProxy => {
                drop(state);
                self.reverse.progress(handle)
            }
            _ => Err(UploadError::new(UploadErrorKind::ProviderUnavailable)),
        }
    }

    pub(crate) async fn report_direct_part(
        &self,
        handle: &UploadHandle,
        index: u32,
        reference: DirectPartReference,
        observed_at: UnixMillis,
    ) -> Result<ChunkReceipt, UploadError> {
        let instruction = {
            let state = lock(&self.state);
            let progress = state
                .bindings
                .get(handle)
                .filter(|binding| binding.mode == UploadProviderMode::Direct)
                .and_then(|binding| binding.direct.as_ref())
                .ok_or_else(|| UploadError::new(UploadErrorKind::UploadConflict))?;
            progress
                .instructions
                .get(&index)
                .filter(|instruction: &&DirectTransferInstruction| {
                    instruction.reference() == &reference && instruction.is_current(observed_at)
                })
                .cloned()
                .ok_or_else(|| UploadError::new(UploadErrorKind::ScopeMismatch))?
        };
        let receipt = self
            .direct
            .report_part(ReportDirectPart::new(
                handle,
                instruction.part().clone(),
                reference,
                observed_at,
            ))
            .await?;
        if receipt.index() != instruction.part().index()
            || receipt.offset() != instruction.part().offset()
            || receipt.bytes() != instruction.part().bytes()
        {
            return Err(UploadError::new(UploadErrorKind::ProviderUnavailable));
        }

        let mut state = lock(&self.state);
        let progress = state
            .bindings
            .get_mut(handle)
            .filter(|binding| binding.mode == UploadProviderMode::Direct)
            .and_then(|binding| binding.direct.as_mut())
            .ok_or_else(|| UploadError::new(UploadErrorKind::UploadConflict))?;
        let end = receipt
            .offset()
            .checked_add(receipt.bytes())
            .filter(|end| *end <= progress.expected_bytes)
            .ok_or_else(|| UploadError::new(UploadErrorKind::InputTooLarge))?;
        if receipt.index() < progress.next_part {
            if receipt.disposition() != ChunkDisposition::ExistingOutcome
                || end > progress.committed_bytes
            {
                return Err(UploadError::new(UploadErrorKind::UploadConflict));
            }
            return Ok(receipt);
        }
        if receipt.index() != progress.next_part || receipt.offset() != progress.committed_bytes {
            return Err(UploadError::new(UploadErrorKind::UploadConflict));
        }
        if let Some(next) = receipt.next_instruction() {
            let next = next
                .as_direct()
                .cloned()
                .ok_or_else(|| UploadError::new(UploadErrorKind::ProviderUnavailable))?;
            if next.part().index() != receipt.index().saturating_add(1)
                || next.part().offset() != end
            {
                return Err(UploadError::new(UploadErrorKind::ProviderUnavailable));
            }
            progress.instructions.insert(next.part().index(), next);
        }
        progress.committed_bytes = end;
        progress.next_part = progress
            .next_part
            .checked_add(1)
            .ok_or_else(|| UploadError::new(UploadErrorKind::ResourceExhausted))?;
        Ok(receipt)
    }

    pub(crate) fn direct_part(
        &self,
        handle: &UploadHandle,
        index: u32,
        reference: &DirectPartReference,
    ) -> Result<suprnova_live::upload::UploadPart, UploadError> {
        let state = lock(&self.state);
        let progress = state
            .bindings
            .get(handle)
            .filter(|binding| binding.mode == UploadProviderMode::Direct)
            .and_then(|binding| binding.direct.as_ref())
            .ok_or_else(|| UploadError::new(UploadErrorKind::UploadConflict))?;
        progress
            .instructions
            .get(&index)
            .filter(|instruction: &&DirectTransferInstruction| instruction.reference() == reference)
            .map(|instruction: &DirectTransferInstruction| instruction.part().clone())
            .ok_or_else(|| UploadError::new(UploadErrorKind::ScopeMismatch))
    }

    fn bind(
        &self,
        handle: &UploadHandle,
        scope: &ScopeFingerprint,
        mode: UploadProviderMode,
        expected_bytes: u64,
    ) -> Result<(), UploadError> {
        let mut state = lock(&self.state);
        if let Some(existing) = state.bindings.get(handle) {
            let exact = existing.scope == *scope
                && existing.mode == mode
                && existing
                    .direct
                    .as_ref()
                    .is_none_or(|progress| progress.expected_bytes == expected_bytes);
            return if exact {
                Ok(())
            } else {
                Err(UploadError::new(UploadErrorKind::UploadConflict))
            };
        }
        let scoped_entries = state.scope_entries.get(scope).copied().unwrap_or_default();
        if state.bindings.len() >= self.maximum_entries
            || scoped_entries >= self.maximum_entries_per_scope
        {
            return Err(UploadError::new(UploadErrorKind::ResourceExhausted));
        }
        state.bindings.insert(
            handle.clone(),
            ProviderBinding {
                scope: scope.clone(),
                mode,
                direct: (mode == UploadProviderMode::Direct).then_some(DirectProgress {
                    expected_bytes,
                    committed_bytes: 0,
                    next_part: 0,
                    instructions: HashMap::new(),
                }),
            },
        );
        *state.scope_entries.entry(scope.clone()).or_default() += 1;
        Ok(())
    }

    fn remove_binding(&self, handle: &UploadHandle) {
        let mut state = lock(&self.state);
        let Some(binding) = state.bindings.remove(handle) else {
            return;
        };
        let remove_scope = state
            .scope_entries
            .get_mut(&binding.scope)
            .is_some_and(|entries| {
                *entries = entries.saturating_sub(1);
                *entries == 0
            });
        if remove_scope {
            state.scope_entries.remove(&binding.scope);
        }
    }

    fn provider_for(&self, handle: &UploadHandle) -> UploadProviderMode {
        lock(&self.state)
            .bindings
            .get(handle)
            .map_or(UploadProviderMode::ReverseProxy, |binding| binding.mode)
    }
}

impl UploadProvider for SuprnovaUploadProviderRouter {
    fn prepare<'a>(
        &'a self,
        request: PrepareTransfer<'a>,
    ) -> UploadFuture<'a, Result<TransferPlan, UploadError>> {
        Box::pin(async move {
            match self.mode(request.handle())? {
                UploadProviderMode::ReverseProxy => self.reverse.prepare(request).await,
                UploadProviderMode::Direct => self.direct.prepare(request).await,
            }
        })
    }

    fn verify<'a>(
        &'a self,
        request: VerifyTransfer<'a>,
    ) -> UploadFuture<'a, Result<IntegrityEvidence, UploadError>> {
        Box::pin(async move {
            match self.provider_for(request.handle()) {
                UploadProviderMode::ReverseProxy => self.reverse.verify(request).await,
                UploadProviderMode::Direct => self.direct.verify(request).await,
            }
        })
    }

    fn read<'a>(
        &'a self,
        request: ReadUpload<'a>,
    ) -> UploadFuture<'a, Result<QuarantineBytes, UploadError>> {
        Box::pin(async move {
            match self.provider_for(request.handle()) {
                UploadProviderMode::ReverseProxy => self.reverse.read(request).await,
                UploadProviderMode::Direct => self.direct.read(request).await,
            }
        })
    }

    fn cancel<'a>(&'a self, handle: &'a UploadHandle) -> UploadFuture<'a, Result<(), UploadError>> {
        Box::pin(async move {
            match self.provider_for(handle) {
                UploadProviderMode::ReverseProxy => self.reverse.cancel(handle).await?,
                UploadProviderMode::Direct => self.direct.cancel(handle).await?,
            }
            self.reverse.remove_create_metadata(handle);
            self.remove_binding(handle);
            Ok(())
        })
    }

    fn expire<'a>(&'a self, handle: &'a UploadHandle) -> UploadFuture<'a, Result<(), UploadError>> {
        Box::pin(async move {
            match self.provider_for(handle) {
                UploadProviderMode::ReverseProxy => self.reverse.expire(handle).await?,
                UploadProviderMode::Direct => self.direct.expire(handle).await?,
            }
            self.reverse.remove_create_metadata(handle);
            self.remove_binding(handle);
            Ok(())
        })
    }

    fn cleanup<'a>(
        &'a self,
        handle: &'a UploadHandle,
    ) -> UploadFuture<'a, Result<(), UploadError>> {
        Box::pin(async move {
            match self.provider_for(handle) {
                UploadProviderMode::ReverseProxy => self.reverse.cleanup(handle).await?,
                UploadProviderMode::Direct => self.direct.cleanup(handle).await?,
            }
            self.reverse.remove_create_metadata(handle);
            self.remove_binding(handle);
            Ok(())
        })
    }
}

impl DirectUploadProvider for SuprnovaUploadProviderRouter {
    fn report_part<'a>(
        &'a self,
        request: ReportDirectPart<'a>,
    ) -> UploadFuture<'a, Result<ChunkReceipt, UploadError>> {
        Box::pin(async move { self.direct.report_part(request).await })
    }
}

pub(crate) struct UnavailableDirectUploadProvider;

impl UploadProvider for UnavailableDirectUploadProvider {
    fn prepare<'a>(
        &'a self,
        _request: PrepareTransfer<'a>,
    ) -> UploadFuture<'a, Result<TransferPlan, UploadError>> {
        unavailable()
    }

    fn verify<'a>(
        &'a self,
        _request: VerifyTransfer<'a>,
    ) -> UploadFuture<'a, Result<IntegrityEvidence, UploadError>> {
        unavailable()
    }

    fn read<'a>(
        &'a self,
        _request: ReadUpload<'a>,
    ) -> UploadFuture<'a, Result<QuarantineBytes, UploadError>> {
        unavailable()
    }

    fn cancel<'a>(
        &'a self,
        _handle: &'a UploadHandle,
    ) -> UploadFuture<'a, Result<(), UploadError>> {
        unavailable()
    }

    fn cleanup<'a>(
        &'a self,
        _handle: &'a UploadHandle,
    ) -> UploadFuture<'a, Result<(), UploadError>> {
        unavailable()
    }
}

impl DirectUploadProvider for UnavailableDirectUploadProvider {
    fn report_part<'a>(
        &'a self,
        _request: ReportDirectPart<'a>,
    ) -> UploadFuture<'a, Result<ChunkReceipt, UploadError>> {
        unavailable()
    }
}

fn spawn_operation<T: Clone + Send + 'static>(
    future: impl Future<Output = Result<T, UploadError>> + Send + 'static,
) -> QuarantineOperation<T> {
    let (operation, completion) = QuarantineOperation::pending();
    tokio::spawn(async move {
        completion.complete(future.await);
    });
    operation
}

fn unavailable<'a, T>() -> UploadFuture<'a, Result<T, UploadError>> {
    Box::pin(async { Err(UploadError::new(UploadErrorKind::ProviderUnavailable)) })
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn provider_error(_error: std::io::Error) -> UploadError {
    UploadError::new(UploadErrorKind::ProviderUnavailable)
}
