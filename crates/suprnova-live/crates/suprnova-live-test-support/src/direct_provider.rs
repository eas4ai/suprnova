//! In-memory direct-transfer adapter used only to prove provider conformance.

use std::{
    collections::{BTreeMap, HashMap},
    fmt::Write as _,
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
};

use sha2::{Digest, Sha256};
use suprnova_live::{
    identity::UnixMillis,
    limits::UploadLimits,
    upload::{
        BoundedHeaders, ChunkDisposition, ChunkReceipt, DirectPartReference,
        DirectTransferInstruction, DirectUploadProvider, IntegrityEvidence, PrepareTransfer,
        QuarantineBytes, ReadUpload, ReportDirectPart, TransferDisposition, TransferMethod,
        TransferPlan, TrustedProviderOrigin, TrustedProviderUrl, UploadChecksum, UploadError,
        UploadErrorKind, UploadFuture, UploadHandle, UploadPart, UploadProvider, VerifyTransfer,
    },
};

const DIRECT_INSTRUCTION_LIFETIME_MS: u64 = 15 * 60 * 1_000;

/// In-memory reference adapter proving direct-storage lifecycle conformance.
///
/// It deliberately models no vendor API and is not a production storage adapter.
pub struct DirectProviderConformanceAdapter {
    limits: UploadLimits,
    origin: TrustedProviderOrigin,
    next_identity: AtomicU64,
    state: Mutex<DirectState>,
}

impl DirectProviderConformanceAdapter {
    /// Creates a bounded adapter for one preconfigured provider origin.
    pub fn new(limits: UploadLimits, origin: TrustedProviderOrigin) -> Result<Self, UploadError> {
        if limits.max_chunk_bytes() == 0 || limits.max_chunks_per_file() == 0 {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        }
        Ok(Self {
            limits,
            origin,
            next_identity: AtomicU64::new(1),
            state: Mutex::new(DirectState::default()),
        })
    }

    /// Emulates the external provider accepting bytes for one exact instruction.
    ///
    /// Tests call this before importing the provider outcome through
    /// [`DirectUploadProvider::report_part`]. The browser cannot supply trusted bytes
    /// or integrity evidence through that report.
    pub fn store_part_for_test(
        &self,
        instruction: &DirectTransferInstruction,
        bytes: &[u8],
        now: UnixMillis,
    ) -> Result<ChunkDisposition, UploadError> {
        if !instruction.is_current(now) || bytes.len() != instruction.maximum_bytes() {
            return Err(UploadError::new(if instruction.is_current(now) {
                UploadErrorKind::InputTooLarge
            } else {
                UploadErrorKind::UploadExpired
            }));
        }
        let mut state = lock(&self.state);
        let binding = state
            .bindings
            .get(instruction.reference())
            .cloned()
            .ok_or_else(|| UploadError::new(UploadErrorKind::UploadConflict))?;
        let entry = state
            .entries
            .get_mut(&binding.handle)
            .ok_or_else(|| UploadError::new(UploadErrorKind::UploadConflict))?;
        let stored = entry
            .parts
            .get_mut(&binding.index)
            .ok_or_else(|| UploadError::new(UploadErrorKind::UploadConflict))?;
        if stored.instruction != *instruction {
            return Err(UploadError::new(UploadErrorKind::ScopeMismatch));
        }
        match &stored.bytes {
            Some(existing) if existing == bytes => Ok(ChunkDisposition::ExistingOutcome),
            Some(_) => Err(UploadError::new(UploadErrorKind::UploadConflict)),
            None => {
                stored.bytes = Some(bytes.to_vec());
                Ok(ChunkDisposition::Stored)
            }
        }
    }

