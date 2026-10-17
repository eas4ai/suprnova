//! A local actor that changes a served tree while requests run against it.
//!
//! The race tests of HRD-005 swap a directory for a symlink, or one file
//! for another, on a thread of their own and assert that no response ever
//! carries what the swap put in place. Shared across test binaries via
//! `#[path]`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How long a racer keeps going if nothing ever stops it.
const BUDGET: Duration = Duration::from_secs(60);

/// Runs `change` in a loop on its own thread until the test stops it.
/// Dropping it stops and joins the thread, so an assertion that fails
/// mid-race cannot leave the thread looping and hang the test.
pub struct Racer {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<u64>>,
}

impl Racer {
    /// Start running `change` until [`Racer::finish`] or a drop.
    pub fn start(mut change: impl FnMut() + Send + 'static) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let running = stop.clone();
        let thread = std::thread::spawn(move || {
            let deadline = Instant::now() + BUDGET;
            let mut rounds = 0u64;
            while !running.load(Ordering::Relaxed) && Instant::now() < deadline {
                change();
                rounds += 1;
                std::thread::yield_now();
            }
            rounds
        });
        Self {
            stop,
            thread: Some(thread),
        }
    }

    /// Stop the racer and return how many times it ran `change`.
    pub fn finish(mut self) -> u64 {
        self.stop.store(true, Ordering::Relaxed);
        self.thread
            .take()
            .map(|thread| thread.join().expect("the racer did not panic"))
            .unwrap_or(0)
    }
}

impl Drop for Racer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
