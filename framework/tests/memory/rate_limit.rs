//! RTC-003 on the in-memory rate limiter: a request on a key that already
//! has a bucket allocates nothing for the key.

use std::time::Duration;

use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::rate_limit::{RateLimiterDriver, SlidingWindowConfig};

use crate::support::{Heap, exclusive};

/// RTC-003: a request on an existing key that the window rejects makes no
/// allocation: the bucket is found by the caller's `&str`, and a rejected
/// hit records nothing.
#[tokio::test]
async fn rtc_a_rejected_request_on_an_existing_key_allocates_nothing() {
    let _lock = exclusive().await;
    let limiter = InMemoryRateLimiter::new();
    let config = SlidingWindowConfig {
        max_requests: 1,
        window: Duration::from_secs(3600),
    };
    let key = "client:203.0.113.7";
    assert!(
        limiter.try_acquire(key, &config).await.expect("an answer"),
        "the first request is under the limit"
    );

    let heap = Heap::start();
    // The trait method boxes its future when it is called, so the request
    // is built before the count starts and the count is its own work.
    let request = limiter.try_acquire(key, &config);
    let before = heap.blocks();
    let accepted = request.await.expect("an answer");
    let allocated = heap.blocks() - before;
    assert!(!accepted, "the second request is over the limit");
    assert_eq!(
        allocated, 0,
        "a rejected request on an existing key made {allocated} allocations"
    );
}
