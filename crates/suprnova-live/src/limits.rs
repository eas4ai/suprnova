//! Resource limits used before external input can amplify work.

use std::error::Error;
use std::fmt;

/// Hard ceiling for one iteration 001 control or snapshot input.
pub const HARD_MAX_INPUT_BYTES: usize = 16 * 1024 * 1024;
/// Hard ceiling for JSON container nesting.
pub const HARD_MAX_DEPTH: usize = 64;
/// Hard ceiling for total array elements plus object members.
pub const HARD_MAX_ENTRIES: usize = 100_000;
/// Hard ceiling for one decoded JSON string.
pub const HARD_MAX_STRING_BYTES: usize = 1024 * 1024;

/// Validated byte, depth, collection, and string limits for an input boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InputLimits {
    max_bytes: usize,
    max_depth: usize,
    max_entries: usize,
    max_string_bytes: usize,
}

impl InputLimits {
    /// Creates limits that are non-zero and below the engine hard ceilings.
    pub fn new(
        max_bytes: usize,
        max_depth: usize,
        max_entries: usize,
        max_string_bytes: usize,
    ) -> Result<Self, LimitConfigurationError> {
        let within_ceiling = max_bytes <= HARD_MAX_INPUT_BYTES
            && max_depth <= HARD_MAX_DEPTH
            && max_entries <= HARD_MAX_ENTRIES
            && max_string_bytes <= HARD_MAX_STRING_BYTES;
        let non_zero = max_bytes > 0 && max_depth > 0 && max_entries > 0 && max_string_bytes > 0;

        if !within_ceiling || !non_zero {
            return Err(LimitConfigurationError);
        }

        Ok(Self {
            max_bytes,
            max_depth,
            max_entries,
            max_string_bytes,
        })
    }

    /// Returns the locked upload protocol-v1 input limits.
    #[must_use]
    pub const fn upload_protocol_v1() -> Self {
        Self {
            max_bytes: 16_384,
            max_depth: 8,
            max_entries: 64,
            max_string_bytes: 4_096,
        }
    }

    /// Maximum encoded input bytes accepted before parsing.
    #[must_use]
    pub const fn max_bytes(self) -> usize {
        self.max_bytes
    }

    /// Maximum nested array/object container count.
    #[must_use]
    pub const fn max_depth(self) -> usize {
        self.max_depth
    }

    /// Maximum total array elements plus object members.
    #[must_use]
    pub const fn max_entries(self) -> usize {
        self.max_entries
    }

    /// Maximum decoded UTF-8 bytes in one string or object key.
    #[must_use]
    pub const fn max_string_bytes(self) -> usize {
        self.max_string_bytes
    }
}

impl Default for InputLimits {
    fn default() -> Self {
        Self {
            max_bytes: 64 * 1024,
            max_depth: 32,
            max_entries: 2_048,
            max_string_bytes: 16 * 1024,
        }
    }
}

/// A configured input limit was zero or exceeded an engine hard ceiling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LimitConfigurationError;

impl fmt::Display for LimitConfigurationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid_limit_configuration")
    }
}

impl Error for LimitConfigurationError {}

/// Raw configurable upload bounds validated as one coherent profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UploadLimitConfig {
    /// Maximum selected files bound to one declared upload field.
    pub max_files_per_field: usize,
    /// Maximum temporary uploads retained for one normalized host scope.
    pub max_pending_per_scope: usize,
    /// Maximum authoritative bytes for one file.
    pub max_file_bytes: u64,
    /// Maximum aggregate pending bytes for one normalized host scope.
    pub max_aggregate_bytes: u64,
    /// Maximum bytes accepted by one chunk request.
    pub max_chunk_bytes: usize,
    /// Maximum accepted chunk records retained for one file.
    pub max_chunks_per_file: usize,
    /// Maximum aggregate chunk bytes admitted to process memory.
    pub max_in_flight_bytes: usize,
    /// Maximum simultaneously active transfers for one resource owner.
    pub max_concurrent_transfers: usize,
    /// Maximum creations admitted during one rate window.
    pub max_creations_per_window: usize,
    /// Creation-rate window duration in milliseconds.
    pub creation_window_ms: u64,
    /// Maximum retries admitted for one operation.
    pub max_retries: u32,
    /// Maximum temporary-upload lifetime in milliseconds.
    pub max_age_ms: u64,
    /// Maximum authoritative validation duration in milliseconds.
    pub max_validation_ms: u64,
    /// Maximum scanner duration in milliseconds.
    pub max_scan_ms: u64,
    /// Maximum temporary storage bytes for one configured provider scope.
    pub max_storage_bytes: u64,
    /// Maximum records processed by one cleanup batch.
    pub max_cleanup_batch: usize,
    /// Maximum retained retry outcomes for one upload record.
    pub max_idempotency_outcomes: usize,
}

impl UploadLimitConfig {
    /// Returns the daemon-free reference profile used by conformance tests.
    #[must_use]
    pub const fn reference() -> Self {
        Self {
            max_files_per_field: 16,
            max_pending_per_scope: 128,
            max_file_bytes: 64 * 1024 * 1024,
            max_aggregate_bytes: 256 * 1024 * 1024,
            max_chunk_bytes: 256 * 1024,
            max_chunks_per_file: 4_096,
            max_in_flight_bytes: 8 * 1024 * 1024,
            max_concurrent_transfers: 8,
            max_creations_per_window: 64,
            creation_window_ms: 60_000,
            max_retries: 16,
            max_age_ms: 24 * 60 * 60 * 1_000,
            max_validation_ms: 30_000,
            max_scan_ms: 120_000,
            max_storage_bytes: 1024 * 1024 * 1024,
            max_cleanup_batch: 256,
            max_idempotency_outcomes: 4_102,
        }
    }
}

