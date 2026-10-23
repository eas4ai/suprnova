//! Replay logs for asynchronous subscriptions, and the one memory budget
//! every log shares.
//!
//! A replay log keeps a subscription's recent events so a browser that
//! reconnects catches up without a fresh render, and it is also the source
//! the subscription's transport reads its next event from. Two settings bound
//! it. `LIVE_ASYNC_MAX_REPLAY_BYTES` bounds one log, so no subscription holds
//! more than its share. `LIVE_ASYNC_REPLAY_BUDGET_BYTES` bounds every log in
//! the process together, so the total stays fixed however many sessions
//! connect: past it, the oldest entries in any log are evicted first. A
//! browser that needed an evicted entry sees a gap and re-renders fresh,
//! which is the recovery the protocol already has.
//!
//! Each entry is charged what it keeps in memory: the typed envelope's heap
//! (its payload tree, measured node by node) plus the entry's own bookkeeping.
//! The encoded form is not retained; a replay encodes its entries when a
//! browser asks for them.

use std::collections::VecDeque;
use std::mem::size_of;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::task::Waker;

use suprnova_live::canonical::CanonicalValue;

/// What one retained item keeps in memory beyond its own inline size.
pub(crate) trait RetainedBytes {
    /// Heap bytes the item holds: strings, collections and their contents.
    fn retained_bytes(&self) -> usize;
}

/// Heap bytes one canonical value holds, counted node by node: a string's
/// capacity, an array's slots and their contents, and an object's keys, values
/// and the map's per-entry bookkeeping.
pub(crate) fn canonical_heap_bytes(value: &CanonicalValue) -> usize {
    match value {
        CanonicalValue::String(text) => text.capacity(),
        CanonicalValue::Array(items) => items
            .capacity()
            .saturating_mul(size_of::<CanonicalValue>())
            .saturating_add(items.iter().map(canonical_heap_bytes).sum::<usize>()),
        CanonicalValue::Object(map) => map
            .iter()
            .map(|(key, item)| {
                key.capacity()
                    .saturating_add(BTREE_ENTRY_BYTES)
                    .saturating_add(canonical_heap_bytes(item))
            })
            .sum(),
        CanonicalValue::Null | CanonicalValue::Bool(_) | CanonicalValue::Number(_) => 0,
    }
}

/// One object member's share of its map: the key and value slots, and a
/// pointer's worth of node overhead (a B-tree node holds up to eleven).
const BTREE_ENTRY_BYTES: usize =
    size_of::<String>() + size_of::<CanonicalValue>() + size_of::<usize>();

/// One retained item with its sequence and the bytes it is charged.
struct ReplayEntry<T> {
    sequence: u64,
    item: T,
    cost: usize,
}

/// One log's place in the shared budget's eviction order.
struct BudgetSlot<T> {
    log: Weak<Mutex<ReplayLog<T>>>,
    sequence: u64,
}

/// Bookkeeping one entry costs besides its item's heap: the entry itself and
/// its slot in the budget's eviction order.
const fn entry_overhead<T>() -> usize {
    size_of::<ReplayEntry<T>>() + size_of::<BudgetSlot<T>>()
}

/// A bounded, ordered log of one subscription's items.
pub(crate) struct ReplayLog<T> {
    epoch: u64,
    next_sequence: u64,
    entries: VecDeque<ReplayEntry<T>>,
    bytes: usize,
    /// `LIVE_ASYNC_MAX_REPLAY_BYTES`: this log's own share.
    max_bytes: usize,
    /// `LIVE_ASYNC_MAX_REPLAY_EVENTS`: as many entries as one replay may
    /// carry, and no more.
    max_entries: usize,
    budget: Arc<ReplayBudget<T>>,
    /// Wakes the transport waiting for this log's next entry.
    pub(crate) waker: Option<Waker>,
    /// When the last entry was appended, for heartbeats on idle logs.
    pub(crate) last_append_ms: u64,
}

impl<T: RetainedBytes> ReplayLog<T> {
    pub(crate) fn new(
        epoch: u64,
        now_ms: u64,
        max_bytes: usize,
        max_entries: usize,
        budget: Arc<ReplayBudget<T>>,
    ) -> Self {
        Self {
            epoch,
            next_sequence: 1,
            entries: VecDeque::new(),
            bytes: 0,
            max_bytes,
            max_entries,
            budget,
            waker: None,
            last_append_ms: now_ms,
        }
    }

    pub(crate) const fn epoch(&self) -> u64 {
        self.epoch
    }

    /// The sequence the next appended item receives.
    pub(crate) const fn next_sequence(&self) -> u64 {
        self.next_sequence
    }

    pub(crate) const fn head(&self) -> u64 {
        self.next_sequence - 1
    }

    pub(crate) fn oldest(&self) -> Option<u64> {
        self.entries.front().map(|entry| entry.sequence)
    }

