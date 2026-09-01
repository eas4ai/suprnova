//! Host-neutral transaction ownership held through response construction.

use std::error::Error;
use std::fmt;
use std::future::poll_fn;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::task::Poll;

use crate::component::LiveFuture;

/// Closed host-service failure category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostErrorKind {
    /// A required host transaction could not begin.
    Begin,
    /// Durable host work could not be committed conclusively.
    Commit,
    /// Best-effort rollback failed while the outcome remained rejected.
    Rollback,
    /// Post-acceptance reporting failed without changing acceptance.
    Reporting,
    /// Pre-commit host response-intent preparation failed.
    ResponseIntent,
}

/// Redacted failure from a host-neutral execution port.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct HostError {
    kind: HostErrorKind,
}

impl HostError {
    /// Creates one closed host failure.
    #[must_use]
    pub const fn new(kind: HostErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the stable failure category.
    #[must_use]
    pub const fn kind(self) -> HostErrorKind {
        self.kind
    }
}

impl fmt::Display for HostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            HostErrorKind::Begin => "live_host_transaction_begin_failure",
            HostErrorKind::Commit => "live_host_transaction_commit_failure",
            HostErrorKind::Rollback => "live_host_transaction_rollback_failure",
            HostErrorKind::Reporting => "live_post_acceptance_reporting_failure",
            HostErrorKind::ResponseIntent => "live_response_intent_preparation_failure",
        })
    }
}

impl fmt::Debug for HostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for HostError {}

/// One host transaction whose ownership proves it has not yet completed.
pub trait HostTransaction: Send {
    /// Attempts to commit durable host effects exactly once.
    ///
    /// Once this future is polled, cancellation or an error does not prove that the effects were
    /// rolled back. The execution coordinator therefore terminally fences the claimed base
    /// revision before polling it. The generic [`HostError`] cannot reopen that revision for
    /// retry.
    fn commit(self: Box<Self>) -> LiveFuture<'static, Result<(), HostError>>;

    /// Rolls back the uncommitted attempt exactly once.
    fn rollback(self: Box<Self>) -> LiveFuture<'static, Result<(), HostError>>;
}

/// Begins explicitly requested host transactions.
pub trait TransactionPort: Send + Sync {
    /// Begins a transaction owned by the caller until commit or rollback.
    fn begin(&self) -> LiveFuture<'_, Result<Box<dyn HostTransaction>, HostError>>;
}

pub(crate) async fn run_host_future<'a, T: Send + 'a>(
    operation: impl FnOnce() -> LiveFuture<'a, Result<T, HostError>>,
    panic_kind: HostErrorKind,
) -> Result<T, HostError> {
    let mut future =
        catch_unwind(AssertUnwindSafe(operation)).map_err(|_| HostError::new(panic_kind))?;
    let result =
        poll_fn(
            |context| match catch_unwind(AssertUnwindSafe(|| future.as_mut().poll(context))) {
                Ok(Poll::Ready(result)) => Poll::Ready(result),
                Ok(Poll::Pending) => Poll::Pending,
                Err(_) => Poll::Ready(Err(HostError::new(panic_kind))),
            },
        )
        .await;
    if catch_unwind(AssertUnwindSafe(|| drop(future))).is_err() {
        return Err(HostError::new(panic_kind));
    }
    result
}
