//! Bounded streaming provider over opaque asynchronous quarantine I/O.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex, MutexGuard};

use sha2::{Digest, Sha256};

use crate::identity::UnixMillis;
use crate::limits::UploadLimits;
use crate::resource::{CancellationFlag, PermitPool, ResourceBounds, ResourceOwner, Retirement};

use super::{
    DirectTransferInstruction, QuarantineBytes, QuarantineObject, QuarantineStore,
    ReportDirectPart, TransferInstruction, UploadChecksum, UploadError, UploadErrorKind,
    UploadFuture, UploadHandle, UploadPart,
};

const MAX_CLIENT_NAME_BYTES: usize = 1_024;
const STREAM_BUFFER_BYTES: usize = 64 * 1024;
const OBJECT_COLLISION_ATTEMPTS: usize = 4;

/// Asynchronous bounded source of request-body byte segments.
pub trait ChunkBody: Send {
    /// Returns at most the requested bytes, `None` at clean end-of-stream.
    fn next_chunk<'a>(
        &'a mut self,
        maximum_bytes: usize,
    ) -> UploadFuture<'a, Result<Option<QuarantineBytes>, UploadError>>;
}

/// Trusted preparation request; the client name is untrusted display metadata only.
#[derive(Clone, Copy)]
pub struct PrepareTransfer<'a> {
    handle: &'a UploadHandle,
    expected_bytes: u64,
    client_name: &'a str,
    created_at: UnixMillis,
}

impl<'a> PrepareTransfer<'a> {
    /// Groups one handle, declared size, display name, and authoritative instant.
    #[must_use]
    pub const fn new(
        handle: &'a UploadHandle,
        expected_bytes: u64,
        client_name: &'a str,
        created_at: UnixMillis,
    ) -> Self {
        Self {
            handle,
            expected_bytes,
            client_name,
            created_at,
        }
    }

    /// Returns the temporary upload identity.
    #[must_use]
    pub const fn handle(&self) -> &UploadHandle {
        self.handle
    }

    /// Returns the untrusted declared byte count subject to provider enforcement.
    #[must_use]
    pub const fn expected_bytes(&self) -> u64 {
        self.expected_bytes
    }

    /// Returns the untrusted display name, never a storage identity.
    #[must_use]
    pub const fn client_name(&self) -> &str {
        self.client_name
    }

    /// Returns the authoritative preparation instant.
    #[must_use]
    pub const fn created_at(&self) -> UnixMillis {
        self.created_at
    }
}

impl fmt::Debug for PrepareTransfer<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<PrepareTransfer:redacted>")
    }
}

/// Whether preparation created or exactly replayed an existing transfer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransferDisposition {
    /// A new opaque quarantine object was created.
    Prepared,
    /// The exact handle and size were already prepared.
    ExistingOutcome,
}

/// Safe provider-neutral transfer instructions for one temporary upload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransferPlan {
    handle: UploadHandle,
    maximum_chunk_bytes: usize,
    disposition: TransferDisposition,
    instructions: Vec<TransferInstruction>,
}

impl TransferPlan {
    pub(crate) fn reverse_proxy(
        handle: UploadHandle,
        maximum_chunk_bytes: usize,
        disposition: TransferDisposition,
    ) -> Self {
        Self {
            handle,
            maximum_chunk_bytes,
            disposition,
            instructions: vec![TransferInstruction::reverse_proxy(maximum_chunk_bytes)],
        }
    }

    /// Creates a bounded direct-provider plan from already checked instructions.
    pub fn direct(
        handle: UploadHandle,
        maximum_chunk_bytes: usize,
        disposition: TransferDisposition,
        instructions: Vec<DirectTransferInstruction>,
        maximum_instructions: usize,
    ) -> Result<Self, UploadError> {
        if maximum_chunk_bytes == 0
            || instructions.len() > maximum_instructions
            || instructions.iter().any(|instruction| {
                !instruction.is_constrained() || instruction.maximum_bytes() > maximum_chunk_bytes
            })
        {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        }
        Ok(Self {
            handle,
            maximum_chunk_bytes,
            disposition,
            instructions: instructions
                .into_iter()
                .map(TransferInstruction::Direct)
                .collect(),
        })
    }

    /// Returns the non-authoritative upload identity.
    #[must_use]
    pub const fn handle(&self) -> &UploadHandle {
        &self.handle
    }

    /// Returns the server-enforced chunk byte ceiling.
    #[must_use]
    pub const fn maximum_chunk_bytes(&self) -> usize {
        self.maximum_chunk_bytes
    }

    /// Returns whether this call prepared or replayed the transfer.
    #[must_use]
    pub const fn disposition(&self) -> TransferDisposition {
        self.disposition
    }

    /// Iterates the bounded capabilities emitted by this preparation.
    pub fn instructions(&self) -> impl ExactSizeIterator<Item = &TransferInstruction> {
        self.instructions.iter()
    }
}

