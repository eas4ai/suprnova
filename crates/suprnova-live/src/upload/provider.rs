//! Bounded streaming provider over opaque asynchronous quarantine I/O.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::fmt::Write as _;
use std::future::poll_fn;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Poll, Waker};

use sha2::{Digest, Sha256};

use crate::identity::UnixMillis;
use crate::limits::UploadLimits;
use crate::resource::{CancellationFlag, PermitPool, ResourceBounds, ResourceOwner, Retirement};

use super::{
    DirectTransferInstruction, QuarantineBytes, QuarantineObject, QuarantineOperation,
    QuarantineStore, ReportDirectPart, TransferInstruction, UploadChecksum, UploadError,
    UploadErrorKind, UploadFuture, UploadHandle, UploadPart,
};

const MAX_CLIENT_NAME_BYTES: usize = 1_024;
const STREAM_BUFFER_BYTES: usize = 64 * 1024;
const OBJECT_COLLISION_ATTEMPTS: usize = 4;
const DEFAULT_RETIREMENT_WAIT_STEPS: usize = 256;
const MAX_RETIREMENT_WAIT_STEPS: usize = 65_536;

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

/// Trusted bounded request to read authoritative completed upload bytes.
#[derive(Clone, Copy)]
pub struct ReadUpload<'a> {
    handle: &'a UploadHandle,
    offset: u64,
    maximum_bytes: usize,
}

impl<'a> ReadUpload<'a> {
    /// Groups an upload identity, byte offset, and hard response ceiling.
    #[must_use]
    pub const fn new(handle: &'a UploadHandle, offset: u64, maximum_bytes: usize) -> Self {
        Self {
            handle,
            offset,
            maximum_bytes,
        }
    }

    /// Returns the upload whose authoritative bytes are requested.
    #[must_use]
    pub const fn handle(&self) -> &UploadHandle {
        self.handle
    }

    /// Returns the zero-based byte offset.
    #[must_use]
    pub const fn offset(&self) -> u64 {
        self.offset
    }

    /// Returns the absolute response byte ceiling.
    #[must_use]
    pub const fn maximum_bytes(&self) -> usize {
        self.maximum_bytes
    }
}

impl fmt::Debug for ReadUpload<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<ReadUpload:redacted>")
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

    /// Reads no more than the requested authoritative completed byte range.
    fn read<'a>(
        &'a self,
        request: ReadUpload<'a>,
    ) -> UploadFuture<'a, Result<QuarantineBytes, UploadError>>;

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
    pending_abandoned: bool,
    physical_operations: usize,
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
            pending_abandoned: false,
            physical_operations: 0,
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
    transfers: Arc<Mutex<HashMap<UploadHandle, TransferEntry>>>,
    resources: ResourceOwner<UploadHandle>,
    admission: ProviderAdmission,
    cleanup_permits: PermitPool,
    descriptor_permits: PermitPool,
    chunk_permits: PermitPool,
    retirement_wait_steps: usize,
}

/// Exact bounded-resource evidence captured when provider retirement fails.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProviderRetirementStatus {
    active_operations: usize,
    owned_transfers: usize,
    active_descriptors: usize,
    active_chunks: usize,
}

/// Exact bounded metadata ownership for one provider-held transfer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProviderTransferAccounting {
    accepted_chunk_records: usize,
    committed_bytes: u64,
    pending_chunk: bool,
}

impl ProviderTransferAccounting {
    /// Returns the accepted `BTreeMap` record cardinality.
    #[must_use]
    pub const fn accepted_chunk_records(self) -> usize {
        self.accepted_chunk_records
    }

    /// Returns bytes represented by accepted provider metadata.
    #[must_use]
    pub const fn committed_bytes(self) -> u64 {
        self.committed_bytes
    }

    /// Returns whether one conditional chunk record is currently pending.
    #[must_use]
    pub const fn pending_chunk(self) -> bool {
        self.pending_chunk
    }
}

impl ProviderRetirementStatus {
    /// Returns operations that still own an admission token.
    #[must_use]
    pub const fn active_operations(self) -> usize {
        self.active_operations
    }

    /// Returns quarantine transfers still fenced by the retired provider.
    #[must_use]
    pub const fn owned_transfers(self) -> usize {
        self.owned_transfers
    }

    /// Returns descriptor permits still held by unfinished operations.
    #[must_use]
    pub const fn active_descriptors(self) -> usize {
        self.active_descriptors
    }

    /// Returns chunk permits still held by unfinished operations.
    #[must_use]
    pub const fn active_chunks(self) -> usize {
        self.active_chunks
    }
}

/// Typed, redacted provider-retirement failure with exact resource counts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProviderRetirementError {
    kind: UploadErrorKind,
    status: ProviderRetirementStatus,
}

impl ProviderRetirementError {
    const fn new(kind: UploadErrorKind, status: ProviderRetirementStatus) -> Self {
        Self { kind, status }
    }

    /// Returns the closed failure category.
    #[must_use]
    pub const fn kind(self) -> UploadErrorKind {
        self.kind
    }

