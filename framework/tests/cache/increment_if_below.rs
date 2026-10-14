//! `Cache::increment_if_below` through the in-memory store: one lock covers
//! the read, the comparison and the write, so a concurrent burst against
//! one key adds exactly up to the ceiling and no further.

use std::sync::Arc;

use suprnova::Cache;
use suprnova::cache::{CacheStore, ConditionalIncrement, InMemoryCache};
use suprnova::container::testing::TestContainer;

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_concurrent_burst_increments_exactly_up_to_the_ceiling() {
    TestContainer::scope(async {
        TestContainer::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));
        let start = Arc::new(tokio::sync::Barrier::new(256));
        let tasks: Vec<_> = (0..256)
            .map(|_| {
                let start = Arc::clone(&start);
                TestContainer::spawn(async move {
                    start.wait().await;
                    Cache::increment_if_below("burst", 1, 50).await
                })
            })
            .collect();

        let mut incremented = Vec::new();
        let mut unchanged = Vec::new();
        for task in tasks {
            match task.await.expect("the task ran").expect("the step ran") {
                ConditionalIncrement::Incremented(value) => incremented.push(value),
                ConditionalIncrement::Unchanged(value) => unchanged.push(value),
            }
        }

        incremented.sort_unstable();
        assert_eq!(
            incremented,
            (1..=50).collect::<Vec<i64>>(),
            "each of the 50 places went to one caller"
        );
        assert_eq!(unchanged.len(), 206);
        assert!(
            unchanged.iter().all(|value| *value == 50),
            "every refused caller read the full count: {unchanged:?}"
        );
        assert_eq!(
            Cache::get::<i64>("burst").await.expect("read the count"),
            Some(50)
        );
    })
    .await;
}
