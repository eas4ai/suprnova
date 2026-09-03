//! Typed dependency identities, observed generation sets, the ledger
//! contract, and the observation window that closes into a coherence check.
//!
//! [`GenerationSet`] keeps the wire form Task 4 fixed: each 32-byte
//! dependency digest key serializes as a 64-character lowercase hex string
//! mapped to its generation counter, and deserialization rejects anything
//! that is not exactly that (wrong length, a non-hex character, or an
//! upper- or mixed-case spelling). The same digests are stored as hex in the
//! database, so this is the one representation both share; a second
//! spelling of one digest would mean two database rows for one dependency.
//!
//! A [`GenerationSet`] is keyed by dependency digest, never by
//! [`DependencyIdentity`], and [`GenerationLedger::current`] reads by digest
//! while [`GenerationLedger::advance`] still commits by identity. A decoded
//! [`crate::render_cache::entry::EntryHeader`] carries only what its
//! [`GenerationSet`] holds, and an identity is not recoverable from its
//! digest, so the freshness recheck on a decoded entry must be addressable
//! by digest alone. The second reason is privacy: an identity spells out an
//! application table name and a record's primary key, which can be a user
//! identifier, so serializing identities into cached entries would put
//! application data into stored bytes and inspection output. The write path
//! always knows its identities, so `advance` is unaffected.
//! [`CoherenceCheck::Moved`] therefore reports digests, and an epoch change
//! is reported as the digest of [`DependencyIdentity::Broad`].

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use sha2::{Digest as _, Sha256};

use super::{RenderCacheError, RenderCacheErrorKind};

/// Most identities one representation may observe.
pub const MAX_OBSERVATIONS: usize = 4_096;
/// Identity encoding version; a change here cannot collide with a digest
/// computed under a prior version.
pub const IDENTITY_VERSION: u8 = 1;
/// Upper bound on a table, class, or other identity name.
const MAX_NAME_BYTES: usize = 128;
/// Upper bound on a record's primary key bytes.
const MAX_KEY_BYTES: usize = 512;

/// A monotonically advancing dependency version.
pub type Generation = u64;

/// A typed dependency of a representation.
#[derive(
    Clone, Debug, Eq, PartialEq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum DependencyIdentity {
    /// Every row of a table.
    Table(String),
    /// One row by primary key bytes.
    Record {
        /// Owning table.
        table: String,
        /// Primary key bytes.
        key: Vec<u8>,
    },
    /// A named query class over a table.
    QueryClass {
        /// Owning table.
        table: String,
        /// Class name.
        class: String,
    },
    /// A relation between two tables.
    Relation {
        /// Parent table.
        parent: String,
        /// Child table.
        child: String,
    },
    /// A configuration key.
    Config(String),
    /// A feature flag.
    Feature(String),
    /// The locale catalog.
    Locale,
    /// Route table version.
    Route,
    /// The broad authority every representation observes; unknown reads and
    /// writes collapse here.
    Broad,
}

/// Rejects an empty, oversized, or control-character name.
fn bounded(name: &str) -> Result<(), RenderCacheError> {
    if name.is_empty() || name.len() > MAX_NAME_BYTES || name.bytes().any(|b| b.is_ascii_control())
    {
        return Err(RenderCacheError::new(RenderCacheErrorKind::VarianceInvalid));
    }
    Ok(())
}

impl DependencyIdentity {
    /// A table identity; panics only on an unbounded name, so callers with
    /// untrusted names use [`Self::try_table`].
    #[must_use]
    pub fn table(name: &str) -> Self {
        Self::try_table(name).expect("bounded table name")
    }

    /// A table identity with bounds checked.
    pub fn try_table(name: &str) -> Result<Self, RenderCacheError> {
        bounded(name)?;
        Ok(Self::Table(name.to_owned()))
    }

    /// A record identity; panics only on an unbounded table name or key, so
    /// callers with untrusted input use [`Self::try_record`].
    #[must_use]
    pub fn record(table: &str, key: &[u8]) -> Self {
        Self::try_record(table, key).expect("bounded record identity")
    }

    /// A record identity with bounds checked.
    pub fn try_record(table: &str, key: &[u8]) -> Result<Self, RenderCacheError> {
        bounded(table)?;
        if key.is_empty() || key.len() > MAX_KEY_BYTES {
            return Err(RenderCacheError::new(RenderCacheErrorKind::VarianceInvalid));
        }
        Ok(Self::Record {
            table: table.to_owned(),
            key: key.to_vec(),
        })
    }

    /// A query class identity; panics only on an unbounded name.
    #[must_use]
    pub fn query_class(table: &str, class: &str) -> Self {
        bounded(table)
            .and(bounded(class))
            .expect("bounded query class");
        Self::QueryClass {
            table: table.to_owned(),
            class: class.to_owned(),
        }
    }

