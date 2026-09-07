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
//! [`DistributedInstanceLedger`] is the kernel that implements
//! [`LiveInstanceLedger`] over the port. It owns no rule of its own: every
//! decision is a pure function in [`state`](super::state), and this module is
//! the loop that reads a record, applies one of those functions, and writes
//! the result back at exactly the version it read. `MemoryInstanceLedger` is
//! this kernel over [`MemoryRecordStore`], so Tier 0 and every distributed
//! tier run one state machine and answer one conformance suite.

use std::cmp::Ordering;
use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;

use super::record::{decode_record, decode_reservation, encode_record, encode_reservation};
use super::state::{self, ClaimDecision, InstanceRecord, PromotionReservation};
use super::{
    AcceptedOutcome, ClaimGrant, ClaimOutcome, ClaimRequest, ClaimToken, InstanceAuthority,
    LedgerError, LedgerErrorKind, LedgerInspection, LedgerLimits, LiveInstanceLedger,
    MountInstanceRecord, PromotionOutcome, PromotionRecord, RefreshReason,
};
use crate::clock::Clock;
use crate::identity::{IdempotencyKey, InstanceId, Revision, ScopeFingerprint, UnixMillis};

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
#[derive(Clone, Eq, PartialEq)]
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

impl fmt::Debug for StoredRecord {
    /// Prints the record's version, expiry, and size, never its bytes.
    ///
    /// Those bytes are an encoded record: revision metadata and the
    /// identities that scope it. This crate redacts identities everywhere
    /// else, and a derived `Debug` would put them in any line that formats a
    /// store's answer.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoredRecord")
            .field("version", &self.version)
            .field("expires_at", &self.expires_at)
            .field(
                "bytes",
                &format_args!("<{} bytes:redacted>", self.bytes.len()),
            )
            .finish()
    }
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

impl CleanupOp {
    /// The claim this retirement acts on.
    pub(crate) const fn token(&self) -> &ClaimToken {
        match self {
            Self::Abandon(token) | Self::Fence(token) => token,
        }
    }

    /// Whether retiring this claim lets store time decide what holds the key.
    const fn timing(&self) -> Timing {
        match self {
            Self::Abandon(_) => Timing::Elapsing,
            Self::Fence(_) => Timing::Regardless,
        }
    }
}

/// Whether one store operation lets store time decide what is there.
#[derive(Clone, Copy)]
enum Timing {
    /// An elapsed record is gone, which is what the port promises and what
    /// every operation that grants or restores authority is decided under.
    Elapsing,
    /// Whatever holds the key is there. Fencing a claim never restores
    /// authority and is retiring it precisely because something failed, so it
    /// must not need a clock the host may be unable to read.
    Regardless,
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
    ///
    /// A retirement a synchronous drop queued is not part of this port: the
    /// kernel drains its own queue through [`InstanceRecordStore::load`] and
    /// [`InstanceRecordStore::compare_and_store`], exactly as it runs every
    /// other operation, so a backend never has to read a record to retire a
    /// claim it cannot interpret.
    async fn count_instances(&self) -> Result<usize, LedgerError>;
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

/// Elapsed records one store operation may reclaim.
///
/// Reclamation is bounded so that no single operation pays for a burst of
/// expiries that arrived together: a store holding a million records that
/// all elapse at once still answers in constant time, and the backlog drains
/// over the operations that follow. Every adapter owes the same rule, which
/// for a SQL or Redis store means a sweep statement with a batch limit
/// (`DELETE ... WHERE expires_at_ms <= ? LIMIT 64`, or a bounded scan over a
/// sorted expiry index) rather than an unbounded delete.
const MAX_RECLAIMED_PER_OPERATION: usize = 64;

/// One key a deadline entry reclaims, in whichever table holds it.
enum Reclaimable {
    Instance(InstanceRecordKey),
    Promotion(PromotionRecordKey),
}

/// Both record tables under one lock, so each operation reads store time and
/// the state it guards as a single atomic step.
///
/// `deadlines` is the reclamation index: one entry per record written, in
/// deadline order, so an operation finds the records that have elapsed
/// without walking the ones that have not, and counting instances stays a
/// map length rather than a scan.
struct RecordState {
    instances: BTreeMap<InstanceRecordKey, RecordSlot>,
    promotions: BTreeMap<PromotionRecordKey, RecordSlot>,
    deadlines: BTreeMap<UnixMillis, Vec<Reclaimable>>,
}

impl RecordState {
    /// Records that the key at `deadline` becomes reclaimable then.
    fn schedule(&mut self, deadline: UnixMillis, key: Reclaimable) {
        self.deadlines.entry(deadline).or_default().push(key);
    }

    /// Drops at most [`MAX_RECLAIMED_PER_OPERATION`] elapsed records.
    ///
    /// An entry whose record was already removed, replaced, or given another
    /// deadline reclaims nothing and is simply spent, which is what keeps the
    /// index from having to be kept in step with every write.
    fn reclaim(&mut self, now: UnixMillis) {
        for _ in 0..MAX_RECLAIMED_PER_OPERATION {
            let Some((deadline, key)) = self.pop_due(now) else {
                break;
            };
            match key {
                Reclaimable::Instance(key) => {
                    if self
                        .instances
                        .get(&key)
                        .is_some_and(|slot| slot.expires_at == deadline)
                    {
                        self.instances.remove(&key);
                    }
                }
                Reclaimable::Promotion(key) => {
                    if self
                        .promotions
                        .get(&key)
                        .is_some_and(|slot| slot.expires_at == deadline)
                    {
                        self.promotions.remove(&key);
                    }
                }
            }
        }
    }

    /// Takes one scheduled key whose deadline has passed.
    fn pop_due(&mut self, now: UnixMillis) -> Option<(UnixMillis, Reclaimable)> {
        loop {
            let mut entry = self.deadlines.first_entry()?;
            let deadline = *entry.key();
            if deadline > now {
                return None;
            }
            let key = entry.get_mut().pop();
            if entry.get().is_empty() {
                entry.remove();
            }
            if let Some(key) = key {
                return Some((deadline, key));
            }
        }
    }
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
                deadlines: BTreeMap::new(),
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

/// The store's operations, each one atomic under the table lock.
///
/// They are inherent and synchronous because nothing an in-process store does
/// can yield. The [`InstanceRecordStore`] implementation below is these
/// methods and nothing else, and the Tier 0 ledger reaches the same methods
/// directly when a synchronous drop has to retire a claim without an
/// executor.
impl MemoryRecordStore {
    fn load_record(
        &self,
        key: &InstanceRecordKey,
        timing: Timing,
    ) -> Result<Option<StoredRecord>, LedgerError> {
        let mut state = self.lock()?;
        match timing {
            Timing::Elapsing => {
                let now = self.store_now()?;
                state.reclaim(now);
                Ok(take_unexpired(&mut state.instances, key, now))
            }
            Timing::Regardless => Ok(state.instances.get(key).map(RecordSlot::stored)),
        }
    }