    /// Returns exact bounded-resource evidence at failure.
    #[must_use]
    pub const fn status(self) -> ProviderRetirementStatus {
        self.status
    }
}

impl fmt::Display for ProviderRetirementError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl std::error::Error for ProviderRetirementError {}

struct ProviderAdmission {
    inner: Arc<ProviderAdmissionInner>,
}

struct ProviderAdmissionInner {
    maximum_active: usize,
    state: Mutex<ProviderAdmissionState>,
}

#[derive(Default)]
struct ProviderAdmissionState {
    retired: bool,
    active: Vec<Arc<ProviderOperationCancellation>>,
    idle_waker: Option<Waker>,
}

struct ProviderOperationCancellation {
    canceled: AtomicBool,
    logical_open: AtomicBool,
    physical_operations: AtomicUsize,
    waker: Mutex<Option<Waker>>,
}

impl Default for ProviderOperationCancellation {
    fn default() -> Self {
        Self {
            canceled: AtomicBool::new(false),
            logical_open: AtomicBool::new(true),
            physical_operations: AtomicUsize::new(0),
            waker: Mutex::new(None),
        }
    }
}

impl ProviderOperationCancellation {
    fn cancel(&self) {
        self.canceled.store(true, Ordering::Release);
        let wake = lock(&self.waker).take();
        if let Some(waker) = wake {
            waker.wake();
        }
    }

    fn poll_canceled(&self, task: &mut std::task::Context<'_>) -> Poll<()> {
        if self.canceled.load(Ordering::Acquire) {
            return Poll::Ready(());
        }
        let mut registered = lock(&self.waker);
        if self.canceled.load(Ordering::Acquire) {
            return Poll::Ready(());
        }
        if registered
            .as_ref()
            .is_none_or(|waker| !waker.will_wake(task.waker()))
        {
            *registered = Some(task.waker().clone());
        }
        Poll::Pending
    }
}

impl ProviderAdmission {
    fn new(maximum_active: usize) -> Self {
        Self {
            inner: Arc::new(ProviderAdmissionInner {
                maximum_active,
                state: Mutex::new(ProviderAdmissionState::default()),
            }),
        }
    }

    fn enter(&self) -> Result<ProviderAdmissionGuard, UploadError> {
        let mut state = lock(&self.inner.state);
        if state.retired {
            return Err(UploadError::new(UploadErrorKind::ServiceRetired));
        }
        if state.active.len() >= self.inner.maximum_active {
            return Err(UploadError::new(UploadErrorKind::ResourceExhausted));
        }
        let cancellation = Arc::new(ProviderOperationCancellation::default());
        state.active.push(Arc::clone(&cancellation));
        Ok(ProviderAdmissionGuard {
            admission: Arc::clone(&self.inner),
            cancellation,
        })
    }

    fn enter_cleanup(&self) -> ProviderAdmissionGuard {
        let cancellation = Arc::new(ProviderOperationCancellation::default());
        lock(&self.inner.state)
            .active
            .push(Arc::clone(&cancellation));
        ProviderAdmissionGuard {
            admission: Arc::clone(&self.inner),
            cancellation,
        }
    }

    fn retire(&self) {
        let active = {
            let mut state = lock(&self.inner.state);
            state.retired = true;
            state.active.clone()
        };
        for cancellation in active {
            cancellation.cancel();
        }
    }

    async fn wait_until_idle(&self, maximum_steps: usize) -> bool {
        let mut remaining = maximum_steps;
        poll_fn(move |task| {
            let mut state = lock(&self.inner.state);
            if state.active.is_empty() {
                return Poll::Ready(true);
            }
            if remaining == 0 {
                return Poll::Ready(false);
            }
            if state
                .idle_waker
                .as_ref()
                .is_none_or(|registered| !registered.will_wake(task.waker()))
            {
                state.idle_waker = Some(task.waker().clone());
            }
            remaining -= 1;
            task.waker().wake_by_ref();
            Poll::Pending
        })
        .await
    }

    fn active(&self) -> usize {
        lock(&self.inner.state).active.len()
    }
}

struct ProviderAdmissionGuard {
    admission: Arc<ProviderAdmissionInner>,
    cancellation: Arc<ProviderOperationCancellation>,
}

impl ProviderAdmissionGuard {
    // Before first poll cancellation can prove that no external effect began.
    // After first poll, keep driving the capability to a terminal result and
    // translate that result to cancellation. Dropping a pending filesystem or
    // object-store future here could let its detached I/O publish after the
    // provider swept and forgot the opaque object.
    async fn wait<T>(
        &self,
        mut operation: UploadFuture<'_, Result<T, UploadError>>,
    ) -> Result<T, UploadError> {
        let mut started = false;
        poll_fn(|task| {
            let canceled = self.cancellation.poll_canceled(task).is_ready();
            if canceled && !started {
                return Poll::Ready(Err(UploadError::new(UploadErrorKind::TransferCanceled)));
            }
            started = true;
            match operation.as_mut().poll(task) {
                Poll::Ready(_)
                    if canceled || self.cancellation.canceled.load(Ordering::Acquire) =>
                {
                    Poll::Ready(Err(UploadError::new(UploadErrorKind::TransferCanceled)))
                }
                ready @ Poll::Ready(_) => ready,
                Poll::Pending => Poll::Pending,
            }
        })
        .await
    }

