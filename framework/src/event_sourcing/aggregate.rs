//! Shared bookkeeping follows the durable outcome when callback delivery is cancelled.
use super::{DomainEvent, EventStore, Fact, Snapshot, StoredEvent, error, store::executor};
use crate::database::transaction::OutcomeHooks;
use crate::{DB, FrameworkError};
use sea_orm::sea_query::{Alias, OnConflict, Query};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::sync::watch;

/// Application state rebuilt by applying declared facts in version order.
pub trait AggregateRoot:
    Default + Clone + Serialize + DeserializeOwned + Send + Sync + 'static
{
    /// Stable aggregate name used to route snapshot commands.
    const NAME: &'static str;
    /// Applies a fact without producing external effects.
    fn apply(&mut self, event: &Fact<'_>) -> Result<(), FrameworkError>;
}

/// Snapshot registration emitted by `#[aggregate]`.
pub struct AggregateEntry {
    /// Stable aggregate name.
    pub name: &'static str,
    /// Reconstructs the state and writes a snapshot.
    pub snapshot:
        fn(String) -> futures::future::BoxFuture<'static, Result<Snapshot, FrameworkError>>,
}
inventory::collect!(AggregateEntry);
impl AggregateEntry {
    /// Builds the snapshot callback for a registered aggregate state.
    pub fn snapshot<A: AggregateRoot>(
        uuid: String,
    ) -> futures::future::BoxFuture<'static, Result<Snapshot, FrameworkError>> {
        Box::pin(async move { Aggregate::<A>::retrieve(&uuid).await?.snapshot().await })
    }
}

