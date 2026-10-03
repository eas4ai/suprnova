//! In-process coalescing for concurrent callers of the same broker record.
//!
//! Purely an optimization layer in front of [`super::lease`]'s storage CAS
//! protocol, which is unconditionally correct without it -- spec 11:
//! "`single_flight` is an optimization, never a correctness precondition."
//! [`BrokerConfig::single_flight`](super::BrokerConfig::single_flight)
//! governs whether callers route through [`SingleFlight::run`] at all; the
//! two-pod concurrency suites run with it disabled to prove the storage
//! layer alone is sufficient.
//!
//! This crate carries no runtime/executor dependency (`tokio` is a
//! dev-dependency only), so coalescing is built on `futures-util`'s
//! executor-agnostic async [`Mutex`], not `tokio::sync`.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use futures_util::lock::Mutex;

/// A per-key async-mutex map: [`SingleFlight::run`] serializes concurrent
/// callers sharing one key onto one lease attempt at a time, so the second
/// caller through the door blocks until the first finishes and then
/// re-enters the lease protocol itself (observing the first caller's
/// freshly committed result on its very first storage read) rather than
/// racing storage independently.
///
/// Per-key entries are never evicted: memory grows with the number of
/// distinct `record_id`/M2M-cache keys ever seen by this process, not with
/// call volume. Acceptable for a broker whose key space is bounded by
/// linked accounts and M2M client/scope combinations; a host with an
/// unusually large or unbounded key space should disable
/// [`super::BrokerConfig::single_flight`] instead of relying on eviction
/// this type does not do.
#[derive(Default)]
pub struct SingleFlight {
    locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl SingleFlight {
    /// An empty coalescing map.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Run `task` while holding the exclusive in-process lock for `key`.
    pub async fn run<T, Fut>(&self, key: &str, task: Fut) -> T
    where
        Fut: Future<Output = T>,
    {
        let lock = {
            let mut locks = self.locks.lock().await;
            locks
                .entry(key.to_owned())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let _permit = lock.lock().await;
        task.await
    }
}