    async fn wait_store<T: Clone>(
        &self,
        mut operation: QuarantineOperation<T>,
    ) -> Result<T, UploadError> {
        self.cancellation
            .physical_operations
            .fetch_add(1, Ordering::AcqRel);
        operation.supervise(ProviderPhysicalOperation {
            admission: Arc::clone(&self.admission),
            cancellation: Arc::clone(&self.cancellation),
        });
        poll_fn(|task| {
            let canceled = self.cancellation.poll_canceled(task).is_ready();
            match Pin::new(&mut operation).poll(task) {
                Poll::Ready(_)
                    if canceled || self.cancellation.canceled.load(Ordering::Acquire) =>
                {
                    Poll::Ready(Err(UploadError::new(UploadErrorKind::TransferCanceled)))
                }
                ready @ Poll::Ready(_) => ready,
                Poll::Pending => Poll::Pending,
            }
        })
        .await
    }
}

impl Drop for ProviderAdmissionGuard {
    fn drop(&mut self) {
        self.cancellation
            .logical_open
            .store(false, Ordering::Release);
        release_provider_operation(&self.admission, &self.cancellation);
    }
}

struct ProviderPhysicalOperation {
    admission: Arc<ProviderAdmissionInner>,
    cancellation: Arc<ProviderOperationCancellation>,
}

impl Drop for ProviderPhysicalOperation {
    fn drop(&mut self) {
        let previous = self
            .cancellation
            .physical_operations
            .fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "provider physical-operation underflow");
        release_provider_operation(&self.admission, &self.cancellation);
    }
}

fn release_provider_operation(
    admission: &ProviderAdmissionInner,
    operation: &Arc<ProviderOperationCancellation>,
) {
    if operation.logical_open.load(Ordering::Acquire)
        || operation.physical_operations.load(Ordering::Acquire) != 0
    {
        return;
    }
    let wake = {
        let mut state = lock(&admission.state);
        if operation.logical_open.load(Ordering::Acquire)
            || operation.physical_operations.load(Ordering::Acquire) != 0
        {
            return;
        }
        state
            .active
            .retain(|active| !Arc::ptr_eq(active, operation));
        state
            .active
            .is_empty()
            .then(|| state.idle_waker.take())
            .flatten()
    };
    if let Some(waker) = wake {
        waker.wake();
    }
}

struct PreparationGuard<'a, S: QuarantineStore> {
    provider: &'a QuarantinedFileProvider<S>,
    handle: UploadHandle,
    object: QuarantineObject,
    armed: bool,
}

impl<'a, S: QuarantineStore> PreparationGuard<'a, S> {
    fn new(
        provider: &'a QuarantinedFileProvider<S>,
        handle: UploadHandle,
        object: QuarantineObject,
    ) -> Self {
        Self {
            provider,
            handle,
            object,
            armed: true,
        }
    }

    const fn object(&self) -> &QuarantineObject {
        &self.object
    }

    fn replace_object(&mut self, object: QuarantineObject) {
        self.object = object;
    }

    fn remove_reservation(&mut self) {
        let mut transfers = lock(&self.provider.transfers);
        if transfers
            .get(&self.handle)
            .is_some_and(|entry| entry.object == self.object && !entry.created)
        {
            transfers.remove(&self.handle);
        }
        self.armed = false;
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl<S: QuarantineStore> Drop for PreparationGuard<'_, S> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut transfers = lock(&self.provider.transfers);
        if let Some(entry) = transfers.get_mut(&self.handle)
            && entry.object == self.object
            && !entry.created
        {
            entry.cancellation.cancel();
        }
    }
}

struct PendingChunkGuard<'a, S: QuarantineStore> {
    provider: &'a QuarantinedFileProvider<S>,
    handle: UploadHandle,
    pending: ChunkShape,
    armed: bool,
}

impl<'a, S: QuarantineStore> PendingChunkGuard<'a, S> {
    fn new(
        provider: &'a QuarantinedFileProvider<S>,
        handle: UploadHandle,
        pending: ChunkShape,
    ) -> Self {
        Self {
            provider,
            handle,
            pending,
            armed: true,
        }
    }

    fn commit(mut self) -> Result<ChunkReceipt, UploadError> {
        let receipt = {
            let mut transfers = lock(&self.provider.transfers);
            let entry = transfers
                .get_mut(&self.handle)
                .ok_or_else(|| UploadError::new(UploadErrorKind::TransferCanceled))?;
            if entry.pending.as_ref() != Some(&self.pending)
                || entry.cancellation.is_canceled()
                || self.provider.resources.cancellation().is_canceled()
            {
                return Err(UploadError::new(UploadErrorKind::TransferCanceled));
            }
            let committed_bytes = entry
                .committed_bytes
                .checked_add(self.pending.size)
                .ok_or_else(|| UploadError::new(UploadErrorKind::InputTooLarge))?;
            entry.pending = None;
            entry.committed_bytes = committed_bytes;
            entry
                .chunks
                .insert(self.pending.index, self.pending.clone());
            self.pending.receipt(ChunkDisposition::Stored)
        };
        self.armed = false;
        Ok(receipt)
    }
}