    fn prepare_inner(&self, request: PrepareTransfer<'_>) -> Result<TransferPlan, UploadError> {
        if request.expected_bytes() > self.limits.max_file_bytes()
            || request.client_name().len() > 1_024
        {
            return Err(UploadError::new(UploadErrorKind::InputTooLarge));
        }
        let required_parts = request
            .expected_bytes()
            .div_ceil(self.limits.max_chunk_bytes() as u64);
        if required_parts > self.limits.max_chunks_per_file() as u64 {
            return Err(UploadError::new(UploadErrorKind::InputTooLarge));
        }
        let mut state = lock(&self.state);
        if state.entries.contains_key(request.handle()) {
            return self.replay_plan(&mut state, request);
        }
        if state.entries.len() >= self.limits.max_pending_per_scope() {
            return Err(UploadError::new(UploadErrorKind::PendingLimitExceeded));
        }
        let reserved_bytes = state
            .reserved_bytes
            .checked_add(request.expected_bytes())
            .ok_or_else(|| UploadError::new(UploadErrorKind::InputTooLarge))?;
        if reserved_bytes
            > self
                .limits
                .max_aggregate_bytes()
                .min(self.limits.max_storage_bytes())
        {
            return Err(UploadError::new(UploadErrorKind::PendingLimitExceeded));
        }
        let upload_expires_at = request
            .created_at()
            .get()
            .checked_add(self.limits.max_age_ms())
            .map(UnixMillis::new)
            .ok_or_else(|| UploadError::new(UploadErrorKind::InvalidField))?;
        let object_identity = self.next_identity()?;
        let initial = if request.expected_bytes() == 0 {
            None
        } else {
            Some(self.issue_instruction(
                object_identity,
                0,
                0,
                request.expected_bytes(),
                request.created_at(),
                upload_expires_at,
            )?)
        };
        let mut entry = DirectEntry {
            expected_bytes: request.expected_bytes(),
            expires_at: upload_expires_at,
            object_identity,
            parts: BTreeMap::new(),
        };
        if let Some(instruction) = &initial {
            entry.parts.insert(
                0,
                DirectStoredPart {
                    instruction: instruction.clone(),
                    bytes: None,
                    reported: false,
                },
            );
            state.bindings.insert(
                instruction.reference().clone(),
                DirectBinding {
                    handle: request.handle().clone(),
                    index: 0,
                },
            );
        }
        state.entries.insert(request.handle().clone(), entry);
        state.reserved_bytes = reserved_bytes;
        TransferPlan::direct(
            request.handle().clone(),
            self.limits.max_chunk_bytes(),
            TransferDisposition::Prepared,
            initial.into_iter().collect(),
            1,
        )
    }

    fn replay_plan(
        &self,
        state: &mut DirectState,
        request: PrepareTransfer<'_>,
    ) -> Result<TransferPlan, UploadError> {
        let (instruction, replaced_reference) = {
            let entry = state
                .entries
                .get_mut(request.handle())
                .ok_or_else(|| UploadError::new(UploadErrorKind::UploadConflict))?;
            if entry.expected_bytes != request.expected_bytes() {
                return Err(UploadError::new(UploadErrorKind::UploadConflict));
            }
            if request.created_at() >= entry.expires_at {
                return Err(UploadError::new(UploadErrorKind::UploadExpired));
            }
            let Some(part) = entry.parts.values_mut().find(|part| !part.reported) else {
                return TransferPlan::direct(
                    request.handle().clone(),
                    self.limits.max_chunk_bytes(),
                    TransferDisposition::ExistingOutcome,
                    Vec::new(),
                    1,
                );
            };
            if part.instruction.is_current(request.created_at()) {
                (part.instruction.clone(), None)
            } else {
                let previous = part.instruction.reference().clone();
                let replacement = self.issue_instruction(
                    entry.object_identity,
                    part.instruction.part().index(),
                    part.instruction.part().offset(),
                    entry.expected_bytes,
                    request.created_at(),
                    entry.expires_at,
                )?;
                part.instruction = replacement.clone();
                (replacement, Some(previous))
            }
        };
        if let Some(previous) = replaced_reference {
            state.bindings.remove(&previous);
            state.bindings.insert(
                instruction.reference().clone(),
                DirectBinding {
                    handle: request.handle().clone(),
                    index: instruction.part().index(),
                },
            );
        }
        TransferPlan::direct(
            request.handle().clone(),
            self.limits.max_chunk_bytes(),
            TransferDisposition::ExistingOutcome,
            vec![instruction],
            1,
        )
    }