    fn insert_record_if_absent(
        &self,
        key: &InstanceRecordKey,
        bytes: &[u8],
        expires_at: UnixMillis,
    ) -> Result<bool, LedgerError> {
        let mut state = self.lock()?;
        let now = self.store_now()?;
        state.reclaim(now);
        if !insert_absent(&mut state.instances, key, bytes, expires_at, now) {
            return Ok(false);
        }
        state.schedule(expires_at, Reclaimable::Instance(key.clone()));
        Ok(true)
    }

    fn replace_record(
        &self,
        key: &InstanceRecordKey,
        expected_version: u64,
        bytes: &[u8],
        expires_at: UnixMillis,
        timing: Timing,
    ) -> Result<CasOutcome, LedgerError> {
        let mut state = self.lock()?;
        if let Timing::Elapsing = timing {
            let now = self.store_now()?;
            state.reclaim(now);
            if state
                .instances
                .get(key)
                .is_some_and(|slot| slot.expires_at <= now)
            {
                state.instances.remove(key);
            }
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
        let rescheduled = slot.expires_at != expires_at;
        slot.bytes = bytes.to_vec();
        slot.version = version;
        slot.expires_at = expires_at;
        // A replacement that keeps the record's deadline keeps its index
        // entry too, so rewriting one record a thousand times costs the index
        // nothing.
        if rescheduled {
            state.schedule(expires_at, Reclaimable::Instance(key.clone()));
        }
        Ok(CasOutcome::Stored { version })
    }

    fn remove_record(&self, key: &InstanceRecordKey) -> Result<(), LedgerError> {
        self.lock()?.instances.remove(key);
        Ok(())
    }

    fn load_reservation(
        &self,
        key: &PromotionRecordKey,
    ) -> Result<Option<StoredRecord>, LedgerError> {
        let mut state = self.lock()?;
        let now = self.store_now()?;
        state.reclaim(now);
        Ok(take_unexpired(&mut state.promotions, key, now))
    }

    fn insert_reservation_if_absent(
        &self,
        key: &PromotionRecordKey,
        bytes: &[u8],
        expires_at: UnixMillis,
    ) -> Result<bool, LedgerError> {
        let mut state = self.lock()?;
        let now = self.store_now()?;
        state.reclaim(now);
        if !insert_absent(&mut state.promotions, key, bytes, expires_at, now) {
            return Ok(false);
        }
        state.schedule(expires_at, Reclaimable::Promotion(key.clone()));
        Ok(true)
    }

    /// Counting is a map length: elapsed records are reclaimed by the bounded
    /// sweep every operation runs, not by walking the table here. An adapter
    /// answers the same question with a count over unexpired rows and sweeps
    /// in batches of the same bounded size.
    fn count_records(&self) -> Result<usize, LedgerError> {
        let mut state = self.lock()?;
        let now = self.store_now()?;
        state.reclaim(now);
        Ok(state.instances.len())
    }
}

#[async_trait]
impl InstanceRecordStore for MemoryRecordStore {
    async fn load(&self, key: &InstanceRecordKey) -> Result<Option<StoredRecord>, LedgerError> {
        self.load_record(key, Timing::Elapsing)
    }

    async fn insert_if_absent(
        &self,
        key: &InstanceRecordKey,
        bytes: &[u8],
        expires_at: UnixMillis,
    ) -> Result<bool, LedgerError> {
        self.insert_record_if_absent(key, bytes, expires_at)
    }

    async fn compare_and_store(
        &self,
        key: &InstanceRecordKey,
        expected_version: u64,
        bytes: &[u8],
        expires_at: UnixMillis,
    ) -> Result<CasOutcome, LedgerError> {
        self.replace_record(key, expected_version, bytes, expires_at, Timing::Elapsing)
    }

    async fn remove(&self, key: &InstanceRecordKey) -> Result<(), LedgerError> {
        self.remove_record(key)
    }

    async fn load_promotion(
        &self,
        key: &PromotionRecordKey,
    ) -> Result<Option<StoredRecord>, LedgerError> {
        self.load_reservation(key)
    }

    async fn insert_promotion_if_absent(
        &self,
        key: &PromotionRecordKey,
        bytes: &[u8],
        expires_at: UnixMillis,
    ) -> Result<bool, LedgerError> {
        self.insert_reservation_if_absent(key, bytes, expires_at)
    }

    async fn count_instances(&self) -> Result<usize, LedgerError> {
        self.count_records()
    }
}

/// Attempts one operation makes before it reports contention: the first
/// read, and one more from a fresh read after a stale one.
const APPLY_ATTEMPTS: usize = 2;

/// How long a store keeps an instance record after the instance lifetime the
/// node clock measures has run out.
///
/// The node clock decides the lifetime and the store's expiry decides
/// eviction, so the two need a gap, and the gap buys two things. A node whose
/// clock trails the store's still finds the record it is entitled to decide
/// about, which is the clock-skew allowance the rest of the crate gives at
/// the same 60 seconds. And a request that arrives just after an instance
/// elapsed is told its instance expired, the exact fresh-render reason,
/// rather than that it never existed.
///
/// A promotion reservation gets no such window: its retry identity has to be
/// free the instant it elapses, or an exact retry after expiry could recover
/// authority over an instance that is gone.
const ELAPSED_RECORD_RETENTION_MS: u64 = 60_000;

/// The store deadline one record's instance expiry earns.
fn store_expiry(expires_at: UnixMillis) -> UnixMillis {
    UnixMillis::new(expires_at.get().saturating_add(ELAPSED_RECORD_RETENTION_MS))
}

/// What one load found at an instance key.
///
/// It carries no expiry decision. Node time is read by the transitions that
/// need it and by nothing else, so retiring a claim whose host effects may
/// have committed still works when the clock provider does not.
enum Loaded {
    /// No record holds the key: either none was ever written, or the store
    /// evicted the one that did.
    Absent,
    /// A record holds the key.
    Present {
        /// The decoded record.
        record: InstanceRecord,
        /// The version it was read at, which is the only version a write
        /// derived from this read may replace.
        version: u64,
    },
}

/// One attempt's decision: what it observed, and the record it must store
/// first for that observation to be true.
struct Attempt<T> {
    /// The version to replace and the record to replace it with, or `None`
    /// when the attempt read the record and changed nothing.
    write: Option<(u64, InstanceRecord)>,
    /// What the caller observes once any write lands.
    outcome: T,
}

impl<T> Attempt<T> {
    /// An attempt that decided without writing.
    const fn read(outcome: T) -> Self {
        Self {
            write: None,
            outcome,
        }
    }

