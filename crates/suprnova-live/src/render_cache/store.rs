//! The RenderStore contract and the immutable in-process L0 store.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use bytes::Bytes;

use super::RenderCacheError;
use super::hot::HotEntry;
use super::key::RenderKey;

/// Coherence fence attached to a publication: a newer epoch or a higher
/// token within an epoch wins; anything else is fenced out.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PublicationFence {
    /// Authority epoch.
    pub epoch: u64,
    /// Digest of the observed generation set. Carried for inspection and for
    /// later coordination tiers; [`Self::supersedes`] compares only `epoch`
    /// and `token`, never this field.
    pub generation_digest: [u8; 32],
    /// Monotonic publication token from the rebuild coordinator.
    pub token: u64,
}

impl PublicationFence {
    /// Whether this fence may replace `current`.
    #[must_use]
    pub fn supersedes(&self, current: &Self) -> bool {
        self.epoch > current.epoch || (self.epoch == current.epoch && self.token > current.token)
    }
}

/// A stored entry as bytes plus its publication facts.
#[derive(Clone, Debug)]
pub struct StoredEntry {
    /// Encoded entry bytes; shared, never copied on read.
    pub bytes: Bytes,
    /// Publication time in Unix milliseconds.
    pub published_at_ms: u64,
    /// Fence the entry was published under.
    pub fence: PublicationFence,
}

/// Result of a publication attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublishOutcome {
    /// Stored and visible.
    Published,
    /// A newer or equal fence already holds the key.
    Fenced,
    /// The entry violates a store bound; nothing changed.
    Rejected,
}

/// Bounded store facts for inspection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoreInspection {
    /// Entries held.
    pub entries: usize,
    /// Bytes held.
    pub bytes: usize,
}

/// Provider contract for RenderCache bytes. Providers implement only what
/// they can prove: atomic publication and torn-write rejection.
#[async_trait]
pub trait RenderStore: Send + Sync {
    /// Returns the current entry for a key.
    async fn get(&self, key: &RenderKey) -> Result<Option<StoredEntry>, RenderCacheError>;
    /// Publishes atomically under a fence.
    ///
    /// `retention_ms` is a plain duration: the total milliseconds after
    /// `now_ms` beyond which a provider that ages entries off disk (such as
    /// a file-backed L1) may remove this one, regardless of its fence or
    /// epoch. `u64::MAX` means "never age-swept" - the correct value for a
    /// caller with no real retention to offer, such as an in-process store
    /// with no age-based expiry of its own, or a generic caller with no
    /// policy in scope. `0` is an ordinary, honoured value ("dead the
    /// instant it is published"), not a sentinel; a provider that has no
    /// concept of retention (an in-process LRU store, for example) is free
    /// to ignore it entirely, since it changes nothing it evicts on.
    async fn publish(
        &self,
        key: &RenderKey,
        bytes: Bytes,
        fence: PublicationFence,
        now_ms: u64,
        retention_ms: u64,
    ) -> Result<PublishOutcome, RenderCacheError>;
    /// Removes an entry.
    async fn evict(&self, key: &RenderKey) -> Result<(), RenderCacheError>;
    /// Bounded facts.
    async fn inspect(&self) -> Result<StoreInspection, RenderCacheError>;
}

/// Bounds of the in-process store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryStoreLimits {
    /// Most entries held.
    pub max_entries: usize,
    /// Most bytes held.
    pub max_bytes: usize,
}

/// Each key holds its stored bytes and, when the publication carried one,
/// the hot entry prepared from exactly those bytes. One map, so both are
/// published, evicted, and fenced together and can never disagree.
type Slot = (StoredEntry, Option<Arc<HotEntry>>);

struct MemoryState {
    entries: BTreeMap<RenderKey, Slot>,
    order: VecDeque<RenderKey>,
    bytes: usize,
}

/// Immutable in-process L0 store with least-recently-used eviction.
pub struct MemoryRenderStore {
    limits: MemoryStoreLimits,
    state: Mutex<MemoryState>,
}

impl MemoryRenderStore {
    /// Creates an empty store.
    #[must_use]
    pub fn new(limits: MemoryStoreLimits) -> Self {
        Self {
            limits,
            state: Mutex::new(MemoryState {
                entries: BTreeMap::new(),
                order: VecDeque::new(),
                bytes: 0,
            }),
        }
    }

    /// [`RenderStore::publish`] with a prepared hot entry beside the bytes,
    /// under the same fence, bound, and LRU rules. See [`Self::hot_get`].
    ///
    /// The caller owes one thing this store cannot check for itself: `hot`
    /// must have been prepared from exactly the entry `bytes` encodes, for
    /// exactly `key`. Checking it here would mean decoding on the
    /// publication path, which is the work a hot entry exists to avoid, so
    /// the obligation stays with the publisher and a debug build asserts
    /// the one part of it that is free to check - that the prepared entry
    /// names this key.
    ///
    /// `max_bytes` still counts only the encoded `bytes`, exactly as it
    /// does for a plain publication, while a hot slot additionally holds a
    /// decoded body and its formed header values. The bound is therefore a
    /// bound on stored encoded bytes, not on this store's total memory,
    /// once hot slots are in use.
    pub fn publish_hot(
        &self,
        key: &RenderKey,
        bytes: Bytes,
        hot: Arc<HotEntry>,
        fence: PublicationFence,
        now_ms: u64,
    ) -> PublishOutcome {
        debug_assert_eq!(
            hot.entry().header().key,
            *key,
            "a hot entry is published under the key it was prepared for"
        );
        self.publish_sync(key, bytes, Some(hot), fence, now_ms)
    }