    /// Bytes this log is charged for.
    #[cfg(test)]
    pub(crate) const fn bytes(&self) -> usize {
        self.bytes
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Appends one item, evicting this log's oldest entries past its own
    /// limits, and returns the item's sequence. The newest entry always
    /// stays: the transport has not read it yet. `this` is the log's own
    /// handle, which the shared budget keeps to evict from it later.
    pub(crate) fn append(&mut self, this: &Arc<Mutex<Self>>, item: T, now_ms: u64) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        let cost = item.retained_bytes().saturating_add(entry_overhead::<T>());
        self.bytes = self.bytes.saturating_add(cost);
        self.entries.push_back(ReplayEntry {
            sequence,
            item,
            cost,
        });
        self.budget.admit(Arc::downgrade(this), sequence, cost);
        let mut freed = 0_usize;
        let mut evicted = 0_usize;
        while self.entries.len() > 1
            && (self.entries.len() > self.max_entries || self.bytes > self.max_bytes)
        {
            let Some(entry) = self.entries.pop_front() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(entry.cost);
            freed = freed.saturating_add(entry.cost);
            evicted += 1;
        }
        if evicted > 0 {
            self.budget.release(freed, evicted);
        }
        self.last_append_ms = now_ms;
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
        sequence
    }

    pub(crate) fn entry_at(&self, sequence: u64) -> Option<&T> {
        let first = self.oldest()?;
        if sequence < first {
            return None;
        }
        let index = usize::try_from(sequence - first).ok()?;
        self.entries
            .get(index)
            .filter(|entry| entry.sequence == sequence)
            .map(|entry| &entry.item)
    }

    /// Every retained item after `position`, or `None` when part of the tail
    /// was evicted and only a fresh render can recover it.
    pub(crate) fn tail_after(&self, epoch: u64, sequence: u64) -> Option<Vec<&T>> {
        if epoch != self.epoch || sequence > self.head() {
            return None;
        }
        if sequence == self.head() {
            return Some(Vec::new());
        }
        let first_needed = sequence.saturating_add(1);
        if self.oldest()? > first_needed {
            return None;
        }
        Some(
            self.entries
                .iter()
                .filter(|entry| entry.sequence >= first_needed)
                .map(|entry| &entry.item)
                .collect(),
        )
    }

    fn contains(&self, sequence: u64) -> bool {
        self.oldest()
            .is_some_and(|oldest| oldest <= sequence && sequence <= self.head())
    }

    /// Evicts every entry up to `sequence`, for the shared budget.
    fn evict_through(&mut self, sequence: u64) {
        let mut freed = 0_usize;
        let mut evicted = 0_usize;
        while self
            .entries
            .front()
            .is_some_and(|entry| entry.sequence <= sequence)
        {
            let Some(entry) = self.entries.pop_front() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(entry.cost);
            freed = freed.saturating_add(entry.cost);
            evicted += 1;
        }
        if evicted > 0 {
            self.budget.release(freed, evicted);
        }
    }
}

impl<T> Drop for ReplayLog<T> {
    fn drop(&mut self) {
        if !self.entries.is_empty() {
            self.budget.release(self.bytes, self.entries.len());
        }
    }
}

/// The memory every replay log in the process shares
/// (`LIVE_ASYNC_REPLAY_BUDGET_BYTES`), with the order entries were appended
/// in, so the oldest entry anywhere is evicted first.
///
/// Lock order: a log's lock may be held while the budget's is taken, never
/// the other way round. The budget releases its own lock before it locks a
/// log to evict from it.
pub(crate) struct ReplayBudget<T> {
    max_bytes: usize,
    state: Mutex<BudgetState<T>>,
}

struct BudgetState<T> {
    used: usize,
    /// Entries alive in every log, to know when the order holds mostly
    /// entries the logs already evicted on their own.
    live: usize,
    order: VecDeque<BudgetSlot<T>>,
    compacting: bool,
}

/// Slots the order may hold beyond twice the live entries before it is
/// compacted.
const COMPACT_SLACK: usize = 1_024;

impl<T> ReplayBudget<T> {
    pub(crate) fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            state: Mutex::new(BudgetState {
                used: 0,
                live: 0,
                order: VecDeque::new(),
                compacting: false,
            }),
        }
    }

    /// Bytes every log is charged for together.
    #[cfg(test)]
    pub(crate) fn used(&self) -> usize {
        self.lock().used
    }

    fn lock(&self) -> MutexGuard<'_, BudgetState<T>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn admit(&self, log: Weak<Mutex<ReplayLog<T>>>, sequence: u64, cost: usize) {
        let mut state = self.lock();
        state.used = state.used.saturating_add(cost);
        state.live = state.live.saturating_add(1);
        state.order.push_back(BudgetSlot { log, sequence });
    }

    fn release(&self, cost: usize, entries: usize) {
        let mut state = self.lock();
        state.used = state.used.saturating_sub(cost);
        state.live = state.live.saturating_sub(entries);
    }
}

impl<T: RetainedBytes> ReplayBudget<T> {
    /// Evicts the oldest entries in any log until every log together fits
    /// the budget. Call it with no log locked.
    pub(crate) fn enforce(&self) {
        loop {
            let slot = {
                let mut state = self.lock();
                if state.used <= self.max_bytes {
                    break;
                }
                match state.order.pop_front() {
                    Some(slot) => slot,
                    None => break,
                }
            };
            if let Some(log) = slot.log.upgrade() {
                log.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .evict_through(slot.sequence);
            }
        }
        self.compact_if_sparse();
    }