/// Trusted request to stream one sequential bounded chunk.
#[derive(Clone, Copy)]
pub struct WriteChunk<'a> {
    handle: &'a UploadHandle,
    index: u32,
    offset: u64,
    size: u64,
    checksum: &'a UploadChecksum,
}

impl<'a> WriteChunk<'a> {
    /// Groups one chunk's upload, ordering, range, and expected integrity.
    #[must_use]
    pub const fn new(
        handle: &'a UploadHandle,
        index: u32,
        offset: u64,
        size: u64,
        checksum: &'a UploadChecksum,
    ) -> Self {
        Self {
            handle,
            index,
            offset,
            size,
            checksum,
        }
    }
}

impl fmt::Debug for WriteChunk<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<WriteChunk:redacted>")
    }
}

/// Whether a chunk was newly stored or exactly replayed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChunkDisposition {
    /// Authoritative bytes and integrity were newly accepted.
    Stored,
    /// The exact indexed range and checksum were already accepted.
    ExistingOutcome,
}

/// Safe result for one accepted chunk.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkReceipt {
    index: u32,
    offset: u64,
    bytes: u64,
    disposition: ChunkDisposition,
    next_instruction: Option<TransferInstruction>,
}

impl ChunkReceipt {
    /// Creates a direct-provider part receipt and optional next bounded instruction.
    #[must_use]
    pub fn for_direct_part(
        part: &UploadPart,
        disposition: ChunkDisposition,
        next_instruction: Option<DirectTransferInstruction>,
    ) -> Self {
        Self {
            index: part.index(),
            offset: part.offset(),
            bytes: part.bytes(),
            disposition,
            next_instruction: next_instruction.map(TransferInstruction::Direct),
        }
    }

    /// Returns the accepted zero-based chunk index.
    #[must_use]
    pub const fn index(&self) -> u32 {
        self.index
    }

    /// Returns the accepted file offset.
    #[must_use]
    pub const fn offset(&self) -> u64 {
        self.offset
    }

    /// Returns the accepted byte count.
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Returns whether bytes were stored or exactly replayed.
    #[must_use]
    pub const fn disposition(&self) -> ChunkDisposition {
        self.disposition
    }

    /// Returns the next bounded provider instruction when sequential issuance continues.
    #[must_use]
    pub const fn next_instruction(&self) -> Option<&TransferInstruction> {
        self.next_instruction.as_ref()
    }
}

/// Trusted request to verify the complete authoritative byte range.
#[derive(Clone, Copy)]
pub struct VerifyTransfer<'a> {
    handle: &'a UploadHandle,
    checksum: &'a UploadChecksum,
}

impl<'a> VerifyTransfer<'a> {
    /// Binds one upload to its expected whole-file checksum.
    #[must_use]
    pub const fn new(handle: &'a UploadHandle, checksum: &'a UploadChecksum) -> Self {
        Self { handle, checksum }
    }

    /// Returns the upload whose authoritative bytes must be verified.
    #[must_use]
    pub const fn handle(&self) -> &UploadHandle {
        self.handle
    }

    /// Returns the expected whole-object checksum.
    #[must_use]
    pub const fn checksum(&self) -> &UploadChecksum {
        self.checksum
    }
}

impl fmt::Debug for VerifyTransfer<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<VerifyTransfer:redacted>")
    }
}

/// Authoritative whole-file integrity evidence produced after synchronization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IntegrityEvidence {
    bytes: u64,
    checksum: UploadChecksum,
}

impl IntegrityEvidence {
    /// Imports integrity evidence produced by a trusted provider adapter.
    #[must_use]
    pub const fn from_provider(bytes: u64, checksum: UploadChecksum) -> Self {
        Self { bytes, checksum }
    }

    /// Returns the verified authoritative byte count.
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Returns the verified SHA-256 checksum.
    #[must_use]
    pub const fn checksum(&self) -> &UploadChecksum {
        &self.checksum
    }
}

/// Provider-neutral authority, verification, and lifecycle boundary.
pub trait UploadProvider: Send + Sync {
    /// Creates or exactly replays one quarantined transfer.
    fn prepare<'a>(
        &'a self,
        request: PrepareTransfer<'a>,
    ) -> UploadFuture<'a, Result<TransferPlan, UploadError>>;

    /// Re-reads, hashes, and synchronizes the complete authoritative file.
    fn verify<'a>(
        &'a self,
        request: VerifyTransfer<'a>,
    ) -> UploadFuture<'a, Result<IntegrityEvidence, UploadError>>;

    /// Cancels and removes one pending upload idempotently.
    fn cancel<'a>(&'a self, handle: &'a UploadHandle) -> UploadFuture<'a, Result<(), UploadError>>;

    /// Expires one pending upload with the same idempotent reclamation contract as cancellation.
    fn expire<'a>(&'a self, handle: &'a UploadHandle) -> UploadFuture<'a, Result<(), UploadError>> {
        self.cancel(handle)
    }

    /// Reclaims one quarantine object idempotently.
    fn cleanup<'a>(&'a self, handle: &'a UploadHandle)
    -> UploadFuture<'a, Result<(), UploadError>>;
}