    fn report_inner(&self, request: ReportDirectPart<'_>) -> Result<ChunkReceipt, UploadError> {
        let mut state = lock(&self.state);
        let binding = state
            .bindings
            .get(request.reference())
            .cloned()
            .ok_or_else(|| UploadError::new(UploadErrorKind::UploadConflict))?;
        if binding.handle != *request.handle() || binding.index != request.part().index() {
            return Err(UploadError::new(UploadErrorKind::ScopeMismatch));
        }

        let (disposition, next, new_binding) = {
            let entry = state
                .entries
                .get_mut(request.handle())
                .ok_or_else(|| UploadError::new(UploadErrorKind::UploadConflict))?;
            let stored = entry
                .parts
                .get_mut(&binding.index)
                .ok_or_else(|| UploadError::new(UploadErrorKind::UploadConflict))?;
            if stored.instruction.reference() != request.reference()
                || stored.instruction.part() != request.part()
            {
                return Err(UploadError::new(UploadErrorKind::ScopeMismatch));
            }
            if !stored.instruction.is_current(request.observed_at()) {
                return Err(UploadError::new(UploadErrorKind::UploadExpired));
            }
            if stored.bytes.is_none() {
                return Err(UploadError::new(UploadErrorKind::IncompleteTransfer));
            }
            let disposition = if stored.reported {
                ChunkDisposition::ExistingOutcome
            } else {
                stored.reported = true;
                ChunkDisposition::Stored
            };
            let next_index = binding
                .index
                .checked_add(1)
                .ok_or_else(|| UploadError::new(UploadErrorKind::ResourceExhausted))?;
            let next_offset = request
                .part()
                .offset()
                .checked_add(request.part().bytes())
                .ok_or_else(|| UploadError::new(UploadErrorKind::InvalidField))?;
            let existing_next = entry
                .parts
                .get(&next_index)
                .map(|part| part.instruction.clone());
            let created_next = if existing_next.is_none() && next_offset < entry.expected_bytes {
                if usize::try_from(next_index)
                    .ok()
                    .is_none_or(|index| index >= self.limits.max_chunks_per_file())
                {
                    return Err(UploadError::new(UploadErrorKind::ResourceExhausted));
                }
                let instruction = self.issue_instruction(
                    entry.object_identity,
                    next_index,
                    next_offset,
                    entry.expected_bytes,
                    request.observed_at(),
                    entry.expires_at,
                )?;
                entry.parts.insert(
                    next_index,
                    DirectStoredPart {
                        instruction: instruction.clone(),
                        bytes: None,
                        reported: false,
                    },
                );
                Some(instruction)
            } else {
                None
            };
            let next = existing_next.or(created_next);
            let binding = next.as_ref().map(|instruction| {
                (
                    instruction.reference().clone(),
                    DirectBinding {
                        handle: request.handle().clone(),
                        index: next_index,
                    },
                )
            });
            (disposition, next, binding)
        };
        if let Some((reference, binding)) = new_binding {
            state.bindings.insert(reference, binding);
        }
        Ok(ChunkReceipt::for_direct_part(
            request.part(),
            disposition,
            next,
        ))
    }

    fn verify_inner(&self, request: VerifyTransfer<'_>) -> Result<IntegrityEvidence, UploadError> {
        let state = lock(&self.state);
        let entry = state
            .entries
            .get(request.handle())
            .ok_or_else(|| UploadError::new(UploadErrorKind::UploadConflict))?;
        let mut cursor = 0_u64;
        let mut hasher = Sha256::new();
        for (expected_index, part) in entry.parts.values().enumerate() {
            let expected_index = u32::try_from(expected_index)
                .map_err(|_| UploadError::new(UploadErrorKind::ResourceExhausted))?;
            let Some(bytes) = part.bytes.as_ref().filter(|_| part.reported) else {
                return Err(UploadError::new(UploadErrorKind::IncompleteTransfer));
            };
            if part.instruction.part().index() != expected_index
                || part.instruction.part().offset() != cursor
            {
                return Err(UploadError::new(UploadErrorKind::IncompleteTransfer));
            }
            cursor = cursor
                .checked_add(bytes.len() as u64)
                .ok_or_else(|| UploadError::new(UploadErrorKind::InputTooLarge))?;
            hasher.update(bytes);
        }
        if cursor != entry.expected_bytes {
            return Err(UploadError::new(UploadErrorKind::IncompleteTransfer));
        }
        let actual = encode_checksum(hasher.finalize().as_slice())?;
        if &actual != request.checksum() {
            return Err(UploadError::new(UploadErrorKind::ChecksumMismatch));
        }
        Ok(IntegrityEvidence::from_provider(cursor, actual))
    }

