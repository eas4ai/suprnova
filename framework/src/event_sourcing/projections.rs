//! Projector checkpoints share the transaction with the read model they describe.
use super::{EventStore, StoredEvent, error, store::executor};
use crate::{DB, FrameworkError, Job, Queue};
use async_trait::async_trait;
use sea_orm::sea_query::{Alias, Expr, ExprTrait, OnConflict, Query};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Builds read models using ORM writes in the owning event transaction.
#[async_trait]
pub trait Projector: Send + Sync + 'static {
    /// Stable checkpoint identity, independent of a Rust type name.
    fn name(&self) -> &'static str;
    /// Selects relevant facts before queueing or applying a projection.
    fn handles(&self, _event: &StoredEvent) -> bool {
        true
    }
    /// Applies a fact. Unhandled fact types may return success without a write.
    async fn project(&self, event: &StoredEvent) -> Result<(), FrameworkError>;
    /// Clears this projector's read model before a full replay.
    async fn reset(&self) -> Result<(), FrameworkError>;
    /// Runs before reset and replay, inside the replay transaction.
    async fn replay_started(&self) -> Result<(), FrameworkError> {
        Ok(())
    }
    /// Runs after the last fact and before the replay commits.
    async fn replay_finished(&self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

/// Performs external effects after persistence commits. Replay never invokes reactors.
#[async_trait]
pub trait Reactor: Send + Sync + 'static {
    /// Stable handler identity used in diagnostics.
    fn name(&self) -> &'static str;
    /// Receives a committed fact. External effects should use its id for idempotency.
    async fn react(&self, event: &StoredEvent) -> Result<(), FrameworkError>;
}

/// Link-time registration emitted by `#[projector]`.
pub struct ProjectorEntry {
    /// Builds a fresh application handler.
    pub build: fn() -> Box<dyn Projector>,
    /// Selects durable queue delivery instead of inline application.
    pub queued: bool,
}
inventory::collect!(ProjectorEntry);
/// Link-time registration emitted by `#[reactor]`.
pub struct ReactorEntry {
    /// Builds a fresh application handler.
    pub build: fn() -> Box<dyn Reactor>,
}
inventory::collect!(ReactorEntry);

pub(crate) struct Registration {
    pub projector: Box<dyn Projector>,
    pub queued: bool,
}
pub(crate) fn projectors() -> Result<Vec<Registration>, FrameworkError> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for entry in inventory::iter::<ProjectorEntry> {
        let projector = (entry.build)();
        validate_name(projector.name())?;
        if !seen.insert(projector.name()) {
            return Err(error(format!("duplicate projector '{}'", projector.name())));
        }
        result.push(Registration {
            projector,
            queued: entry.queued,
        });
    }
    result.sort_by_key(|entry| entry.projector.name());
    Ok(result)
}
fn reactors() -> Result<Vec<Box<dyn Reactor>>, FrameworkError> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for entry in inventory::iter::<ReactorEntry> {
        let reactor = (entry.build)();
        validate_name(reactor.name())?;
        if !seen.insert(reactor.name()) {
            return Err(error(format!("duplicate reactor '{}'", reactor.name())));
        }
        result.push(reactor);
    }
    result.sort_by_key(|reactor| reactor.name());
    Ok(result)
}
fn validate_name(name: &str) -> Result<(), FrameworkError> {
    if name.is_empty() || name.len() > 191 || !name.is_ascii() || name.starts_with("__") {
        return Err(error(format!("invalid event handler name '{name}'")));
    }
    Ok(())
}

