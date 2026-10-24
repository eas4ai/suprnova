//! Hub-level topic signals used by Inertia Live.

use super::hub::BroadcastEnvelope;
use crate::FrameworkError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, hash_map::Entry};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tokio::time::Instant;

/// The data carried by an Inertia Live topic signal.
///
/// Signals carry invalidation metadata only. Page props are recomputed by the
/// receiving page's normal Inertia request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LiveTopicSignal {
    /// The application-defined topic that changed.
    pub topic: String,
    /// The publication position within the retained instance lifetime. Retirement resets it to zero.
    pub cursor: u64,
    /// Lifetime allocation, unique and increasing within its authority.
    /// Memory preserves uniqueness across in-process restarts and uses a process
    /// nonce to make reuse across process restarts negligibly likely.
    /// Clients compare generations for inequality only.
    pub generation: u64,
    /// The page props to reload. An empty list requests a full reload.
    pub props: Vec<String>,
    /// Internal hub routing metadata, removed before delivery to a browser.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclusion: Option<LiveTopicExclusion>,
}

/// One authority baseline identifies both the retained state and its publication position.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LiveTopicPosition {
    /// Lifetime allocation, unique and increasing within its authority.
    /// Memory preserves uniqueness across in-process restarts and uses a process
    /// nonce to make reuse across process restarts negligibly likely.
    /// Clients compare generations for inequality only.
    pub generation: u64,
    /// Accepted publications within this generation.
    pub cursor: u64,
}

struct TopicClock {
    wall_millis: u64,
    start: Instant,
    override_millis: std::sync::atomic::AtomicU64,
}
impl Default for TopicClock {
    fn default() -> Self {
        let wall_millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .clamp(1, 9_007_199_254_740_991) as u64;
        Self {
            wall_millis,
            start: Instant::now(),
            override_millis: std::sync::atomic::AtomicU64::new(0),
        }
    }
}
impl TopicClock {
    fn now(&self) -> u64 {
        let value = self
            .override_millis
            .load(std::sync::atomic::Ordering::Relaxed);
        if value != 0 {
            return value;
        }
        self.wall_millis
            .saturating_add(self.start.elapsed().as_millis().min(u64::MAX as u128) as u64)
            .min(9_007_199_254_740_991)
    }
}

/// Sender identity used only to route an internal hub exclusion.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum LiveTopicSender {
    /// A browser session owns the originating document.
    Session(String),
    /// A sessionless request owns the document through its authenticated principal.
    Principal(String),
}

/// Binds an untrusted socket header to the request that published the signal.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LiveTopicExclusion {
    /// The originating Live document instance.
    pub socket: String,
    /// The identity captured before deferring publication.
    pub sender: LiveTopicSender,
}

/// Reserved hub channel prefix for Inertia Live topic signals.
pub(crate) const LIVE_TOPIC_CHANNEL_PREFIX: &str = "__live.topic.";

/// Event name used by every Inertia Live topic signal envelope.
pub(crate) const LIVE_TOPIC_EVENT: &str = "inertia.live";

/// Return the reserved hub channel for `topic`.
pub(crate) fn live_topic_channel(topic: &str) -> String {
    format!("{LIVE_TOPIC_CHANNEL_PREFIX}{topic}")
}

/// Build the reserved envelope for one typed signal without a fallible
/// serialization step.
pub(crate) fn live_topic_envelope(
    signal: LiveTopicSignal,
    except: Option<LiveTopicExclusion>,
) -> BroadcastEnvelope {
    let channel = live_topic_channel(&signal.topic);
    let mut data = serde_json::json!({
        "topic": signal.topic,
        "cursor": signal.cursor,
        "generation": signal.generation,
        "props": signal.props,
    });
    if let Some(exclusion) = &except {
        data["exclusion"] = serde_json::json!(exclusion);
    }
    BroadcastEnvelope {
        channel,
        event: LIVE_TOPIC_EVENT.to_owned(),
        data,
        except: except.map(|exclusion| exclusion.socket),
    }
}

