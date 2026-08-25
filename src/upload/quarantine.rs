//! Opaque quarantine objects and executor-neutral bounded byte I/O.

use std::fmt;
use std::fmt::Write as _;

use bytes::Bytes;

use super::{UploadError, UploadErrorKind, UploadFuture};

const QUARANTINE_RANDOM_BYTES: usize = 32;
const QUARANTINE_KEY_BYTES: usize = QUARANTINE_RANDOM_BYTES * 2;

/// Shared immutable byte segment exchanged with a quarantine store.
pub type QuarantineBytes = Bytes;

/// Server-random storage identity with a fixed path-segment-safe representation.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct QuarantineObject(String);

impl QuarantineObject {
    /// Generates 256 bits of server randomness and encodes lowercase hexadecimal.
    pub fn generate() -> Result<Self, UploadError> {
        let mut bytes = [0_u8; QUARANTINE_RANDOM_BYTES];
        getrandom::fill(&mut bytes)
            .map_err(|_| UploadError::new(UploadErrorKind::RandomUnavailable))?;
        let mut key = String::with_capacity(QUARANTINE_KEY_BYTES);
        for byte in bytes {
            write!(&mut key, "{byte:02x}")
                .map_err(|_| UploadError::new(UploadErrorKind::RandomUnavailable))?;
        }
        Ok(Self(key))
    }

    /// Parses a persisted canonical storage key during bounded process recovery.
    pub fn parse_storage_key(value: &str) -> Result<Self, UploadError> {
        let valid = value.len() == QUARANTINE_KEY_BYTES
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'));
        if valid {
            Ok(Self(value.to_owned()))
        } else {
            Err(UploadError::new(UploadErrorKind::InvalidField))
        }
    }

    /// Returns the fixed safe storage key for a trusted host adapter.
    #[must_use]
    pub fn storage_key(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for QuarantineObject {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<QuarantineObject:redacted>")
    }
}

/// Idempotent result of removing one opaque quarantine object.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoveDisposition {
    /// Existing quarantined bytes were removed.
    Removed,
    /// The object was already absent.
    AlreadyAbsent,
}

/// Host-owned asynchronous raw quarantine I/O.
///
/// Every byte count is caller-bounded. Implementations must write the complete
/// supplied slice or return an error and must never derive a path from browser
/// metadata.
pub trait QuarantineStore: Send + Sync {
    /// Atomically creates one absent opaque object.
    fn create_exclusive<'a>(
        &'a self,
        object: &'a QuarantineObject,
    ) -> UploadFuture<'a, Result<(), UploadError>>;

    /// Writes the complete supplied slice at one trusted bounded offset.
    fn write_at<'a>(
        &'a self,
        object: &'a QuarantineObject,
        offset: u64,
        bytes: &'a [u8],
    ) -> UploadFuture<'a, Result<(), UploadError>>;

    /// Synchronizes accepted bytes before readiness can be published.
    fn sync<'a>(
        &'a self,
        object: &'a QuarantineObject,
    ) -> UploadFuture<'a, Result<(), UploadError>>;

    /// Reads at most `maximum_bytes` beginning at a trusted offset.
    fn read_at<'a>(
        &'a self,
        object: &'a QuarantineObject,
        offset: u64,
        maximum_bytes: usize,
    ) -> UploadFuture<'a, Result<QuarantineBytes, UploadError>>;

    /// Reads at most one bounded prefix for later authoritative inspection.
    fn read_prefix<'a>(
        &'a self,
        object: &'a QuarantineObject,
        maximum_bytes: usize,
    ) -> UploadFuture<'a, Result<QuarantineBytes, UploadError>> {
        self.read_at(object, 0, maximum_bytes)
    }

    /// Idempotently removes one opaque quarantine object.
    fn remove<'a>(
        &'a self,
        object: &'a QuarantineObject,
    ) -> UploadFuture<'a, Result<RemoveDisposition, UploadError>>;
}
