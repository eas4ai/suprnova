//! One committed dependency notification per request or queue attempt.

use std::collections::BTreeSet;
use std::future::Future;
use std::sync::{Arc, Mutex};

use tokio::task::JoinHandle;

#[derive(Default)]
struct State {
    closed: bool,
    transactions: usize,
    digests: BTreeSet<[u8; 32]>,
}

#[derive(Default)]
struct Batch(Mutex<State>);

impl Batch {
    fn settle(&self, closing: bool, transaction_finished: bool) -> Vec<[u8; 32]> {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.closed |= closing;
        if transaction_finished {
            state.transactions = state.transactions.saturating_sub(1);
        }
        if state.closed && state.transactions == 0 {
            std::mem::take(&mut state.digests).into_iter().collect()
        } else {
            Vec::new()
        }
    }
}

tokio::task_local! {
    static CURRENT: Arc<Batch>;
}

/// Captured when a transaction is created, then held by its commit callbacks.
/// The scope cannot flush before those callbacks establish its actual outcome.
pub(crate) struct Lease(Arc<Batch>);

impl Lease {
    pub(crate) fn committed(&self, digests: Vec<[u8; 32]>) {
        self.0
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .digests
            .extend(digests);
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        dispatch(self.0.settle(false, true));
    }
}

/// Capture the surrounding request/job without inheriting task-local state
/// into transaction finalizer tasks.
pub(crate) fn capture() -> Option<Arc<Lease>> {
    CURRENT
        .try_with(|batch| {
            batch
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .transactions += 1;
            Arc::new(Lease(batch.clone()))
        })
        .ok()
}

/// A queue attempt re-enters this scope for every lifecycle callback.
pub(crate) struct Scope {
    batch: Arc<Batch>,
    closed: bool,
}

impl Scope {
    pub(crate) fn new() -> Self {
        Self {
            batch: Arc::new(Batch::default()),
            closed: false,
        }
    }

    pub(crate) async fn run<F: Future>(&self, future: F) -> F::Output {
        CURRENT.scope(self.batch.clone(), future).await
    }

    fn close(&mut self) -> Option<JoinHandle<()>> {
        self.closed = true;
        dispatch(self.batch.settle(true, false))
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        if !self.closed {
            self.close();
        }
    }
}

/// Coalesce independent committed transactions across a complete request.
/// Cancellation closes the scope too; outstanding finalizers retain leases
/// and deliver their committed tags when the last outcome becomes known.
pub(crate) async fn with_batch<F: Future>(future: F) -> F::Output {
    if CURRENT.try_with(|_| ()).is_ok() {
        return future.await;
    }
    let mut scope = Scope::new();
    let output = scope.run(future).await;
    if let Some(flush) = scope.close()
        && let Err(error) = flush.await
    {
        tracing::error!(error = %error, "static publication dependency batch task failed");
    }
    output
}

fn dispatch(digests: Vec<[u8; 32]>) -> Option<JoinHandle<()>> {
    if digests.is_empty() {
        return None;
    }
    if tokio::runtime::Handle::try_current().is_err() {
        tracing::error!(
            "static publication dependency batch could not flush without an async runtime"
        );
        return None;
    }
    Some(crate::database::after_commit::spawn_owned(async move {
        if let Err(error) = super::committed_generations(digests).await {
            tracing::error!(error = %error, "static publication committed dependency batch failed");
        }
    }))
}
