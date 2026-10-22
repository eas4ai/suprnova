//! The read events: what `Cache::get`, `Cache::has` and `Cache::missing`
//! dispatch, as Laravel's `Repository::get` fires `CacheHit` and
//! `CacheMissed`, so a listener can count hits and misses or trace a key.

use super::store::CacheStore;
use crate::events::EventFacade;

/// A read found the key, as Laravel's `CacheHit` reports it.
///
/// Its `Debug` output leaves the value out: a cached value can be a token
/// or a user's data, and `Debug` reaches logs and test failure messages.
#[derive(Clone, PartialEq)]
#[non_exhaustive]
pub struct CacheHit {
    /// The store that answered: `memory`, `redis`, or the name a store of
    /// your own gives itself through [`CacheStore::name`].
    pub store: String,
    /// The key as the caller gave it, without the store's prefix.
    pub key: String,
    /// The stored value, when the read fetched it. `Cache::get` and the
    /// reads built on it fetch it; `Cache::has` and `Cache::missing` check
    /// presence only, and carry `None`. A stored JSON `null` is
    /// `Some(Value::Null)`: the key holds a value.
    pub value: Option<serde_json::Value>,
}

impl std::fmt::Debug for CacheHit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CacheHit")
            .field("store", &self.store)
            .field("key", &self.key)
            .field("value", &self.value.as_ref().map(|_| "<hidden>"))
            .finish()
    }
}

/// A read found nothing under the key, as Laravel's `CacheMissed` reports
/// it.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct CacheMissed {
    /// The store that answered, as [`CacheHit::store`].
    pub store: String,
    /// The key as the caller gave it, without the store's prefix.
    pub key: String,
}

impl crate::events::Event for CacheHit {
    fn event_name() -> &'static str {
        "CacheHit"
    }
}

impl crate::events::Event for CacheMissed {
    fn event_name() -> &'static str {
        "CacheMissed"
    }
}

/// Dispatch a [`CacheHit`] for `key`, with the value parsed from `raw` when
/// the read fetched it. Nothing is built when nothing listens, so a read
/// costs nothing extra by default. A listener's failure is logged, never
/// returned: the read already answered.
pub(super) async fn hit(store: &dyn CacheStore, key: &str, raw: Option<&str>) {
    if !EventFacade::is_observed::<CacheHit>() {
        return;
    }
    let event = CacheHit {
        store: store.name().to_owned(),
        key: key.to_owned(),
        value: raw.and_then(|raw| serde_json::from_str(raw).ok()),
    };
    if let Err(error) = EventFacade::dispatch_best_effort(event).await {
        tracing::warn!(error = %error, "a CacheHit listener failed");
    }
}

/// Dispatch a [`CacheMissed`] for `key`, on the terms of [`hit`].
pub(super) async fn missed(store: &dyn CacheStore, key: &str) {
    if !EventFacade::is_observed::<CacheMissed>() {
        return;
    }
    let event = CacheMissed {
        store: store.name().to_owned(),
        key: key.to_owned(),
    };
    if let Err(error) = EventFacade::dispatch_best_effort(event).await {
        tracing::warn!(error = %error, "a CacheMissed listener failed");
    }
}