/// One publication accepted by the in-process rate limiter.
pub(crate) enum AcceptedLiveTopicPublication {
    /// Publish this signal now.
    Immediate {
        signal: LiveTopicSignal,
        except: Option<LiveTopicExclusion>,
    },
    /// A task must flush the pending union at this deadline.
    Pending { deadline: Instant },
    /// Another task already owns the pending flush.
    Coalesced,
}

struct LiveTopicEntry {
    last_activity: Instant,
    idle_deadline: Option<Instant>,
    retention: Option<Duration>,
    memberships: usize,
    cursor: u64,
    generation: u64,
    next_publication: Option<Instant>,
    pending: Option<PendingLiveTopicSignal>,
}

impl LiveTopicEntry {
    fn new(retention: Option<Duration>, generation: u64) -> Self {
        Self {
            last_activity: Instant::now(),
            idle_deadline: None,
            retention,
            memberships: 0,
            cursor: 0,
            generation,
            next_publication: None,
            pending: None,
        }
    }

    fn expired(&self) -> bool {
        self.memberships == 0
            && self.pending.is_none()
            && self
                .retention
                .is_some_and(|window| self.last_activity.elapsed() >= window)
    }
}

/// Boot-registered family policies do not retain individual instance names.
#[derive(Clone, Default)]
pub(crate) struct LiveTopicRetention(Arc<Mutex<HashMap<String, Duration>>>);

impl LiveTopicRetention {
    pub(crate) fn configure(
        &self,
        family: &str,
        retention: Duration,
    ) -> Result<(), FrameworkError> {
        if retention.is_zero() {
            return Err(FrameworkError::internal(
                "Live topic instance retention must be positive",
            ));
        }
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(family.to_owned(), retention);
        Ok(())
    }

    pub(crate) fn window(&self, topic: &str) -> Option<Duration> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .find_map(|(family, retention)| {
                topic
                    .strip_prefix(family)
                    .filter(|key| key.starts_with('.') && key.len() > 1)
                    .map(|_| *retention)
            })
    }
}

struct PendingLiveTopicSignal {
    /// `None` means one accepted publication requested every prop.
    props: Option<BTreeSet<String>>,
    except: Option<LiveTopicExclusion>,
    interval: Duration,
}

impl PendingLiveTopicSignal {
    fn new(props: &[String], except: Option<LiveTopicExclusion>, interval: Duration) -> Self {
        Self {
            props: normalized_props(props),
            except,
            interval,
        }
    }

    fn merge(&mut self, props: &[String], except: Option<LiveTopicExclusion>, interval: Duration) {
        if let Some(current) = self.props.as_mut() {
            match normalized_props(props) {
                Some(incoming) => current.extend(incoming),
                None => self.props = None,
            }
            // A full invalidation preserves changes beyond the wire's name budget.
            if self.props.as_ref().is_some_and(|props| props.len() > 256) {
                self.props = None;
            }
        }
        if self.except != except {
            self.except = None;
        }
        self.interval = interval;
    }

    fn into_parts(self) -> (Vec<String>, Option<LiveTopicExclusion>, Duration) {
        (
            self.props
                .map(|props| props.into_iter().collect())
                .unwrap_or_default(),
            self.except,
            self.interval,
        )
    }
}

fn normalized_props(props: &[String]) -> Option<BTreeSet<String>> {
    let props = props.iter().cloned().collect::<BTreeSet<_>>();
    (!props.is_empty() && props.len() <= 256).then_some(props)
}

#[derive(Default)]
struct LiveTopicTable {
    topics: HashMap<String, LiveTopicEntry>,
    idle: BTreeSet<(Instant, String)>,
    last_prune: Option<Instant>,
}

/// Shared cursor, expiry index and coalescing state for one authority.
#[derive(Clone, Default)]
pub(crate) struct LiveTopicRuntime {
    table: Arc<Mutex<LiveTopicTable>>,
    clock: Arc<TopicClock>,
    retention: LiveTopicRetention,
    generations: Arc<TopicGenerationAllocator>,
    sweeps: Arc<std::sync::atomic::AtomicU64>,
    visits: Arc<std::sync::atomic::AtomicU64>,
}

