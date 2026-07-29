//! A keyed cache of expensive-to-build handles that never holds its write
//! lock across construction.
//!
//! The natural way to write get-or-build is to take the write lock, check
//! for a miss, build, and insert — all under the one guard. It reads as
//! correct and it is, for a synchronous build. The moment construction
//! becomes `.await`, the same shape means the write lock is held across a
//! network round trip, and every other caller — including one that would
//! have *hit* the cache for a completely unrelated key — waits behind it.
//! `tokio::sync::RwLock` is fair, so a queued writer blocks subsequent
//! readers too: one cold index stalls every warm one.
//!
//! This module exists as a separate, ungated unit so the discipline can be
//! tested. Its only production caller is the Pinecone driver, which is
//! behind an off-by-default feature; putting the logic inside that driver
//! would mean the test never ran in the default gate, which is how the
//! defect got there in the first place.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Return the cached handle for `key`, building it with `build` on a miss.
///
/// `build` runs with **no lock held**. Two callers racing on the same cold
/// key therefore both build; the first to reach the insert wins and the
/// loser adopts the winner's handle, dropping its own. That matters beyond
/// tidiness: the cached value is typically an `Arc<Mutex<_>>` whose whole
/// job is to serialise access, and handing two callers two different
/// mutexes for the same resource would quietly defeat it.
///
/// The cost of that race is one redundant construction, which is the right
/// trade against serialising every acquisition behind the slowest one.
#[cfg_attr(
    not(feature = "vector-pinecone"),
    allow(
        dead_code,
        reason = "only the Pinecone driver builds handles this \
     way, but the module stays ungated so its tests run in the default gate"
    )
)]
pub(crate) async fn get_or_build<T, F, Fut, E>(
    cache: &RwLock<HashMap<String, Arc<T>>>,
    key: &str,
    build: F,
) -> Result<Arc<T>, E>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Arc<T>, E>>,
{
    {
        let read = cache.read().await;
        if let Some(hit) = read.get(key) {
            return Ok(hit.clone());
        }
    }

    let built = build().await?;

    let mut write = cache.write().await;
    Ok(write.entry(key.to_string()).or_insert(built).clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    type Cache = RwLock<HashMap<String, Arc<u32>>>;

    #[tokio::test]
    async fn a_hit_does_not_build() {
        let cache: Cache = RwLock::new(HashMap::new());
        let builds = AtomicUsize::new(0);

        let build = || async {
            builds.fetch_add(1, Ordering::SeqCst);
            Ok::<_, ()>(Arc::new(7u32))
        };

        let first = get_or_build(&cache, "a", build).await.expect("first build");
        let second = get_or_build(&cache, "a", build)
            .await
            .expect("second must hit");

        assert_eq!(builds.load(Ordering::SeqCst), 1, "the second call must hit");
        assert!(
            Arc::ptr_eq(&first, &second),
            "a hit must return the very same handle, not an equal one"
        );
    }

    /// The property the whole module exists for. Two cold keys must be
    /// able to build at the same time.
    ///
    /// Driven by a barrier rather than a sleep, so it is deterministic:
    /// each build parks until *both* have started. If construction ran
    /// under a shared write lock the second could never start, the barrier
    /// would never open, and both would park forever — which is why the
    /// whole thing is wrapped in a timeout. Without it this test would
    /// hang instead of failing, and a hang in CI reads as an infrastructure
    /// problem rather than a regression.
    #[tokio::test]
    async fn building_one_key_does_not_block_building_another() {
        let cache: Cache = RwLock::new(HashMap::new());
        let barrier = Arc::new(tokio::sync::Barrier::new(2));

        let one = {
            let barrier = barrier.clone();
            get_or_build(&cache, "one", || async move {
                barrier.wait().await;
                Ok::<_, ()>(Arc::new(1u32))
            })
        };
        let two = {
            let barrier = barrier.clone();
            get_or_build(&cache, "two", || async move {
                barrier.wait().await;
                Ok::<_, ()>(Arc::new(2u32))
            })
        };

        let joined = tokio::time::timeout(Duration::from_secs(5), async { tokio::join!(one, two) })
            .await
            .expect(
                "two cold keys must build concurrently; this timed out, which \
             means construction is serialised behind a shared lock — the \
             exact defect this module was extracted to prevent",
            );

        assert_eq!(*joined.0.expect("key one built"), 1);
        assert_eq!(*joined.1.expect("key two built"), 2);
    }

    /// A cache hit for a warm key must not wait on an unrelated cold key's
    /// construction. This is the case that hurts most in production: the
    /// caller is not even using the index that is slow.
    #[tokio::test]
    async fn a_hit_does_not_wait_on_an_unrelated_cold_build() {
        let cache: Cache = RwLock::new(HashMap::new());
        cache
            .write()
            .await
            .insert("warm".to_string(), Arc::new(99u32));

        let release = Arc::new(tokio::sync::Notify::new());

        let cold = {
            let release = release.clone();
            get_or_build(&cache, "cold", || async move {
                release.notified().await;
                Ok::<_, ()>(Arc::new(1u32))
            })
        };

        let hit = async {
            // Turbofished because the builder diverges, so nothing in it
            // pins the error type.
            let value = get_or_build::<u32, _, _, ()>(&cache, "warm", || async {
                unreachable!("`warm` is already cached; this must not build")
            })
            .await
            .expect("warm hit");
            // Only now let the cold build finish, proving the hit did not
            // need it to complete first.
            release.notify_one();
            value
        };

        let (cold, warm) =
            tokio::time::timeout(Duration::from_secs(5), async { tokio::join!(cold, hit) })
                .await
                .expect(
                    "a warm cache hit must not block behind an unrelated cold \
             build; timing out here means one slow index stalls every \
             other one",
                );

        assert_eq!(*warm, 99);
        assert_eq!(*cold.expect("cold built"), 1);
    }

    /// Racing builders of the *same* key must converge on one handle. The
    /// loser's freshly-built value is dropped, so callers can rely on the
    /// cached `Arc` being the only one in circulation — which is what makes
    /// it safe to wrap a `Mutex` around a resource that must be serialised.
    #[tokio::test]
    async fn racing_builders_of_one_key_converge_on_a_single_handle() {
        let cache: Cache = RwLock::new(HashMap::new());
        let barrier = Arc::new(tokio::sync::Barrier::new(2));

        let racer = |n: u32| {
            let barrier = barrier.clone();
            get_or_build(&cache, "same", move || async move {
                barrier.wait().await;
                Ok::<_, ()>(Arc::new(n))
            })
        };

        let (a, b) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(racer(1), racer(2))
        })
        .await
        .expect("both racers must build concurrently, then converge");

        let a = a.expect("racer a");
        let b = b.expect("racer b");
        assert!(
            Arc::ptr_eq(&a, &b),
            "both racers must end up with the same handle; two handles for \
             one key would defeat any mutex the handle carries"
        );

        let cached = cache.read().await.get("same").cloned().expect("cached");
        assert!(
            Arc::ptr_eq(&cached, &a),
            "and the cache must hold that same handle"
        );
    }

    /// A failed build must not be cached, or one transient network error
    /// would poison the key for the process lifetime.
    #[tokio::test]
    async fn a_failed_build_is_not_cached() {
        let cache: Cache = RwLock::new(HashMap::new());

        let err = get_or_build(&cache, "flaky", || async { Err::<Arc<u32>, _>("boom") }).await;
        assert_eq!(err, Err("boom"));
        assert!(
            cache.read().await.is_empty(),
            "a failed build must leave the cache untouched"
        );

        let ok = get_or_build(&cache, "flaky", || async { Ok::<_, &str>(Arc::new(5u32)) })
            .await
            .expect("a retry after a failure must be able to succeed");
        assert_eq!(*ok, 5);
    }
}
