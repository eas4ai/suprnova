//! Instance authority over a store that holds bytes: the
//! [`InstanceRecordStore`] port, the keys and outcomes a backend answers
//! with, and [`MemoryRecordStore`], the in-process reference implementation.
//!
//! A backend implements only atomicity and time. It never reads a record: the
//! kernel encodes one, hands the store opaque bytes with the version it read
//! them at, and the store either replaces them at that exact version or says
//! it could not. Everything a record means, and every transition between
//! records, belongs to the kernel that runs over this port.
//!
//! `DistributedInstanceLedger`, the kernel that implements
//! [`LiveInstanceLedger`](super::LiveInstanceLedger) over the port, arrives
//! with the state machine extracted from the Tier 0 provider. This module
//! owns what that kernel runs against, and [`MemoryRecordStore`] is both the
//! conformance reference every adapter must match and the kernel's test
//! double.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;

use super::{ClaimToken, LedgerError, LedgerErrorKind};
use crate::clock::Clock;
use crate::identity::{IdempotencyKey, InstanceId, ScopeFingerprint, UnixMillis};

/// The version an [`InstanceRecordStore`] gives a record it has just created.
const FIRST_VERSION: u64 = 1;

/// One instance record's address: the trusted scope that owns the instance
/// and the server-assigned identity within it.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct InstanceRecordKey {
    /// Trusted principal, session, and tenant scope the instance belongs to.
    pub scope: ScopeFingerprint,
    /// Server-assigned opaque instance identity.
    pub instance_id: InstanceId,
}

impl Ord for InstanceRecordKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.scope
            .as_bytes()
            .cmp(other.scope.as_bytes())
            .then_with(|| {
                self.instance_id
                    .as_bytes()
                    .cmp(other.instance_id.as_bytes())
            })
    }
}

impl PartialOrd for InstanceRecordKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// One promotion reservation's address: the trusted scope and the retry
/// identity a repeated promotion presents.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PromotionRecordKey {
    /// Trusted principal, session, and tenant scope the promotion belongs to.
    pub scope: ScopeFingerprint,
    /// Bounded retry identity the promotion was reserved under.
    pub idempotency_key: IdempotencyKey,
}

impl Ord for PromotionRecordKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.scope
            .as_bytes()
            .cmp(other.scope.as_bytes())
            .then_with(|| {
                self.idempotency_key
                    .as_bytes()
                    .cmp(other.idempotency_key.as_bytes())
            })
    }
}

impl PartialOrd for PromotionRecordKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A record as a store holds it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredRecord {
    /// The encoded record. Only the kernel's codec reads these bytes; a store
    /// keeps and returns them unchanged.
    pub bytes: Vec<u8>,
    /// The version these bytes were stored at. A compare-and-store that
    /// carries it replaces exactly this state of the record and nothing else.
    ///
    /// Versions belong to a record, not to a key: a record created again at a
    /// key that expired starts over, so a version identifies the state it was
    /// read from and never the record's lifetime.
    pub version: u64,
    /// Store-time deadline after which the record is gone. A store answers
    /// for an elapsed record exactly as it answers for one that was never
    /// written.
    pub expires_at: UnixMillis,
}

/// What one compare-and-store did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CasOutcome {
    /// The record was replaced and now carries `version`.
    Stored {
        /// The version the replacement was stored at.
        version: u64,
    },
    /// A record holds the key at a different version, so the caller's read is
    /// stale and its transition was not applied.
    Conflict,
    /// No unexpired record holds the key, so there was nothing to replace.
    Missing,
}

/// A claim retirement a synchronous drop queued for the kernel's next
/// asynchronous step.
///
/// Dropping an uncommitted claim must not block on remote input or output, so
/// the kernel records what to do and drains it later. The two operations are
/// not interchangeable: one returns the base revision to a retryable state,
/// the other must never do so.
#[derive(Debug)]
pub enum CleanupOp {
    /// Release the claim so an exact retry can take its base revision again.
    Abandon(ClaimToken),
    /// Retire the claim without ever restoring base-revision authority,
    /// because coordinated host effects may already have committed.
    Fence(ClaimToken),
}

/// Provider contract for distributed instance authority. A backend supplies
/// atomicity and time; it never interprets a record.
///
/// Every operation is decided against the store's own clock, inside the same
/// atomic step as the state it guards, so no node's clock can extend a record
/// it holds or shorten one it does not. Failures name the store: a backend
/// that cannot answer reports [`LedgerErrorKind::ProviderUnavailable`], never
/// a fabricated absence.
#[async_trait]
pub trait InstanceRecordStore: Send + Sync {
    /// Reads the record at `key`, or `None` when none is stored or the stored
    /// one has elapsed by store time.
    async fn load(&self, key: &InstanceRecordKey) -> Result<Option<StoredRecord>, LedgerError>;

