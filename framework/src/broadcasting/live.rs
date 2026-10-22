//! Hub-level topic signals used by Inertia Live.

use super::hub::BroadcastEnvelope;
use crate::FrameworkError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::Mutex;
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
    /// The page props to reload. An empty list requests a full reload.
    pub props: Vec<String>,
    /// Internal hub routing metadata, removed before delivery to a browser.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclusion: Option<LiveTopicExclusion>,
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
    retention: Option<Duration>,
    memberships: usize,
    cursor: u64,
    next_publication: Option<Instant>,
    pending: Option<PendingLiveTopicSignal>,
}

impl LiveTopicEntry {
    fn new(retention: Option<Duration>) -> Self {
        Self {
            last_activity: Instant::now(),
            retention,
            memberships: 0,
            cursor: 0,
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

/// Shared cursor and coalescing state for one in-process authority.
#[derive(Clone, Default)]
pub(crate) struct LiveTopicRuntime {
    topics: Arc<Mutex<HashMap<String, LiveTopicEntry>>>,
    retention: LiveTopicRetention,
    last_prune: Arc<Mutex<Option<Instant>>>,
}

impl LiveTopicRuntime {
    pub(crate) fn configure_family(
        &self,
        family: &str,
        retention: Duration,
    ) -> Result<(), FrameworkError> {
        self.retention.configure(family, retention)
    }

    fn prune_on_activity(&self) {
        let due = {
            let mut last = self
                .last_prune
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if last.is_none_or(|at| at.elapsed() >= Duration::from_secs(1)) {
                *last = Some(Instant::now());
                true
            } else {
                false
            }
        };
        if due {
            self.prune();
        }
    }

    fn entry<'a>(
        &self,
        topics: &'a mut HashMap<String, LiveTopicEntry>,
        topic: &str,
    ) -> &'a mut LiveTopicEntry {
        if topics.get(topic).is_some_and(LiveTopicEntry::expired) {
            topics.remove(topic);
        }
        let entry = topics
            .entry(topic.to_owned())
            .or_insert_with(|| LiveTopicEntry::new(self.retention.window(topic)));
        entry.last_activity = Instant::now();
        entry
    }

    pub(crate) fn membership(&self, topic: &str, active: bool) {
        let mut topics = self
            .topics
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = self.entry(&mut topics, topic);
        if active {
            entry.memberships += 1;
        } else {
            entry.memberships = entry.memberships.saturating_sub(1);
        }
    }

    pub(crate) fn prune(&self) -> usize {
        let mut topics = self
            .topics
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let before = topics.len();
        topics.retain(|_, entry| !entry.expired());
        before - topics.len()
    }

    /// Reads count as activity, so a rendered instance keeps its baseline for the window.
    pub(crate) async fn cursor(&self, topic: &str) -> u64 {
        self.cursors(&[topic.to_owned()]).await[0]
    }

    /// Captures all requested baselines under one authority lock before a loader runs.
    pub(crate) async fn cursors(&self, names: &[String]) -> Vec<u64> {
        self.prune_on_activity();
        let mut topics = self
            .topics
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        names
            .iter()
            .map(|name| self.entry(&mut topics, name).cursor)
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
        self.prune_on_activity();
        let now = Instant::now();
        let interval = interval.max(Duration::from_millis(1));
        let mut topics = self
            .topics
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = self.entry(&mut topics, topic);
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
    }

    /// Take the pending union and start its next rate window.
    pub(crate) async fn take_pending(
        &self,
        topic: &str,
    ) -> Option<(LiveTopicSignal, Option<LiveTopicExclusion>)> {
        let mut topics = self
            .topics
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = topics.get_mut(topic)?;
        let pending = entry.pending.take()?;
        entry.last_activity = Instant::now();
        let (props, except, interval) = pending.into_parts();
        entry.next_publication = Some(Instant::now() + interval);
        Some((
            LiveTopicSignal {
                topic: topic.to_owned(),
                cursor: entry.cursor,
                exclusion: None,
                props,
            },
            except,
        ))
    }
}