/// Reverse-proxy capability for streaming authenticated request bodies into quarantine.
pub trait ReverseProxyUploadProvider: UploadProvider {
    /// Streams and verifies one bounded sequential chunk.
    fn write_chunk<'a>(
        &'a self,
        request: WriteChunk<'a>,
        body: &'a mut dyn ChunkBody,
    ) -> UploadFuture<'a, Result<ChunkReceipt, UploadError>>;
}

/// Direct-storage capability for importing trusted provider part outcomes.
pub trait DirectUploadProvider: UploadProvider {
    /// Imports one provider part without trusting a browser completion claim.
    fn report_part<'a>(
        &'a self,
        request: ReportDirectPart<'a>,
    ) -> UploadFuture<'a, Result<ChunkReceipt, UploadError>>;
}

#[derive(Clone, Eq, PartialEq)]
struct ChunkShape {
    index: u32,
    offset: u64,
    size: u64,
    checksum: UploadChecksum,
}

/// Persistable accepted-chunk facts for provider process recovery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointChunk {
    index: u32,
    offset: u64,
    size: u64,
    checksum: UploadChecksum,
}

impl CheckpointChunk {
    /// Constructs one nonzero accepted range for a recovered checkpoint.
    pub fn new(
        index: u32,
        offset: u64,
        size: u64,
        checksum: UploadChecksum,
    ) -> Result<Self, UploadError> {
        if size == 0 || offset.checked_add(size).is_none() {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        }
        Ok(Self {
            index,
            offset,
            size,
            checksum,
        })
    }

    /// Returns the accepted zero-based chunk index.
    #[must_use]
    pub const fn index(&self) -> u32 {
        self.index
    }

    /// Returns the accepted byte offset.
    #[must_use]
    pub const fn offset(&self) -> u64 {
        self.offset
    }

    /// Returns the accepted byte count.
    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }

    /// Returns the accepted chunk checksum.
    #[must_use]
    pub const fn checksum(&self) -> &UploadChecksum {
        &self.checksum
    }
}

impl From<&ChunkShape> for CheckpointChunk {
    fn from(chunk: &ChunkShape) -> Self {
        Self {
            index: chunk.index,
            offset: chunk.offset,
            size: chunk.size,
            checksum: chunk.checksum.clone(),
        }
    }
}

impl From<CheckpointChunk> for ChunkShape {
    fn from(chunk: CheckpointChunk) -> Self {
        Self {
            index: chunk.index,
            offset: chunk.offset,
            size: chunk.size,
            checksum: chunk.checksum,
        }
    }
}

impl ChunkShape {
    fn from_request(request: WriteChunk<'_>) -> Self {
        Self {
            index: request.index,
            offset: request.offset,
            size: request.size,
            checksum: request.checksum.clone(),
        }
    }

    const fn receipt(&self, disposition: ChunkDisposition) -> ChunkReceipt {
        ChunkReceipt {
            index: self.index,
            offset: self.offset,
            bytes: self.size,
            disposition,
            next_instruction: None,
        }
    }
}

struct TransferEntry {
    object: QuarantineObject,
    expected_bytes: u64,
    created_at: UnixMillis,
    created: bool,
    chunks: BTreeMap<u32, ChunkShape>,
    committed_bytes: u64,
    pending: Option<ChunkShape>,
    cancellation: CancellationFlag,
    evidence: Option<IntegrityEvidence>,
}

enum PreparationReservation {
    Existing(TransferPlan),
    Reserved(QuarantineObject),
}

impl TransferEntry {
    fn preparing(object: QuarantineObject, expected_bytes: u64, created_at: UnixMillis) -> Self {
        Self {
            object,
            expected_bytes,
            created_at,
            created: false,
            chunks: BTreeMap::new(),
            committed_bytes: 0,
            pending: None,
            cancellation: CancellationFlag::new(),
            evidence: None,
        }
    }
}

/// Persistable non-path checkpoint for bounded provider process recovery.
#[derive(Clone)]
pub struct TransferCheckpoint {
    handle: UploadHandle,
    object: QuarantineObject,
    expected_bytes: u64,
    created_at: UnixMillis,
    chunks: BTreeMap<u32, ChunkShape>,
    committed_bytes: u64,
    evidence: Option<IntegrityEvidence>,
}

