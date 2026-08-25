//! Queued chains: dispatch a sequence of jobs where each runs only after
//! the previous one ack's.
//!
//! Mirrors Laravel 13's `Bus::chain([...])`. The first envelope is pushed
//! to the queue with the rest serialized inside a `chain_remaining` field;
//! after each successful settlement the worker pops the next entry and
//! dispatches it.
//!
//! Internally chained envelopes use the queue's normal driver; no special
//! storage layer is required because the chain state travels with the
//! current envelope payload.

use crate::error::FrameworkError;
use crate::queue::Job;
use crate::queue::envelope::Envelope;
use crate::queue::job::BackoffSchedule;
use serde::{Deserialize, Serialize};

/// Serialized form of one chained envelope, persisted on the active
/// envelope's `chain_remaining` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainLink {
    /// Fully-qualified job type name (matches `Job::job_name()`).
    pub job_name: String,
    /// Serialized job payload, captured at chain-build time.
    pub payload: serde_json::Value,
    /// Maximum dispatch attempts for this link.
    pub max_tries: u32,
    /// Per-attempt timeout budget in seconds; `None` disables.
    pub timeout_secs: Option<u64>,
    /// When `true`, a timeout consumes the attempt as a permanent failure.
    pub fail_on_timeout: bool,
    /// Job-side backoff schedule captured at chain-build time. `#[serde(default)]`
    /// keeps schema-v2 chain payloads (which omitted this field) decoding —
    /// they get the framework default just as they did before.
    #[serde(default)]
    pub backoff: BackoffSchedule,
    /// Queue the job declared for itself via [`Job::queue`], captured at
    /// chain-build time because the link stores its job type-erased and the
    /// trait method is unreachable at dispatch. `skip_serializing_if` keeps
    /// an undeclared queue off the wire, and `serde(default)` keeps chain
    /// payloads written before this field existed decoding — those links
    /// behave exactly as they did: a registered route or the driver default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue: Option<String>,
}

impl ChainLink {
    /// Build a chain-link entry from a typed `Job`. Uses the job's
    /// `J::max_tries()` / `J::timeout()` / `J::backoff()` defaults exactly
    /// the way `Queue::push` would.
    pub fn from_job<J: Job>(job: J) -> Result<Self, FrameworkError> {
        Ok(Self {
            job_name: J::job_name().to_string(),
            payload: serde_json::to_value(&job)
                .map_err(|e| FrameworkError::internal(format!("encode chain link: {e}")))?,
            max_tries: J::max_tries(),
            timeout_secs: J::timeout().map(|d| d.as_secs()),
            fail_on_timeout: J::fail_on_timeout(),
            backoff: J::backoff(),
            queue: J::queue().map(str::to_owned),
        })
    }

    /// Reify into a dispatchable envelope with a fresh random id.
    ///
    /// The worker does **not** use this for chain continuation — see
    /// [`to_envelope_after`](Self::to_envelope_after) and the reason why.
    /// This remains for callers reifying a link outside a running chain.
    pub fn to_envelope(&self) -> Envelope {
        self.to_envelope_with_id(uuid::Uuid::new_v4())
    }

    /// Reify into a dispatchable envelope whose id is derived from the
    /// envelope this link runs after.
    ///
    /// # Why the id must not be random here (DATA-02b)
    ///
    /// On a driver without transactional settlement
    /// ([`QueueDriver::settle`](crate::queue::QueueDriver::settle) answering
    /// [`Settled::Unsupported`](crate::queue::Settled::Unsupported)), the
    /// worker pushes the next link *before* acking the current job, so a crash
    /// or a failed ack in that window redelivers the current job and runs the
    /// push a second time. With [`to_envelope`](Self::to_envelope)'s
    /// `Uuid::new_v4()` the two pushes produced envelopes with *different*
    /// ids, so nothing downstream could tell they were the same logical step:
    /// not the driver, not an outbox, and not a handler.
    ///
    /// That last one matters most. The framework's delivery contract is
    /// at-least-once and its answer to duplicates is "handlers must be
    /// idempotent" — but a handler keyed on `env.id`, the one identifier it
    /// is handed, could not satisfy that contract for a chained job, because
    /// the duplicate arrived under a new id every time. The contract was
    /// unsatisfiable by construction.
    ///
    /// Deriving the id from the predecessor fixes that without any schema
    /// change: `predecessor` is stable across its own redeliveries (the
    /// driver re-delivers the same serialized envelope), so the successor's
    /// id is stable too. Step *k* of a chain is a hash chain from its head,
    /// and a redelivered step re-pushes the id it pushed before.
    ///
    /// This makes the duplicate **detectable** on every driver. On drivers
    /// that implement [`QueueDriver::settle`](crate::queue::QueueDriver::settle)
    /// the duplicate does not arise in the first place, because the successor
    /// and the acknowledgement commit together — the stable id is then what
    /// keeps a replayed settlement addressing the same logical step rather
    /// than minting a new one.
    pub fn to_envelope_after(&self, predecessor: uuid::Uuid) -> Envelope {
        self.to_envelope_with_id(next_link_id(predecessor))
    }

