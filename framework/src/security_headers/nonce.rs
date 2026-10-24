//! A nonce belongs to one request, including its explicitly propagated child tasks.

use std::future::Future;
use std::sync::{Arc, RwLock};

#[derive(Clone, Default)]
pub(crate) struct NonceState(pub(crate) Arc<RwLock<Option<String>>>);

tokio::task_local! {
    static NONCE: NonceState;
}

/// Reads the request's nonce without requiring ownership of its HTTP request.
///
/// Tokio tasks need explicit propagation, just as request context does. Wrap a
/// spawned future in [`Self::propagate`] while still inside the request scope.
pub struct CspNonce;

impl CspNonce {
    pub(crate) fn state() -> Option<NonceState> {
        NONCE.try_with(Clone::clone).ok()
    }

    /// Views and asset writers share this value so their tags match the policy.
    pub fn current() -> Option<String> {
        NONCE.try_with(NonceState::get).ok().flatten()
    }

    /// Captures the current scope before a future is spawned into a child task.
    ///
    /// Call this before `tokio::spawn`; the returned future retains the request's
    /// live nonce state and never borrows the caller's task-local storage.
    pub fn propagate<F: Future>(future: F) -> impl Future<Output = F::Output> {
        let state = NONCE.try_with(Clone::clone).unwrap_or_default();
        NONCE.scope(state, future)
    }

    #[cfg(test)]
    pub(crate) async fn scope<F: Future>(nonce: Option<String>, future: F) -> F::Output {
        Self::scope_state(NonceState(Arc::new(RwLock::new(nonce))), future).await
    }

    pub(crate) async fn scope_state<F: Future>(state: NonceState, future: F) -> F::Output {
        NONCE.scope(state, future).await
    }
}

impl NonceState {
    pub(crate) fn get(&self) -> Option<String> {
        self.0.read().unwrap_or_else(|p| p.into_inner()).clone()
    }
}