impl TransferCheckpoint {
    /// Reconstructs a persisted checkpoint from bounded non-path authority facts.
    pub fn new(
        handle: UploadHandle,
        object: QuarantineObject,
        expected_bytes: u64,
        created_at: UnixMillis,
        chunks: Vec<CheckpointChunk>,
        committed_bytes: u64,
    ) -> Result<Self, UploadError> {
        let mut by_index = BTreeMap::new();
        for chunk in chunks {
            let index = chunk.index();
            if by_index.insert(index, chunk.into()).is_some() {
                return Err(UploadError::new(UploadErrorKind::UploadConflict));
            }
        }
        let checkpoint = Self {
            handle,
            object,
            expected_bytes,
            created_at,
            chunks: by_index,
            committed_bytes,
            evidence: None,
        };
        validate_checkpoint(&checkpoint)?;
        Ok(checkpoint)
    }

    /// Returns the non-authoritative upload identity.
    #[must_use]
    pub const fn handle(&self) -> &UploadHandle {
        &self.handle
    }

    /// Returns the non-path opaque quarantine identity for host persistence.
    #[must_use]
    pub const fn object(&self) -> &QuarantineObject {
        &self.object
    }

    /// Returns the complete expected file byte count.
    #[must_use]
    pub const fn expected_bytes(&self) -> u64 {
        self.expected_bytes
    }

    /// Returns the authoritative transfer creation instant.
    #[must_use]
    pub const fn created_at(&self) -> UnixMillis {
        self.created_at
    }

    /// Returns the committed sequential byte count.
    #[must_use]
    pub const fn committed_bytes(&self) -> u64 {
        self.committed_bytes
    }

    /// Iterates persistable accepted-chunk facts in index order.
    pub fn chunks(&self) -> impl ExactSizeIterator<Item = CheckpointChunk> + '_ {
        self.chunks.values().map(CheckpointChunk::from)
    }
}

impl fmt::Debug for TransferCheckpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<TransferCheckpoint:redacted>")
    }
}

/// Reference reverse-proxy provider retaining policy while delegating raw I/O.
pub struct QuarantinedFileProvider<S: QuarantineStore> {
    store: Arc<S>,
    limits: UploadLimits,
    transfers: Mutex<HashMap<UploadHandle, TransferEntry>>,
    resources: ResourceOwner<UploadHandle>,
    descriptor_permits: PermitPool,
    chunk_permits: PermitPool,
}

impl<S: QuarantineStore> QuarantinedFileProvider<S> {
    /// Creates one bounded provider without an executor or filesystem dependency.
    pub fn new(store: Arc<S>, limits: UploadLimits) -> Result<Self, UploadError> {
        let resource_bounds = ResourceBounds::new(
            limits.max_concurrent_transfers(),
            limits.max_in_flight_bytes(),
        )
        .map_err(|_| UploadError::new(UploadErrorKind::InvalidField))?;
        let descriptor_permits = PermitPool::new(limits.max_concurrent_transfers())
            .map_err(|_| UploadError::new(UploadErrorKind::InvalidField))?;
        let chunk_permits = PermitPool::new(limits.max_concurrent_transfers())
            .map_err(|_| UploadError::new(UploadErrorKind::InvalidField))?;
        Ok(Self {
            store,
            limits,
            transfers: Mutex::new(HashMap::new()),
            resources: ResourceOwner::new(resource_bounds),
            descriptor_permits,
            chunk_permits,
        })
    }

    /// Returns a bounded non-path checkpoint for one quiescent transfer.
    pub fn checkpoint(&self, handle: &UploadHandle) -> Result<TransferCheckpoint, UploadError> {
        let transfers = lock(&self.transfers);
        let entry = transfers
            .get(handle)
            .ok_or_else(|| UploadError::new(UploadErrorKind::UploadConflict))?;
        if !entry.created || entry.pending.is_some() || entry.cancellation.is_canceled() {
            return Err(UploadError::new(UploadErrorKind::UploadConflict));
        }
        Ok(TransferCheckpoint {
            handle: handle.clone(),
            object: entry.object.clone(),
            expected_bytes: entry.expected_bytes,
            created_at: entry.created_at,
            chunks: entry.chunks.clone(),
            committed_bytes: entry.committed_bytes,
            evidence: entry.evidence.clone(),
        })
    }

    /// Restores one quiescent checkpoint without accepting client path material.
    pub fn recover(&self, checkpoint: TransferCheckpoint) -> Result<(), UploadError> {
        self.require_active()?;
        if checkpoint.expected_bytes > self.limits.max_file_bytes()
            || checkpoint.committed_bytes > checkpoint.expected_bytes
            || checkpoint.chunks.len() > self.limits.max_chunks_per_file()
            || checkpoint
                .chunks
                .values()
                .any(|chunk| chunk.size > self.limits.max_chunk_bytes() as u64)
        {
            return Err(UploadError::new(UploadErrorKind::InputTooLarge));
        }
        validate_checkpoint(&checkpoint)?;
        let mut transfers = lock(&self.transfers);
        if transfers.len() >= self.limits.max_pending_per_scope()
            || transfers.contains_key(&checkpoint.handle)
        {
            return Err(UploadError::new(UploadErrorKind::UploadConflict));
        }
        transfers.insert(
            checkpoint.handle,
            TransferEntry {
                object: checkpoint.object,
                expected_bytes: checkpoint.expected_bytes,
                created_at: checkpoint.created_at,
                created: true,
                chunks: checkpoint.chunks,
                committed_bytes: checkpoint.committed_bytes,
                pending: None,
                cancellation: CancellationFlag::new(),
                evidence: checkpoint.evidence,
            },
        );
        Ok(())
    }