    /// Creates the record at `key` when no unexpired record holds it,
    /// reporting whether this call is the one that created it. An elapsed
    /// record is not a holder and is replaced.
    async fn insert_if_absent(
        &self,
        key: &InstanceRecordKey,
        bytes: &[u8],
        expires_at: UnixMillis,
    ) -> Result<bool, LedgerError>;

    /// Replaces the record at `key` only while it still carries
    /// `expected_version`, advancing the version by one.
    async fn compare_and_store(
        &self,
        key: &InstanceRecordKey,
        expected_version: u64,
        bytes: &[u8],
        expires_at: UnixMillis,
    ) -> Result<CasOutcome, LedgerError>;

    /// Deletes the record at `key`. Deleting a record that is not there is
    /// not a failure.
    async fn remove(&self, key: &InstanceRecordKey) -> Result<(), LedgerError>;

    /// Reads the promotion reservation at `key` under the same rules as
    /// [`InstanceRecordStore::load`].
    async fn load_promotion(
        &self,
        key: &PromotionRecordKey,
    ) -> Result<Option<StoredRecord>, LedgerError>;

    /// Creates the promotion reservation at `key` under the same rules as
    /// [`InstanceRecordStore::insert_if_absent`].
    async fn insert_promotion_if_absent(
        &self,
        key: &PromotionRecordKey,
        bytes: &[u8],
        expires_at: UnixMillis,
    ) -> Result<bool, LedgerError>;

    /// Counts the unexpired instance records the store holds, which is what
    /// the kernel's configured instance capacity is checked against.
    async fn count_instances(&self) -> Result<usize, LedgerError>;

    /// Runs one queued synchronous cleanup ([`CleanupOp`]) that the backend
    /// can retire on its own.
    ///
    /// A store that cannot retire a claim without the kernel's own
    /// read-modify-write reports success and leaves the transition to the
    /// kernel's next step; it must never invent one, because it cannot read
    /// the record it would be transitioning.
    async fn apply_cleanup(&self, op: CleanupOp) -> Result<(), LedgerError>;
}

/// One key's row in [`MemoryRecordStore`].
struct RecordSlot {
    bytes: Vec<u8>,
    version: u64,
    expires_at: UnixMillis,
}

impl RecordSlot {
    fn stored(&self) -> StoredRecord {
        StoredRecord {
            bytes: self.bytes.clone(),
            version: self.version,
            expires_at: self.expires_at,
        }
    }
}

/// Both record tables under one lock, so each operation reads store time and
/// the state it guards as a single atomic step.
struct RecordState {
    instances: BTreeMap<InstanceRecordKey, RecordSlot>,
    promotions: BTreeMap<PromotionRecordKey, RecordSlot>,
}

/// In-process reference implementation of [`InstanceRecordStore`] over an
/// injected [`Clock`], which stands in for the backing store's clock.
///
/// It is the conformance reference for the port and the kernel's test double.
/// Records are opaque here as they are everywhere else: this store bounds
/// nothing about their contents, because the codec that writes them already
/// does.
pub struct MemoryRecordStore {
    clock: Arc<dyn Clock>,
    state: Mutex<RecordState>,
}

impl MemoryRecordStore {
    /// Creates an empty store that reads store time from `clock`.
    #[must_use]
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            state: Mutex::new(RecordState {
                instances: BTreeMap::new(),
                promotions: BTreeMap::new(),
            }),
        }
    }

    /// Locks both tables, reporting a lock this process has already poisoned
    /// as a provider failure rather than continuing over torn state.
    fn lock(&self) -> Result<MutexGuard<'_, RecordState>, LedgerError> {
        self.state
            .lock()
            .map_err(|_| LedgerError::new(LedgerErrorKind::ProviderUnavailable))
    }

    /// Reads store time. A clock the host cannot read decides no expiry, so it
    /// is reported rather than guessed.
    fn store_now(&self) -> Result<UnixMillis, LedgerError> {
        self.clock
            .now()
            .map_err(|_| LedgerError::new(LedgerErrorKind::ClockUnavailable))
    }
}