    /// The broad authority.
    #[must_use]
    pub const fn broad() -> Self {
        Self::Broad
    }

    /// Stable 32-byte digest of the versioned identity.
    #[must_use]
    pub fn digest(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update([IDENTITY_VERSION]);
        let (tag, parts): (u8, Vec<&[u8]>) = match self {
            Self::Table(name) => (1, vec![name.as_bytes()]),
            Self::Record { table, key } => (2, vec![table.as_bytes(), key.as_slice()]),
            Self::QueryClass { table, class } => (3, vec![table.as_bytes(), class.as_bytes()]),
            Self::Relation { parent, child } => (4, vec![parent.as_bytes(), child.as_bytes()]),
            Self::Config(key) => (5, vec![key.as_bytes()]),
            Self::Feature(name) => (6, vec![name.as_bytes()]),
            Self::Locale => (7, vec![]),
            Self::Route => (8, vec![]),
            Self::Broad => (9, vec![]),
        };
        hasher.update([tag]);
        for part in parts {
            hasher.update((part.len() as u32).to_be_bytes());
            hasher.update(part);
        }
        hasher.finalize().into()
    }
}

/// Observed generations keyed by dependency digest. Never carries the
/// identities behind those digests; see the module documentation for why.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GenerationSet {
    entries: BTreeMap<[u8; 32], Generation>,
}

impl GenerationSet {
    /// Records the generation observed for one dependency digest; fails
    /// once [`MAX_OBSERVATIONS`] distinct digests are already held.
    pub fn insert_digest(
        &mut self,
        dependency: [u8; 32],
        generation: Generation,
    ) -> Result<(), RenderCacheError> {
        if self.entries.len() >= MAX_OBSERVATIONS && !self.entries.contains_key(&dependency) {
            return Err(RenderCacheError::new(RenderCacheErrorKind::EntryInvalid));
        }
        self.entries.insert(dependency, generation);
        Ok(())
    }

    /// Records the generation observed for one identity; fails under the
    /// same bound as [`Self::insert_digest`].
    pub fn insert(
        &mut self,
        identity: &DependencyIdentity,
        generation: Generation,
    ) -> Result<(), RenderCacheError> {
        self.insert_digest(identity.digest(), generation)
    }

    /// The generation observed for an identity.
    #[must_use]
    pub fn get(&self, identity: &DependencyIdentity) -> Option<Generation> {
        self.get_digest(&identity.digest())
    }

    /// The generation observed for a dependency digest.
    #[must_use]
    pub fn get_digest(&self, dependency: &[u8; 32]) -> Option<Generation> {
        self.entries.get(dependency).copied()
    }

    /// Number of dependencies observed.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when nothing was observed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The dependency digests, in canonical (ascending) order.
    #[must_use]
    pub fn digests(&self) -> Vec<[u8; 32]> {
        self.entries.keys().copied().collect()
    }

    /// Digest over every (dependency, generation) pair, for publication
    /// fences.
    #[must_use]
    pub fn digest(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        for (dependency, generation) in &self.entries {
            hasher.update(dependency);
            hasher.update(generation.to_be_bytes());
        }
        hasher.finalize().into()
    }
}

impl serde::Serialize for GenerationSet {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let hex_keyed: BTreeMap<String, Generation> = self
            .entries
            .iter()
            .map(|(digest, generation)| (to_hex(digest), *generation))
            .collect();
        hex_keyed.serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for GenerationSet {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let hex_keyed = BTreeMap::<String, Generation>::deserialize(deserializer)?;
        let mut entries = BTreeMap::new();
        for (key, generation) in hex_keyed {
            let digest = from_hex(&key).ok_or_else(|| {
                serde::de::Error::custom("dependency digest must be 64 hex characters")
            })?;
            entries.insert(digest, generation);
        }
        Ok(Self { entries })
    }
}

fn to_hex(digest: &[u8; 32]) -> String {
    use std::fmt::Write as _;

    let mut text = String::with_capacity(64);
    for byte in digest {
        write!(&mut text, "{byte:02x}").expect("formatting into a String cannot fail");
    }
    text
}

fn from_hex(text: &str) -> Option<[u8; 32]> {
    let bytes = text.as_bytes();
    if bytes.len() != 64 {
        return None;
    }
    let mut digest = [0_u8; 32];
    for (index, slot) in digest.iter_mut().enumerate() {
        let high = hex_value(bytes[index * 2])?;
        let low = hex_value(bytes[index * 2 + 1])?;
        *slot = (high << 4) | low;
    }
    Some(digest)
}

const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

