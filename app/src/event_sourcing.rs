//! A durable tally separates aggregate history, read state and committed activity.
use serde::{Deserialize, Serialize};
use suprnova::live::LiveTopics;
use suprnova::{
    Aggregate, AggregateRoot, FirstOrCreate, FrameworkError, Model, Projector, Reactor,
    StoredEvent, attrs,
};

/// The demo keeps one shared tally so two signed-in pages see the same update.
pub const TALLY_UUID: &str = "a9c9bce3-8b35-4530-b215-14cbb0d46f01";
/// The Live stream reloads its read model after this topic changes.
pub const TALLY_TOPIC: &str = "event-tally";
/// The ordinary command route commits before the Live stream reloads.
pub const ADD_PATH: &str = "/live/event-tally/add";

/// A fact whose stored identity survives a Rust type rename.
#[suprnova::domain_event(name = "dogfood.tally.added", version = 1)]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TallyAdded {
    /// Positive increment accepted by the domain.
    pub amount: i64,
}

/// The aggregate state rebuilt from domain history.
#[suprnova::aggregate]
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TallyState {
    /// Sum of all accepted additions.
    pub total: i64,
}
impl AggregateRoot for TallyState {
    const NAME: &'static str = "dogfood.event-tally";
    fn apply(&mut self, fact: &StoredEvent) -> Result<(), FrameworkError> {
        if let Some(fact) = fact.decode::<TallyAdded>()? {
            if fact.amount <= 0 {
                return Err(FrameworkError::bad_request(
                    "tally additions must be positive",
                ));
            }
            self.total = self
                .total
                .checked_add(fact.amount)
                .ok_or_else(|| FrameworkError::bad_request("tally total is too large"))?;
        }
        Ok(())
    }
}

/// The read model used by the Live page, independent of aggregate reconstruction.
#[suprnova::model(table = "event_tally_totals", key_type = "String", timestamps = false, fillable = ["id", "total"])]
pub struct TallyTotal {
    /// Aggregate UUID.
    pub id: String,
    /// Projected sum.
    pub total: i64,
}

/// The reactor's idempotent activity record, excluded from projection replay.
#[suprnova::model(table = "event_tally_activity", timestamps = false, fillable = ["stored_event_id", "aggregate_uuid"])]
pub struct TallyActivity {
    /// Activity row identity.
    pub id: i64,
    /// Stored event identity, protected by a unique constraint.
    pub stored_event_id: i64,
    /// Aggregate whose committed addition produced the activity.
    pub aggregate_uuid: String,
}

/// Builds the tally total in the same transaction as its stored facts.
#[suprnova::projector]
#[derive(Default)]
pub struct TallyProjector;
#[suprnova::async_trait]
impl Projector for TallyProjector {
    fn name(&self) -> &'static str {
        "dogfood.tally-total"
    }
    fn handles(&self, fact: &StoredEvent) -> bool {
        fact.event_type == "dogfood.tally.added"
    }
    async fn project(&self, fact: &StoredEvent) -> Result<(), FrameworkError> {
        if let Some(event) = fact.decode::<TallyAdded>()? {
            let id = fact.aggregate_uuid.clone();
            let current = TallyTotal::find(id.clone())
                .await?
                .map(|row| row.total)
                .unwrap_or(0);
            let total = current
                .checked_add(event.amount)
                .ok_or_else(|| FrameworkError::bad_request("projected tally is too large"))?;
            TallyTotal::update_or_create(attrs! { id: id }, attrs! { total: total }).await?;
            LiveTopics::resolve()?
                .changed(TALLY_TOPIC, &["total"])
                .await?;
        }
        Ok(())
    }
    async fn reset(&self) -> Result<(), FrameworkError> {
        TallyTotal::query().delete_all().await?;
        LiveTopics::resolve()?
            .changed(TALLY_TOPIC, &["total"])
            .await
    }
}

/// Records committed activity once per fact. Replay never invokes this handler.
#[suprnova::reactor]
#[derive(Default)]
pub struct TallyReactor;
#[suprnova::async_trait]
impl Reactor for TallyReactor {
    fn name(&self) -> &'static str {
        "dogfood.tally-activity"
    }
    async fn react(&self, fact: &StoredEvent) -> Result<(), FrameworkError> {
        if fact.decode::<TallyAdded>()?.is_some() {
            TallyActivity::query().upsert(
                vec![attrs! { stored_event_id: fact.id, aggregate_uuid: fact.aggregate_uuid.clone() }],
                vec!["stored_event_id"], Some(vec!["aggregate_uuid"]),
            ).await?;
        }
        Ok(())
    }
}

/// Executes a domain command. The aggregate owns the persistence transaction.
pub async fn add(amount: i64) -> Result<i64, FrameworkError> {
    let mut tally = Aggregate::<TallyState>::retrieve(TALLY_UUID).await?;
    tally.record(TallyAdded { amount })?;
    tally.persist().await?;
    Ok(tally.state().total)
}
/// Reads the projection, so a Live refresh does not reconstruct the aggregate.
pub async fn total() -> Result<i64, FrameworkError> {
    Ok(TallyTotal::find(TALLY_UUID.to_owned())
        .await?
        .map(|row| row.total)
        .unwrap_or(0))
}
/// Receives the protected form command outside the Live action transaction.
pub async fn increment(_request: suprnova::Request) -> suprnova::Response {
    add(1).await?;
    suprnova::Redirect::to(super::live::EVENT_TALLY_PATH).into()
}

impl suprnova::live::EventPayloadMetadata for TallyAdded {
    const NAME: &'static str = "dogfood.tally.added";
    const VERSION: u16 = 1;
}