/// Reads the unexpired record at `key`, dropping an elapsed one as it goes so
/// a key that is read after its record died stops costing memory.
fn take_unexpired<K: Clone + Ord>(
    table: &mut BTreeMap<K, RecordSlot>,
    key: &K,
    now: UnixMillis,
) -> Option<StoredRecord> {
    match table.get(key) {
        Some(slot) if slot.expires_at > now => Some(slot.stored()),
        Some(_) => {
            table.remove(key);
            None
        }
        None => None,
    }
}

/// Creates the record at `key` unless an unexpired one holds it.
fn insert_absent<K: Clone + Ord>(
    table: &mut BTreeMap<K, RecordSlot>,
    key: &K,
    bytes: &[u8],
    expires_at: UnixMillis,
    now: UnixMillis,
) -> bool {
    if table.get(key).is_some_and(|slot| slot.expires_at > now) {
        return false;
    }
    table.insert(
        key.clone(),
        RecordSlot {
            bytes: bytes.to_vec(),
            version: FIRST_VERSION,
            expires_at,
        },
    );
    true
}

#[async_trait]
impl InstanceRecordStore for MemoryRecordStore {
    async fn load(&self, key: &InstanceRecordKey) -> Result<Option<StoredRecord>, LedgerError> {
        let mut state = self.lock()?;
        let now = self.store_now()?;
        Ok(take_unexpired(&mut state.instances, key, now))
    }

    async fn insert_if_absent(
        &self,
        key: &InstanceRecordKey,
        bytes: &[u8],
        expires_at: UnixMillis,
    ) -> Result<bool, LedgerError> {
        let mut state = self.lock()?;
        let now = self.store_now()?;
        Ok(insert_absent(
            &mut state.instances,
            key,
            bytes,
            expires_at,
            now,
        ))
    }

    async fn compare_and_store(
        &self,
        key: &InstanceRecordKey,
        expected_version: u64,
        bytes: &[u8],
        expires_at: UnixMillis,
    ) -> Result<CasOutcome, LedgerError> {
        let mut state = self.lock()?;
        let now = self.store_now()?;
        if state
            .instances
            .get(key)
            .is_some_and(|slot| slot.expires_at <= now)
        {
            state.instances.remove(key);
        }
        let Some(slot) = state.instances.get_mut(key) else {
            return Ok(CasOutcome::Missing);
        };
        if slot.version != expected_version {
            return Ok(CasOutcome::Conflict);
        }
        let version = slot
            .version
            .checked_add(1)
            .ok_or_else(|| LedgerError::new(LedgerErrorKind::CounterExhausted))?;
        slot.bytes = bytes.to_vec();
        slot.version = version;
        slot.expires_at = expires_at;
        Ok(CasOutcome::Stored { version })
    }

    async fn remove(&self, key: &InstanceRecordKey) -> Result<(), LedgerError> {
        self.lock()?.instances.remove(key);
        Ok(())
    }

    async fn load_promotion(
        &self,
        key: &PromotionRecordKey,
    ) -> Result<Option<StoredRecord>, LedgerError> {
        let mut state = self.lock()?;
        let now = self.store_now()?;
        Ok(take_unexpired(&mut state.promotions, key, now))
    }

    async fn insert_promotion_if_absent(
        &self,
        key: &PromotionRecordKey,
        bytes: &[u8],
        expires_at: UnixMillis,
    ) -> Result<bool, LedgerError> {
        let mut state = self.lock()?;
        let now = self.store_now()?;
        Ok(insert_absent(
            &mut state.promotions,
            key,
            bytes,
            expires_at,
            now,
        ))
    }

    /// Counting walks every instance record, so it reclaims the elapsed ones
    /// it passes: an adapter answers the same question with a bounded query
    /// over unexpired rows.
    async fn count_instances(&self) -> Result<usize, LedgerError> {
        let mut state = self.lock()?;
        let now = self.store_now()?;
        state.instances.retain(|_, slot| slot.expires_at > now);
        Ok(state.instances.len())
    }

    /// This store has no deferred work to run. Every record it holds is in
    /// this process behind one lock, so the kernel applies each queued
    /// cleanup through [`InstanceRecordStore::load`] and
    /// [`InstanceRecordStore::compare_and_store`] before its next step, and
    /// there is nothing to hand a backend. The store cannot read a record and
    /// therefore never invents a transition of its own.
    async fn apply_cleanup(&self, _op: CleanupOp) -> Result<(), LedgerError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::clock::ClockError;

    struct FixedClock {
        now: AtomicU64,
    }

    impl FixedClock {
        fn new(now: u64) -> Self {
            Self {
                now: AtomicU64::new(now),
            }
        }

        fn set(&self, now: u64) {
            self.now.store(now, Ordering::SeqCst);
        }
    }