impl Default for UploadLimitConfig {
    fn default() -> Self {
        Self::reference()
    }
}

/// Validated finite limits for upload admission, work, retention, and cleanup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UploadLimits(UploadLimitConfig);

impl UploadLimits {
    /// Validates a non-zero internally coherent profile under engine ceilings.
    pub fn new(config: UploadLimitConfig) -> Result<Self, LimitConfigurationError> {
        const TIB: u64 = 1024 * 1024 * 1024 * 1024;
        const DAY_MS: u64 = 24 * 60 * 60 * 1_000;

        let non_zero = config.max_files_per_field > 0
            && config.max_pending_per_scope > 0
            && config.max_file_bytes > 0
            && config.max_aggregate_bytes > 0
            && config.max_chunk_bytes > 0
            && config.max_chunks_per_file > 0
            && config.max_in_flight_bytes > 0
            && config.max_concurrent_transfers > 0
            && config.max_creations_per_window > 0
            && config.creation_window_ms > 0
            && config.max_retries > 0
            && config.max_age_ms > 0
            && config.max_validation_ms > 0
            && config.max_scan_ms > 0
            && config.max_storage_bytes > 0
            && config.max_cleanup_batch > 0
            && config.max_idempotency_outcomes > 0;
        let coherent = config.max_pending_per_scope >= config.max_files_per_field
            && config.max_file_bytes >= config.max_chunk_bytes as u64
            && config.max_aggregate_bytes >= config.max_file_bytes
            && config.max_in_flight_bytes >= config.max_chunk_bytes
            && config.max_storage_bytes >= config.max_aggregate_bytes
            && config.max_idempotency_outcomes >= config.max_chunks_per_file.saturating_add(6);
        let finite = config.max_files_per_field <= 1_024
            && config.max_pending_per_scope <= 100_000
            && config.max_file_bytes <= TIB
            && config.max_aggregate_bytes <= 16 * TIB
            && config.max_chunk_bytes <= 64 * 1024 * 1024
            && config.max_chunks_per_file <= 1_000_000
            && config.max_in_flight_bytes <= 1024 * 1024 * 1024
            && config.max_concurrent_transfers <= 1_024
            && config.max_creations_per_window <= 1_000_000
            && config.creation_window_ms <= DAY_MS
            && config.max_retries <= 10_000
            && config.max_age_ms <= 30 * DAY_MS
            && config.max_validation_ms <= DAY_MS
            && config.max_scan_ms <= DAY_MS
            && config.max_storage_bytes <= 64 * TIB
            && config.max_cleanup_batch <= 100_000
            && config.max_idempotency_outcomes <= 100_000;
        if !non_zero || !coherent || !finite {
            return Err(LimitConfigurationError);
        }
        Ok(Self(config))
    }

    /// Returns the per-field file count bound.
    #[must_use]
    pub const fn max_files_per_field(self) -> usize {
        self.0.max_files_per_field
    }

    /// Returns the per-scope pending upload count bound.
    #[must_use]
    pub const fn max_pending_per_scope(self) -> usize {
        self.0.max_pending_per_scope
    }

    /// Returns the per-file byte bound.
    #[must_use]
    pub const fn max_file_bytes(self) -> u64 {
        self.0.max_file_bytes
    }

    /// Returns the aggregate pending byte bound.
    #[must_use]
    pub const fn max_aggregate_bytes(self) -> u64 {
        self.0.max_aggregate_bytes
    }

    /// Returns the per-request chunk byte bound.
    #[must_use]
    pub const fn max_chunk_bytes(self) -> usize {
        self.0.max_chunk_bytes
    }

    /// Returns the accepted chunk-record bound for one file.
    #[must_use]
    pub const fn max_chunks_per_file(self) -> usize {
        self.0.max_chunks_per_file
    }

    /// Returns the admitted in-flight byte bound.
    #[must_use]
    pub const fn max_in_flight_bytes(self) -> usize {
        self.0.max_in_flight_bytes
    }

    /// Returns the active transfer concurrency bound.
    #[must_use]
    pub const fn max_concurrent_transfers(self) -> usize {
        self.0.max_concurrent_transfers
    }

    /// Returns the creation count allowed in one rate window.
    #[must_use]
    pub const fn max_creations_per_window(self) -> usize {
        self.0.max_creations_per_window
    }

    /// Returns the creation-rate window in milliseconds.
    #[must_use]
    pub const fn creation_window_ms(self) -> u64 {
        self.0.creation_window_ms
    }

    /// Returns the per-operation retry bound.
    #[must_use]
    pub const fn max_retries(self) -> u32 {
        self.0.max_retries
    }

    /// Returns the temporary-upload lifetime bound in milliseconds.
    #[must_use]
    pub const fn max_age_ms(self) -> u64 {
        self.0.max_age_ms
    }

    /// Returns the validation-time bound in milliseconds.
    #[must_use]
    pub const fn max_validation_ms(self) -> u64 {
        self.0.max_validation_ms
    }

    /// Returns the scanner-time bound in milliseconds.
    #[must_use]
    pub const fn max_scan_ms(self) -> u64 {
        self.0.max_scan_ms
    }

    /// Returns the configured temporary-storage byte bound.
    #[must_use]
    pub const fn max_storage_bytes(self) -> u64 {
        self.0.max_storage_bytes
    }

    /// Returns the cleanup batch count bound.
    #[must_use]
    pub const fn max_cleanup_batch(self) -> usize {
        self.0.max_cleanup_batch
    }

    /// Returns the retained idempotency outcome bound.
    #[must_use]
    pub const fn max_idempotency_outcomes(self) -> usize {
        self.0.max_idempotency_outcomes
    }
}