    /// Returns the bounded provider-level descriptor permits.
    #[must_use]
    pub const fn descriptor_permits(&self) -> &PermitPool {
        &self.descriptor_permits
    }

    /// Returns the bounded active chunk permits.
    #[must_use]
    pub const fn chunk_permits(&self) -> &PermitPool {
        &self.chunk_permits
    }

    /// Retires provider admission and cancels in-flight work observation.
    pub fn retire(&self) -> Retirement {
        self.resources.retire()
    }

    fn require_active(&self) -> Result<(), UploadError> {
        if self.resources.cancellation().is_canceled() {
            Err(UploadError::new(UploadErrorKind::ServiceRetired))
        } else {
            Ok(())
        }
    }

    fn reserve_preparation(
        &self,
        request: PrepareTransfer<'_>,
    ) -> Result<PreparationReservation, UploadError> {
        {
            let transfers = lock(&self.transfers);
            if let Some(plan) = self.existing_preparation(request, &transfers)? {
                return Ok(PreparationReservation::Existing(plan));
            }
            if transfers.len() >= self.limits.max_pending_per_scope() {
                return Err(UploadError::new(UploadErrorKind::PendingLimitExceeded));
            }
        }
        let object = QuarantineObject::generate()?;
        let mut transfers = lock(&self.transfers);
        if let Some(plan) = self.existing_preparation(request, &transfers)? {
            return Ok(PreparationReservation::Existing(plan));
        }
        if transfers.len() >= self.limits.max_pending_per_scope() {
            return Err(UploadError::new(UploadErrorKind::PendingLimitExceeded));
        }
        transfers.insert(
            request.handle.clone(),
            TransferEntry::preparing(object.clone(), request.expected_bytes, request.created_at),
        );
        Ok(PreparationReservation::Reserved(object))
    }

    fn existing_preparation(
        &self,
        request: PrepareTransfer<'_>,
        transfers: &HashMap<UploadHandle, TransferEntry>,
    ) -> Result<Option<TransferPlan>, UploadError> {
        let Some(existing) = transfers.get(request.handle) else {
            return Ok(None);
        };
        if existing.created
            && !existing.cancellation.is_canceled()
            && existing.expected_bytes == request.expected_bytes
        {
            Ok(Some(TransferPlan::reverse_proxy(
                request.handle.clone(),
                self.limits.max_chunk_bytes(),
                TransferDisposition::ExistingOutcome,
            )))
        } else {
            Err(UploadError::new(UploadErrorKind::UploadConflict))
        }
    }

    async fn create_reserved_object(
        &self,
        handle: &UploadHandle,
        mut selected: QuarantineObject,
    ) -> Result<QuarantineObject, UploadError> {
        for _ in 0..OBJECT_COLLISION_ATTEMPTS {
            match self.store.create_exclusive(&selected).await {
                Ok(()) => return Ok(selected),
                Err(error) if error.kind() == UploadErrorKind::StorageConflict => {
                    selected = match QuarantineObject::generate() {
                        Ok(object) => object,
                        Err(error) => {
                            lock(&self.transfers).remove(handle);
                            return Err(error);
                        }
                    };
                    if let Some(entry) = lock(&self.transfers).get_mut(handle) {
                        entry.object = selected.clone();
                    } else {
                        return Err(UploadError::new(UploadErrorKind::TransferCanceled));
                    }
                }
                Err(error) => {
                    lock(&self.transfers).remove(handle);
                    return Err(error);
                }
            }
        }
        lock(&self.transfers).remove(handle);
        Err(UploadError::new(UploadErrorKind::StorageConflict))
    }

    fn publish_preparation(&self, handle: &UploadHandle, selected: &QuarantineObject) -> bool {
        let mut transfers = lock(&self.transfers);
        match transfers.get_mut(handle) {
            Some(entry)
                if entry.object == *selected
                    && !entry.cancellation.is_canceled()
                    && !self.resources.cancellation().is_canceled() =>
            {
                entry.created = true;
                true
            }
            _ => false,
        }
    }

    async fn discard_unpublished_preparation(
        &self,
        request: PrepareTransfer<'_>,
        selected: QuarantineObject,
    ) -> Result<(), UploadError> {
        if let Err(error) = self.store.remove(&selected).await {
            let mut transfers = lock(&self.transfers);
            transfers.entry(request.handle.clone()).or_insert_with(|| {
                let mut retained = TransferEntry::preparing(
                    selected.clone(),
                    request.expected_bytes,
                    request.created_at,
                );
                retained.created = true;
                retained.cancellation.cancel();
                retained
            });
            return Err(error);
        }
        let mut transfers = lock(&self.transfers);
        if transfers
            .get(request.handle)
            .is_some_and(|entry| entry.object == selected && entry.cancellation.is_canceled())
        {
            transfers.remove(request.handle);
        }
        Ok(())
    }