    impl Clock for FixedClock {
        fn now(&self) -> Result<UnixMillis, ClockError> {
            Ok(UnixMillis::new(self.now.load(Ordering::SeqCst)))
        }
    }

    struct UnreadableClock;

    impl Clock for UnreadableClock {
        fn now(&self) -> Result<UnixMillis, ClockError> {
            Err(ClockError::timestamp_overflow())
        }
    }

    fn bytes<const LENGTH: usize>(start: u8) -> [u8; LENGTH] {
        std::array::from_fn(|offset| start.wrapping_add(offset as u8))
    }

    fn instance_key(start: u8) -> InstanceRecordKey {
        InstanceRecordKey {
            scope: ScopeFingerprint::from_bytes(&bytes::<32>(start)).expect("scope is valid"),
            instance_id: InstanceId::from_bytes(&bytes::<16>(start)).expect("instance is valid"),
        }
    }

    fn promotion_key(start: u8) -> PromotionRecordKey {
        PromotionRecordKey {
            scope: ScopeFingerprint::from_bytes(&bytes::<32>(start)).expect("scope is valid"),
            idempotency_key: IdempotencyKey::from_bytes(&bytes::<16>(start))
                .expect("idempotency key is valid"),
        }
    }

    fn stored_version(outcome: CasOutcome) -> u64 {
        match outcome {
            CasOutcome::Stored { version } => version,
            other => panic!("the record was expected to be replaced, not {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_record_is_created_once_and_then_belongs_to_its_holder() {
        let store = MemoryRecordStore::new(Arc::new(FixedClock::new(0)));
        let key = instance_key(0x10);

        assert!(
            store
                .insert_if_absent(&key, b"first", UnixMillis::new(1_000))
                .await
                .expect("the store answers"),
            "the first writer creates the record"
        );
        assert!(
            !store
                .insert_if_absent(&key, b"second", UnixMillis::new(1_000))
                .await
                .expect("the store answers"),
            "a second writer finds the key held"
        );

        let stored = store
            .load(&key)
            .await
            .expect("the store answers")
            .expect("the record is there");
        assert_eq!(stored.bytes, b"first".to_vec(), "the holder's bytes stand");
        assert_eq!(stored.version, FIRST_VERSION);
        assert_eq!(stored.expires_at, UnixMillis::new(1_000));
    }

    #[tokio::test]
    async fn compare_and_store_replaces_one_version_and_conflicts_on_any_other() {
        let store = MemoryRecordStore::new(Arc::new(FixedClock::new(0)));
        let key = instance_key(0x10);
        store
            .insert_if_absent(&key, b"first", UnixMillis::new(1_000))
            .await
            .expect("the store answers");

        let next = stored_version(
            store
                .compare_and_store(&key, FIRST_VERSION, b"second", UnixMillis::new(1_000))
                .await
                .expect("the store answers"),
        );
        assert_eq!(
            next,
            FIRST_VERSION + 1,
            "a replacement advances the version"
        );
        assert_eq!(
            store
                .compare_and_store(&key, FIRST_VERSION, b"third", UnixMillis::new(1_000))
                .await
                .expect("the store answers"),
            CasOutcome::Conflict,
            "the version that was already replaced no longer matches"
        );
        assert_eq!(
            store
                .load(&key)
                .await
                .expect("the store answers")
                .expect("the record is there")
                .bytes,
            b"second".to_vec(),
            "the conflicting write left the record alone"
        );
    }

    #[tokio::test]
    async fn compare_and_store_on_a_key_no_record_holds_is_missing() {
        let store = MemoryRecordStore::new(Arc::new(FixedClock::new(0)));

        assert_eq!(
            store
                .compare_and_store(
                    &instance_key(0x10),
                    FIRST_VERSION,
                    b"any",
                    UnixMillis::new(1_000)
                )
                .await
                .expect("the store answers"),
            CasOutcome::Missing
        );
    }

    #[tokio::test]
    async fn a_record_past_its_store_expiry_is_gone_and_its_key_is_free() {
        let clock = Arc::new(FixedClock::new(0));
        let store = MemoryRecordStore::new(clock.clone());
        let key = instance_key(0x10);
        store
            .insert_if_absent(&key, b"first", UnixMillis::new(1_000))
            .await
            .expect("the store answers");

        clock.set(1_000);
        assert!(
            store.load(&key).await.expect("the store answers").is_none(),
            "expiry is decided by store time"
        );
        assert_eq!(
            store
                .compare_and_store(&key, FIRST_VERSION, b"second", UnixMillis::new(2_000))
                .await
                .expect("the store answers"),
            CasOutcome::Missing,
            "an elapsed record cannot be replaced"
        );
        assert!(
            store
                .insert_if_absent(&key, b"second", UnixMillis::new(2_000))
                .await
                .expect("the store answers"),
            "an elapsed record does not hold its key"
        );
    }

    #[tokio::test]
    async fn removing_a_record_frees_its_key_and_missing_records_remove_cleanly() {
        let store = MemoryRecordStore::new(Arc::new(FixedClock::new(0)));
        let key = instance_key(0x10);
        store
            .insert_if_absent(&key, b"first", UnixMillis::new(1_000))
            .await
            .expect("the store answers");

        store.remove(&key).await.expect("the store answers");
        assert!(store.load(&key).await.expect("the store answers").is_none());
        store
            .remove(&key)
            .await
            .expect("removing what is not there is not a failure");
    }

    #[tokio::test]
    async fn promotion_records_follow_the_same_rules_in_their_own_key_space() {
        let clock = Arc::new(FixedClock::new(0));
        let store = MemoryRecordStore::new(clock.clone());
        let key = promotion_key(0x10);

        assert!(
            store
                .insert_promotion_if_absent(&key, b"reservation", UnixMillis::new(1_000))
                .await
                .expect("the store answers")
        );
        assert!(
            !store
                .insert_promotion_if_absent(&key, b"other", UnixMillis::new(1_000))
                .await
                .expect("the store answers")
        );
        assert_eq!(
            store
                .load_promotion(&key)
                .await
                .expect("the store answers")
                .expect("the reservation is there")
                .bytes,
            b"reservation".to_vec()
        );
        assert_eq!(
            store.count_instances().await.expect("the store answers"),
            0,
            "a reservation is not an instance"
        );

        clock.set(1_000);
        assert!(
            store
                .load_promotion(&key)
                .await
                .expect("the store answers")
                .is_none(),
            "reservations expire by store time too"
        );
    }

    #[tokio::test]
    async fn count_instances_counts_only_the_records_that_are_still_alive() {
        let clock = Arc::new(FixedClock::new(0));
        let store = MemoryRecordStore::new(clock.clone());
        store
            .insert_if_absent(&instance_key(0x10), b"short", UnixMillis::new(1_000))
            .await
            .expect("the store answers");
        store
            .insert_if_absent(&instance_key(0x20), b"long", UnixMillis::new(3_000))
            .await
            .expect("the store answers");

        assert_eq!(store.count_instances().await.expect("the store answers"), 2);
        clock.set(1_000);
        assert_eq!(
            store.count_instances().await.expect("the store answers"),
            1,
            "an elapsed record is not held against the instance capacity"
        );
        clock.set(3_000);
        assert_eq!(store.count_instances().await.expect("the store answers"), 0);
    }

    #[tokio::test]
    async fn a_store_clock_that_cannot_be_read_decides_nothing() {
        let store = MemoryRecordStore::new(Arc::new(UnreadableClock));

        assert_eq!(
            store
                .load(&instance_key(0x10))
                .await
                .expect_err("no expiry decision is possible")
                .kind(),
            LedgerErrorKind::ClockUnavailable
        );
        assert_eq!(
            store
                .insert_if_absent(&instance_key(0x10), b"first", UnixMillis::new(1_000))
                .await
                .expect_err("no expiry decision is possible")
                .kind(),
            LedgerErrorKind::ClockUnavailable
        );
    }

    #[tokio::test]
    async fn a_queued_cleanup_is_accepted_without_the_store_inventing_a_transition() {
        let store = MemoryRecordStore::new(Arc::new(FixedClock::new(0)));
        let key = instance_key(0x10);
        store
            .insert_if_absent(&key, b"first", UnixMillis::new(1_000))
            .await
            .expect("the store answers");
        let token = || ClaimToken {
            provider_identity: Arc::new(()),
            scope: key.scope.clone(),
            instance_id: key.instance_id.clone(),
            claim_id: 1,
        };

        store
            .apply_cleanup(CleanupOp::Abandon(token()))
            .await
            .expect("the store answers");
        store
            .apply_cleanup(CleanupOp::Fence(token()))
            .await
            .expect("the store answers");

        let stored = store
            .load(&key)
            .await
            .expect("the store answers")
            .expect("the record is there");
        assert_eq!(stored.bytes, b"first".to_vec());
        assert_eq!(
            stored.version, FIRST_VERSION,
            "the store cannot read a record, so it never rewrites one"
        );
    }
}