    /// An attempt whose transition must reach the store at `version`.
    fn from_transition<D>(transition: state::Transition<D>, version: u64, outcome: T) -> Self {
        Self {
            write: transition.stored.map(|record| (version, record)),
            outcome,
        }
    }
}

/// Whether one creation attempt created the record, found the key held, or
/// lost a race and should read again.
enum Created {
    /// This attempt created the record.
    Yes,
    /// An unexpired record already holds the key.
    Held,
    /// Another writer got there first; the key must be read again.
    Raced,
}

/// Scoped instance revision authority over any store that holds bytes.
///
/// The kernel owns the whole state machine and the store owns atomicity and
/// time, so one implementation serves every deployment tier: the same
/// transitions run over an in-process map, a database row, or an external
/// key-value store. Each operation reads the record, applies one pure
/// transition function, and replaces the record at exactly the version it
/// read. A stale read is read again once and then reported as
/// [`LedgerErrorKind::InstanceConflict`], which is a classified rejection and
/// never a partial state.
///
/// Claim tokens belong to the handle that issued them. Two handles over one
/// store are two providers: each mints tokens against its own identity, and a
/// token minted by one is [`LedgerErrorKind::ClaimMismatch`] on the other,
/// so a claim is always resolved by the node that took it or by its lease.
pub struct DistributedInstanceLedger<S: InstanceRecordStore> {
    store: Arc<S>,
    clock: Arc<dyn Clock>,
    limits: LedgerLimits,
    provider_identity: Arc<()>,
    retirements: Arc<Mutex<VecDeque<QueuedRetirement>>>,
}

/// One queued retirement and whether it has already failed once.
struct QueuedRetirement {
    op: CleanupOp,
    retried: bool,
}

impl<S: InstanceRecordStore> Clone for DistributedInstanceLedger<S> {
    /// Clones the handle, not the provider: a clone shares the store, the
    /// retirement queue, and the identity its tokens are bound to.
    fn clone(&self) -> Self {
        Self {
            store: Arc::clone(&self.store),
            clock: Arc::clone(&self.clock),
            limits: self.limits,
            provider_identity: Arc::clone(&self.provider_identity),
            retirements: Arc::clone(&self.retirements),
        }
    }
}

impl<S: InstanceRecordStore> DistributedInstanceLedger<S> {
    /// Creates one provider handle over `store`, bounded by `limits`.
    ///
    /// `clock` is node time: it measures instance lifetimes and claim leases.
    /// The store's own clock decides eviction, and the two are independent by
    /// design.
    #[must_use]
    pub fn new(store: Arc<S>, clock: Arc<dyn Clock>, limits: LedgerLimits) -> Self {
        Self {
            store,
            clock,
            limits,
            provider_identity: Arc::new(()),
            retirements: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Applies the retirements a synchronous drop queued, stopping at the
    /// first store, clock, or codec failure and reporting it.
    ///
    /// Each operation drains the queue before it runs, so calling this is
    /// only necessary when a caller needs a dropped claim retired without
    /// making another ledger request. A retirement that failed for the
    /// first time is still queued when this returns and has one more
    /// attempt; a second failure drops it, and then the claim's lease is
    /// what retires it.
    pub async fn flush_cleanup(&self) -> Result<(), LedgerError> {
        self.drain_retirements().await
    }

    /// Returns metadata-only provider inspection for tests and trusted
    /// diagnostics.
    pub async fn inspect(
        &self,
        scope: &ScopeFingerprint,
        instance_id: &InstanceId,
    ) -> Result<Option<LedgerInspection>, LedgerError> {
        let _ = self.drain_retirements().await;
        let now = self.now()?;
        let key = InstanceRecordKey {
            scope: scope.clone(),
            instance_id: instance_id.clone(),
        };
        match self.load(&key).await? {
            Loaded::Present { record, .. } if record.expires_at > now => {
                Ok(Some(state::inspection(&record, now)))
            }
            _ => Ok(None),
        }
    }

    /// Reads node time, reporting a clock the host cannot read rather than
    /// deciding an expiry without one.
    fn now(&self) -> Result<UnixMillis, LedgerError> {
        self.clock
            .now()
            .map_err(|_| LedgerError::new(LedgerErrorKind::ClockUnavailable))
    }

    /// Whether this handle issued `claim`.
    fn owns(&self, claim: &ClaimToken) -> bool {
        Arc::ptr_eq(&self.provider_identity, &claim.provider_identity)
    }

    /// The key one token's claim lives at.
    fn token_key(claim: &ClaimToken) -> InstanceRecordKey {
        InstanceRecordKey {
            scope: claim.scope.clone(),
            instance_id: claim.instance_id.clone(),
        }
    }

    /// Reads and decodes the record at `key`.
    async fn load(&self, key: &InstanceRecordKey) -> Result<Loaded, LedgerError> {
        interpret(self.store.load(key).await?)
    }

    /// Decides one queued retirement against the record it was read from.
    ///
    /// A retirement is an ordinary transition, and the two differ in more
    /// than which state function they run. Releasing a claim restores a
    /// retryable base revision, so it must know whether the instance is still
    /// alive and therefore needs node time. Fencing one never restores
    /// anything, so it asks the clock nothing: a claim whose host effects may
    /// already have committed has to be retirable even when the clock
    /// provider is the reason the commit failed.
    fn retirement(
        &self,
        op: &CleanupOp,
        loaded: Loaded,
    ) -> Result<Attempt<Result<(), LedgerError>>, LedgerError> {
        let Loaded::Present { record, version } = loaded else {
            return Ok(Attempt::read(Err(LedgerError::new(
                LedgerErrorKind::ClaimMismatch,
            ))));
        };
        let claim_id = op.token().claim_id;
        let transition = match op {
            CleanupOp::Abandon(_) => {
                let now = self.now()?;
                if record.expires_at <= now {
                    return Ok(Attempt::read(Err(LedgerError::new(
                        LedgerErrorKind::InstanceExpired,
                    ))));
                }
                state::release(record, claim_id, now)
            }
            CleanupOp::Fence(_) => state::fence(record, claim_id),
        };
        let outcome = match &transition.outcome {
            Ok(()) => Ok(()),
            Err(error) => Err(*error),
        };
        Ok(Attempt::from_transition(transition, version, outcome))
    }

    /// Runs one transition against the record at `key`: read, apply, and
    /// replace at the version that was read. A stale read is read again once,
    /// and then the operation is a classified rejection.
    async fn apply<T: Send>(
        &self,
        key: &InstanceRecordKey,
        mut step: impl FnMut(Loaded) -> Result<Attempt<T>, LedgerError> + Send,
    ) -> Result<T, LedgerError> {
        for _ in 0..APPLY_ATTEMPTS {
            let loaded = self.load(key).await?;
            let attempt = step(loaded)?;
            let Some((version, record)) = attempt.write else {
                return Ok(attempt.outcome);
            };
            let bytes = encode_record(&record)?;
            match self
                .store
                .compare_and_store(key, version, &bytes, store_expiry(record.expires_at))
                .await?
            {
                CasOutcome::Stored { .. } => return Ok(attempt.outcome),
                CasOutcome::Conflict | CasOutcome::Missing => {}
            }
        }
        Err(LedgerError::new(LedgerErrorKind::InstanceConflict))
    }

    /// Creates `record` at `key` unless an unexpired record already holds it.
    ///
    /// An elapsed record does not hold its key: it is replaced at the version
    /// it was read at, which is the distributed form of the pruning an
    /// in-process ledger did under its lock. Only a key nothing holds is
    /// measured against the configured instance capacity, so replacing an
    /// elapsed record never fails for capacity.
    async fn create(
        &self,
        key: &InstanceRecordKey,
        now: UnixMillis,
        record: &InstanceRecord,
    ) -> Result<Created, LedgerError> {
        match self.load(key).await? {
            Loaded::Present { record, .. } if record.expires_at > now => Ok(Created::Held),
            Loaded::Present { version, .. } => {
                let bytes = encode_record(record)?;
                match self
                    .store
                    .compare_and_store(key, version, &bytes, store_expiry(record.expires_at))
                    .await?
                {
                    CasOutcome::Stored { .. } => Ok(Created::Yes),
                    CasOutcome::Conflict | CasOutcome::Missing => Ok(Created::Raced),
                }
            }
            Loaded::Absent => {
                if self.store.count_instances().await? >= self.limits.max_instances() {
                    return Err(LedgerError::new(LedgerErrorKind::CapacityExceeded));
                }
                let bytes = encode_record(record)?;
                if self
                    .store
                    .insert_if_absent(key, &bytes, store_expiry(record.expires_at))
                    .await?
                {
                    Ok(Created::Yes)
                } else {
                    Ok(Created::Raced)
                }
            }
        }
    }

    /// Resolves a promotion against the reservation its retry identity holds,
    /// or `None` when the identity is free.
    async fn recover_promotion(
        &self,
        key: &PromotionRecordKey,
        request: &PromotionRecord,
    ) -> Result<Option<PromotionOutcome>, LedgerError> {
        let Some(stored) = self.store.load_promotion(key).await? else {
            return Ok(None);
        };
        let reservation = decode_reservation(&stored.bytes)?;
        if reservation.request_digest == request.request_digest {
            return Ok(Some(PromotionOutcome::Existing(InstanceAuthority::new(
                reservation.instance_id,
                reservation.initial_revision,
                reservation.expires_at,
            ))));
        }
        Ok(Some(PromotionOutcome::IdempotencyConflict))
    }

    /// Queues one retirement for the next asynchronous step.
    ///
    /// A token this handle did not issue is dropped rather than queued: it
    /// belongs to another provider, whose own queue is the only one that can
    /// resolve it. A queue at its bound drops its **oldest** entry to make
    /// room, because the oldest retirement names the claim whose lease is
    /// closest to elapsing and is therefore the one the lease backstop
    /// already covers; discarding the newest would throw away the release
    /// most likely still to be worth something.
    fn queue_retirement(&self, op: CleanupOp) {
        if !self.owns(op.token()) {
            return;
        }
        let Ok(mut queue) = self.retirements.lock() else {
            return;
        };
        while queue.len() >= self.limits.max_instances() {
            queue.pop_front();
        }
        queue.push_back(QueuedRetirement { op, retried: false });
    }

    /// Takes the next queued retirement.
    fn take_retirement(&self) -> Option<QueuedRetirement> {
        self.retirements.lock().ok()?.pop_front()
    }

    /// Returns one retirement to the head of the queue for its second and
    /// last attempt.
    fn requeue_retirement(&self, op: CleanupOp) {
        if let Ok(mut queue) = self.retirements.lock() {
            queue.push_front(QueuedRetirement { op, retried: true });
        }
    }

    /// Applies queued retirements, stopping at the first the store could not
    /// take.
    ///
    /// A retirement that failed goes back to the head of the queue for one
    /// more attempt at the next operation, and a second failure drops it: the
    /// claim's lease is the backstop, and dropping fails towards terminal
    /// authority, never towards a base revision that could be claimed twice.
    /// Draining stops on that first failure because a store that refused one
    /// retirement will refuse the next.
    ///
    /// Returns the first failure, which only [`Self::flush_cleanup`] reports;
    /// an ordinary operation drains and carries on.
    async fn drain_retirements(&self) -> Result<(), LedgerError> {
        while let Some(queued) = self.take_retirement() {
            let Err(error) = self.retire(&queued.op).await else {
                continue;
            };
            if !queued.retried {
                self.requeue_retirement(queued.op);
            }
            return Err(error);
        }
        Ok(())
    }

    /// Runs one retirement, reporting only store, clock, and codec failures.
    ///
    /// What the state machine answered about the claim is not a failure to
    /// retire it: a claim that already committed, expired, or belongs to a
    /// record that is gone has nothing left to release.
    async fn retire(&self, op: &CleanupOp) -> Result<(), LedgerError> {
        let key = Self::token_key(op.token());
        let _resolved: Result<(), LedgerError> = self
            .apply(&key, |loaded| self.retirement(op, loaded))
            .await?;
        Ok(())
    }
}

/// Decodes what a store returned into what the kernel decides against.
fn interpret(stored: Option<StoredRecord>) -> Result<Loaded, LedgerError> {
    let Some(stored) = stored else {
        return Ok(Loaded::Absent);
    };
    Ok(Loaded::Present {
        record: decode_record(&stored.bytes)?,
        version: stored.version,
    })
}

#[async_trait]
impl<S: InstanceRecordStore + 'static> LiveInstanceLedger for DistributedInstanceLedger<S> {
    async fn mount_instance(
        &self,
        record: MountInstanceRecord,
    ) -> Result<InstanceAuthority, LedgerError> {
        let _ = self.drain_retirements().await;
        let key = InstanceRecordKey {
            scope: record.scope.clone(),
            instance_id: record.instance_id.clone(),
        };
        for _ in 0..APPLY_ATTEMPTS {
            let now = self.now()?;
            state::validate_expiry(now, record.expires_at, self.limits)?;
            let created = state::created_record(
                Some(record.component_contract.clone()),
                record.initial_revision,
                record.expires_at,
            );
            match self.create(&key, now, &created).await? {
                Created::Yes => {
                    return Ok(InstanceAuthority::new(
                        record.instance_id,
                        record.initial_revision,
                        record.expires_at,
                    ));
                }
                Created::Held => {
                    return Err(LedgerError::new(LedgerErrorKind::InstanceConflict));
                }
                Created::Raced => {}
            }
        }
        Err(LedgerError::new(LedgerErrorKind::InstanceConflict))
    }

    async fn promote(&self, request: PromotionRecord) -> Result<PromotionOutcome, LedgerError> {
        let _ = self.drain_retirements().await;
        let promotion_key = PromotionRecordKey {
            scope: request.scope.clone(),
            idempotency_key: request.idempotency_key.clone(),
        };
        let instance_key = InstanceRecordKey {
            scope: request.scope.clone(),
            instance_id: request.instance_id.clone(),
        };
        for _ in 0..APPLY_ATTEMPTS {
            let now = self.now()?;
            state::validate_expiry(now, request.expires_at, self.limits)?;
            if let Some(outcome) = self.recover_promotion(&promotion_key, &request).await? {
                return Ok(outcome);
            }
            let created = state::created_record(None, request.initial_revision, request.expires_at);
            match self.create(&instance_key, now, &created).await? {
                Created::Held => {
                    // The proposed identity is held, which is a collision only
                    // when nothing explains it. A node that reached this retry
                    // identity first, proposing the same identity, created
                    // this very record, and its reservation is the answer this
                    // call owes: reporting a conflict would make an exact
                    // retry fail for having been retried.
                    if let Some(outcome) = self.recover_promotion(&promotion_key, &request).await? {
                        return Ok(outcome);
                    }
                    return Err(LedgerError::new(LedgerErrorKind::InstanceConflict));
                }
                Created::Raced => continue,
                Created::Yes => {}
            }
            // The instance exists before its retry identity is reserved. The
            // other order would let an exact retry recover authority over an
            // instance that was never created.
            let reservation = encode_reservation(&PromotionReservation {
                request_digest: request.request_digest.clone(),
                instance_id: request.instance_id.clone(),
                initial_revision: request.initial_revision,
                expires_at: request.expires_at,
            })?;
            if self
                .store
                .insert_promotion_if_absent(&promotion_key, &reservation, request.expires_at)
                .await?
            {
                return Ok(PromotionOutcome::Created(InstanceAuthority::new(
                    request.instance_id,
                    request.initial_revision,
                    request.expires_at,
                )));
            }
            // Another node reserved this retry identity first, so its
            // reservation is the authority and the instance this call created
            // a moment ago is unreachable: nobody will ever be told its
            // identity. Compensate for it here, or it would sit out its whole
            // lifetime against the configured instance capacity.
            self.store.remove(&instance_key).await?;
            if let Some(outcome) = self.recover_promotion(&promotion_key, &request).await? {
                return Ok(outcome);
            }
        }
        Err(LedgerError::new(LedgerErrorKind::InstanceConflict))
    }

    async fn claim(&self, request: ClaimRequest) -> Result<ClaimOutcome, LedgerError> {
        let _ = self.drain_retirements().await;
        let key = InstanceRecordKey {
            scope: request.scope.clone(),
            instance_id: request.instance_id.clone(),
        };
        self.apply(&key, |loaded| {
            let Loaded::Present { record, version } = loaded else {
                return Ok(Attempt::read(ClaimOutcome::RefreshRequired(
                    RefreshReason::Missing,
                )));
            };
            let now = self.now()?;
            if record.expires_at <= now {
                return Ok(Attempt::read(ClaimOutcome::RefreshRequired(
                    RefreshReason::InstanceExpired,
                )));
            }
            // The version the record was read at names the claim. Versions
            // rise for as long as a record lives and only one write can land
            // at any of them, so within a record's life no two claims share a
            // name, and a token from a released claim can never match the
            // claim that replaced it. Across lives the name may repeat, so
            // the guarantee then rests on the token's other half: a record
            // recreated at the same key would have to carry the same
            // server-generated instance identity, which the runtime's
            // 128 bit random generator makes negligible.
            let state::Transition { stored, outcome } =
                state::claim(record, &request, now, self.limits, version)?;
            let outcome = match outcome {
                ClaimDecision::Granted { successor_revision } => {
                    ClaimOutcome::Granted(ClaimGrant::new(
                        ClaimToken {
                            provider_identity: Arc::clone(&self.provider_identity),
                            scope: request.scope.clone(),
                            instance_id: request.instance_id.clone(),
                            claim_id: version,
                        },
                        successor_revision,
                    ))
                }
                ClaimDecision::Classified(classified) => classified,
            };
            Ok(Attempt {
                write: stored.map(|record| (version, record)),
                outcome,
            })
        })
        .await
    }

    async fn current_accepted_revision(
        &self,
        scope: &ScopeFingerprint,
        instance_id: &InstanceId,
    ) -> Result<Option<Revision>, LedgerError> {
        let _ = self.drain_retirements().await;
        let now = self.now()?;
        let key = InstanceRecordKey {
            scope: scope.clone(),
            instance_id: instance_id.clone(),
        };
        match self.load(&key).await? {
            Loaded::Present { record, .. } if record.expires_at > now => {
                Ok(state::accepted_revision(&record, now))
            }
            _ => Ok(None),
        }
    }

    async fn commit(
        &self,
        claim: &ClaimToken,
        outcome: AcceptedOutcome,
    ) -> Result<(), LedgerError> {
        let _ = self.drain_retirements().await;
        if !self.owns(claim) {
            return Err(LedgerError::new(LedgerErrorKind::ClaimMismatch));
        }
        let key = Self::token_key(claim);
        self.apply(&key, |loaded| {
            let Loaded::Present { record, version } = loaded else {
                return Ok(Attempt::read(Err(LedgerError::new(
                    LedgerErrorKind::ClaimMismatch,
                ))));
            };
            let now = self.now()?;
            if record.expires_at <= now {
                return Ok(Attempt::read(Err(LedgerError::new(
                    LedgerErrorKind::InstanceExpired,
                ))));
            }
            let transition = state::commit(
                record,
                &key,
                claim.claim_id,
                outcome.clone(),
                now,
                self.limits,
            );
            let resolved = match &transition.outcome {
                Ok(()) => Ok(()),
                Err(error) => Err(*error),
            };
            Ok(Attempt::from_transition(transition, version, resolved))
        })
        .await?
    }

    async fn abandon(&self, claim: &ClaimToken) -> Result<(), LedgerError> {
        let _ = self.drain_retirements().await;
        if !self.owns(claim) {
            return Err(LedgerError::new(LedgerErrorKind::ClaimMismatch));
        }
        let key = Self::token_key(claim);
        self.apply(&key, |loaded| {
            let Loaded::Present { record, version } = loaded else {
                return Ok(Attempt::read(Err(LedgerError::new(
                    LedgerErrorKind::ClaimMismatch,
                ))));
            };
            let now = self.now()?;
            if record.expires_at <= now {
                return Ok(Attempt::read(Err(LedgerError::new(
                    LedgerErrorKind::InstanceExpired,
                ))));
            }
            let transition = state::abandon(record, claim.claim_id, now);
            let resolved = match &transition.outcome {
                Ok(()) => Ok(()),
                Err(error) => Err(*error),
            };
            Ok(Attempt::from_transition(transition, version, resolved))
        })
        .await?
    }

    fn abandon_on_drop(&self, claim: ClaimToken) {
        self.queue_retirement(CleanupOp::Abandon(claim));
    }

    fn fence_on_drop(&self, claim: ClaimToken) {
        self.queue_retirement(CleanupOp::Fence(claim));
    }
}

impl DistributedInstanceLedger<MemoryRecordStore> {
    /// Reads inspection without an executor.
    ///
    /// Every [`MemoryRecordStore`] operation completes under one lock without
    /// yielding, so the Tier 0 provider can answer a synchronous caller by
    /// running the same read the asynchronous [`Self::inspect`] runs.
    pub(crate) fn inspect_in_process(
        &self,
        scope: &ScopeFingerprint,
        instance_id: &InstanceId,
    ) -> Result<Option<LedgerInspection>, LedgerError> {
        let now = self.now()?;
        let key = InstanceRecordKey {
            scope: scope.clone(),
            instance_id: instance_id.clone(),
        };
        match interpret(self.store.load_record(&key, Timing::Elapsing)?)? {
            Loaded::Present { record, .. } if record.expires_at > now => {
                Ok(Some(state::inspection(&record, now)))
            }
            _ => Ok(None),
        }
    }

