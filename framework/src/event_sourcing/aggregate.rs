//! Recording and persistence have separate state so failed writes retain pending facts.
use super::{DomainEvent, EventStore, Snapshot, StoredEvent, error, store::executor};
use crate::{DB, FrameworkError};
use sea_orm::sea_query::{Alias, OnConflict, Query};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

/// Application state rebuilt by applying declared facts in version order.
pub trait AggregateRoot:
    Default + Clone + Serialize + DeserializeOwned + Send + Sync + 'static
{
    /// Stable aggregate name used to route snapshot commands.
    const NAME: &'static str;
    /// Applies a fact without producing external effects.
    fn apply(&mut self, event: &StoredEvent) -> Result<(), FrameworkError>;
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
    expected: i64,
    pending: Vec<StoredEvent>,
}
impl<A: AggregateRoot> Aggregate<A> {
    /// Creates an empty aggregate. Persist rejects an existing history.
    pub fn new(uuid: &str) -> Result<Self, FrameworkError> {
        Ok(Self {
            uuid: EventStore::uuid(uuid)?,
            state: A::default(),
            expected: 0,
            pending: Vec::new(),
        })
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
        self.expected
    }
    /// Returns recorded facts that persistence has not committed yet.
    pub fn recorded(&self) -> &[StoredEvent] {
        &self.pending
    }
    /// Applies a fact before recording it. Failure leaves state unchanged.
    pub fn record<E: DomainEvent>(&mut self, event: E) -> Result<&mut Self, FrameworkError> {
        self.record_with_metadata(event, serde_json::json!({}))
    }
    /// Adds application context while preserving the aggregate identity.
    pub fn record_with_metadata<E: DomainEvent>(
        &mut self,
        event: E,
        mut metadata: Value,
    ) -> Result<&mut Self, FrameworkError> {
        let object = metadata
            .as_object_mut()
            .ok_or_else(|| error("event metadata must be an object"))?;
        object.insert("aggregate_type".into(), A::NAME.into());
        let version = self
            .expected
            .checked_add(i64::try_from(self.pending.len()).map_err(|e| error(e.to_string()))?)
            .and_then(|v| v.checked_add(1))
            .ok_or_else(|| error("aggregate version exhausted"))?;
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
        state.apply(&event)?;
        self.state = state;
        self.pending.push(event);
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
            aggregate.expected = snapshot.aggregate_version;
        }
        loop {
            let page = EventStore::aggregate_page(&aggregate.uuid, aggregate.expected, 256).await?;
            if page.is_empty() {
                break;
            }
            for event in page {
                if event.aggregate_version != aggregate.expected + 1 {
                    return Err(error("aggregate history has a version gap"));
                }
                if event.metadata.get("aggregate_type").and_then(Value::as_str) != Some(A::NAME) {
                    return Err(error(format!(
                        "aggregate '{}' belongs to a different aggregate type",
                        aggregate.uuid
                    )));
                }
                aggregate.state.apply(&event)?;
                aggregate.expected = event.aggregate_version;
            }
        }
        Ok(aggregate)
    }
    /// Commits all recorded facts atomically. This operation owns its transaction.
    pub async fn persist(&mut self) -> Result<Vec<StoredEvent>, FrameworkError> {
        if DB::transaction_level() > 0 {
            return Err(error("aggregate persist must own its transaction"));
        }
        if self.pending.is_empty() {
            return Ok(Vec::new());
        }
        let uuid = self.uuid.clone();
        let expected = self.expected;
        let pending = self.pending.clone();
        let result = DB::transaction_with_isolation(None, move |_tx| Box::pin(async move {
            EventStore::lock().await?;
            let actual = EventStore::version(&uuid).await?;
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
        })).await??;
        let (events, callback_error) = result;
        if let Some(last) = events.last() {
            self.expected = last.aggregate_version;
        }
        self.pending.clear();
        if let Some(cause) = callback_error {
            return Err(error(format!(
                "aggregate '{}' committed through version {}; after-commit delivery failed: {cause}",
                self.uuid, self.expected
            )));
        }
        Ok(events)
    }
    /// Captures only persisted state. Repeating a capture at one version is safe.
    pub async fn snapshot(&self) -> Result<Snapshot, FrameworkError> {
        if self.expected == 0 {
            return Err(error(
                "cannot snapshot an aggregate without persisted history",
            ));
        }
        if !self.pending.is_empty() {
            return Err(error("persist recorded events before taking a snapshot"));
        }
        let snapshot = Snapshot {
            uuid: self.uuid.clone(),
            aggregate_version: self.expected,
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
    error(format!(
        "aggregate '{uuid}' expected version {expected}, actual version {actual}"
    ))
}