const TOPIC_NONCE_BITS: u32 = 22;
const TOPIC_NONCE_MASK: u32 = (1 << TOPIC_NONCE_BITS) - 1;
static TOPIC_GENERATION_HIGH_WATER: AtomicU64 = AtomicU64::new(0);

fn process_topic_nonce() -> &'static Result<u32, String> {
    static NONCE: OnceLock<Result<u32, String>> = OnceLock::new();
    NONCE.get_or_init(|| {
        let mut bytes = [0; 4];
        getrandom::fill(&mut bytes)
            .map_err(|error| format!("Live topic process nonce: OS randomness failed: {error}"))?;
        Ok(u32::from_le_bytes(bytes) & TOPIC_NONCE_MASK)
    })
}

enum TopicGenerationAllocator {
    Process(&'static Result<u32, String>),
    #[cfg(test)]
    SimulatedProcess {
        nonce: u32,
        high_water: AtomicU64,
    },
}

impl Default for TopicGenerationAllocator {
    fn default() -> Self {
        Self::Process(process_topic_nonce())
    }
}

impl TopicGenerationAllocator {
    fn allocate(&self, millis: u64) -> Result<u64, FrameworkError> {
        let seconds = millis / 1000;
        if seconds >= 1 << 31 {
            return Err(FrameworkError::internal("Live topic generation exhausted"));
        }
        let (nonce, high_water) = match self {
            Self::Process(nonce) => (
                *nonce
                    .as_ref()
                    .map_err(|error| FrameworkError::internal(error.clone()))?,
                &TOPIC_GENERATION_HIGH_WATER,
            ),
            #[cfg(test)]
            Self::SimulatedProcess { nonce, high_water } => (*nonce, high_water),
        };
        let clock_input = (seconds << TOPIC_NONCE_BITS) | u64::from(nonce);
        let mut last = high_water.load(Ordering::Relaxed);
        loop {
            let next = last
                .checked_add(1)
                .filter(|value| *value <= suprnova_live::canonical::MAX_SAFE_INTEGER)
                .ok_or_else(|| FrameworkError::internal("Live topic generation exhausted"))?;
            let generation = clock_input.max(next);
            match high_water.compare_exchange_weak(
                last,
                generation,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Ok(generation),
                Err(current) => last = current,
            }
        }
    }
}

impl LiveTopicRuntime {
    /// Keeps exhaustion and process simulations separate from production allocations.
    #[cfg(test)]
    pub(crate) fn with_nonce_for_test(nonce: u32) -> Result<Self, FrameworkError> {
        if nonce > TOPIC_NONCE_MASK {
            return Err(FrameworkError::internal(
                "Live topic nonce must fit in 22 bits",
            ));
        }
        Ok(Self {
            generations: Arc::new(TopicGenerationAllocator::SimulatedProcess {
                nonce,
                high_water: AtomicU64::new(0),
            }),
            ..Self::default()
        })
    }