    /// Retires one claim immediately rather than queueing it.
    ///
    /// An in-process store has no remote input or output to block on, so a
    /// dropped claim releases its base revision before the caller's next
    /// statement, which is what lets an exact retry proceed at once. The
    /// transition is the same one the queue would have run.
    pub(crate) fn retire_in_process(&self, op: CleanupOp) {
        if !self.owns(op.token()) {
            return;
        }
        let key = Self::token_key(op.token());
        let timing = op.timing();
        let _resolved = self.apply_in_process(&key, timing, |loaded| self.retirement(&op, loaded));
    }

    /// The synchronous twin of [`Self::apply`], over a store that never
    /// yields.
    fn apply_in_process<T>(
        &self,
        key: &InstanceRecordKey,
        timing: Timing,
        mut step: impl FnMut(Loaded) -> Result<Attempt<T>, LedgerError>,
    ) -> Result<T, LedgerError> {
        for _ in 0..APPLY_ATTEMPTS {
            let loaded = interpret(self.store.load_record(key, timing)?)?;
            let attempt = step(loaded)?;
            let Some((version, record)) = attempt.write else {
                return Ok(attempt.outcome);
            };
            let bytes = encode_record(&record)?;
            match self.store.replace_record(
                key,
                version,
                &bytes,
                store_expiry(record.expires_at),
                timing,
            )? {
                CasOutcome::Stored { .. } => return Ok(attempt.outcome),
                CasOutcome::Conflict | CasOutcome::Missing => {}
            }
        }
        Err(LedgerError::new(LedgerErrorKind::InstanceConflict))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    use super::*;
    use crate::clock::ClockError;
    use crate::identity::ContentDigest;

    struct FixedClock {
        now: AtomicU64,
        failing: AtomicBool,
    }

    impl FixedClock {
        fn new(now: u64) -> Self {
            Self {
                now: AtomicU64::new(now),
                failing: AtomicBool::new(false),
            }
        }

        fn set(&self, now: u64) {
            self.now.store(now, Ordering::SeqCst);
        }

        fn set_failing(&self, failing: bool) {
            self.failing.store(failing, Ordering::SeqCst);
        }
    }

    impl Clock for FixedClock {
        fn now(&self) -> Result<UnixMillis, ClockError> {
            if self.failing.load(Ordering::SeqCst) {
                return Err(ClockError::timestamp_overflow());
            }
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

    /// How a [`ContendedStore`] answers a compare-and-store.
    #[derive(Clone, Copy)]
    enum Contention {
        /// Answer exactly as the inner store would.
        Faithful,
        /// Never let a write land, which is what a record under continuous
        /// contention looks like from one node.
        RefuseEvery,
        /// Let the first write land and report it as a conflict anyway, which
        /// is what a node sees when another node's write reached the record
        /// between its read and its own write.
        StealFirst,
    }

    /// A store that decides for the test rather than for the record: it can
    /// refuse or steal a write, stop answering reads, and hide a promotion
    /// reservation that another node has already made, which is the one
    /// interleaving a single-threaded test cannot otherwise reach.
    struct ContendedStore {
        inner: MemoryRecordStore,
        contention: Contention,
        refused: AtomicU64,
        stolen: AtomicBool,
        unreadable: AtomicBool,
        hidden_reservations: AtomicU64,
    }

    impl ContendedStore {
        fn new(clock: Arc<FixedClock>, contention: Contention) -> Self {
            Self {
                inner: MemoryRecordStore::new(clock),
                contention,
                refused: AtomicU64::new(0),
                stolen: AtomicBool::new(false),
                unreadable: AtomicBool::new(false),
                hidden_reservations: AtomicU64::new(0),
            }
        }

        fn set_unreadable(&self, unreadable: bool) {
            self.unreadable.store(unreadable, Ordering::SeqCst);
        }

        /// Makes the next `reads` reservation loads answer as though the
        /// reservation had not been made yet.
        fn hide_reservation(&self, reads: u64) {
            self.hidden_reservations.store(reads, Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl InstanceRecordStore for ContendedStore {
        async fn load(&self, key: &InstanceRecordKey) -> Result<Option<StoredRecord>, LedgerError> {
            if self.unreadable.load(Ordering::SeqCst) {
                return Err(LedgerError::new(LedgerErrorKind::ProviderUnavailable));
            }
            self.inner.load(key).await
        }

        async fn insert_if_absent(
            &self,
            key: &InstanceRecordKey,
            bytes: &[u8],
            expires_at: UnixMillis,
        ) -> Result<bool, LedgerError> {
            self.inner.insert_if_absent(key, bytes, expires_at).await
        }

        async fn compare_and_store(
            &self,
            key: &InstanceRecordKey,
            expected_version: u64,
            bytes: &[u8],
            expires_at: UnixMillis,
        ) -> Result<CasOutcome, LedgerError> {
            match self.contention {
                Contention::RefuseEvery => {
                    self.refused.fetch_add(1, Ordering::SeqCst);
                    Ok(CasOutcome::Conflict)
                }
                Contention::StealFirst if !self.stolen.swap(true, Ordering::SeqCst) => {
                    self.refused.fetch_add(1, Ordering::SeqCst);
                    self.inner
                        .compare_and_store(key, expected_version, bytes, expires_at)
                        .await?;
                    Ok(CasOutcome::Conflict)
                }
                Contention::Faithful | Contention::StealFirst => {
                    self.inner
                        .compare_and_store(key, expected_version, bytes, expires_at)
                        .await
                }
            }
        }

        async fn remove(&self, key: &InstanceRecordKey) -> Result<(), LedgerError> {
            self.inner.remove(key).await
        }

        async fn load_promotion(
            &self,
            key: &PromotionRecordKey,
        ) -> Result<Option<StoredRecord>, LedgerError> {
            if self
                .hidden_reservations
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |hidden| {
                    hidden.checked_sub(1)
                })
                .is_ok()
            {
                return Ok(None);
            }
            self.inner.load_promotion(key).await
        }

        async fn insert_promotion_if_absent(
            &self,
            key: &PromotionRecordKey,
            bytes: &[u8],
            expires_at: UnixMillis,
        ) -> Result<bool, LedgerError> {
            self.inner
                .insert_promotion_if_absent(key, bytes, expires_at)
                .await
        }

        async fn count_instances(&self) -> Result<usize, LedgerError> {
            self.inner.count_instances().await
        }
    }

    fn digest(start: u8) -> ContentDigest {
        ContentDigest::from_bytes(&bytes::<32>(start)).expect("digest is valid")
    }

    fn limits() -> LedgerLimits {
        LedgerLimits::new(100, 10_000, 2, 64).expect("limits are valid")
    }

    fn kernel_mount() -> MountInstanceRecord {
        let key = instance_key(0x10);
        MountInstanceRecord::new(
            key.scope,
            key.instance_id,
            digest(0x30),
            Revision::new(0),
            UnixMillis::new(5_000),
        )
    }

    fn kernel_token(
        ledger: &DistributedInstanceLedger<MemoryRecordStore>,
        claim_id: u64,
    ) -> ClaimToken {
        let key = instance_key(0x10);
        ClaimToken {
            provider_identity: Arc::clone(&ledger.provider_identity),
            scope: key.scope,
            instance_id: key.instance_id,
            claim_id,
        }
    }

    fn kernel_promotion(instance_start: u8, retry: u8, digest_start: u8) -> PromotionRecord {
        PromotionRecord::new(
            instance_key(0x10).scope,
            InstanceId::from_bytes(&bytes::<16>(instance_start)).expect("instance is valid"),
            IdempotencyKey::from_bytes(&bytes::<16>(retry)).expect("retry key is valid"),
            digest(digest_start),
            Revision::new(0),
            UnixMillis::new(5_000),
        )
    }

    /// Two provider handles over one store, which is what two nodes are.
    fn two_nodes(
        store: &Arc<ContendedStore>,
        clock: &Arc<FixedClock>,
    ) -> (
        DistributedInstanceLedger<ContendedStore>,
        DistributedInstanceLedger<ContendedStore>,
    ) {
        (
            DistributedInstanceLedger::new(Arc::clone(store), clock.clone(), limits()),
            DistributedInstanceLedger::new(Arc::clone(store), clock.clone(), limits()),
        )
    }

    fn kernel_claim() -> ClaimRequest {
        let key = instance_key(0x10);
        ClaimRequest::new(
            key.scope,
            key.instance_id,
            Revision::new(0),
            IdempotencyKey::from_bytes(&bytes::<16>(0x40)).expect("retry key is valid"),
            digest(0x50),
        )
    }

    #[tokio::test]
    async fn a_stored_record_never_prints_the_bytes_it_carries() {
        let store = MemoryRecordStore::new(Arc::new(FixedClock::new(0)));
        let key = instance_key(0x10);
        store
            .insert_if_absent(&key, b"a-record-of-authority", UnixMillis::new(1_000))
            .await
            .expect("the store answers");

        let printed = format!(
            "{:?}",
            store
                .load(&key)
                .await
                .expect("the store answers")
                .expect("the record is there")
        );

        assert!(
            printed.contains("<21 bytes:redacted>"),
            "a store's answer reports the size of what it holds: {printed}"
        );
        assert!(
            !printed.contains("97"),
            "a store's answer never prints the record itself: {printed}"
        );
        assert!(printed.contains("version: 1"), "{printed}");
    }

    #[tokio::test]
    async fn a_fence_retires_a_claim_even_when_the_clock_cannot_be_read() {
        let clock = Arc::new(FixedClock::new(1_000));
        let ledger = DistributedInstanceLedger::new(
            Arc::new(MemoryRecordStore::new(clock.clone())),
            clock.clone(),
            limits(),
        );
        ledger
            .mount_instance(kernel_mount())
            .await
            .expect("the mount is created");
        let ClaimOutcome::Granted(grant) = ledger
            .claim(kernel_claim())
            .await
            .expect("the claim classifies")
        else {
            panic!("the current base revision is claimable");
        };

        // An unreadable clock is exactly what makes a commit fail, and the
        // claim it leaves behind is the one that must never come back.
        clock.set_failing(true);
        ledger.retire_in_process(CleanupOp::Fence(grant.into_token()));
        clock.set_failing(false);

        assert!(
            matches!(
                ledger
                    .claim(kernel_claim())
                    .await
                    .expect("the claim classifies"),
                ClaimOutcome::RefreshRequired(RefreshReason::Consumed)
            ),
            "a fenced claim never restores its base revision"
        );
    }

    #[tokio::test]
    async fn the_loser_of_a_promotion_race_leaves_no_instance_behind() {
        let clock = Arc::new(FixedClock::new(1_000));
        let store = Arc::new(ContendedStore::new(clock.clone(), Contention::Faithful));
        let (first, second) = two_nodes(&store, &clock);
        first
            .promote(kernel_promotion(0x20, 0x40, 0x50))
            .await
            .expect("the first node promotes");

        // The second node read the retry identity before the first node
        // reserved it, proposing an identity of its own. It creates that
        // instance, finds the reservation taken, and must compensate: nobody
        // will ever be told the identity it made.
        store.hide_reservation(1);
        let recovered = second
            .promote(kernel_promotion(0x21, 0x40, 0x50))
            .await
            .expect("the losing node recovers the reservation");

        let PromotionOutcome::Existing(authority) = recovered else {
            panic!("a raced promotion recovers the winner's authority, not {recovered:?}");
        };
        assert_eq!(
            authority.instance_id().as_bytes(),
            bytes::<16>(0x20),
            "the winner's instance is the authority"
        );
        assert_eq!(
            store.count_instances().await.expect("the store answers"),
            1,
            "the instance the losing promotion created is compensated for"
        );
    }

    #[tokio::test]
    async fn a_held_identity_yields_to_the_reservation_that_explains_it() {
        let clock = Arc::new(FixedClock::new(1_000));
        let store = Arc::new(ContendedStore::new(clock.clone(), Contention::Faithful));
        let (first, second) = two_nodes(&store, &clock);
        first
            .promote(kernel_promotion(0x20, 0x40, 0x50))
            .await
            .expect("the first node promotes");

        // The same proposal, from a node that read the retry identity before
        // it was reserved: the identity it proposes is held by the record the
        // winner created, and the reservation is what explains that.
        store.hide_reservation(1);
        let recovered = second
            .promote(kernel_promotion(0x20, 0x40, 0x50))
            .await
            .expect("a held identity with a reservation behind it is not a conflict");

        assert!(
            matches!(recovered, PromotionOutcome::Existing(_)),
            "an exact retry recovers rather than colliding, got {recovered:?}"
        );
    }

    #[tokio::test]
    async fn a_held_identity_with_nothing_behind_it_is_still_a_conflict() {
        let clock = Arc::new(FixedClock::new(1_000));
        let store = Arc::new(ContendedStore::new(clock.clone(), Contention::Faithful));
        let (first, second) = two_nodes(&store, &clock);
        first
            .mount_instance(kernel_mount())
            .await
            .expect("the identity is taken by a mount");

        assert_eq!(
            second
                .promote(kernel_promotion(0x10, 0x40, 0x50))
                .await
                .expect_err("nothing explains the held identity")
                .kind(),
            LedgerErrorKind::InstanceConflict
        );
    }

    #[tokio::test]
    async fn one_operation_reclaims_a_bounded_number_of_elapsed_records() {
        let clock = Arc::new(FixedClock::new(0));
        let store = MemoryRecordStore::new(clock.clone());
        let due = 200_usize;
        for start in 0..due {
            store
                .insert_if_absent(
                    &instance_key(u8::try_from(start).expect("the fixture fits a byte")),
                    b"record",
                    UnixMillis::new(1_000),
                )
                .await
                .expect("the store answers");
        }

        clock.set(1_000);

        assert_eq!(
            store.count_instances().await.expect("the store answers"),
            due - MAX_RECLAIMED_PER_OPERATION,
            "one operation reclaims at most its bounded share of a burst"
        );
        assert_eq!(
            store.count_instances().await.expect("the store answers"),
            due - 2 * MAX_RECLAIMED_PER_OPERATION,
            "the backlog drains over the operations that follow"
        );
    }

    #[tokio::test]
    async fn a_stale_read_is_read_again_and_reclassified_rather_than_forced() {
        let clock = Arc::new(FixedClock::new(1_000));
        let store = Arc::new(ContendedStore::new(clock.clone(), Contention::StealFirst));
        let ledger = DistributedInstanceLedger::new(Arc::clone(&store), clock, limits());
        ledger
            .mount_instance(kernel_mount())
            .await
            .expect("the mount is created");

        // The first write lands and is reported as a conflict, which is a
        // node discovering that someone else's write got there first.
        let outcome = ledger
            .claim(kernel_claim())
            .await
            .expect("the claim classifies");

        assert_eq!(store.refused.load(Ordering::SeqCst), 1);
        assert!(
            matches!(outcome, ClaimOutcome::InProgress { .. }),
            "a stale read is read again and classified against what is there now"
        );
    }

    #[tokio::test]
    async fn a_failed_retirement_gets_one_more_attempt_and_is_then_dropped() {
        let clock = Arc::new(FixedClock::new(1_000));
        let store = Arc::new(ContendedStore::new(clock.clone(), Contention::Faithful));
        let ledger = DistributedInstanceLedger::new(Arc::clone(&store), clock, limits());
        ledger
            .mount_instance(kernel_mount())
            .await
            .expect("the mount is created");
        let ClaimOutcome::Granted(grant) = ledger
            .claim(kernel_claim())
            .await
            .expect("the claim classifies")
        else {
            panic!("the current base revision is claimable");
        };

        store.set_unreadable(true);
        ledger.abandon_on_drop(grant.into_token());
        assert_eq!(
            ledger
                .flush_cleanup()
                .await
                .expect_err("a store that cannot answer cannot retire a claim")
                .kind(),
            LedgerErrorKind::ProviderUnavailable
        );

        store.set_unreadable(false);
        ledger
            .flush_cleanup()
            .await
            .expect("the second attempt reaches the store");
        assert!(
            matches!(
                ledger
                    .claim(kernel_claim())
                    .await
                    .expect("the claim classifies"),
                ClaimOutcome::Granted(_)
            ),
            "the retirement that was retried released its base revision"
        );
    }

    #[tokio::test]
    async fn a_retirement_that_fails_twice_is_dropped_and_left_to_the_lease() {
        let clock = Arc::new(FixedClock::new(1_000));
        let store = Arc::new(ContendedStore::new(clock.clone(), Contention::Faithful));
        let ledger = DistributedInstanceLedger::new(Arc::clone(&store), clock, limits());
        ledger
            .mount_instance(kernel_mount())
            .await
            .expect("the mount is created");
        let ClaimOutcome::Granted(grant) = ledger
            .claim(kernel_claim())
            .await
            .expect("the claim classifies")
        else {
            panic!("the current base revision is claimable");
        };

        store.set_unreadable(true);
        ledger.abandon_on_drop(grant.into_token());
        for _ in 0..2 {
            ledger
                .flush_cleanup()
                .await
                .expect_err("a store that cannot answer cannot retire a claim");
        }

        store.set_unreadable(false);
        ledger
            .flush_cleanup()
            .await
            .expect("the queue is empty, so there is nothing left to fail");
        assert!(
            matches!(
                ledger
                    .claim(kernel_claim())
                    .await
                    .expect("the claim classifies"),
                ClaimOutcome::InProgress { .. }
            ),
            "a retirement dropped after two attempts leaves the claim to its lease"
        );
    }

    #[tokio::test]
    async fn a_full_retirement_queue_drops_its_oldest_entry() {
        let clock = Arc::new(FixedClock::new(1_000));
        let ledger = DistributedInstanceLedger::new(
            Arc::new(MemoryRecordStore::new(clock.clone())),
            clock,
            LedgerLimits::new(100, 10_000, 2, 1).expect("limits are valid"),
        );

        ledger.abandon_on_drop(kernel_token(&ledger, 1));
        ledger.abandon_on_drop(kernel_token(&ledger, 2));

        let queue = ledger.retirements.lock().expect("the queue is readable");
        assert_eq!(queue.len(), 1, "the queue holds its configured bound");
        assert_eq!(
            queue
                .front()
                .expect("the queue holds one retirement")
                .op
                .token()
                .claim_id,
            2,
            "the newest retirement survives and the oldest makes room"
        );
    }

    #[tokio::test]
    async fn a_write_that_never_lands_is_a_classified_rejection_after_one_retry() {
        let clock = Arc::new(FixedClock::new(1_000));
        let store = Arc::new(ContendedStore::new(clock.clone(), Contention::RefuseEvery));
        let ledger = DistributedInstanceLedger::new(Arc::clone(&store), clock, limits());
        ledger
            .mount_instance(kernel_mount())
            .await
            .expect("creating a record does not compare and store");

        let error = ledger
            .claim(kernel_claim())
            .await
            .expect_err("a claim whose write never lands is rejected");

        assert_eq!(
            error.kind(),
            LedgerErrorKind::InstanceConflict,
            "contention is a classified rejection, not a partial state"
        );
        assert_eq!(
            store.refused.load(Ordering::SeqCst),
            APPLY_ATTEMPTS as u64,
            "the transition is read again once and then reported"
        );
        assert_eq!(
            ledger
                .current_accepted_revision(
                    &instance_key(0x10).scope,
                    &instance_key(0x10).instance_id
                )
                .await
                .expect("the store answers"),
            Some(Revision::new(0)),
            "a rejected claim left the record exactly as it was"
        );
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
}