    fn read_inner(&self, request: ReadUpload<'_>) -> Result<QuarantineBytes, UploadError> {
        if request.maximum_bytes() == 0 || request.maximum_bytes() > self.limits.max_chunk_bytes() {
            return Err(UploadError::new(UploadErrorKind::InputTooLarge));
        }
        let state = lock(&self.state);
        let entry = state
            .entries
            .get(request.handle())
            .ok_or_else(|| UploadError::new(UploadErrorKind::UploadConflict))?;
        if request.offset() > entry.expected_bytes {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        }
        let mut cursor = 0_u64;
        for (expected_index, part) in entry.parts.values().enumerate() {
            let expected_index = u32::try_from(expected_index)
                .map_err(|_| UploadError::new(UploadErrorKind::ResourceExhausted))?;
            let bytes = part
                .bytes
                .as_ref()
                .filter(|_| part.reported)
                .ok_or_else(|| UploadError::new(UploadErrorKind::IncompleteTransfer))?;
            if part.instruction.part().index() != expected_index
                || part.instruction.part().offset() != cursor
            {
                return Err(UploadError::new(UploadErrorKind::IncompleteTransfer));
            }
            cursor = cursor
                .checked_add(bytes.len() as u64)
                .ok_or_else(|| UploadError::new(UploadErrorKind::InputTooLarge))?;
        }
        if cursor != entry.expected_bytes {
            return Err(UploadError::new(UploadErrorKind::IncompleteTransfer));
        }

        let target = usize::try_from(
            (entry.expected_bytes - request.offset()).min(request.maximum_bytes() as u64),
        )
        .map_err(|_| UploadError::new(UploadErrorKind::InputTooLarge))?;
        let end = request
            .offset()
            .checked_add(target as u64)
            .ok_or_else(|| UploadError::new(UploadErrorKind::InvalidField))?;
        let mut output = Vec::with_capacity(target);
        for part in entry.parts.values() {
            let bytes = part
                .bytes
                .as_ref()
                .filter(|_| part.reported)
                .ok_or_else(|| UploadError::new(UploadErrorKind::IncompleteTransfer))?;
            let part_start = part.instruction.part().offset();
            let part_end = part_start
                .checked_add(bytes.len() as u64)
                .ok_or_else(|| UploadError::new(UploadErrorKind::InputTooLarge))?;
            let overlap_start = part_start.max(request.offset());
            let overlap_end = part_end.min(end);
            if overlap_start < overlap_end {
                let start = usize::try_from(overlap_start - part_start)
                    .map_err(|_| UploadError::new(UploadErrorKind::InputTooLarge))?;
                let finish = usize::try_from(overlap_end - part_start)
                    .map_err(|_| UploadError::new(UploadErrorKind::InputTooLarge))?;
                output.extend_from_slice(&bytes[start..finish]);
            }
        }
        if output.len() != target {
            return Err(UploadError::new(UploadErrorKind::IncompleteTransfer));
        }
        Ok(QuarantineBytes::from(output))
    }

