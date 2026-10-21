//! Fetching and caching the JSON Web Key Sets of trusted providers.
//!
//! A key set is fetched through the authorization server's fenced fetch:
//! HTTPS only, every address the host resolves to checked, the connection
//! made to the checked address, no redirect, the body and the time bounded.
//! A fetched set is cached for as long as its `Cache-Control` asks, but
//! never less than the floor or more than the ceiling (10 minutes and 24
//! hours by default). A token naming a key the cached set lacks refetches
//! the set once, so a provider's key rotation works without waiting for the
//! cache; such refetches are spaced out so forged tokens cannot turn the
//! server into a request amplifier against the provider.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use jsonwebtoken::jwk::Jwk;
use serde_json::Value;
use url::Url;

use crate::authorization_server::metadata_document::pinned_fetch;
use crate::authorization_server::{
    HostResolver, PinnedHttpClient, ReqwestPinnedClient, SystemResolver, is_public_address,
};

/// Bounds on fetching and caching key sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeySetPolicy {
    /// The largest key set body read, in bytes.
    pub max_bytes: usize,
    /// The time allowed for one whole fetch.
    pub timeout: Duration,
    /// The shortest time a fetched set is reused.
    pub min_ttl: Duration,
    /// The longest time a fetched set is reused, whatever its
    /// `Cache-Control` says.
    pub max_ttl: Duration,
    /// The shortest time between two refetches caused by an unknown key
    /// identifier.
    pub refetch_interval: Duration,
    /// How many key sets the cache holds at most.
    pub cache_capacity: usize,
    /// How long a failed fetch is remembered, so tokens that cannot be
    /// verified do not each cost the provider a request.
    pub failure_ttl: Duration,
}

impl Default for KeySetPolicy {
    fn default() -> Self {
        Self {
            max_bytes: 64 * 1024,
            timeout: Duration::from_secs(5),
            min_ttl: Duration::from_secs(600),
            max_ttl: Duration::from_secs(24 * 3600),
            refetch_interval: Duration::from_secs(60),
            cache_capacity: 64,
            failure_ttl: Duration::from_secs(60),
        }
    }
}

/// Why a key could not be looked up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeySetError {
    /// The key set could not be fetched or read.
    Unavailable(String),
    /// The key set has no key with the identifier the token names.
    UnknownKey,
}

impl std::fmt::Display for KeySetError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(reason) => write!(formatter, "the key set is unavailable: {reason}"),
            Self::UnknownKey => write!(formatter, "the key set has no such key"),
        }
    }
}

struct CachedKeySet {
    keys: Vec<Jwk>,
    fetched_at: Instant,
    ttl: Duration,
    last_forced_refetch: Option<Instant>,
}

#[derive(Default)]
struct PendingKeySet {
    result: tokio::sync::Mutex<Option<Result<Vec<Jwk>, KeySetError>>>,
}

#[derive(Default)]
struct KeySetCache {
    entries: HashMap<String, CachedKeySet>,
    in_flight: HashMap<String, Weak<PendingKeySet>>,
}

/// Fetches, caches and looks up provider keys.
pub struct KeySetFetcher {
    resolver: Arc<dyn HostResolver>,
    client: Arc<dyn PinnedHttpClient>,
    policy: KeySetPolicy,
    cache: Mutex<KeySetCache>,
    /// When each key set's last fetch failed, so the next tokens wait out
    /// [`KeySetPolicy::failure_ttl`] instead of each fetching again.
    failures: Mutex<HashMap<String, Instant>>,
    address_allowed: fn(IpAddr) -> bool,
}

impl KeySetFetcher {
    /// A fetcher over a resolver and a pinned client the host chose.
    #[must_use]
    pub fn new(
        resolver: Arc<dyn HostResolver>,
        client: Arc<dyn PinnedHttpClient>,
        policy: KeySetPolicy,
    ) -> Self {
        Self {
            resolver,
            client,
            policy,
            cache: Mutex::new(KeySetCache::default()),
            failures: Mutex::new(HashMap::new()),
            address_allowed: is_public_address,
        }
    }

    /// A fetcher over the system resolver and [`ReqwestPinnedClient`].
    #[must_use]
    pub fn system(policy: KeySetPolicy) -> Self {
        Self::new(
            Arc::new(SystemResolver),
            Arc::new(ReqwestPinnedClient::new(policy.timeout)),
            policy,
        )
    }

