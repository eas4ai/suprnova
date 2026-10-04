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
use std::sync::{Arc, PoisonError};

use futures_util::lock::Mutex;

/// A per-key async-mutex map: [`SingleFlight::run`] serializes concurrent
/// callers sharing one key onto one lease attempt at a time, so the second
/// caller through the door blocks until the first finishes and then
/// re-enters the lease protocol itself (observing the first caller's
/// freshly committed result on its very first storage read) rather than
/// racing storage independently.
///
/// A key is held only while a caller holds or waits on its lock: the last
/// caller to leave, whether it finished, failed or was cancelled while it
/// waited, removes the key. The map therefore grows with the keys in use
/// at once, not with every key the process has seen.
#[derive(Default)]
pub struct SingleFlight {
    /// The per-key locks. A plain mutex: it is held only to look up, add
    /// or remove a key, never across an `.await`, and a caller leaving
    /// must reach it from a destructor, which cannot await.
    locks: std::sync::Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl SingleFlight {
    /// An empty coalescing map.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn locks(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<Mutex<()>>>> {
        // A panic while the map was locked cannot leave it half-updated:
        // each critical section is one lookup, insert or remove.
        self.locks.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The keys the map holds right now.
    #[cfg(test)]
    async fn tracked_keys(&self) -> usize {
        self.locks().len()
    }

    /// Run `task` while holding the exclusive in-process lock for `key`.
    pub async fn run<T, Fut>(&self, key: &str, task: Fut) -> T
    where
        Fut: Future<Output = T>,
    {
        let lock = Arc::clone(
            self.locks()
                .entry(key.to_owned())
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        );
        let caller = Caller {
            flight: self,
            key,
            lock: Some(lock),
        };
        let Some(lock) = caller.lock.as_ref() else {
            return task.await;
        };
        let _permit = lock.lock().await;
        task.await
    }
}

/// One caller of a key, from taking its lock out of the map until it is
/// done with it, however it ends.
struct Caller<'a> {
    flight: &'a SingleFlight,
    key: &'a str,
    /// Always `Some` until `drop` takes it.
    lock: Option<Arc<Mutex<()>>>,
}

impl Drop for Caller<'_> {
    fn drop(&mut self) {
        let mut locks = self.flight.locks();
        // This caller's reference is released under the map lock, so two
        // callers leaving at once cannot each see the other's and both
        // keep the key: the last one sees only the map's.
        drop(self.lock.take());
        if locks
            .get(self.key)
            .is_some_and(|lock| Arc::strong_count(lock) == 1)
        {
            locks.remove(self.key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MEM-001: a key is forgotten once every caller of it has finished.
    #[tokio::test]
    async fn mem_audit_finished_keys_are_forgotten() {
        let flight = SingleFlight::new();
        for i in 0..1_000 {
            flight.run(&format!("record-{i}"), async {}).await;
        }
        assert_eq!(flight.tracked_keys().await, 0);
    }

    /// MEM-001: a key stays while a second caller waits on it, and goes
    /// when both are done.
    #[tokio::test]
    async fn mem_audit_a_waiting_caller_keeps_the_key() {
        let flight = Arc::new(SingleFlight::new());
        let (release, gate) = tokio::sync::oneshot::channel::<()>();
        let first = tokio::spawn({
            let flight = flight.clone();
            async move {
                flight
                    .run("shared", async {
                        let _ = gate.await;
                    })
                    .await;
            }
        });
        tokio::task::yield_now().await;
        let second = tokio::spawn({
            let flight = flight.clone();
            async move { flight.run("shared", async {}).await }
        });
        tokio::task::yield_now().await;
        assert_eq!(flight.tracked_keys().await, 1);
        release.send(()).expect("the first caller waits");
        first.await.expect("first");
        second.await.expect("second");
        assert_eq!(flight.tracked_keys().await, 0);
    }

    /// MEM-001: a caller dropped mid-task forgets its key too.
    #[tokio::test]
    async fn mem_audit_a_cancelled_caller_forgets_its_key() {
        let flight = SingleFlight::new();
        let pending = flight.run("cancelled", std::future::pending::<()>());
        let _ = tokio::time::timeout(std::time::Duration::from_millis(10), pending).await;
        assert_eq!(flight.tracked_keys().await, 0);
    }
}