pub(crate) async fn checkpoint(name: &str) -> Result<i64, FrameworkError> {
    let exec = executor().await?;
    let insert = Query::insert()
        .into_table(Alias::new("projector_checkpoints"))
        .columns([Alias::new("name"), Alias::new("last_event_id")])
        .values([name.into(), 0i64.into()])
        .map_err(|e| error(e.to_string()))?
        .on_conflict(
            OnConflict::column(Alias::new("name"))
                .do_nothing()
                .to_owned(),
        )
        .to_owned();
    let inserted = exec.run(exec.backend().build(&insert)).await?;
    if inserted.rows_affected() > 0 {
        crate::render_cache::orm::after_table_write("projector_checkpoints").await?;
    }
    let query = Query::select()
        .column(Alias::new("last_event_id"))
        .from(Alias::new("projector_checkpoints"))
        .and_where(Expr::col(Alias::new("name")).eq(name))
        .to_owned();
    let row = exec
        .query_one(exec.backend().build(&query))
        .await?
        .ok_or_else(|| error(format!("missing checkpoint '{name}'")))?;
    row.try_get_by_index(0).map_err(|e| error(e.to_string()))
}
pub(crate) async fn set_checkpoint(name: &str, id: i64) -> Result<(), FrameworkError> {
    let exec = executor().await?;
    let query = Query::update()
        .table(Alias::new("projector_checkpoints"))
        .value(Alias::new("last_event_id"), id)
        .and_where(Expr::col(Alias::new("name")).eq(name))
        .to_owned();
    exec.run(exec.backend().build(&query)).await?;
    crate::render_cache::orm::after_table_write("projector_checkpoints").await?;
    Ok(())
}
pub(crate) async fn apply(
    projector: &dyn Projector,
    event: &StoredEvent,
) -> Result<(), FrameworkError> {
    if event.id <= checkpoint(projector.name()).await? {
        return Ok(());
    }
    if projector.handles(event) {
        projector.project(event).await.map_err(|e| {
            error(format!(
                "projector '{}' event {}: {e}",
                projector.name(),
                event.id
            ))
        })?;
    }
    set_checkpoint(projector.name(), event.id).await
}

pub(crate) async fn persisted(events: &[StoredEvent]) -> Result<(), FrameworkError> {
    let projectors = projectors()?;
    let reactors = reactors()?;
    for registration in &projectors {
        if registration.queued {
            if events
                .iter()
                .any(|event| registration.projector.handles(event))
                && let Some(last) = events.last()
            {
                Queue::push_after_commit(QueuedProjector::new(
                    registration.projector.name(),
                    last.id,
                )?)
                .await?;
            }
        } else {
            for event in events {
                apply(registration.projector.as_ref(), event).await?;
            }
        }
    }
    for reactor in reactors {
        let events = events.to_vec();
        DB::after_commit(move || async move {
            for event in events {
                reactor.react(&event).await?;
            }
            Ok(())
        })
        .await?;
    }
    Ok(())
}

/// A retryable queue wake-up that catches a projector up in stored-event order.
/// Duplicate or shuffled wake-ups do not repeat committed projection writes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueuedProjector {
    name: String,
    through: i64,
}
impl QueuedProjector {
    /// Targets a registered queued projector through an inclusive global event id.
    pub fn new(name: &str, through: i64) -> Result<Self, FrameworkError> {
        validate_name(name)?;
        if through < 0 {
            return Err(error("queued projector position cannot be negative"));
        }
        Ok(Self {
            name: name.into(),
            through,
        })
    }
    /// Returns the queue target's stable name.
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Returns the inclusive upper position this wake-up covers.
    pub fn through(&self) -> i64 {
        self.through
    }
}
#[async_trait]
impl Job for QueuedProjector {
    fn job_name() -> &'static str {
        "suprnova.event_sourcing.projector"
    }
    fn max_tries() -> u32 {
        5
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        let entry = projectors()?
            .into_iter()
            .find(|entry| entry.projector.name() == self.name && entry.queued)
            .ok_or_else(|| error(format!("unknown queued projector '{}'", self.name)))?;
        DB::transaction(move |_tx| {
            Box::pin(async move {
                EventStore::lock().await?;
                let mut after = checkpoint(&self.name).await?;
                loop {
                    let page = EventStore::page(after, self.through, 256).await?;
                    if page.is_empty() {
                        break;
                    }
                    for event in page {
                        apply(entry.projector.as_ref(), &event).await?;
                        after = event.id;
                    }
                }
                Ok(())
            })
        })
        .await
    }
}
