//! RTC-001: password hash work runs under one process-wide limit, and the
//! excess waits as tasks. A hasher that waits for the test's signal shows
//! how many pieces of work are inside the hasher at once; with the limit at
//! two, a third piece must not enter until one of the first two returns.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use suprnova::FrameworkError;
use suprnova::hashing::{self, Algorithm, Hasher};

/// A hasher that counts the pieces of work inside it and holds each until
/// the test opens the gate.
struct Gated {
    entered: Arc<AtomicUsize>,
    gate: Arc<(Mutex<bool>, Condvar)>,
}

impl Hasher for Gated {
    fn algorithm(&self) -> Algorithm {
        Algorithm::Bcrypt
    }

    fn hash(&self, password: &str) -> Result<String, FrameworkError> {
        self.entered.fetch_add(1, Ordering::SeqCst);
        let (open, signal) = &*self.gate;
        let mut open = open.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        while !*open {
            open = signal.wait(open).unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        Ok(format!("$gated${password}"))
    }

    fn verify(&self, password: &str, hash: &str) -> Result<bool, FrameworkError> {
        Ok(hash == format!("$gated${password}"))
    }

    fn needs_rehash(&self, _hash: &str) -> bool {
        false
    }
}

async fn wait_until(entered: &AtomicUsize, count: usize) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while entered.load(Ordering::SeqCst) < count {
        assert!(
            tokio::time::Instant::now() < deadline,
            "only {} pieces of hash work entered the hasher",
            entered.load(Ordering::SeqCst)
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// RTC-001: with `HASH_MAX_CONCURRENCY=2`, the third piece of hash work
/// waits outside the hasher until one of the first two returns.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rtc_hash_work_past_the_limit_waits_as_a_task() {
    // SAFETY: nextest runs this test in a process of its own, and nothing
    // else in it reads the environment while this call runs.
    unsafe { std::env::set_var("HASH_MAX_CONCURRENCY", "2") };
    let entered = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    hashing::set_default_driver(Box::new(Gated {
        entered: entered.clone(),
        gate: gate.clone(),
    }))
    .expect("install the gated hasher");

    let work: Vec<_> = (0..3)
        .map(|i| tokio::spawn(async move { hashing::hash_async(&format!("password {i}")).await }))
        .collect();
    wait_until(&entered, 2).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        entered.load(Ordering::SeqCst),
        2,
        "a third piece of hash work entered the hasher while two held the permits"
    );

    {
        let (open, signal) = &*gate;
        *open.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
        signal.notify_all();
    }
    for piece in work {
        let hash = piece.await.expect("the task ran").expect("the hash");
        assert!(hash.starts_with("$gated$"));
    }
    assert_eq!(entered.load(Ordering::SeqCst), 3, "every piece ran once the gate opened");
}
