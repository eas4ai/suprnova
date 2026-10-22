//! Primary-database reads keep reconstruction consistent with persistence.
use super::error;
use crate::{FrameworkError, database::transaction::ExecutorChoice};
use sea_orm::sea_query::{Alias, Expr, ExprTrait, Order, Query};
use sea_orm::{FromQueryResult, QueryResult};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

/// A serialized domain fact whose identity survives Rust type renames.
pub trait DomainEvent: Serialize + DeserializeOwned + Send + Sync + 'static {
    /// Stable application name stored in every envelope.
    const NAME: &'static str;
    /// Current payload schema version, starting at one.
    const VERSION: i32;
    /// Upgrades an old payload before deserialization. Rejects unsupported versions.
    fn upgrade(version: i32, value: Value) -> Result<Value, FrameworkError> {
        if version != Self::VERSION {
            return Err(error(format!(
                "event '{}' cannot upgrade version {version} to {}",
                Self::NAME,
                Self::VERSION
            )));
        }
        Ok(value)
    }
}

/// Link-time schema registration emitted by `#[domain_event]`.
pub struct DomainEventEntry {
    /// Persisted identity.
    pub name: &'static str,
    /// Validates and upgrades an envelope without exposing a Rust type name.
    pub decode: fn(i32, Value) -> Result<Value, FrameworkError>,
}
inventory::collect!(DomainEventEntry);

impl DomainEventEntry {
    /// Checks upgraded payloads against their declared Rust schema.
    pub fn decode<E: DomainEvent>(version: i32, payload: Value) -> Result<Value, FrameworkError> {
        if version < 1 || version > E::VERSION {
            return Err(error(format!(
                "event '{}' has unsupported version {version}",
                E::NAME
            )));
        }
        let payload = E::upgrade(version, payload)?;
        let event: E = serde_json::from_value(payload)
            .map_err(|e| error(format!("event '{}': {e}", E::NAME)))?;
        serde_json::to_value(event).map_err(|e| error(e.to_string()))
    }
}

/// A durable event envelope. Payloads remain in their stored schema until decoded.
#[derive(Clone, Debug, Serialize, serde::Deserialize, FromQueryResult)]
pub struct StoredEvent {
    /// Global event position.
    pub id: i64,
    /// Canonical aggregate UUID.
    pub aggregate_uuid: String,
    /// Aggregate version after this fact.
    pub aggregate_version: i64,
    /// Stored payload schema version.
    pub event_version: i32,
    /// Stable domain event identity.
    pub event_type: String,
    /// Original serialized fact.
    pub payload: Value,
    /// Application context, including the aggregate's stable name.
    pub metadata: Value,
    /// UTC recording time.
    pub created_at: chrono::NaiveDateTime,
}

impl StoredEvent {
    /// Reads a typed fact only when its stable identity matches.
    pub fn decode<E: DomainEvent>(&self) -> Result<Option<E>, FrameworkError> {
        if self.event_type != E::NAME {
            return Ok(None);
        }
        let value = DomainEventEntry::decode::<E>(self.event_version, self.payload.clone())?;
        serde_json::from_value(value)
            .map(Some)
            .map_err(|e| error(e.to_string()))
    }

    pub(crate) fn validate(&self) -> Result<(), FrameworkError> {
        let mut entries = inventory::iter::<DomainEventEntry>
            .into_iter()
            .filter(|e| e.name == self.event_type);
        let entry = entries
            .next()
            .ok_or_else(|| error(format!("unknown event type '{}'", self.event_type)))?;
        if entries.next().is_some() {
            return Err(error(format!("duplicate event type '{}'", self.event_type)));
        }
        (entry.decode)(self.event_version, self.payload.clone())?;
        Ok(())
    }
}

/// An aggregate state captured at an already persisted version.
#[derive(Clone, Debug, Serialize, serde::Deserialize, FromQueryResult)]
pub struct Snapshot {
    /// Canonical aggregate UUID.
    pub uuid: String,
    /// Last event included in state.
    pub aggregate_version: i64,
    /// Serialized aggregate state.
    pub state: Value,
    /// UTC capture time.
    pub created_at: chrono::NaiveDateTime,
}

pub(crate) async fn executor() -> Result<ExecutorChoice, FrameworkError> {
    ExecutorChoice::resolve_write(None, None, None).await
}

pub(crate) fn rows<T: FromQueryResult>(rows: Vec<QueryResult>) -> Result<Vec<T>, FrameworkError> {
    rows.iter()
        .map(|row| T::from_query_result(row, "").map_err(|e| error(e.to_string())))
        .collect()
}

/// Reads stored facts and snapshots from the primary database.
pub struct EventStore;