impl<S: QuarantineStore> Drop for PendingChunkGuard<'_, S> {
    fn drop(&mut self) {
        if self.armed {
            self.provider.abandon_pending(&self.handle, &self.pending);
        }
    }
}

struct TransferPhysicalOperation {
    transfers: Arc<Mutex<HashMap<UploadHandle, TransferEntry>>>,
    handle: UploadHandle,
    object: QuarantineObject,
}

impl Drop for TransferPhysicalOperation {
    fn drop(&mut self) {
        let mut transfers = lock(&self.transfers);
        let Some(entry) = transfers.get_mut(&self.handle) else {
            return;
        };
        if entry.object != self.object {
            return;
        }
        debug_assert!(
            entry.physical_operations > 0,
            "transfer physical-operation underflow"
        );
        entry.physical_operations = entry.physical_operations.saturating_sub(1);
        if entry.physical_operations == 0 && entry.pending_abandoned {
            entry.pending = None;
            entry.pending_abandoned = false;
        }
    }
}

impl<S: QuarantineStore> QuarantinedFileProvider<S> {
    /// Creates one bounded provider without an executor or filesystem dependency.
    pub fn new(store: Arc<S>, limits: UploadLimits) -> Result<Self, UploadError> {
        Self::new_with_retirement_wait_steps(store, limits, DEFAULT_RETIREMENT_WAIT_STEPS)
    }

    /// Creates a provider with an injected scheduler-step retirement budget.
    ///
    /// The budget is executor-neutral: each pending idle observation consumes
    /// one step, so shutdown always reaches a terminal result without a timer.
    pub fn new_with_retirement_wait_steps(
        store: Arc<S>,
        limits: UploadLimits,
        retirement_wait_steps: usize,
    ) -> Result<Self, UploadError> {
        if retirement_wait_steps == 0 || retirement_wait_steps > MAX_RETIREMENT_WAIT_STEPS {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        }
        let resource_bounds = ResourceBounds::new(
            limits.max_concurrent_transfers(),
            limits.max_in_flight_bytes(),
        )
        .map_err(|_| UploadError::new(UploadErrorKind::InvalidField))?;
        let descriptor_permits = PermitPool::new(limits.max_concurrent_transfers())
            .map_err(|_| UploadError::new(UploadErrorKind::InvalidField))?;
        let chunk_permits = PermitPool::new(limits.max_concurrent_transfers())
            .map_err(|_| UploadError::new(UploadErrorKind::InvalidField))?;
        let cleanup_permits =
            PermitPool::new(1).map_err(|_| UploadError::new(UploadErrorKind::InvalidField))?;
        Ok(Self {
            store,
            limits,
            transfers: Arc::new(Mutex::new(HashMap::new())),
            resources: ResourceOwner::new(resource_bounds),
            admission: ProviderAdmission::new(limits.max_concurrent_transfers()),
            cleanup_permits,
            descriptor_permits,
            chunk_permits,
            retirement_wait_steps,
        })
    }