/// Owns aggregate state, persisted version and unapplied storage work.
#[derive(Debug)]
pub struct Aggregate<A: AggregateRoot> {
    uuid: String,
    state: A,
    bookkeeping: Arc<Mutex<Bookkeeping>>,
    unsettled: Option<PendingPersistence>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum PersistenceResult {
    Pending,
    Committed,
    Failed,
}
#[derive(Debug)]
struct PendingPersistence {
    receiver: watch::Receiver<PersistenceResult>,
    timeout: Duration,
}

#[derive(Debug, Default)]
struct Bookkeeping {
    expected: i64,
    pending: Vec<StoredEvent>,
}
impl<A: AggregateRoot> Aggregate<A> {
    /// Creates an empty aggregate. Persist rejects an existing history.
    pub fn new(uuid: &str) -> Result<Self, FrameworkError> {
        Ok(Self {
            uuid: EventStore::uuid(uuid)?,
            state: A::default(),
            bookkeeping: Arc::new(Mutex::new(Bookkeeping::default())),
            unsettled: None,
        })
    }
    fn bookkeeping(&self) -> MutexGuard<'_, Bookkeeping> {
        self.bookkeeping
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
    /// Reports whether a cancelled persistence has been reconciled by this instance.
    pub fn is_settled(&self) -> bool {
        self.unsettled.is_none()
    }
    async fn settle(&mut self) -> Result<(), FrameworkError> {
        let Some(pending) = self.unsettled.as_mut() else {
            return Ok(());
        };
        let result = tokio::time::timeout(pending.timeout, async {
            loop {
                if *pending.receiver.borrow_and_update() != PersistenceResult::Pending {
                    return Ok::<(), ()>(());
                }
                pending.receiver.changed().await.map_err(|_| ())?;
            }
        })
        .await;
        match result {
            Ok(Ok(())) => {
                self.unsettled = None;
                Ok(())
            }
            _ => Err(FrameworkError::internal(format!(
                "aggregate '{}' transaction outcome is unknown; refusing reuse",
                self.uuid
            ))),
        }
    }
    /// Returns the current state, including recorded facts.
    pub fn state(&self) -> &A {
        &self.state
    }
    /// Returns the aggregate's canonical UUID.
    pub fn uuid(&self) -> &str {
        &self.uuid
    }
    /// Returns the last durable version known by this instance.
    pub fn persisted_version(&self) -> i64 {
        self.bookkeeping().expected
    }
    /// Returns a copy of facts that persistence has not committed yet.
    pub fn recorded(&self) -> Vec<StoredEvent> {
        self.bookkeeping().pending.clone()
    }
    /// Reconciles cancelled persistence before applying a new fact to a copy.
    pub async fn record<E: DomainEvent>(&mut self, event: E) -> Result<&mut Self, FrameworkError> {
        self.record_with_metadata(event, serde_json::json!({}))
            .await
    }
    /// Reconciles the prior outcome before recording application context.
    pub async fn record_with_metadata<E: DomainEvent>(
        &mut self,
        event: E,
        mut metadata: Value,
    ) -> Result<&mut Self, FrameworkError> {
        self.settle().await?;
        let object = metadata
            .as_object_mut()
            .ok_or_else(|| error("event metadata must be an object"))?;
        object.insert("aggregate_type".into(), A::NAME.into());
        let version = {
            let bookkeeping = self.bookkeeping();
            bookkeeping
                .expected
                .checked_add(
                    i64::try_from(bookkeeping.pending.len()).map_err(|e| error(e.to_string()))?,
                )
                .and_then(|v| v.checked_add(1))
                .ok_or_else(|| error("aggregate version exhausted"))?
        };
        let event = StoredEvent {
            id: 0,
            aggregate_uuid: self.uuid.clone(),
            aggregate_version: version,
            event_version: E::VERSION,
            event_type: E::NAME.into(),
            payload: serde_json::to_value(event).map_err(|e| error(e.to_string()))?,
            metadata,
            created_at: crate::clock::now().naive_utc(),
        };
        event.validate()?;
        let mut state = self.state.clone();
        state.apply(&event.fact())?;
        self.state = state;
        self.bookkeeping().pending.push(event);
        Ok(self)
    }
    /// Rebuilds from the latest snapshot and subsequent history.
    pub async fn retrieve(uuid: &str) -> Result<Self, FrameworkError> {
        Self::rebuild(uuid, true).await
    }
    /// Rebuilds from history alone to verify snapshots or recover state.
    pub async fn retrieve_without_snapshot(uuid: &str) -> Result<Self, FrameworkError> {
        Self::rebuild(uuid, false).await
    }
    async fn rebuild(uuid: &str, snapshot: bool) -> Result<Self, FrameworkError> {
        let mut aggregate = Self::new(uuid)?;
        if snapshot && let Some(snapshot) = EventStore::snapshot(uuid).await? {
            if snapshot.aggregate_version < 1 {
                return Err(error("snapshot version must be positive"));
            }
            let fact =
                EventStore::aggregate_page(&aggregate.uuid, snapshot.aggregate_version - 1, 1)
                    .await?;
            if !fact.first().is_some_and(|event| {
                event.aggregate_version == snapshot.aggregate_version
                    && event.metadata.get("aggregate_type").and_then(Value::as_str) == Some(A::NAME)
            }) {
                return Err(error(format!(
                    "snapshot for '{}' has no matching aggregate history",
                    aggregate.uuid
                )));
            }
            aggregate.state = serde_json::from_value(snapshot.state)
                .map_err(|e| error(format!("aggregate '{}': {e}", A::NAME)))?;
            aggregate.bookkeeping().expected = snapshot.aggregate_version;
        }
        loop {
            let page =
                EventStore::aggregate_page(&aggregate.uuid, aggregate.persisted_version(), 256)
                    .await?;
            if page.is_empty() {
                break;
            }
            for event in page {
                if event.aggregate_version != aggregate.persisted_version() + 1 {
                    return Err(error("aggregate history has a version gap"));
                }
                if event.metadata.get("aggregate_type").and_then(Value::as_str) != Some(A::NAME) {
                    return Err(error(format!(
                        "aggregate '{}' belongs to a different aggregate type",
                        aggregate.uuid
                    )));
                }
                aggregate.state.apply(&event.fact())?;
                aggregate.bookkeeping().expected = event.aggregate_version;
            }
        }
        Ok(aggregate)
    }
    /// Commits all recorded facts atomically. This operation owns its transaction.
    pub async fn persist(&mut self) -> Result<Vec<StoredEvent>, FrameworkError> {
        self.settle().await?;
        if DB::transaction_level() > 0 {
            return Err(FrameworkError::internal(format!(
                "aggregate '{}' persist must own its transaction",
                self.uuid
            )));
        }
        let (expected, pending) = {
            let bookkeeping = self.bookkeeping();
            (bookkeeping.expected, bookkeeping.pending.clone())
        };
        let Some(last) = pending.last() else {
            return Ok(Vec::new());
        };
        let committed = last.aggregate_version;
        let attempted = pending
            .first()
            .map_or(committed, |event| event.aggregate_version);
        let bookkeeping = Arc::clone(&self.bookkeeping);
        let timeout = DB::connection()?.outcome_settle_timeout();
        let (status, receiver) = watch::channel(PersistenceResult::Pending);
        self.unsettled = Some(PendingPersistence { receiver, timeout });
        let failed = status.clone();
        let hooks = OutcomeHooks::new(
            move || {
                let mut bookkeeping = bookkeeping
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                bookkeeping.expected = committed;
                bookkeeping
                    .pending
                    .retain(|event| event.aggregate_version > committed);
                status.send_replace(PersistenceResult::Committed);
            },
            move || {
                failed.send_replace(PersistenceResult::Failed);
            },
        );
        let uuid = self.uuid.clone();
        let result = DB::transaction_with_outcome(None, hooks, move |_tx| Box::pin(async move {
            EventStore::lock().await.map_err(|cause| storage(&uuid, attempted, cause))?;
            let actual = EventStore::version(&uuid).await.map_err(|cause| storage(&uuid, attempted, cause))?;
            if actual != expected { return Err(conflict(&uuid, expected, actual)); }
            let mut stored = Vec::new();
            for event in pending {
                let event = EventStore::insert(&event).await.map_err(|e| {
                    error(format!("aggregate '{uuid}' expected version {expected}, attempted version {}: {e}", event.aggregate_version))
                })?;
                stored.push(event);
            }
            super::projections::persisted(&stored).await?;
            Ok(stored)
        })).await;
        self.settle().await?;
        let (events, callback_error) = result??;
        if let Some(cause) = callback_error {
            return Err(error(format!(
                "aggregate '{}' committed through version {}; after-commit delivery failed: {cause}",
                self.uuid,
                self.persisted_version()
            )));
        }
        Ok(events)
    }
    /// Captures only persisted state. Repeating a capture at one version is safe.
    pub async fn snapshot(&self) -> Result<Snapshot, FrameworkError> {
        if !self.is_settled() {
            return Err(FrameworkError::internal(format!(
                "aggregate '{}' must settle before taking a snapshot",
                self.uuid
            )));
        }
        let (expected, pending_empty) = {
            let bookkeeping = self.bookkeeping();
            (bookkeeping.expected, bookkeeping.pending.is_empty())
        };
        if expected == 0 {
            return Err(error(
                "cannot snapshot an aggregate without persisted history",
            ));
        }
        if !pending_empty {
            return Err(error("persist recorded events before taking a snapshot"));
        }
        let snapshot = Snapshot {
            uuid: self.uuid.clone(),
            aggregate_version: expected,
            state: serde_json::to_value(&self.state).map_err(|e| error(e.to_string()))?,
            created_at: crate::clock::now().naive_utc(),
        };
        crate::render_cache::orm::atomic(None, || async move {
            let exec = executor().await?;
            let query = Query::insert()
                .into_table(Alias::new("snapshots"))
                .columns(["uuid", "aggregate_version", "state", "created_at"].map(Alias::new))
                .values([
                    snapshot.uuid.clone().into(),
                    snapshot.aggregate_version.into(),
                    snapshot.state.clone().into(),
                    snapshot.created_at.into(),
                ])
                .map_err(|e| error(e.to_string()))?
                .on_conflict(
                    OnConflict::columns([Alias::new("uuid"), Alias::new("aggregate_version")])
                        .update_columns([Alias::new("state"), Alias::new("created_at")])
                        .to_owned(),
                )
                .to_owned();
            exec.run(exec.backend().build(&query)).await?;
            crate::render_cache::orm::after_table_write("snapshots").await?;
            Ok(snapshot)
        })
        .await
    }
}
fn conflict(uuid: &str, expected: i64, actual: i64) -> FrameworkError {
    FrameworkError::conflict(format!(
        "aggregate '{uuid}' expected version {expected}, actual version {actual}"
    ))
}

fn storage(uuid: &str, attempted: i64, cause: FrameworkError) -> FrameworkError {
    FrameworkError::database(format!(
        "aggregate '{uuid}' attempted version {attempted}: {cause}"
    ))
}

#[cfg(test)]
mod settlement_tests {
    use super::*;
    #[derive(Clone, Default, Serialize, serde::Deserialize, Debug)]
    struct State;
    impl AggregateRoot for State {
        const NAME: &'static str = "tests.settlement";
        fn apply(&mut self, _fact: &Fact<'_>) -> Result<(), FrameworkError> {
            Ok(())
        }
    }
    #[derive(Serialize, serde::Deserialize)]
    struct Added;
    impl DomainEvent for Added {
        const NAME: &'static str = "tests.settlement.added";
        const VERSION: i32 = 1;
    }
    #[tokio::test]
    async fn es_001_unresolved_outcome_is_bounded_by_the_settlement_wait() {
        let config = crate::DatabaseConfig::builder()
            .url("sqlite::memory:")
            .outcome_settle_timeout(1)
            .build();
        let connection = crate::DbConnection::connect(&config).await.unwrap();
        assert_eq!(connection.outcome_settle_timeout(), Duration::from_secs(1));
        let mut root = Aggregate::<State>::new("30436483-b26b-4d9e-8cd2-46177926f838").unwrap();
        let (status, receiver) = watch::channel(PersistenceResult::Pending);
        root.unsettled = Some(PendingPersistence {
            receiver,
            timeout: connection.outcome_settle_timeout(),
        });
        let started = std::time::Instant::now();
        let error = root.record(Added).await.unwrap_err();
        assert!(matches!(error, FrameworkError::Internal { .. }));
        assert!(error.to_string().contains(root.uuid()));
        assert!(error.to_string().contains("outcome is unknown"));
        assert!(started.elapsed() >= Duration::from_secs(1));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(!root.is_settled());
        assert!(root.recorded().is_empty());
        status.send_replace(PersistenceResult::Failed);
        root.settle().await.unwrap();
        assert!(root.is_settled());
        assert_eq!(root.persisted_version(), 0);
        assert!(
            crate::DatabaseConfig::builder()
                .outcome_settle_timeout(0)
                .build()
                .validate_pool()
                .is_err()
        );
    }
}
