//! Opaque quarantine objects and executor-neutral bounded byte I/O.

use std::fmt;
use std::fmt::Write as _;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};

use bytes::Bytes;

use super::{UploadError, UploadErrorKind};

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

struct QuarantineOperationState<T> {
    result: Option<Result<T, UploadError>>,
    waiters: Vec<Waker>,
    supervisors: Vec<Box<dyn Send + 'static>>,
}

/// Shared completion witness for one independently owned physical store operation.
///
/// Dropping a request future never drops the store's completion half. The host
/// adapter owns that half until the actual physical effect has reached a
/// terminal result, allowing the provider to fence retries and retirement.
pub struct QuarantineOperation<T> {
    state: Arc<Mutex<QuarantineOperationState<T>>>,
}

impl<T> QuarantineOperation<T> {
    /// Creates a pending operation and the single-use host completion authority.
    #[must_use]
    pub fn pending() -> (Self, QuarantineCompletion<T>) {
        let state = Arc::new(Mutex::new(QuarantineOperationState {
            result: None,
            waiters: Vec::new(),
            supervisors: Vec::new(),
        }));
        (
            Self {
                state: Arc::clone(&state),
            },
            QuarantineCompletion {
                state,
                completed: false,
            },
        )
    }

    /// Creates an already completed physical-operation witness.
    #[must_use]
    pub fn ready(result: Result<T, UploadError>) -> Self {
        Self {
            state: Arc::new(Mutex::new(QuarantineOperationState {
                result: Some(result),
                waiters: Vec::new(),
                supervisors: Vec::new(),
            })),
        }
    }

    pub(crate) fn supervise(&self, supervisor: impl Send + 'static) {
        let mut state = lock(&self.state);
        if state.result.is_none() {
            state.supervisors.push(Box::new(supervisor));
        }
    }
}

impl<T: Clone> Future for QuarantineOperation<T> {
    type Output = Result<T, UploadError>;

    fn poll(self: Pin<&mut Self>, task: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = lock(&self.state);
        if let Some(result) = &state.result {
            return Poll::Ready(result.clone());
        }
        if !state
            .waiters
            .iter()
            .any(|registered| registered.will_wake(task.waker()))
        {
            state.waiters.push(task.waker().clone());
        }
        Poll::Pending
    }
}

/// Single-use authority that publishes a physical store operation's result.
pub struct QuarantineCompletion<T> {
    state: Arc<Mutex<QuarantineOperationState<T>>>,
    completed: bool,
}

impl<T> QuarantineCompletion<T> {
    /// Completes the operation and releases all provider fences exactly once.
    pub fn complete(mut self, result: Result<T, UploadError>) {
        self.publish(result);
        self.completed = true;
    }

    fn publish(&self, result: Result<T, UploadError>) {
        let (waiters, supervisors) = {
            let mut state = lock(&self.state);
            if state.result.is_some() {
                return;
            }
            state.result = Some(result);
            (
                std::mem::take(&mut state.waiters),
                std::mem::take(&mut state.supervisors),
            )
        };
        drop(supervisors);
        for waiter in waiters {
            waiter.wake();
        }
    }
}

impl<T> Drop for QuarantineCompletion<T> {
    fn drop(&mut self) {
        if !self.completed {
            self.publish(Err(UploadError::new(UploadErrorKind::ProviderUnavailable)));
        }
    }
}

/// Host-owned asynchronous raw quarantine I/O.
///
/// Every byte count is caller-bounded. Implementations must write the complete
/// supplied slice or return an error and must never derive a path from browser
/// metadata.
pub trait QuarantineStore: Send + Sync {
    /// Atomically creates one absent opaque object.
    fn create_exclusive(&self, object: &QuarantineObject) -> QuarantineOperation<()>;

    /// Writes the complete supplied slice at one trusted bounded offset.
    fn write_at(
        &self,
        object: &QuarantineObject,
        offset: u64,
        bytes: &[u8],
    ) -> QuarantineOperation<()>;

    /// Synchronizes accepted bytes before readiness can be published.
    fn sync(&self, object: &QuarantineObject) -> QuarantineOperation<()>;

    /// Reads at most `maximum_bytes` beginning at a trusted offset.
    fn read_at(
        &self,
        object: &QuarantineObject,
        offset: u64,
        maximum_bytes: usize,
    ) -> QuarantineOperation<QuarantineBytes>;

    /// Reads at most one bounded prefix for later authoritative inspection.
    fn read_prefix(
        &self,
        object: &QuarantineObject,
        maximum_bytes: usize,
    ) -> QuarantineOperation<QuarantineBytes> {
        self.read_at(object, 0, maximum_bytes)
    }

    /// Idempotently removes one opaque quarantine object.
    fn remove(&self, object: &QuarantineObject) -> QuarantineOperation<RemoveDisposition>;
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
