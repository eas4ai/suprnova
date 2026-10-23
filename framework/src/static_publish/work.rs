//! Shutdown tracks lease cleanup and owned delivery even when a caller is cancelled.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Notify;

#[derive(Default)]
pub(crate) struct Work {
    pending: AtomicUsize,
    drained: Notify,
}
impl Work {
    pub(crate) fn begin(self: &Arc<Self>) -> WorkGuard {
        self.pending.fetch_add(1, Ordering::SeqCst);
        WorkGuard(self.clone())
    }
    pub(crate) fn pending(&self) -> usize {
        self.pending.load(Ordering::SeqCst)
    }
    pub(crate) async fn drain(&self) {
        loop {
            let notified = self.drained.notified();
            if self.pending() == 0 {
                return;
            }
            notified.await;
        }
    }
}
pub(crate) struct WorkGuard(Arc<Work>);
impl Drop for WorkGuard {
    fn drop(&mut self) {
        self.0.pending.fetch_sub(1, Ordering::SeqCst);
        self.0.drained.notify_waiters();
    }
}
