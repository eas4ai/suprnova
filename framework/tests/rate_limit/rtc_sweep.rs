//! RTC-003: while the in-memory rate limiter sweeps its buckets, requests
//! on other keys keep completing; a sweep that holds every bucket stops
//! them all until it ends.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::rate_limit::{RateLimiterDriver, SlidingWindowConfig};

const BUCKETS: usize = 200_000;

/// How far past now the sweep looks: every bucket the test fills ages out
/// by then.
const SWEEP_AHEAD: Duration = Duration::from_secs(7200);

fn config() -> SlidingWindowConfig {
    SlidingWindowConfig {
        max_requests: 10,
        window: Duration::from_secs(3600),
    }
}

/// The requests on other keys keep their history longer than the sweep
/// looks ahead, so the sweep keeps their buckets whenever it reaches them
/// and drops exactly the ones the test filled.
fn other_config() -> SlidingWindowConfig {
    SlidingWindowConfig {
        max_requests: 10,
        window: SWEEP_AHEAD * 2,
    }
}

/// RTC-003: at least 100 requests on other keys start and finish strictly
/// inside the span of a sweep over 200,000 buckets.
#[test]
fn rtc_requests_on_other_keys_complete_while_a_sweep_runs() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let limiter = Arc::new(InMemoryRateLimiter::new());
    runtime.block_on(async {
        for i in 0..BUCKETS {
            assert!(
                limiter
                    .try_acquire(&format!("sweep:{i}"), &config())
                    .await
                    .expect("a slot")
            );
        }
    });
    assert_eq!(limiter.bucket_count(), BUCKETS);

    let sweeping = Arc::new(AtomicBool::new(true));
    let requester = {
        let limiter = limiter.clone();
        let sweeping = sweeping.clone();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a runtime");
            let mut spans = Vec::new();
            let mut i = 0usize;
            while sweeping.load(Ordering::SeqCst) {
                let start = Instant::now();
                runtime
                    .block_on(limiter.try_acquire(&format!("other:{}", i % 64), &other_config()))
                    .expect("an answer");
                spans.push((start, Instant::now()));
                i += 1;
            }
            spans
        })
    };

    let sweep_start = Instant::now();
    let dropped = limiter.purge_inactive(Duration::ZERO, tokio::time::Instant::now() + SWEEP_AHEAD);
    let sweep_end = Instant::now();
    sweeping.store(false, Ordering::SeqCst);
    let spans = requester.join().expect("the requester thread");
    assert_eq!(dropped, BUCKETS, "the sweep dropped every aged bucket");

    let inside = spans
        .iter()
        .filter(|(start, end)| *start > sweep_start && *end < sweep_end)
        .count();
    assert!(
        inside >= 100,
        "{inside} requests on other keys completed inside the {:?} sweep ({} requests ran in all)",
        sweep_end - sweep_start,
        spans.len()
    );
}