impl EventStore {
    /// Returns a canonical UUID or a boundary error.
    pub fn uuid(uuid: &str) -> Result<String, FrameworkError> {
        uuid::Uuid::parse_str(uuid)
            .map(|uuid| uuid.to_string())
            .map_err(|e| error(format!("invalid aggregate UUID '{uuid}': {e}")))
    }

    /// Lists the aggregate's facts, checking every type and schema.
    pub async fn events(uuid: &str) -> Result<Vec<StoredEvent>, FrameworkError> {
        let uuid = Self::uuid(uuid)?;
        let mut result = Vec::new();
        let mut version = 0;
        loop {
            let page = Self::aggregate_page(&uuid, version, 256).await?;
            if page.is_empty() {
                break;
            }
            for event in page {
                version = event.aggregate_version;
                result.push(event);
            }
        }
        Ok(result)
    }

    pub(crate) async fn aggregate_page(
        uuid: &str,
        after: i64,
        limit: u64,
    ) -> Result<Vec<StoredEvent>, FrameworkError> {
        let exec = executor().await?;
        let query = Query::select()
            .column(sea_orm::sea_query::Asterisk)
            .from(Alias::new("stored_events"))
            .and_where(Expr::col(Alias::new("aggregate_uuid")).eq(uuid))
            .and_where(Expr::col(Alias::new("aggregate_version")).gt(after))
            .order_by(Alias::new("aggregate_version"), Order::Asc)
            .limit(limit)
            .to_owned();
        let events: Vec<StoredEvent> = rows(exec.query_all(exec.backend().build(&query)).await?)?;
        for event in &events {
            event.validate()?;
        }
        Ok(events)
    }

    /// Reads the latest snapshot. Multiple captures at different versions are retained.
    pub async fn snapshot(uuid: &str) -> Result<Option<Snapshot>, FrameworkError> {
        let uuid = Self::uuid(uuid)?;
        let exec = executor().await?;
        let query = Query::select()
            .column(sea_orm::sea_query::Asterisk)
            .from(Alias::new("snapshots"))
            .and_where(Expr::col(Alias::new("uuid")).eq(uuid))
            .order_by(Alias::new("aggregate_version"), Order::Desc)
            .limit(1)
            .to_owned();
        Ok(rows::<Snapshot>(exec.query_all(exec.backend().build(&query)).await?)?.pop())
    }

    pub(crate) async fn lock() -> Result<(), FrameworkError> {
        // Serial append and replay order prevents a late commit from falling behind a checkpoint.
        let exec = executor().await?;
        let query = Query::update()
            .table(Alias::new("projector_checkpoints"))
            .value(
                Alias::new("last_event_id"),
                Expr::col(Alias::new("last_event_id")),
            )
            .and_where(Expr::col(Alias::new("name")).eq("__event_store__"))
            .to_owned();
        exec.run(exec.backend().build(&query)).await?;
        Ok(())
    }

    pub(crate) async fn version(uuid: &str) -> Result<i64, FrameworkError> {
        let exec = executor().await?;
        let query = Query::select()
            .expr(Expr::col(Alias::new("aggregate_version")).max())
            .from(Alias::new("stored_events"))
            .and_where(Expr::col(Alias::new("aggregate_uuid")).eq(uuid))
            .to_owned();
        let row = exec
            .query_one(exec.backend().build(&query))
            .await?
            .ok_or_else(|| error("missing aggregate version result"))?;
        Ok(row.try_get_by_index::<Option<i64>>(0)?.unwrap_or(0))
    }

    pub(crate) async fn insert(event: &StoredEvent) -> Result<StoredEvent, FrameworkError> {
        let exec = executor().await?;
        let query = Query::insert()
            .into_table(Alias::new("stored_events"))
            .columns(
                [
                    "aggregate_uuid",
                    "aggregate_version",
                    "event_version",
                    "event_type",
                    "payload",
                    "metadata",
                    "created_at",
                ]
                .map(Alias::new),
            )
            .values([
                event.aggregate_uuid.clone().into(),
                event.aggregate_version.into(),
                event.event_version.into(),
                event.event_type.clone().into(),
                event.payload.clone().into(),
                event.metadata.clone().into(),
                event.created_at.into(),
            ])
            .map_err(|e| error(e.to_string()))?
            .to_owned();
        exec.run(exec.backend().build(&query)).await?;
        let query = Query::select()
            .column(sea_orm::sea_query::Asterisk)
            .from(Alias::new("stored_events"))
            .and_where(Expr::col(Alias::new("aggregate_uuid")).eq(event.aggregate_uuid.clone()))
            .and_where(Expr::col(Alias::new("aggregate_version")).eq(event.aggregate_version))
            .to_owned();
        rows(exec.query_all(exec.backend().build(&query)).await?)?
            .pop()
            .ok_or_else(|| error("inserted event not found"))
    }
}