    /// Drops order slots whose entries their logs already evicted, once
    /// they outnumber the live entries, so the order stays proportional to
    /// what the logs hold.
    fn compact_if_sparse(&self) {
        let taken = {
            let mut state = self.lock();
            if state.compacting
                || state.order.len() <= state.live.saturating_mul(2).saturating_add(COMPACT_SLACK)
            {
                return;
            }
            state.compacting = true;
            std::mem::take(&mut state.order)
        };
        let kept = taken
            .into_iter()
            .filter(|slot| {
                slot.log.upgrade().is_some_and(|log| {
                    log.lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .contains(slot.sequence)
                })
            })
            .collect::<VecDeque<_>>();
        let mut state = self.lock();
        let newer = std::mem::replace(&mut state.order, kept);
        state.order.extend(newer);
        state.compacting = false;
    }

    #[cfg(test)]
    fn slots(&self) -> usize {
        self.lock().order.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An item charged exactly the heap bytes it declares.
    struct Item(usize);

    impl RetainedBytes for Item {
        fn retained_bytes(&self) -> usize {
            self.0
        }
    }

    type Log = Arc<Mutex<ReplayLog<Item>>>;

    fn log(budget: &Arc<ReplayBudget<Item>>, max_bytes: usize, max_entries: usize) -> Log {
        Arc::new(Mutex::new(ReplayLog::new(
            1,
            0,
            max_bytes,
            max_entries,
            Arc::clone(budget),
        )))
    }

    /// Appends as the framework does: the log locked for the append, then
    /// the budget enforced with no log locked.
    fn append(log: &Log, bytes: usize) -> u64 {
        let sequence = log.lock().expect("log").append(log, Item(bytes), 0);
        let budget = Arc::clone(&log.lock().expect("log").budget);
        budget.enforce();
        sequence
    }

    const KIB: usize = 1024;

    #[test]
    fn one_log_keeps_its_own_share_and_its_newest_entry() {
        let budget = Arc::new(ReplayBudget::new(1024 * KIB));
        let first = log(&budget, 10 * KIB, 4_096);
        for _ in 0..5 {
            append(&first, 4 * KIB);
        }
        let guard = first.lock().expect("log");
        assert!(guard.bytes() <= 10 * KIB, "{}", guard.bytes());
        assert_eq!(guard.len(), 2);
        assert_eq!(guard.oldest(), Some(4));
        drop(guard);
        // One entry larger than the share still stays until the next one.
        append(&first, 64 * KIB);
        assert_eq!(first.lock().expect("log").len(), 1);
        assert_eq!(budget.used(), first.lock().expect("log").bytes());
    }

    #[test]
    fn the_shared_budget_evicts_the_oldest_entry_in_any_log_first() {
        let budget = Arc::new(ReplayBudget::new(40 * KIB));
        let older = log(&budget, 1024 * KIB, 4_096);
        let newer = log(&budget, 1024 * KIB, 4_096);
        for _ in 0..3 {
            append(&older, 8 * KIB);
        }
        for _ in 0..3 {
            append(&newer, 8 * KIB);
        }
        assert!(budget.used() <= 40 * KIB, "{}", budget.used());
        // The first entries the older log appended went first; the newer
        // log kept all of its own.
        assert_eq!(older.lock().expect("log").oldest(), Some(3));
        assert_eq!(newer.lock().expect("log").len(), 3);
        assert_eq!(
            budget.used(),
            older.lock().expect("log").bytes() + newer.lock().expect("log").bytes()
        );
    }

    #[test]
    fn a_dropped_log_returns_its_bytes_and_stale_slots_are_compacted() {
        let budget = Arc::new(ReplayBudget::new(1024 * 1024 * KIB));
        let short = log(&budget, 1024 * 1024 * KIB, 1);
        for _ in 0..(3 * COMPACT_SLACK) {
            append(&short, 16);
        }
        // The log keeps one entry; the order kept no more than the slack
        // beyond twice that.
        assert_eq!(short.lock().expect("log").len(), 1);
        assert!(
            budget.slots() <= 2 + COMPACT_SLACK + 1,
            "{}",
            budget.slots()
        );
        let held = log(&budget, 1024 * KIB, 4_096);
        append(&held, KIB);
        drop(short);
        assert_eq!(budget.used(), held.lock().expect("log").bytes());
        drop(held);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn a_canonical_tree_is_charged_more_than_its_encoded_bytes() {
        let tree = CanonicalValue::Array(
            (0..1_000)
                .map(|index| CanonicalValue::String(format!("item-{index}")))
                .collect(),
        );
        let encoded = suprnova_live::canonical::to_canonical_bytes(
            &tree,
            &suprnova_live::limits::InputLimits::default(),
        )
        .expect("encode");
        assert!(
            canonical_heap_bytes(&tree) > encoded.len() * 2,
            "{} heap bytes for {} encoded",
            canonical_heap_bytes(&tree),
            encoded.len()
        );
    }
}