    /// The key `kid` of the set at `jwks_uri`: from the cache, from a fetch
    /// when the set is not cached or has aged out, or from one refetch when
    /// the cached set lacks it.
    ///
    /// Reserve the URI and forced-refetch time before awaiting the fetch,
    /// so concurrent unverified tokens share its result instead of sending
    /// more provider requests. Cancellation keeps the spacing reservation.
    /// Fetch failures and missing keys return errors because an unverified
    /// token must not interrupt the server.
    pub(crate) async fn key(&self, jwks_uri: &str, kid: &str) -> Result<Jwk, KeySetError> {
        let pending = {
            let mut cache = self.lock();
            // Weak entries do not keep completed results or idle URI locks alive.
            cache
                .in_flight
                .retain(|_, pending| pending.strong_count() > 0);
            match cache.in_flight.get(jwks_uri).and_then(Weak::upgrade) {
                Some(pending) => pending,
                None => {
                    let pending = Arc::new(PendingKeySet::default());
                    cache
                        .in_flight
                        .insert(jwks_uri.to_owned(), Arc::downgrade(&pending));
                    pending
                }
            }
        };
        let mut shared = pending.result.lock().await;
        if let Some(result) = shared.as_ref() {
            return match result {
                Ok(keys) => find(keys, kid).ok_or(KeySetError::UnknownKey),
                Err(error) => Err(error.clone()),
            };
        }
        let now = Instant::now();
        let refetch = {
            let cache = self.lock();
            match cache.entries.get(jwks_uri) {
                Some(entry) if now.duration_since(entry.fetched_at) < entry.ttl => {
                    if let Some(key) = find(&entry.keys, kid) {
                        return Ok(key);
                    }
                    let spaced = entry.last_forced_refetch.is_none_or(|last| {
                        now.duration_since(last) >= self.policy.refetch_interval
                    });
                    if !spaced {
                        return Err(KeySetError::UnknownKey);
                    }
                    true
                }
                _ => false,
            }
        };
        let failed_recently = {
            let failures = match self.failures.lock() {
                Ok(failures) => failures,
                Err(poisoned) => poisoned.into_inner(),
            };
            failures
                .get(jwks_uri)
                .is_some_and(|failed| now.duration_since(*failed) < self.policy.failure_ttl)
        };
        if failed_recently {
            return Err(KeySetError::Unavailable(
                "the last fetch failed; the key set is retried after a pause".to_owned(),
            ));
        }
        if refetch && let Some(entry) = self.lock().entries.get_mut(jwks_uri) {
            entry.last_forced_refetch = Some(now);
        }
        let fetched = self.fetch(jwks_uri).await;
        let mut failures = match self.failures.lock() {
            Ok(failures) => failures,
            Err(poisoned) => poisoned.into_inner(),
        };
        let (keys, ttl) = match fetched {
            Ok(fetched) => {
                failures.remove(jwks_uri);
                drop(failures);
                fetched
            }
            Err(error) => {
                if failures.len() >= self.policy.cache_capacity.max(1) {
                    failures
                        .retain(|_, failed| now.duration_since(*failed) < self.policy.failure_ttl);
                }
                failures.insert(jwks_uri.to_owned(), Instant::now());
                *shared = Some(Err(error.clone()));
                return Err(error);
            }
        };
        let found = find(&keys, kid);
        *shared = Some(Ok(keys.clone()));
        self.remember(jwks_uri, keys, ttl, refetch.then_some(now));
        found.ok_or(KeySetError::UnknownKey)
    }

    async fn fetch(&self, jwks_uri: &str) -> Result<(Vec<Jwk>, Duration), KeySetError> {
        let url = Url::parse(jwks_uri)
            .map_err(|error| KeySetError::Unavailable(format!("the URL is invalid: {error}")))?;
        if url.scheme() != "https" {
            return Err(KeySetError::Unavailable(
                "the URL must use https".to_owned(),
            ));
        }
        let fetched = tokio::time::timeout(
            self.policy.timeout,
            pinned_fetch(
                self.resolver.as_ref(),
                self.client.as_ref(),
                self.address_allowed,
                &url,
                self.policy.max_bytes,
            ),
        )
        .await
        .map_err(|_| KeySetError::Unavailable("the fetch timed out".to_owned()))?
        .map_err(|error| KeySetError::Unavailable(error.to_string()))?;
        let document: Value = serde_json::from_slice(&fetched.body).map_err(|error| {
            KeySetError::Unavailable(format!(
                "the key set {}",
                crate::crypto::decode_failure(&error)
            ))
        })?;
        let Some(entries) = document.get("keys").and_then(Value::as_array) else {
            return Err(KeySetError::Unavailable(
                "the key set has no keys array".to_owned(),
            ));
        };
        // A key of a type this server cannot read, or one meant for
        // encryption, is skipped rather than failing the whole set.
        let keys = entries
            .iter()
            .filter(|entry| entry.get("use").and_then(Value::as_str) != Some("enc"))
            .filter_map(|entry| serde_json::from_value::<Jwk>(entry.clone()).ok())
            .collect();
        Ok((keys, self.ttl(fetched.cache_control.as_deref())))
    }

