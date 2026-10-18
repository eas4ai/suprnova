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
use std::sync::{Arc, Mutex};
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

/// Fetches, caches and looks up provider keys.
pub struct KeySetFetcher {
    resolver: Arc<dyn HostResolver>,
    client: Arc<dyn PinnedHttpClient>,
    policy: KeySetPolicy,
    cache: Mutex<HashMap<String, CachedKeySet>>,
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
            cache: Mutex::new(HashMap::new()),
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
    pub(crate) async fn key(&self, jwks_uri: &str, kid: &str) -> Result<Jwk, KeySetError> {
        let now = Instant::now();
        let refetch = {
            let cache = self.lock();
            match cache.get(jwks_uri) {
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
                return Err(error);
            }
        };
        let found = find(&keys, kid);
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
        let mut cache = self.lock();
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

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, CachedKeySet>> {
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
    use async_trait::async_trait;

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