    /// Returns a bounded non-path checkpoint for one quiescent transfer.
    pub fn checkpoint(&self, handle: &UploadHandle) -> Result<TransferCheckpoint, UploadError> {
        let transfers = lock(&self.transfers);
        let entry = transfers
            .get(handle)
            .ok_or_else(|| UploadError::new(UploadErrorKind::UploadConflict))?;
        if !entry.created
            || entry.pending.is_some()
            || entry.physical_operations != 0
            || entry.cancellation.is_canceled()
        {
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

    /// Returns exact count-only ownership evidence without exposing paths or payload bytes.
    pub fn transfer_accounting(
        &self,
        handle: &UploadHandle,
    ) -> Result<ProviderTransferAccounting, UploadError> {
        let transfers = lock(&self.transfers);
        let entry = transfers
            .get(handle)
            .ok_or_else(|| UploadError::new(UploadErrorKind::UploadConflict))?;
        Ok(ProviderTransferAccounting {
            accepted_chunk_records: entry.chunks.len(),
            committed_bytes: entry.committed_bytes,
            pending_chunk: entry.pending.is_some(),
        })
    }

    /// Restores one quiescent checkpoint without accepting client path material.
    pub fn recover(&self, checkpoint: TransferCheckpoint) -> Result<(), UploadError> {
        let admission = self.admission.enter()?;
        self.recover_with_admission(checkpoint, admission)
    }

    fn recover_with_admission(
        &self,
        checkpoint: TransferCheckpoint,
        _admission: ProviderAdmissionGuard,
    ) -> Result<(), UploadError> {
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
                pending_abandoned: false,
                physical_operations: 0,
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
        self.admission.retire();
        self.resources.retire()
    }

    /// Retires admission and idempotently reclaims every provider-owned object.
    pub async fn retire_and_cleanup(&self) -> Result<Retirement, ProviderRetirementError> {
        let _cleanup = self.cleanup_permits.try_acquire().map_err(|_| {
            ProviderRetirementError::new(
                UploadErrorKind::ResourceExhausted,
                self.retirement_status(),
            )
        })?;
        let retirement = self.retire();
        if !self
            .admission
            .wait_until_idle(self.retirement_wait_steps)
            .await
        {
            return Err(ProviderRetirementError::new(
                UploadErrorKind::CleanupTimedOut,
                self.retirement_status(),
            ));
        }
        let mut first_error: Option<UploadErrorKind> = None;
        for _ in 0..=self.limits.max_pending_per_scope() {
            let handles = lock(&self.transfers).keys().cloned().collect::<Vec<_>>();
            if handles.is_empty() {
                return first_error.map_or(Ok(retirement), |kind| {
                    Err(ProviderRetirementError::new(kind, self.retirement_status()))
                });
            }
            let previous = handles.len();
            for handle in handles {
                if let Err(error) = self.remove_entry(&handle).await
                    && first_error.is_none()
                {
                    first_error = Some(error.kind());
                }
            }
            if lock(&self.transfers).len() >= previous {
                break;
            }
        }
        Err(ProviderRetirementError::new(
            first_error.unwrap_or(UploadErrorKind::ProviderUnavailable),
            self.retirement_status(),
        ))
    }

    /// Returns exact resource counts without exposing handles or paths.
    #[must_use]
    pub fn retirement_status(&self) -> ProviderRetirementStatus {
        ProviderRetirementStatus {
            active_operations: self.admission.active(),
            owned_transfers: lock(&self.transfers).len(),
            active_descriptors: self.descriptor_permits.active(),
            active_chunks: self.chunk_permits.active(),
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
        admission: &ProviderAdmissionGuard,
        reservation: &mut PreparationGuard<'_, S>,
    ) -> Result<(), UploadError> {
        for _ in 0..OBJECT_COLLISION_ATTEMPTS {
            let operation =
                self.supervise_store_operation(&reservation.handle, reservation.object(), || {
                    self.store.create_exclusive(reservation.object())
                })?;
            match admission.wait_store(operation).await {
                Ok(()) => return Ok(()),
                Err(error) if error.kind() == UploadErrorKind::StorageConflict => {
                    let selected = match QuarantineObject::generate() {
                        Ok(object) => object,
                        Err(error) => {
                            reservation.remove_reservation();
                            return Err(error);
                        }
                    };
                    if let Some(entry) = lock(&self.transfers).get_mut(&reservation.handle) {
                        entry.object = selected.clone();
                    } else {
                        return Err(UploadError::new(UploadErrorKind::TransferCanceled));
                    }
                    reservation.replace_object(selected);
                }
                Err(error) => {
                    // The store may have created bytes before reporting or
                    // observing failure. Keep the opaque object reserved and
                    // canceled so retirement/retry owns its exact cleanup.
                    return Err(error);
                }
            }
        }
        reservation.remove_reservation();
        Err(UploadError::new(UploadErrorKind::StorageConflict))
    }

    async fn reclaim_abandoned_preparation(
        &self,
        admission: &ProviderAdmissionGuard,
        handle: &UploadHandle,
    ) -> Result<(), UploadError> {
        let abandoned = lock(&self.transfers)
            .get(handle)
            .is_some_and(|entry| !entry.created && entry.cancellation.is_canceled());
        if abandoned {
            self.remove_entry_with_admission(handle, Some(admission))
                .await?;
        }
        Ok(())
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
        admission: &ProviderAdmissionGuard,
        request: PrepareTransfer<'_>,
        selected: QuarantineObject,
    ) -> Result<(), UploadError> {
        let operation =
            self.supervise_cleanup_store_operation(request.handle, &selected, || {
                self.store.remove(&selected)
            })?;
        if let Err(error) = admission.wait_store(operation).await {
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
            .is_some_and(|entry| entry.object == selected && !entry.created)
        {
            transfers.remove(request.handle);
        }
        Ok(())
    }

    fn abandon_pending(&self, handle: &UploadHandle, pending: &ChunkShape) {
        if let Some(entry) = lock(&self.transfers).get_mut(handle)
            && entry.pending.as_ref() == Some(pending)
        {
            if entry.physical_operations == 0 {
                entry.pending = None;
            } else {
                entry.pending_abandoned = true;
            }
        }
    }

    fn supervise_store_operation<T>(
        &self,
        handle: &UploadHandle,
        object: &QuarantineObject,
        start: impl FnOnce() -> QuarantineOperation<T>,
    ) -> Result<QuarantineOperation<T>, UploadError> {
        self.supervise_store_operation_inner(handle, object, false, start)
    }

    fn supervise_cleanup_store_operation<T>(
        &self,
        handle: &UploadHandle,
        object: &QuarantineObject,
        start: impl FnOnce() -> QuarantineOperation<T>,
    ) -> Result<QuarantineOperation<T>, UploadError> {
        self.supervise_store_operation_inner(handle, object, true, start)
    }

    fn supervise_store_operation_inner<T>(
        &self,
        handle: &UploadHandle,
        object: &QuarantineObject,
        allow_canceled: bool,
        start: impl FnOnce() -> QuarantineOperation<T>,
    ) -> Result<QuarantineOperation<T>, UploadError> {
        {
            let mut transfers = lock(&self.transfers);
            let entry = transfers
                .get_mut(handle)
                .ok_or_else(|| UploadError::new(UploadErrorKind::TransferCanceled))?;
            if entry.object != *object || (!allow_canceled && entry.cancellation.is_canceled()) {
                return Err(UploadError::new(UploadErrorKind::TransferCanceled));
            }
            entry.physical_operations = entry
                .physical_operations
                .checked_add(1)
                .ok_or_else(|| UploadError::new(UploadErrorKind::ResourceExhausted))?;
        }
        let supervisor = TransferPhysicalOperation {
            transfers: Arc::clone(&self.transfers),
            handle: handle.clone(),
            object: object.clone(),
        };
        let operation = start();
        operation.supervise(supervisor);
        Ok(operation)
    }

    async fn remove_entry(&self, handle: &UploadHandle) -> Result<(), UploadError> {
        self.remove_entry_with_admission(handle, None).await
    }

    async fn remove_entry_with_admission(
        &self,
        handle: &UploadHandle,
        admission: Option<&ProviderAdmissionGuard>,
    ) -> Result<(), UploadError> {
        let selected = {
            let transfers = lock(&self.transfers);
            transfers.get(handle).map(|entry| {
                // Closing transfer admission and observing the physical count
                // are one critical section. A sibling cannot pass its final
                // pre-I/O check after cleanup has decided the object is idle.
                entry.cancellation.cancel();
                (
                    entry.object.clone(),
                    entry.cancellation.clone(),
                    entry.physical_operations,
                )
            })
        };
        let Some((object, cancellation, physical_operations)) = selected else {
            return Ok(());
        };
        debug_assert!(cancellation.is_canceled());
        if physical_operations != 0 {
            return Err(UploadError::new(UploadErrorKind::UploadConflict));
        }
        let _descriptor = self
            .descriptor_permits
            .try_acquire()
            .map_err(|_| UploadError::new(UploadErrorKind::ResourceExhausted))?;
        let cleanup_admission;
        let admission = match admission {
            Some(admission) => admission,
            None => {
                cleanup_admission = self.admission.enter_cleanup();
                &cleanup_admission
            }
        };
        let operation =
            self.supervise_cleanup_store_operation(handle, &object, || self.store.remove(&object))?;
        admission.wait_store(operation).await?;
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
            let admission = self.admission.enter()?;
            if request.expected_bytes > self.limits.max_file_bytes()
                || request.client_name.len() > MAX_CLIENT_NAME_BYTES
            {
                return Err(UploadError::new(UploadErrorKind::InputTooLarge));
            }
            self.reclaim_abandoned_preparation(&admission, request.handle)
                .await?;
            let _descriptor = self
                .descriptor_permits
                .try_acquire()
                .map_err(|_| UploadError::new(UploadErrorKind::ResourceExhausted))?;
            match self.reserve_preparation(request)? {
                PreparationReservation::Existing(plan) => return Ok(plan),
                PreparationReservation::Reserved(object) => {
                    let mut reservation =
                        PreparationGuard::new(self, request.handle.clone(), object);
                    self.create_reserved_object(&admission, &mut reservation)
                        .await?;
                    let selected = reservation.object().clone();
                    if !self.publish_preparation(request.handle, &selected) {
                        self.discard_unpublished_preparation(&admission, request, selected)
                            .await?;
                        return Err(UploadError::new(UploadErrorKind::TransferCanceled));
                    }
                    reservation.disarm();
                }
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
            let admission = self.admission.enter()?;
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
            let pending_guard =
                PendingChunkGuard::new(self, request.handle.clone(), pending.clone());

            async {
                let provider_cancellation = self.resources.cancellation();
                let mut received = 0_usize;
                let mut hasher = Sha256::new();
                while received < size {
                    if cancellation.is_canceled() || provider_cancellation.is_canceled() {
                        return Err(UploadError::new(UploadErrorKind::TransferCanceled));
                    }
                    let maximum = (size - received).min(STREAM_BUFFER_BYTES);
                    let bytes = admission
                        .wait(body.next_chunk(maximum))
                        .await?
                        .ok_or_else(|| UploadError::new(UploadErrorKind::IncompleteTransfer))?;
                    if bytes.is_empty() || bytes.len() > maximum {
                        return Err(UploadError::new(UploadErrorKind::InputTooLarge));
                    }
                    if cancellation.is_canceled() || provider_cancellation.is_canceled() {
                        return Err(UploadError::new(UploadErrorKind::TransferCanceled));
                    }
                    hasher.update(&bytes);
                    let operation =
                        self.supervise_store_operation(request.handle, &object, || {
                            self.store.write_at(
                                &object,
                                request.offset + received as u64,
                                bytes.as_ref(),
                            )
                        })?;
                    admission.wait_store(operation).await?;
                    received += bytes.len();
                }
                if admission.wait(body.next_chunk(1)).await?.is_some() {
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
            .await?;
            pending_guard.commit()
        })
    }

    fn verify<'a>(
        &'a self,
        request: VerifyTransfer<'a>,
    ) -> UploadFuture<'a, Result<IntegrityEvidence, UploadError>> {
        Box::pin(async move {
            let admission = self.admission.enter()?;
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
                let operation = self.supervise_store_operation(request.handle, &object, || {
                    self.store.read_at(&object, offset, remaining)
                })?;
                let bytes = admission.wait_store(operation).await?;
                if bytes.is_empty() || bytes.len() > remaining {
                    return Err(UploadError::new(UploadErrorKind::IncompleteTransfer));
                }
                hasher.update(&bytes);
                offset += bytes.len() as u64;
            }
            let operation = self.supervise_store_operation(request.handle, &object, || {
                self.store.read_at(&object, expected_bytes, 1)
            })?;
            if !admission.wait_store(operation).await?.is_empty() {
                return Err(UploadError::new(UploadErrorKind::IncompleteTransfer));
            }
            let actual = checksum_from_hasher(hasher)?;
            if actual != *request.checksum {
                return Err(UploadError::new(UploadErrorKind::ChecksumMismatch));
            }
            let operation = self
                .supervise_store_operation(request.handle, &object, || self.store.sync(&object))?;
            admission.wait_store(operation).await?;
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
                || self.resources.cancellation().is_canceled()
                || entry.committed_bytes != expected_bytes
            {
                return Err(UploadError::new(UploadErrorKind::TransferCanceled));
            }
            entry.evidence = Some(evidence.clone());
            Ok(evidence)
        })
    }

    fn read<'a>(
        &'a self,
        request: ReadUpload<'a>,
    ) -> UploadFuture<'a, Result<QuarantineBytes, UploadError>> {
        Box::pin(async move {
            let admission = self.admission.enter()?;
            if request.maximum_bytes == 0 || request.maximum_bytes > self.limits.max_chunk_bytes() {
                return Err(UploadError::new(UploadErrorKind::InputTooLarge));
            }
            let _descriptor = self
                .descriptor_permits
                .try_acquire()
                .map_err(|_| UploadError::new(UploadErrorKind::ResourceExhausted))?;
            let (object, expected_bytes, cancellation) = {
                let transfers = lock(&self.transfers);
                let entry = transfers
                    .get(request.handle)
                    .ok_or_else(|| UploadError::new(UploadErrorKind::UploadConflict))?;
                if !entry.created || entry.pending.is_some() || entry.evidence.is_none() {
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
            if request.offset > expected_bytes {
                return Err(UploadError::new(UploadErrorKind::InvalidField));
            }
            let target = usize::try_from(
                (expected_bytes - request.offset).min(request.maximum_bytes as u64),
            )
            .map_err(|_| UploadError::new(UploadErrorKind::InputTooLarge))?;
            let mut output = Vec::with_capacity(target);
            while output.len() < target {
                if cancellation.is_canceled() || self.resources.cancellation().is_canceled() {
                    return Err(UploadError::new(UploadErrorKind::TransferCanceled));
                }
                let offset = request
                    .offset
                    .checked_add(output.len() as u64)
                    .ok_or_else(|| UploadError::new(UploadErrorKind::InvalidField))?;
                let operation = self.supervise_store_operation(request.handle, &object, || {
                    self.store.read_at(&object, offset, target - output.len())
                })?;
                let bytes = admission.wait_store(operation).await?;
                if bytes.is_empty() || bytes.len() > target - output.len() {
                    return Err(UploadError::new(UploadErrorKind::IncompleteTransfer));
                }
                output.extend_from_slice(&bytes);
            }
            if cancellation.is_canceled() || self.resources.cancellation().is_canceled() {
                return Err(UploadError::new(UploadErrorKind::TransferCanceled));
            }
            Ok(QuarantineBytes::from(output))
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

    fn read<'a>(
        &'a self,
        request: ReadUpload<'a>,
    ) -> UploadFuture<'a, Result<QuarantineBytes, UploadError>> {
        QuarantinedFileProvider::read(self, request)
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

#[cfg(test)]
mod admission_tests {
    use std::future::Future as _;
    use std::io::{Read, Seek, Write};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::upload::RemoveDisposition;

    static ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "suprnova-live-provider-admission-{}-{}",
                std::process::id(),
                ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).expect("create provider admission test root");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct PhysicalStore {
        root: PathBuf,
    }

    impl PhysicalStore {
        fn new(root: &TestRoot) -> Self {
            Self {
                root: root.path().to_path_buf(),
            }
        }

        fn path_for(&self, object: &QuarantineObject) -> PathBuf {
            self.root.join(object.storage_key())
        }
    }

    impl QuarantineStore for PhysicalStore {
        fn create_exclusive(&self, object: &QuarantineObject) -> QuarantineOperation<()> {
            QuarantineOperation::ready(
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(self.path_for(object))
                {
                    Ok(_) => Ok(()),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        Err(UploadError::new(UploadErrorKind::StorageConflict))
                    }
                    Err(_) => Err(UploadError::new(UploadErrorKind::ProviderUnavailable)),
                },
            )
        }

        fn write_at(
            &self,
            object: &QuarantineObject,
            offset: u64,
            bytes: &[u8],
        ) -> QuarantineOperation<()> {
            QuarantineOperation::ready((|| {
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .open(self.path_for(object))
                    .map_err(|_| UploadError::new(UploadErrorKind::ProviderUnavailable))?;
                file.seek(std::io::SeekFrom::Start(offset))
                    .and_then(|_| file.write_all(bytes))
                    .map_err(|_| UploadError::new(UploadErrorKind::ProviderUnavailable))
            })())
        }

        fn sync(&self, object: &QuarantineObject) -> QuarantineOperation<()> {
            QuarantineOperation::ready(
                std::fs::OpenOptions::new()
                    .write(true)
                    .open(self.path_for(object))
                    .and_then(|file| file.sync_data())
                    .map_err(|_| UploadError::new(UploadErrorKind::ProviderUnavailable)),
            )
        }

        fn read_at(
            &self,
            object: &QuarantineObject,
            offset: u64,
            maximum_bytes: usize,
        ) -> QuarantineOperation<QuarantineBytes> {
            QuarantineOperation::ready((|| {
                let mut file = std::fs::File::open(self.path_for(object))
                    .map_err(|_| UploadError::new(UploadErrorKind::ProviderUnavailable))?;
                file.seek(std::io::SeekFrom::Start(offset))
                    .map_err(|_| UploadError::new(UploadErrorKind::ProviderUnavailable))?;
                let mut bytes = vec![0; maximum_bytes];
                let read = file
                    .read(&mut bytes)
                    .map_err(|_| UploadError::new(UploadErrorKind::ProviderUnavailable))?;
                bytes.truncate(read);
                Ok(QuarantineBytes::from(bytes))
            })())
        }

        fn remove(&self, object: &QuarantineObject) -> QuarantineOperation<RemoveDisposition> {
            QuarantineOperation::ready(match std::fs::remove_file(self.path_for(object)) {
                Ok(()) => Ok(RemoveDisposition::Removed),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    Ok(RemoveDisposition::AlreadyAbsent)
                }
                Err(_) => Err(UploadError::new(UploadErrorKind::ProviderUnavailable)),
            })
        }
    }

    async fn physical_files(root: &TestRoot) -> usize {
        let mut entries = tokio::fs::read_dir(root.path())
            .await
            .expect("read provider admission test root");
        let mut count = 0;
        while entries
            .next_entry()
            .await
            .expect("read provider admission test entry")
            .is_some()
        {
            count += 1;
        }
        count
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn retirement_drains_a_recovery_admitted_before_the_barrier_closed() {
        let root = TestRoot::new();
        let store = Arc::new(PhysicalStore::new(&root));
        let limits = UploadLimits::new(crate::limits::UploadLimitConfig::reference())
            .expect("reference upload limits");
        let first = QuarantinedFileProvider::new(store.clone(), limits).expect("first provider");
        let handle = UploadHandle::parse("018f47c1-2af0-7cc4-a001-000000000001")
            .expect("fixture upload handle");
        first
            .prepare(PrepareTransfer::new(
                &handle,
                4,
                "recovery-race.bin",
                UnixMillis::new(1_000),
            ))
            .await
            .expect("prepare physical recovery object");
        let checkpoint = first.checkpoint(&handle).expect("quiescent checkpoint");
        assert_eq!(physical_files(&root).await, 1);

        let recovered = Arc::new(
            QuarantinedFileProvider::new_with_retirement_wait_steps(store, limits, 1)
                .expect("recovered provider"),
        );
        let admission = recovered
            .admission
            .enter()
            .expect("recovery admission remains open");
        assert_eq!(recovered.admission.active(), 1);

        let retirement_started = recovered.retire();
        assert!(retirement_started.canceled);
        let mut retirement = Box::pin(recovered.retire_and_cleanup());
        let first_poll = poll_fn(|context| Poll::Ready(retirement.as_mut().poll(context))).await;
        assert!(
            first_poll.is_pending(),
            "retirement waits for admitted recovery"
        );

        recovered
            .recover_with_admission(checkpoint.clone(), admission)
            .expect("pre-retirement recovery remains admitted");
        let retirement_result = tokio::time::timeout(std::time::Duration::from_secs(1), retirement)
            .await
            .expect("retirement reaches a bounded outcome");
        retirement_result.expect("retirement cleanup succeeds");
        assert_eq!(recovered.admission.active(), 0);
        assert_eq!(recovered.descriptor_permits().active(), 0);
        assert_eq!(recovered.chunk_permits().active(), 0);
        assert_eq!(physical_files(&root).await, 0);

        let error = recovered
            .recover(checkpoint)
            .expect_err("closed recovery admission cannot revive the object");
        assert_eq!(error.kind(), UploadErrorKind::ServiceRetired);
    }
}
