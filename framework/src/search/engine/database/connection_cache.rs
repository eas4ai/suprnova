//! Cache successful server probes without retaining failures.

use std::{collections::BTreeMap, future::Future, sync::Arc};
use tokio::sync::{Mutex, OnceCell};

pub(super) struct ConnectionCache<T> {
    values: Mutex<BTreeMap<String, Arc<OnceCell<T>>>>,
}

impl<T> Default for ConnectionCache<T> {
    fn default() -> Self {
        Self {
            values: Mutex::new(BTreeMap::new()),
        }
    }
}

impl<T: Copy> ConnectionCache<T> {
    pub(super) async fn get_or_try_init<E>(
        &self,
        connection: String,
        probe: impl Future<Output = Result<T, E>>,
    ) -> Result<T, E> {
        let cell = self
            .values
            .lock()
            .await
            .entry(connection)
            .or_default()
            .clone();
        // Release the map before probing so different servers can resolve concurrently.
        cell.get_or_try_init(|| probe).await.copied()
    }
}

#[cfg(test)]
mod tests {
    use super::ConnectionCache;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn mysql_flavor_cache_probes_each_named_connection_once() {
        let cache = ConnectionCache::default();
        let probes = AtomicUsize::new(0);
        for _ in 0..2 {
            for (connection, is_mariadb) in [("maria", true), ("mysql8", false)] {
                let flavor = cache
                    .get_or_try_init(connection.into(), async {
                        probes.fetch_add(1, Ordering::SeqCst);
                        tokio::task::yield_now().await;
                        Ok::<_, &str>(is_mariadb)
                    })
                    .await
                    .unwrap();
                assert_eq!(flavor, is_mariadb, "wrong flavor for {connection}");
            }
        }
        assert_eq!(probes.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn mysql_flavor_cache_coalesces_concurrent_probes() {
        let cache = ConnectionCache::default();
        let probes = AtomicUsize::new(0);
        let probe = || async {
            probes.fetch_add(1, Ordering::SeqCst);
            tokio::task::yield_now().await;
            Ok::<_, &str>(true)
        };
        let (first, second) = tokio::join!(
            cache.get_or_try_init("maria".into(), probe()),
            cache.get_or_try_init("maria".into(), probe()),
        );
        assert_eq!(first, Ok(true));
        assert_eq!(second, Ok(true));
        assert_eq!(probes.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn mysql_flavor_cache_retries_a_failed_probe() {
        let cache = ConnectionCache::default();
        assert!(
            cache
                .get_or_try_init("mysql8".into(), async { Err::<bool, _>("offline") })
                .await
                .is_err()
        );
        assert!(
            !cache
                .get_or_try_init("mysql8".into(), async { Ok::<_, &str>(false) })
                .await
                .unwrap()
        );
    }
}