    fn clear_pending(&self, handle: &UploadHandle, pending: &ChunkShape) {
        if let Some(entry) = lock(&self.transfers).get_mut(handle)
            && entry.pending.as_ref() == Some(pending)
        {
            entry.pending = None;
        }
    }

    async fn remove_entry(&self, handle: &UploadHandle) -> Result<(), UploadError> {
        let selected = {
            let transfers = lock(&self.transfers);
            transfers
                .get(handle)
                .map(|entry| (entry.object.clone(), entry.cancellation.clone()))
        };
        let Some((object, cancellation)) = selected else {
            return Ok(());
        };
        cancellation.cancel();
        let _descriptor = self
            .descriptor_permits
            .try_acquire()
            .map_err(|_| UploadError::new(UploadErrorKind::ResourceExhausted))?;
        self.store.remove(&object).await?;
        let mut transfers = lock(&self.transfers);
        if transfers
            .get(handle)
            .is_some_and(|entry| entry.object == object)
        {
            transfers.remove(handle);
        }
        Ok(())
    }
}

impl<S: QuarantineStore> QuarantinedFileProvider<S> {
    fn prepare<'a>(
        &'a self,
        request: PrepareTransfer<'a>,
    ) -> UploadFuture<'a, Result<TransferPlan, UploadError>> {
        Box::pin(async move {
            self.require_active()?;
            if request.expected_bytes > self.limits.max_file_bytes()
                || request.client_name.len() > MAX_CLIENT_NAME_BYTES
            {
                return Err(UploadError::new(UploadErrorKind::InputTooLarge));
            }
            let _descriptor = self
                .descriptor_permits
                .try_acquire()
                .map_err(|_| UploadError::new(UploadErrorKind::ResourceExhausted))?;
            let selected = match self.reserve_preparation(request)? {
                PreparationReservation::Existing(plan) => return Ok(plan),
                PreparationReservation::Reserved(object) => {
                    self.create_reserved_object(request.handle, object).await?
                }
            };
            if !self.publish_preparation(request.handle, &selected) {
                self.discard_unpublished_preparation(request, selected)
                    .await?;
                return Err(UploadError::new(UploadErrorKind::TransferCanceled));
            }
            Ok(TransferPlan::reverse_proxy(
                request.handle.clone(),
                self.limits.max_chunk_bytes(),
                TransferDisposition::Prepared,
            ))
        })
    }

    fn write_chunk<'a>(
        &'a self,
        request: WriteChunk<'a>,
        body: &'a mut dyn ChunkBody,
    ) -> UploadFuture<'a, Result<ChunkReceipt, UploadError>> {
        Box::pin(async move {
            self.require_active()?;
            let size = usize::try_from(request.size)
                .map_err(|_| UploadError::new(UploadErrorKind::InputTooLarge))?;
            if size == 0
                || size > self.limits.max_chunk_bytes()
                || size > self.limits.max_in_flight_bytes()
            {
                return Err(UploadError::new(UploadErrorKind::InputTooLarge));
            }
            let _chunk = self
                .chunk_permits
                .try_acquire()
                .map_err(|_| UploadError::new(UploadErrorKind::ResourceExhausted))?;
            let _descriptor = self
                .descriptor_permits
                .try_acquire()
                .map_err(|_| UploadError::new(UploadErrorKind::ResourceExhausted))?;
            let pending = ChunkShape::from_request(request);
            let (object, cancellation) = {
                let mut transfers = lock(&self.transfers);
                let entry = transfers
                    .get_mut(request.handle)
                    .ok_or_else(|| UploadError::new(UploadErrorKind::UploadConflict))?;
                if !entry.created {
                    return Err(UploadError::new(UploadErrorKind::UploadConflict));
                }
                if entry.cancellation.is_canceled() {
                    return Err(UploadError::new(UploadErrorKind::TransferCanceled));
                }
                if let Some(existing) = entry.chunks.get(&request.index) {
                    return if existing == &pending {
                        Ok(existing.receipt(ChunkDisposition::ExistingOutcome))
                    } else {
                        Err(UploadError::new(UploadErrorKind::UploadConflict))
                    };
                }
                let next_index = u32::try_from(entry.chunks.len())
                    .map_err(|_| UploadError::new(UploadErrorKind::InputTooLarge))?;
                if entry.chunks.len() >= self.limits.max_chunks_per_file() {
                    return Err(UploadError::new(UploadErrorKind::ResourceExhausted));
                }
                let end = request
                    .offset
                    .checked_add(request.size)
                    .ok_or_else(|| UploadError::new(UploadErrorKind::InputTooLarge))?;
                if request.index != next_index
                    || request.offset != entry.committed_bytes
                    || end > entry.expected_bytes
                    || entry.pending.is_some()
                    || entry.evidence.is_some()
                {
                    return Err(UploadError::new(UploadErrorKind::UploadConflict));
                }
                entry.pending = Some(pending.clone());
                (entry.object.clone(), entry.cancellation.clone())
            };

            let result = async {
                let provider_cancellation = self.resources.cancellation();
                let mut received = 0_usize;
                let mut hasher = Sha256::new();
                while received < size {
                    if cancellation.is_canceled() || provider_cancellation.is_canceled() {
                        return Err(UploadError::new(UploadErrorKind::TransferCanceled));
                    }
                    let maximum = (size - received).min(STREAM_BUFFER_BYTES);
                    let bytes = body
                        .next_chunk(maximum)
                        .await?
                        .ok_or_else(|| UploadError::new(UploadErrorKind::IncompleteTransfer))?;
                    if bytes.is_empty() || bytes.len() > maximum {
                        return Err(UploadError::new(UploadErrorKind::InputTooLarge));
                    }
                    if cancellation.is_canceled() || provider_cancellation.is_canceled() {
                        return Err(UploadError::new(UploadErrorKind::TransferCanceled));
                    }
                    hasher.update(&bytes);
                    self.store
                        .write_at(&object, request.offset + received as u64, bytes.as_ref())
                        .await?;
                    received += bytes.len();
                }
                if body.next_chunk(1).await?.is_some() {
                    return Err(UploadError::new(UploadErrorKind::InputTooLarge));
                }
                let actual = checksum_from_hasher(hasher)?;
                if actual != *request.checksum {
                    return Err(UploadError::new(UploadErrorKind::ChecksumMismatch));
                }
                if cancellation.is_canceled() || provider_cancellation.is_canceled() {
                    return Err(UploadError::new(UploadErrorKind::TransferCanceled));
                }
                Ok(())
            }
            .await;
            if let Err(error) = result {
                self.clear_pending(request.handle, &pending);
                return Err(error);
            }

            let mut transfers = lock(&self.transfers);
            let entry = transfers
                .get_mut(request.handle)
                .ok_or_else(|| UploadError::new(UploadErrorKind::TransferCanceled))?;
            if entry.pending.as_ref() != Some(&pending)
                || entry.cancellation.is_canceled()
                || self.resources.cancellation().is_canceled()
            {
                entry.pending = None;
                return Err(UploadError::new(UploadErrorKind::TransferCanceled));
            }
            entry.pending = None;
            entry.committed_bytes = entry
                .committed_bytes
                .checked_add(request.size)
                .ok_or_else(|| UploadError::new(UploadErrorKind::InputTooLarge))?;
            entry.chunks.insert(request.index, pending.clone());
            Ok(pending.receipt(ChunkDisposition::Stored))
        })
    }

    fn verify<'a>(
        &'a self,
        request: VerifyTransfer<'a>,
    ) -> UploadFuture<'a, Result<IntegrityEvidence, UploadError>> {
        Box::pin(async move {
            self.require_active()?;
            let _chunk = self
                .chunk_permits
                .try_acquire()
                .map_err(|_| UploadError::new(UploadErrorKind::ResourceExhausted))?;
            let _descriptor = self
                .descriptor_permits
                .try_acquire()
                .map_err(|_| UploadError::new(UploadErrorKind::ResourceExhausted))?;
            let (object, expected_bytes, cancellation) = {
                let transfers = lock(&self.transfers);
                let entry = transfers
                    .get(request.handle)
                    .ok_or_else(|| UploadError::new(UploadErrorKind::UploadConflict))?;
                if let Some(evidence) = &entry.evidence {
                    return if evidence.checksum() == request.checksum {
                        Ok(evidence.clone())
                    } else {
                        Err(UploadError::new(UploadErrorKind::ChecksumMismatch))
                    };
                }
                if !entry.created
                    || entry.pending.is_some()
                    || entry.committed_bytes != entry.expected_bytes
                {
                    return Err(UploadError::new(UploadErrorKind::IncompleteTransfer));
                }
                if entry.cancellation.is_canceled() {
                    return Err(UploadError::new(UploadErrorKind::TransferCanceled));
                }
                (
                    entry.object.clone(),
                    entry.expected_bytes,
                    entry.cancellation.clone(),
                )
            };

            let provider_cancellation = self.resources.cancellation();
            let mut hasher = Sha256::new();
            let mut offset = 0_u64;
            while offset < expected_bytes {
                if cancellation.is_canceled() || provider_cancellation.is_canceled() {
                    return Err(UploadError::new(UploadErrorKind::TransferCanceled));
                }
                let remaining =
                    usize::try_from((expected_bytes - offset).min(STREAM_BUFFER_BYTES as u64))
                        .map_err(|_| UploadError::new(UploadErrorKind::InputTooLarge))?;
                let bytes = self.store.read_at(&object, offset, remaining).await?;
                if bytes.is_empty() || bytes.len() > remaining {
                    return Err(UploadError::new(UploadErrorKind::IncompleteTransfer));
                }
                hasher.update(&bytes);
                offset += bytes.len() as u64;
            }
            if !self
                .store
                .read_at(&object, expected_bytes, 1)
                .await?
                .is_empty()
            {
                return Err(UploadError::new(UploadErrorKind::IncompleteTransfer));
            }
            let actual = checksum_from_hasher(hasher)?;
            if actual != *request.checksum {
                return Err(UploadError::new(UploadErrorKind::ChecksumMismatch));
            }
            self.store.sync(&object).await?;
            if cancellation.is_canceled() || provider_cancellation.is_canceled() {
                return Err(UploadError::new(UploadErrorKind::TransferCanceled));
            }
            let evidence = IntegrityEvidence {
                bytes: expected_bytes,
                checksum: actual,
            };
            let mut transfers = lock(&self.transfers);
            let entry = transfers
                .get_mut(request.handle)
                .ok_or_else(|| UploadError::new(UploadErrorKind::TransferCanceled))?;
            if entry.object != object
                || entry.pending.is_some()
                || entry.cancellation.is_canceled()
                || entry.committed_bytes != expected_bytes
            {
                return Err(UploadError::new(UploadErrorKind::TransferCanceled));
            }
            entry.evidence = Some(evidence.clone());
            Ok(evidence)
        })
    }

    fn cancel<'a>(&'a self, handle: &'a UploadHandle) -> UploadFuture<'a, Result<(), UploadError>> {
        Box::pin(async move { self.remove_entry(handle).await })
    }

    fn cleanup<'a>(
        &'a self,
        handle: &'a UploadHandle,
    ) -> UploadFuture<'a, Result<(), UploadError>> {
        Box::pin(async move { self.remove_entry(handle).await })
    }
}