/// Database-authoritative generation truth. Implementations advance inside
/// the owning data transaction so rollback advances nothing.
#[async_trait]
pub trait GenerationLedger: Send + Sync {
    /// Current generations for the given dependency digests; an unobserved
    /// digest is 0. Reads by digest, since a decoded stored entry carries
    /// only digests: see the module documentation.
    async fn current(&self, dependencies: &[[u8; 32]]) -> Result<GenerationSet, RenderCacheError>;
    /// Advances each identity by one within the caller's transaction scope.
    /// Commits by identity, since the write path always knows it.
    async fn advance(&self, identities: &[DependencyIdentity]) -> Result<(), RenderCacheError>;
    /// The authority epoch.
    async fn epoch(&self) -> Result<u64, RenderCacheError>;
}

struct LedgerState {
    generations: BTreeMap<[u8; 32], Generation>,
    epoch: u64,
}

/// In-memory ledger for tests and the engine's own reference behavior; it is
/// never a production authority.
pub struct MemoryGenerationLedger {
    state: Mutex<LedgerState>,
}

impl MemoryGenerationLedger {
    /// Creates an empty ledger at epoch 1.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Mutex::new(LedgerState {
                generations: BTreeMap::new(),
                epoch: 1,
            }),
        }
    }

    /// Advances the authority epoch.
    pub fn advance_epoch(&self) {
        self.lock_state().epoch += 1;
    }

    /// Locks the state, recovering it from poison rather than propagating a
    /// panic across this ledger's operations.
    fn lock_state(&self) -> MutexGuard<'_, LedgerState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Default for MemoryGenerationLedger {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl GenerationLedger for MemoryGenerationLedger {
    async fn current(&self, dependencies: &[[u8; 32]]) -> Result<GenerationSet, RenderCacheError> {
        let state = self.lock_state();
        let mut set = GenerationSet::default();
        for dependency in dependencies {
            let generation = *state.generations.get(dependency).unwrap_or(&0);
            set.insert_digest(*dependency, generation)?;
        }
        Ok(set)
    }

    async fn advance(&self, identities: &[DependencyIdentity]) -> Result<(), RenderCacheError> {
        let mut state = self.lock_state();
        for identity in identities {
            *state.generations.entry(identity.digest()).or_insert(0) += 1;
        }
        Ok(())
    }

    async fn epoch(&self) -> Result<u64, RenderCacheError> {
        Ok(self.lock_state().epoch)
    }
}

/// The set of identities a render observed, closed into their generations.
#[derive(Debug)]
pub struct ObservationWindow {
    epoch: u64,
    identities: BTreeSet<DependencyIdentity>,
}

impl ObservationWindow {
    /// Opens a window at the authority epoch; the broad authority is always
    /// observed and does not count against [`MAX_OBSERVATIONS`].
    #[must_use]
    pub fn open(epoch: u64) -> Self {
        Self {
            epoch,
            identities: BTreeSet::from([DependencyIdentity::Broad]),
        }
    }

    /// Records one identity; bounded and idempotent. The always-present
    /// broad authority does not count against the bound, so a window may
    /// hold at most [`MAX_OBSERVATIONS`] identities beyond it.
    pub fn observe(&mut self, identity: DependencyIdentity) -> Result<(), RenderCacheError> {
        let observed = self
            .identities
            .iter()
            .filter(|existing| **existing != DependencyIdentity::Broad)
            .count();
        let already_present = self.identities.contains(&identity);
        if observed >= MAX_OBSERVATIONS && !already_present && identity != DependencyIdentity::Broad
        {
            return Err(RenderCacheError::new(RenderCacheErrorKind::VarianceInvalid));
        }
        self.identities.insert(identity);
        Ok(())
    }

    /// The epoch the window opened at.
    #[must_use]
    pub const fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Reads the generations of every observed identity from the ledger.
    pub async fn close(
        self,
        ledger: &dyn GenerationLedger,
    ) -> Result<GenerationSet, RenderCacheError> {
        let dependencies: Vec<[u8; 32]> = self
            .identities
            .iter()
            .map(DependencyIdentity::digest)
            .collect();
        ledger.current(&dependencies).await
    }
}

/// The result of comparing observed generations with current authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CoherenceCheck {
    /// Every observed generation and the epoch are unchanged.
    Coherent,
    /// These dependency digests moved (an epoch change is reported as the
    /// digest of [`DependencyIdentity::Broad`]).
    Moved(Vec<[u8; 32]>),
}

impl CoherenceCheck {
    /// Compares an observed set against a current reread at `epoch`,
    /// noting that the observation itself was made at `observed_epoch`.
    #[must_use]
    pub fn compare(
        observed: &GenerationSet,
        current: &GenerationSet,
        epoch: u64,
        observed_epoch: u64,
    ) -> Self {
        let mut moved = Vec::new();
        if epoch != observed_epoch {
            moved.push(DependencyIdentity::Broad.digest());
        }
        for dependency in observed.digests() {
            if observed.get_digest(&dependency) != current.get_digest(&dependency) {
                moved.push(dependency);
            }
        }
        if moved.is_empty() {
            Self::Coherent
        } else {
            Self::Moved(moved)
        }
    }
}
