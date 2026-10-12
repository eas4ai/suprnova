//! M2M (client-credentials) cache key derivation and jittered pre-expiry
//! freshness policy (`docs/specs/suprnova-magnetar/11-token-broker.md`'s
//! "M2M cache" section).

use std::time::Duration;

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

const RECORD_ID_PREFIX: &str = "m2m:v1:";
const RECORD_ID_DOMAIN: &[u8] = b"magnetar-m2m-cache-v1\0";

fn append_component(hasher: &mut Sha256, component: &[u8]) {
    hasher.update((component.len() as u64).to_be_bytes());
    hasher.update(component);
}

/// Identifies one cached machine-to-machine token: a `(provider, client,
/// scope-set)` tuple. Scope order/duplication is irrelevant --
/// [`Self::record_id`] canonicalizes it (sorted, deduplicated) before
/// deriving the broker's opaque record identifier, so
/// `["a", "b"]` and `["b", "a", "b"]` collide onto the same cached record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct M2MCacheKey {
    /// The provider registry name.
    pub provider: String,
    /// The client identifier this cache entry belongs to (ordinarily
    /// [`crate::oauth::provider::OAuthProvider::client_id`], carried
    /// explicitly here so the caller does not need a provider lookup just
    /// to name the cache entry).
    pub client_id: String,
    /// The requested scopes, in any order.
    pub scopes: Vec<String>,
}

impl M2MCacheKey {
    /// Build a cache key.
    pub fn new(
        provider: impl Into<String>,
        client_id: impl Into<String>,
        scopes: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            provider: provider.into(),
            client_id: client_id.into(),
            scopes: scopes.into_iter().collect(),
        }
    }

    /// The normalized, sorted, deduplicated scope list this key resolves
    /// to.
    #[must_use]
    pub fn normalized_scopes(&self) -> Vec<String> {
        let mut scopes = self.scopes.clone();
        scopes.sort();
        scopes.dedup();
        scopes
    }

    /// Deterministically derive the broker's opaque `record_id` for this
    /// key. A versioned domain and length-prefixed components make every
    /// provider, client, and normalized scope boundary unambiguous.
    #[must_use]
    pub fn record_id(&self) -> String {
        let scopes = self.normalized_scopes();
        let mut hasher = Sha256::new();
        hasher.update(RECORD_ID_DOMAIN);
        append_component(&mut hasher, self.provider.as_bytes());
        append_component(&mut hasher, self.client_id.as_bytes());
        append_component(&mut hasher, &(scopes.len() as u64).to_be_bytes());
        for scope in scopes {
            append_component(&mut hasher, scope.as_bytes());
        }
        format!("{RECORD_ID_PREFIX}{:x}", hasher.finalize())
    }
}

/// M2M cache freshness policy: how far ahead of an access token's stated
/// expiry the broker starts refreshing it, randomized within `jitter` so
/// that many broker instances holding the same cache entry do not all
/// decide to refresh in the same instant (a thundering herd against the
/// provider).
#[derive(Clone, Debug)]
pub struct M2MCacheConfig {
    /// Base lead time before expiry at which a refresh becomes due.
    pub refresh_before: Duration,
    /// Additional randomized lead time added on top of `refresh_before`,
    /// in `[0, jitter)`.
    pub jitter: Duration,
}

impl Default for M2MCacheConfig {
    fn default() -> Self {
        Self {
            refresh_before: Duration::from_secs(60),
            jitter: Duration::from_secs(15),
        }
    }
}

/// Whether an M2M-cached token due to expire at `expires_at` needs
/// refreshing as of `now`.
///
/// `jitter_fraction` (clamped to `[0.0, 1.0]`) selects where in the jitter
/// window this particular check lands; production callers draw it from an
/// RNG on every call (so the *effective* threshold is re-randomized each
/// time, not fixed per cache entry) and tests drive the exact `0.0`/`1.0`
/// edges deterministically. A token with no known expiry, or one already
/// past its expiry, always needs refreshing regardless of jitter -- the
/// cache never serves an expired token.
#[must_use]
pub fn needs_refresh(
    expires_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    config: &M2MCacheConfig,
    jitter_fraction: f64,
) -> bool {
    let Some(expires_at) = expires_at else {
        return true;
    };
    if now >= expires_at {
        return true;
    }
    let jitter_fraction = jitter_fraction.clamp(0.0, 1.0);
    let lead_seconds =
        config.refresh_before.as_secs_f64() + config.jitter.as_secs_f64() * jitter_fraction;
    let lead = chrono::Duration::milliseconds((lead_seconds * 1000.0).round() as i64);
    match expires_at.checked_sub_signed(lead) {
        Some(threshold) => now >= threshold,
        None => true,
    }
}