impl<S: QuarantineStore> UploadProvider for QuarantinedFileProvider<S> {
    fn prepare<'a>(
        &'a self,
        request: PrepareTransfer<'a>,
    ) -> UploadFuture<'a, Result<TransferPlan, UploadError>> {
        QuarantinedFileProvider::prepare(self, request)
    }

    fn verify<'a>(
        &'a self,
        request: VerifyTransfer<'a>,
    ) -> UploadFuture<'a, Result<IntegrityEvidence, UploadError>> {
        QuarantinedFileProvider::verify(self, request)
    }

    fn cancel<'a>(&'a self, handle: &'a UploadHandle) -> UploadFuture<'a, Result<(), UploadError>> {
        QuarantinedFileProvider::cancel(self, handle)
    }

    fn cleanup<'a>(
        &'a self,
        handle: &'a UploadHandle,
    ) -> UploadFuture<'a, Result<(), UploadError>> {
        QuarantinedFileProvider::cleanup(self, handle)
    }
}

impl<S: QuarantineStore> ReverseProxyUploadProvider for QuarantinedFileProvider<S> {
    fn write_chunk<'a>(
        &'a self,
        request: WriteChunk<'a>,
        body: &'a mut dyn ChunkBody,
    ) -> UploadFuture<'a, Result<ChunkReceipt, UploadError>> {
        QuarantinedFileProvider::write_chunk(self, request, body)
    }
}