    fn to_envelope_with_id(&self, id: uuid::Uuid) -> Envelope {
        let now = chrono::Utc::now();
        Envelope {
            schema_version: crate::queue::CURRENT_SCHEMA_VERSION,
            id,
            job_name: self.job_name.clone(),
            // Mirrors `routing::resolve_queue`: a centrally registered route
            // wins, then the queue the job declared for itself (captured into
            // `self.queue` at chain-build time, because the job is stored
            // type-erased here), then the driver default. Without the captured
            // fallback, `Job::queue()` was silently dropped for every chained
            // job — routed to a dedicated pool when pushed directly, dumped on
            // `default` when dispatched as part of a chain.
            queue: crate::queue::routing::route_for(&self.job_name)
                .and_then(|r| r.queue)
                .or_else(|| self.queue.clone()),
            payload: self.payload.clone(),
            dispatched_at: now,
            available_at: now,
            attempts: 0,
            max_tries: self.max_tries,
            backoff: self.backoff.clone(),
            timeout_secs: self.timeout_secs,
            fail_on_timeout: self.fail_on_timeout,
            idempotency_key: None,
            unique_lock_owner: None,
            batch_id: None,
            chain_remaining: Vec::new(),
        }
    }
}

/// Derive the envelope id for the link that runs after `predecessor`.
///
/// A UUIDv5 over the predecessor's id, so it is deterministic, collision-free
/// against a v4 space, and needs nothing persisted alongside it. See
/// [`ChainLink::to_envelope_after`] for why chain continuation must not mint
/// a random id.
pub fn next_link_id(predecessor: uuid::Uuid) -> uuid::Uuid {
    uuid::Uuid::new_v5(&predecessor, b"suprnova:queue:chain-next")
}

/// Builder used by [`Queue::chain`](crate::queue::Queue::chain). Mirrors
/// Laravel's `Bus::chain([...])->dispatch()`.
pub struct PendingChain {
    links: Vec<ChainLink>,
}

impl Default for PendingChain {
    fn default() -> Self {
        Self::new()
    }
}

impl PendingChain {
    /// Construct an empty pending chain with no links.
    pub fn new() -> Self {
        Self { links: Vec::new() }
    }

    /// Append a typed job to the chain.
    #[allow(clippy::should_implement_trait)]
    pub fn add<J: Job>(mut self, job: J) -> Result<Self, FrameworkError> {
        self.links.push(ChainLink::from_job(job)?);
        Ok(self)
    }

    /// Number of links queued so far.
    pub fn len(&self) -> usize {
        self.links.len()
    }
    /// `true` when the chain has no links queued yet.
    pub fn is_empty(&self) -> bool {
        self.links.is_empty()
    }

    /// Dispatch the chain. The first link is pushed immediately; the rest
    /// travel on its `chain_remaining` payload field.
    pub async fn dispatch(self) -> Result<(), FrameworkError> {
        if self.links.is_empty() {
            return Ok(());
        }
        let driver = crate::queue::current_driver()?;
        let mut iter = self.links.into_iter();
        let head = iter.next().unwrap();
        let tail: Vec<ChainLink> = iter.collect();
        let mut env = head.to_envelope();
        env.chain_remaining = tail;
        driver.push(env).await
    }
}