    pub(crate) fn set_clock_for_test(&self, millis: u64) -> Result<(), FrameworkError> {
        if millis == 0 || millis > suprnova_live::canonical::MAX_SAFE_INTEGER {
            return Err(FrameworkError::internal("invalid Live topic test clock"));
        }
        self.clock
            .override_millis
            .store(millis, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    pub(crate) fn configure_family(
        &self,
        family: &str,
        retention: Duration,
    ) -> Result<(), FrameworkError> {
        self.retention.configure(family, retention)
    }

    fn with_entry<R>(
        &self,
        table: &mut LiveTopicTable,
        topic: &str,
        update: impl FnOnce(&mut LiveTopicEntry) -> R,
    ) -> Result<R, FrameworkError> {
        if let Some(deadline) = table
            .topics
            .get(topic)
            .and_then(|entry| entry.idle_deadline)
        {
            table.idle.remove(&(deadline, topic.to_owned()));
        }
        if table.topics.get(topic).is_some_and(LiveTopicEntry::expired) {
            table.topics.remove(topic);
        }
        let entry = match table.topics.entry(topic.to_owned()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let generation = self.generations.allocate(self.clock.now())?;
                entry.insert(LiveTopicEntry::new(
                    self.retention.window(topic),
                    generation,
                ))
            }
        };
        entry.last_activity = Instant::now();
        let result = update(entry);
        entry.idle_deadline = entry
            .retention
            .filter(|_| entry.memberships == 0 && entry.pending.is_none())
            .and_then(|window| entry.last_activity.checked_add(window));
        if let Some(deadline) = entry.idle_deadline {
            table.idle.insert((deadline, topic.to_owned()));
        }
        Ok(result)
    }

    pub(crate) fn membership(&self, topic: &str, active: bool) -> Result<(), FrameworkError> {
        let mut table = self
            .table
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.with_entry(&mut table, topic, |entry| {
            if active {
                entry.memberships += 1;
            } else {
                entry.memberships = entry.memberships.saturating_sub(1);
            }
        })
    }

    pub(crate) fn sweep_stats(&self) -> (u64, u64) {
        use std::sync::atomic::Ordering;
        (
            self.sweeps.load(Ordering::Relaxed),
            self.visits.load(Ordering::Relaxed),
        )
    }

    /// All callers share one throttle and visit only eligible candidates.
    pub(crate) fn prune(&self) -> usize {
        self.sweep().unwrap_or(0)
    }

    pub(crate) fn sweep(&self) -> Option<usize> {
        let mut table = self
            .table
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if table
            .last_prune
            .is_some_and(|at| at.elapsed() < Duration::from_secs(1))
        {
            return None;
        }
        table.last_prune = Some(Instant::now());
        self.sweeps
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let now = Instant::now();
        let mut retired = 0;
        while table
            .idle
            .first()
            .is_some_and(|(deadline, _)| *deadline <= now)
        {
            let Some((_, topic)) = table.idle.pop_first() else {
                break;
            };
            self.visits
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            table.topics.remove(&topic);
            retired += 1;
        }
        Some(retired)
    }

    /// Reads count as activity, so a rendered instance keeps its baseline for the window.
    pub(crate) async fn cursor(&self, topic: &str) -> Result<u64, FrameworkError> {
        Ok(self.cursors(&[topic.to_owned()]).await?[0])
    }

    pub(crate) async fn cursors(&self, names: &[String]) -> Result<Vec<u64>, FrameworkError> {
        Ok(self
            .positions(names)
            .await?
            .into_iter()
            .map(|position| position.cursor)
            .collect())
    }

    /// Generation and cursor are captured together under the same authority lock.
    pub(crate) async fn positions(
        &self,
        names: &[String],
    ) -> Result<Vec<LiveTopicPosition>, FrameworkError> {
        self.prune();
        let mut table = self
            .table
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        names
            .iter()
            .map(|name| {
                self.with_entry(&mut table, name, |entry| LiveTopicPosition {
                    cursor: entry.cursor,
                    generation: entry.generation,
                })
            })
            .collect()
    }

    /// Accept a publication and decide whether it publishes or coalesces.
    pub(crate) async fn accept(
        &self,
        topic: &str,
        props: &[String],
        except: Option<LiveTopicExclusion>,
        interval: Duration,
    ) -> Result<AcceptedLiveTopicPublication, FrameworkError> {
        self.prune();
        let now = Instant::now();
        let interval = interval.max(Duration::from_millis(1));
        let mut table = self
            .table
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.with_entry(&mut table, topic, |entry| {
            entry.cursor = entry.cursor.checked_add(1).ok_or_else(|| {
                FrameworkError::internal(format!(
                    "BroadcastHub::publish_live_topic: cursor exhausted for topic '{topic}'"
                ))
            })?;

            if entry
                .next_publication
                .is_none_or(|deadline| deadline <= now)
                && entry.pending.is_none()
            {
                entry.next_publication = Some(now + interval);
                return Ok(AcceptedLiveTopicPublication::Immediate {
                    signal: LiveTopicSignal {
                        topic: topic.to_owned(),
                        cursor: entry.cursor,
                        generation: entry.generation,
                        exclusion: None,
                        props: normalized_props(props)
                            .map(|props| props.into_iter().collect())
                            .unwrap_or_default(),
                    },
                    except,
                });
            }

            let needs_task = entry.pending.is_none();
            match entry.pending.as_mut() {
                Some(pending) => pending.merge(props, except, interval),
                None => {
                    entry.pending = Some(PendingLiveTopicSignal::new(props, except, interval));
                }
            }
            if needs_task {
                Ok(AcceptedLiveTopicPublication::Pending {
                    deadline: entry.next_publication.unwrap_or(now),
                })
            } else {
                Ok(AcceptedLiveTopicPublication::Coalesced)
            }
        })?
    }

    /// Take the pending union and start its next rate window.
    pub(crate) async fn take_pending(
        &self,
        topic: &str,
    ) -> Result<Option<(LiveTopicSignal, Option<LiveTopicExclusion>)>, FrameworkError> {
        let mut table = self
            .table
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !table.topics.contains_key(topic) {
            return Ok(None);
        }
        self.with_entry(&mut table, topic, |entry| {
            let pending = entry.pending.take()?;
            entry.last_activity = Instant::now();
            let (props, except, interval) = pending.into_parts();
            entry.next_publication = Some(Instant::now() + interval);
            Some((
                LiveTopicSignal {
                    topic: topic.to_owned(),
                    cursor: entry.cursor,
                    generation: entry.generation,
                    exclusion: None,
                    props,
                },
                except,
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::hub::{BroadcastHub, InMemoryBroadcastHub};
    use std::time::Duration;

    #[tokio::test(start_paused = true)]
    async fn signal_fix4_001_generation_exhaustion_fails_closed() {
        let hub = InMemoryBroadcastHub::with_live_topic_nonce_for_test((1 << 22) - 1).unwrap();
        hub.configure_live_topic_family("allocation", Duration::from_secs(1))
            .unwrap();
        hub.set_live_topic_clock_for_test(((1_u64 << 31) - 1) * 1000)
            .unwrap();
        let first = hub
            .live_topic_positions(&["allocation.first".into()])
            .await
            .unwrap()[0];
        assert_eq!(first.generation, suprnova_live::canonical::MAX_SAFE_INTEGER);
        let error = hub
            .live_topic_positions(&["allocation.second".into()])
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("generation exhausted"),
            "{error}"
        );
        assert_eq!(
            hub.live_topic_positions(&["allocation.first".into()])
                .await
                .unwrap()[0],
            first
        );
    }

    #[tokio::test(start_paused = true)]
    async fn signal_fix5_001_process_nonces_distinguish_the_same_clock_reading() {
        let first = InMemoryBroadcastHub::with_live_topic_nonce_for_test(17).unwrap();
        let restarted = InMemoryBroadcastHub::with_live_topic_nonce_for_test(29).unwrap();
        first.set_live_topic_clock_for_test(10000).unwrap();
        restarted.set_live_topic_clock_for_test(10000).unwrap();
        let topics = ["restart.owner".into()];
        let old = first.live_topic_positions(&topics).await.unwrap()[0];
        let new = restarted.live_topic_positions(&topics).await.unwrap()[0];
        assert_ne!(old.generation, new.generation);
        assert_eq!(old.generation, (10 << 22) | 17);
        assert_eq!(new.generation, (10 << 22) | 29);
    }

    #[tokio::test(start_paused = true)]
    async fn signal_fix5_001_clock_overflow_fails_without_consuming_a_generation() {
        let hub = InMemoryBroadcastHub::with_live_topic_nonce_for_test(17).unwrap();
        hub.set_live_topic_clock_for_test((1_u64 << 31) * 1000)
            .unwrap();
        let topics = ["restart.owner".into()];
        let error = hub.live_topic_positions(&topics).await.unwrap_err();
        assert!(
            error.to_string().contains("generation exhausted"),
            "{error}"
        );
        hub.set_live_topic_clock_for_test(10000).unwrap();
        let position = hub.live_topic_positions(&topics).await.unwrap()[0];
        assert_eq!(position.generation, (10 << 22) | 17);
    }
}
