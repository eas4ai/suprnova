//! The RenderStore contract and the immutable in-process L0 store.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use bytes::Bytes;

use super::RenderCacheError;
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

struct MemoryState {
    entries: BTreeMap<RenderKey, StoredEntry>,
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
        let Some(entry) = state.entries.get(key).cloned() else {
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
        // This in-process store has no age-based expiry of its own - it
        // only ever evicts under LRU pressure on insert (`while` loop
        // below) or a full `clear()` (an epoch advance) - so retention is
        // accepted, per the trait's own contract, and ignored rather than
        // tracked for a sweep this store does not perform.
        if self.limits.max_entries == 0
            || self.limits.max_bytes == 0
            || bytes.len() > self.limits.max_bytes
        {
            return Ok(PublishOutcome::Rejected);
        }
        let mut state = self.lock_state();
        if let Some(current) = state.entries.get(key)
            && !fence.supersedes(&current.fence)
        {
            return Ok(PublishOutcome::Fenced);
        }
        if let Some(previous) = state.entries.remove(key) {
            state.bytes -= previous.bytes.len();
        }
        while state.entries.len() >= self.limits.max_entries
            || state.bytes + bytes.len() > self.limits.max_bytes
        {
            let Some(oldest) = state.order.pop_front() else {
                break;
            };
            if let Some(evicted) = state.entries.remove(&oldest) {
                state.bytes -= evicted.bytes.len();
            }
        }
        state.bytes += bytes.len();
        state.entries.insert(
            key.clone(),
            StoredEntry {
                bytes,
                published_at_ms: now_ms,
                fence,
            },
        );
        Self::touch(&mut state.order, key);
        Ok(PublishOutcome::Published)
    }

    async fn evict(&self, key: &RenderKey) -> Result<(), RenderCacheError> {
        let mut state = self.lock_state();
        if let Some(previous) = state.entries.remove(key) {
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
