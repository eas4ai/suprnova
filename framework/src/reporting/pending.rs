use std::sync::{Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

#[derive(Default)]
pub(crate) struct Pending {
    count: Mutex<usize>,
    changed: Condvar,
}

impl Pending {
    pub(crate) fn begin(&self) {
        *self.count.lock().unwrap_or_else(PoisonError::into_inner) += 1;
    }
    pub(crate) fn finish(&self) {
        let mut count = self.count.lock().unwrap_or_else(PoisonError::into_inner);
        *count = count.saturating_sub(1);
        self.changed.notify_all();
    }
    pub(crate) fn wait(&self, timeout: Duration) -> bool {
        let start = Instant::now();
        let mut count = self.count.lock().unwrap_or_else(PoisonError::into_inner);
        while *count != 0 {
            let remaining = timeout.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                return false;
            }
            let (guard, result) = self
                .changed
                .wait_timeout(count, remaining)
                .unwrap_or_else(PoisonError::into_inner);
            count = guard;
            if result.timed_out() && *count != 0 {
                return false;
            }
        }
        true
    }
}