    fn issue_instruction(
        &self,
        object_identity: u64,
        index: u32,
        offset: u64,
        expected_bytes: u64,
        issued_at: UnixMillis,
        upload_expires_at: UnixMillis,
    ) -> Result<DirectTransferInstruction, UploadError> {
        let remaining = expected_bytes
            .checked_sub(offset)
            .ok_or_else(|| UploadError::new(UploadErrorKind::InvalidField))?;
        let bytes = remaining.min(self.limits.max_chunk_bytes() as u64);
        let part = UploadPart::new(index, offset, bytes)?;
        let identity = self.next_identity()?;
        let reference = DirectPartReference::parse(&format!("{identity:032x}"))?;
        let endpoint = TrustedProviderUrl::parse(
            &format!(
                "{}/temporary/{object_identity}/part/{index}?credential={}",
                self.origin.as_str(),
                reference.as_str()
            ),
            &self.origin,
        )?;
        let headers = BoundedHeaders::parse(&[("x-suprnova-part", reference.as_str())])?;
        let instruction_expires_at = issued_at
            .get()
            .checked_add(DIRECT_INSTRUCTION_LIFETIME_MS)
            .map(|deadline| UnixMillis::new(deadline.min(upload_expires_at.get())))
            .ok_or_else(|| UploadError::new(UploadErrorKind::InvalidField))?;
        if instruction_expires_at <= issued_at {
            return Err(UploadError::new(UploadErrorKind::UploadExpired));
        }
        DirectTransferInstruction::new(
            TransferMethod::Put,
            endpoint,
            headers,
            part,
            reference,
            issued_at,
            instruction_expires_at,
            usize::try_from(bytes).map_err(|_| UploadError::new(UploadErrorKind::InputTooLarge))?,
        )
    }

    fn remove(&self, handle: &UploadHandle) {
        let mut state = lock(&self.state);
        let Some(entry) = state.entries.remove(handle) else {
            return;
        };
        state.reserved_bytes = state.reserved_bytes.saturating_sub(entry.expected_bytes);
        for part in entry.parts.into_values() {
            state.bindings.remove(part.instruction.reference());
        }
    }

    fn next_identity(&self) -> Result<u64, UploadError> {
        self.next_identity
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| UploadError::new(UploadErrorKind::ResourceExhausted))
    }
}

impl UploadProvider for DirectProviderConformanceAdapter {
    fn prepare<'a>(
        &'a self,
        request: PrepareTransfer<'a>,
    ) -> UploadFuture<'a, Result<TransferPlan, UploadError>> {
        Box::pin(async move { self.prepare_inner(request) })
    }

    fn verify<'a>(
        &'a self,
        request: VerifyTransfer<'a>,
    ) -> UploadFuture<'a, Result<IntegrityEvidence, UploadError>> {
        Box::pin(async move { self.verify_inner(request) })
    }

    fn read<'a>(
        &'a self,
        request: ReadUpload<'a>,
    ) -> UploadFuture<'a, Result<QuarantineBytes, UploadError>> {
        Box::pin(async move { self.read_inner(request) })
    }

    fn cancel<'a>(&'a self, handle: &'a UploadHandle) -> UploadFuture<'a, Result<(), UploadError>> {
        Box::pin(async move {
            self.remove(handle);
            Ok(())
        })
    }

    fn cleanup<'a>(
        &'a self,
        handle: &'a UploadHandle,
    ) -> UploadFuture<'a, Result<(), UploadError>> {
        Box::pin(async move {
            self.remove(handle);
            Ok(())
        })
    }
}

impl DirectUploadProvider for DirectProviderConformanceAdapter {
    fn report_part<'a>(
        &'a self,
        request: ReportDirectPart<'a>,
    ) -> UploadFuture<'a, Result<ChunkReceipt, UploadError>> {
        Box::pin(async move { self.report_inner(request) })
    }
}

#[derive(Default)]
struct DirectState {
    entries: HashMap<UploadHandle, DirectEntry>,
    bindings: HashMap<DirectPartReference, DirectBinding>,
    reserved_bytes: u64,
}

struct DirectEntry {
    expected_bytes: u64,
    expires_at: UnixMillis,
    object_identity: u64,
    parts: BTreeMap<u32, DirectStoredPart>,
}

struct DirectStoredPart {
    instruction: DirectTransferInstruction,
    bytes: Option<Vec<u8>>,
    reported: bool,
}

#[derive(Clone)]
struct DirectBinding {
    handle: UploadHandle,
    index: u32,
}

fn encode_checksum(bytes: &[u8]) -> Result<UploadChecksum, UploadError> {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
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
