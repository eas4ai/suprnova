//! Tests for the `laravel_auth_gaps` block PAR-127: a rate-limiter key is
//! cleaned the way Laravel's `cleanRateLimiterKey` cleans it, so keys that
//! differ only by a named HTML entity share one bucket.

use std::sync::Arc;

use suprnova::cache::{CacheStore, InMemoryCache};
use suprnova::container::testing::TestContainer;
use suprnova::RateLimiter;

#[tokio::test]
async fn hit_counts_a_key_and_its_entity_free_spelling_in_one_bucket() {
    let _guard = TestContainer::fake();
    TestContainer::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));

    let first = RateLimiter::hit("café", 60).await.expect("first hit");
    let second = RateLimiter::hit("cafe", 60).await.expect("second hit");

    assert_eq!(first, 1, "the first hit opens the bucket");
    assert_eq!(
        second, 2,
        "Laravel cleans \"café\" to \"cafe\" before counting, so the second hit shares its bucket"
    );
}