    /// The cache lifetime a `Cache-Control` header asks for, within the
    /// policy's floor and ceiling.
    fn ttl(&self, cache_control: Option<&str>) -> Duration {
        let asked = cache_control.and_then(|header| {
            header.split(',').find_map(|directive| {
                let directive = directive.trim();
                if directive.eq_ignore_ascii_case("no-store")
                    || directive.eq_ignore_ascii_case("no-cache")
                {
                    return Some(Duration::ZERO);
                }
                let (name, value) = directive.split_once('=')?;
                name.trim()
                    .eq_ignore_ascii_case("max-age")
                    .then(|| value.trim().trim_matches('"').parse::<u64>().ok())
                    .flatten()
                    .map(Duration::from_secs)
            })
        });
        asked.unwrap_or(self.policy.min_ttl).clamp(
            self.policy.min_ttl,
            self.policy.max_ttl.max(self.policy.min_ttl),
        )
    }

    fn remember(&self, jwks_uri: &str, keys: Vec<Jwk>, ttl: Duration, forced: Option<Instant>) {
        if self.policy.cache_capacity == 0 {
            return;
        }
        let mut state = self.lock();
        let cache = &mut state.entries;
        let previous_forced = cache
            .get(jwks_uri)
            .and_then(|entry| entry.last_forced_refetch);
        if cache.len() >= self.policy.cache_capacity
            && !cache.contains_key(jwks_uri)
            && let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, entry)| entry.fetched_at)
                .map(|(key, _)| key.clone())
        {
            cache.remove(&oldest);
        }
        cache.insert(
            jwks_uri.to_owned(),
            CachedKeySet {
                keys,
                fetched_at: Instant::now(),
                ttl,
                last_forced_refetch: forced.or(previous_forced),
            },
        );
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, KeySetCache> {
        match self.cache.lock() {
            Ok(cache) => cache,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

fn find(keys: &[Jwk], kid: &str) -> Option<Jwk> {
    keys.iter()
        .find(|key| key.common.key_id.as_deref() == Some(kid))
        .cloned()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use futures_util::future::join_all;
    use tokio::sync::watch;

    use super::*;
    use crate::authorization_server::{PinnedResponse, ResponseBody};

    struct NoResolver;

    #[async_trait]
    impl HostResolver for NoResolver {
        async fn resolve(&self, _host: &str, _port: u16) -> crate::Result<Vec<IpAddr>> {
            Ok(Vec::new())
        }
    }

    struct NoClient;

    #[async_trait]
    impl PinnedHttpClient for NoClient {
        async fn get(
            &self,
            _url: &Url,
            _address: std::net::SocketAddr,
        ) -> crate::Result<PinnedResponse> {
            struct Empty;
            #[async_trait]
            impl ResponseBody for Empty {
                async fn next_chunk(&mut self) -> crate::Result<Option<Vec<u8>>> {
                    Ok(None)
                }
            }
            Ok(PinnedResponse {
                status: 404,
                cache_control: None,
                body: Box::new(Empty),
            })
        }
    }

    fn fetcher() -> KeySetFetcher {
        KeySetFetcher::new(
            Arc::new(NoResolver),
            Arc::new(NoClient),
            KeySetPolicy::default(),
        )
    }

    struct PublicResolver;

    #[async_trait]
    impl HostResolver for PublicResolver {
        async fn resolve(&self, _host: &str, _port: u16) -> crate::Result<Vec<IpAddr>> {
            Ok(vec![IpAddr::from([8, 8, 8, 8])])
        }
    }

    struct BlockingClient {
        requests: AtomicUsize,
        release: watch::Sender<bool>,
        status: u16,
    }

    #[async_trait]
    impl PinnedHttpClient for BlockingClient {
        async fn get(
            &self,
            _url: &Url,
            _address: std::net::SocketAddr,
        ) -> crate::Result<PinnedResponse> {
            self.requests.fetch_add(1, Ordering::SeqCst);
            self.release
                .subscribe()
                .wait_for(|released| *released)
                .await
                .expect("release fetch");
            struct Body(Option<Vec<u8>>);
            #[async_trait]
            impl ResponseBody for Body {
                async fn next_chunk(&mut self) -> crate::Result<Option<Vec<u8>>> {
                    Ok(self.0.take())
                }
            }
            Ok(PinnedResponse {
                status: self.status,
                cache_control: None,
                body: Box::new(Body(Some(br#"{"keys":[]}"#.to_vec()))),
            })
        }
    }

    const JWKS_URI: &str = "https://trusted-provider.example/jwks";

    fn blocking_fetcher(status: u16) -> (KeySetFetcher, Arc<BlockingClient>) {
        let client = Arc::new(BlockingClient {
            requests: AtomicUsize::new(0),
            release: watch::channel(false).0,
            status,
        });
        let fetcher = KeySetFetcher::new(
            Arc::new(PublicResolver),
            client.clone(),
            KeySetPolicy::default(),
        );
        (fetcher, client)
    }

    async fn concurrent_unknown_keys_share_one_fetch(warm: bool, status: u16) {
        const CALLERS: usize = 8;
        let (fetcher, client) = blocking_fetcher(status);
        if warm {
            fetcher.remember(JWKS_URI, Vec::new(), Duration::from_secs(600), None);
        }
        let mut lookups: Vec<_> = (0..CALLERS)
            .map(|_| Box::pin(fetcher.key(JWKS_URI, "unknown")))
            .collect();
        // Poll every caller into the blocked fetch or its waiter before release.
        for lookup in &mut lookups {
            assert!(futures_util::poll!(lookup).is_pending());
        }
        client.release.send_replace(true);
        let answers = tokio::time::timeout(Duration::from_secs(2), join_all(lookups))
            .await
            .expect("all lookups finish");
        let errors: Vec<_> = answers
            .into_iter()
            .map(|answer| answer.expect_err("unknown key cannot verify"))
            .collect();
        assert!(errors.iter().all(|error| error == &errors[0]));
        if status == 200 {
            assert_eq!(errors[0], KeySetError::UnknownKey);
        } else {
            assert!(matches!(errors[0], KeySetError::Unavailable(_)));
        }
        assert_eq!(client.requests.load(Ordering::SeqCst), 1);

        if warm || status != 200 {
            let second = fetcher.key(JWKS_URI, "another-unknown").await;
            if warm {
                assert_eq!(
                    second.expect_err("forced refetch is spaced"),
                    KeySetError::UnknownKey
                );
            } else {
                assert!(matches!(second, Err(KeySetError::Unavailable(_))));
            }
            assert_eq!(client.requests.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn concurrent_cold_unknown_keys_share_one_fetch() {
        concurrent_unknown_keys_share_one_fetch(false, 200).await;
    }

    #[tokio::test]
    async fn concurrent_forced_refetches_share_one_fetch_and_respect_the_interval() {
        concurrent_unknown_keys_share_one_fetch(true, 200).await;
    }

    #[tokio::test]
    async fn concurrent_cold_failures_share_one_fetch_and_pause() {
        concurrent_unknown_keys_share_one_fetch(false, 503).await;
    }

    #[tokio::test]
    async fn concurrent_forced_failures_share_one_fetch_and_respect_the_interval() {
        concurrent_unknown_keys_share_one_fetch(true, 503).await;
    }

    #[tokio::test]
    async fn cancelled_forced_refetch_keeps_its_reservation() {
        let (fetcher, client) = blocking_fetcher(200);
        fetcher.remember(JWKS_URI, Vec::new(), Duration::from_secs(600), None);
        let mut lookup = Box::pin(fetcher.key(JWKS_URI, "unknown"));
        assert!(futures_util::poll!(&mut lookup).is_pending());
        drop(lookup);
        client.release.send_replace(true);
        assert_eq!(
            fetcher
                .key(JWKS_URI, "another-unknown")
                .await
                .expect_err("reservation survives cancellation"),
            KeySetError::UnknownKey
        );
        assert_eq!(client.requests.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn agent_008_cache_control_is_honoured_between_the_floor_and_the_ceiling() {
        let fetcher = fetcher();
        let minutes = |count: u64| Duration::from_secs(count * 60);
        assert_eq!(fetcher.ttl(None), minutes(10), "no header: the floor");
        assert_eq!(
            fetcher.ttl(Some("max-age=60")),
            minutes(10),
            "below the floor"
        );
        assert_eq!(fetcher.ttl(Some("public, max-age=3600")), minutes(60));
        assert_eq!(
            fetcher.ttl(Some("max-age=999999")),
            minutes(24 * 60),
            "above the ceiling"
        );
        assert_eq!(fetcher.ttl(Some("no-store")), minutes(10));
        assert_eq!(fetcher.ttl(Some("max-age=nonsense")), minutes(10));
    }
}