    /// The hot entry published for `key`, if the current publication
    /// carried one. It takes no async step and allocates nothing.
    ///
    /// It touches the LRU order only when it hands back a hot entry. A key
    /// that is present without a hot slot is left untouched, since the
    /// caller must fall through to [`RenderStore::get`] for it and that
    /// call does the touching; a miss here is never a use.
    pub fn hot_get(&self, key: &RenderKey) -> Option<Arc<HotEntry>> {
        let mut state = self.lock_state();
        let hot = state.entries.get(key).and_then(|(_, hot)| hot.clone())?;
        Self::touch(&mut state.order, key);
        Some(hot)
    }

    /// The publication both entry points share; the trait's `publish`
    /// passes no hot entry, [`Self::publish_hot`] passes one, and every
    /// bound, fence, and eviction rule below applies to both.
    ///
    /// `retention_ms` never reaches here: this in-process store has no
    /// age-based expiry of its own - it only ever evicts under LRU pressure
    /// on insert (the `while` loop below) or a full `clear()` (an epoch
    /// advance) - so retention is accepted by the trait, per its own
    /// contract, and ignored rather than tracked for a sweep this store
    /// does not perform.
    ///
    /// Both bounds count the encoded `bytes` alone. A hot slot rides along
    /// free of charge against `max_bytes` while holding a decoded body, so
    /// with hot slots in use the byte bound bounds stored encoded bytes,
    /// not total memory. See [`Self::publish_hot`].
    fn publish_sync(
        &self,
        key: &RenderKey,
        bytes: Bytes,
        hot: Option<Arc<HotEntry>>,
        fence: PublicationFence,
        now_ms: u64,
    ) -> PublishOutcome {
        if self.limits.max_entries == 0
            || self.limits.max_bytes == 0
            || bytes.len() > self.limits.max_bytes
        {
            return PublishOutcome::Rejected;
        }
        let mut state = self.lock_state();
        if let Some((current, _)) = state.entries.get(key)
            && !fence.supersedes(&current.fence)
        {
            return PublishOutcome::Fenced;
        }
        if let Some((previous, _)) = state.entries.remove(key) {
            state.bytes -= previous.bytes.len();
        }
        while state.entries.len() >= self.limits.max_entries
            || state.bytes + bytes.len() > self.limits.max_bytes
        {
            let Some(oldest) = state.order.pop_front() else {
                break;
            };
            if let Some((evicted, _)) = state.entries.remove(&oldest) {
                state.bytes -= evicted.bytes.len();
            }
        }
        state.bytes += bytes.len();
        state.entries.insert(
            key.clone(),
            (
                StoredEntry {
                    bytes,
                    published_at_ms: now_ms,
                    fence,
                },
                hot,
            ),
        );
        Self::touch(&mut state.order, key);
        PublishOutcome::Published
    }

    fn touch(order: &mut VecDeque<RenderKey>, key: &RenderKey) {
        if let Some(position) = order.iter().position(|k| k == key) {
            order.remove(position);
        }
        order.push_back(key.clone());
    }

    /// Locks the state, recovering it from poison rather than propagating a
    /// panic across this store's operations.
    fn lock_state(&self) -> MutexGuard<'_, MemoryState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Removes every entry, resetting occupancy to empty.
    ///
    /// Used for an emergency epoch advance: every key this store holds
    /// embeds the epoch it was derived under (see
    /// [`crate::render_cache::key::RenderKey::derive`]), so the instant the
    /// authority epoch moves, every existing key is unreachable to an
    /// ordinary lookup - it names a key no future request can ever derive
    /// again. Unlike a file-backed store, there is no filesystem to
    /// reconcile a partial clear against, so a full, unconditional clear is
    /// both correct and free.
    pub fn clear(&self) {
        let mut state = self.lock_state();
        state.entries.clear();
        state.order.clear();
        state.bytes = 0;
    }
}

#[async_trait]
impl RenderStore for MemoryRenderStore {
    async fn get(&self, key: &RenderKey) -> Result<Option<StoredEntry>, RenderCacheError> {
        let mut state = self.lock_state();
        let Some(entry) = state.entries.get(key).map(|(entry, _)| entry.clone()) else {
            return Ok(None);
        };
        Self::touch(&mut state.order, key);
        Ok(Some(entry))
    }

    async fn publish(
        &self,
        key: &RenderKey,
        bytes: Bytes,
        fence: PublicationFence,
        now_ms: u64,
        _retention_ms: u64,
    ) -> Result<PublishOutcome, RenderCacheError> {
        Ok(self.publish_sync(key, bytes, None, fence, now_ms))
    }

    async fn evict(&self, key: &RenderKey) -> Result<(), RenderCacheError> {
        let mut state = self.lock_state();
        if let Some((previous, _)) = state.entries.remove(key) {
            state.bytes -= previous.bytes.len();
        }
        state.order.retain(|k| k != key);
        Ok(())
    }

    async fn inspect(&self) -> Result<StoreInspection, RenderCacheError> {
        let state = self.lock_state();
        Ok(StoreInspection {
            entries: state.entries.len(),
            bytes: state.bytes,
        })
    }
}