impl<S: QuarantineStore> fmt::Debug for QuarantinedFileProvider<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("QuarantinedFileProvider")
            .field("limits", &self.limits)
            .field("transfers", &lock(&self.transfers).len())
            .field("resources", &self.resources)
            .finish_non_exhaustive()
    }
}

fn validate_checkpoint(checkpoint: &TransferCheckpoint) -> Result<(), UploadError> {
    let mut expected_index = 0_u32;
    let mut expected_offset = 0_u64;
    for chunk in checkpoint.chunks.values() {
        if chunk.index != expected_index || chunk.offset != expected_offset || chunk.size == 0 {
            return Err(UploadError::new(UploadErrorKind::UploadConflict));
        }
        expected_offset = expected_offset
            .checked_add(chunk.size)
            .ok_or_else(|| UploadError::new(UploadErrorKind::InputTooLarge))?;
        expected_index = expected_index
            .checked_add(1)
            .ok_or_else(|| UploadError::new(UploadErrorKind::InputTooLarge))?;
    }
    if expected_offset != checkpoint.committed_bytes {
        return Err(UploadError::new(UploadErrorKind::UploadConflict));
    }
    Ok(())
}

fn checksum_from_hasher(hasher: Sha256) -> Result<UploadChecksum, UploadError> {
    let digest = hasher.finalize();
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}")
            .map_err(|_| UploadError::new(UploadErrorKind::ProviderUnavailable))?;
    }
    UploadChecksum::parse(&encoded)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
