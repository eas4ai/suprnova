//! Hub-level topic signals used by Inertia Live.

use super::hub::BroadcastEnvelope;
use crate::FrameworkError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;

/// The data carried by an Inertia Live topic signal.
///
/// Signals carry invalidation metadata only. Page props are recomputed by the
/// receiving page's normal Inertia request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LiveTopicSignal {
    /// The application-defined topic that changed.
    pub topic: String,
    /// The topic's monotonically increasing publication cursor.
    pub cursor: u64,
    /// The page props to reload. An empty list requests a full reload.
    pub props: Vec<String>,
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
    except: Option<String>,
) -> BroadcastEnvelope {
    let channel = live_topic_channel(&signal.topic);
    let data = serde_json::json!({
        "topic": signal.topic,
        "cursor": signal.cursor,
        "props": signal.props,
    });
    BroadcastEnvelope {
        channel,
        event: LIVE_TOPIC_EVENT.to_owned(),
        data,
        except,
    }
}

/// One publication accepted by the in-process rate limiter.
pub(crate) enum AcceptedLiveTopicPublication {
    /// Publish this signal now.
    Immediate {
        signal: LiveTopicSignal,
        except: Option<String>,
    },
    /// A task must flush the pending union at this deadline.
    Pending { deadline: Instant },
    /// Another task already owns the pending flush.
    Coalesced,
}

#[derive(Default)]
struct LiveTopicEntry {
    cursor: u64,
    next_publication: Option<Instant>,
    pending: Option<PendingLiveTopicSignal>,
}

struct PendingLiveTopicSignal {
    /// `None` means one accepted publication requested every prop.
    props: Option<BTreeSet<String>>,
    except: Option<String>,
    interval: Duration,
}

impl PendingLiveTopicSignal {
    fn new(props: &[String], except: Option<String>, interval: Duration) -> Self {
        Self {
            props: normalized_props(props),
            except,
            interval,
        }
    }

    fn merge(&mut self, props: &[String], except: Option<String>, interval: Duration) {
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

    fn into_parts(self) -> (Vec<String>, Option<String>, Duration) {
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
}

impl LiveTopicRuntime {
    /// Read the last cursor accepted for `topic`.
    pub(crate) async fn cursor(&self, topic: &str) -> u64 {
        self.topics
            .lock()
            .await
            .get(topic)
            .map_or(0, |entry| entry.cursor)
    }

    /// Captures all requested baselines under the same authority lock before a loader runs.
    pub(crate) async fn cursors(&self, names: &[String]) -> Vec<u64> {
        let topics = self.topics.lock().await;
        names
            .iter()
            .map(|name| topics.get(name).map_or(0, |entry| entry.cursor))
            .collect()
    }

    /// Accept a publication and decide whether it publishes or coalesces.
    pub(crate) async fn accept(
        &self,
        topic: &str,
        props: &[String],
        except: Option<String>,
        interval: Duration,
    ) -> Result<AcceptedLiveTopicPublication, FrameworkError> {
        let now = Instant::now();
        let interval = interval.max(Duration::from_millis(1));
        let mut topics = self.topics.lock().await;
        let entry = topics.entry(topic.to_owned()).or_default();
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
    ) -> Option<(LiveTopicSignal, Option<String>)> {
        let mut topics = self.topics.lock().await;
        let entry = topics.get_mut(topic)?;
        let pending = entry.pending.take()?;
        let (props, except, interval) = pending.into_parts();
        entry.next_publication = Some(Instant::now() + interval);
        Some((
            LiveTopicSignal {
                topic: topic.to_owned(),
                cursor: entry.cursor,
                props,
            },
            except,
        ))
    }
}
