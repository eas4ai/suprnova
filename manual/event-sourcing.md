# Event sourcing

Event sourcing keeps domain history as typed facts. Ordinary [events](events.md)
remain in-process messages. [Workflows](workflows.md) keep step outputs for
recovery. Neither requires an aggregate.

## Register the tables

Append the framework migration to your application migrator:

```rust
fn migrations() -> Vec<Box<dyn suprnova::sea_orm_migration::MigrationTrait>> {
    suprnova::event_sourcing::migrations()
}
```

`stored_events` has a global id, aggregate UUID, aggregate version, event version,
stable event type, JSON payload, JSON metadata and UTC recording time. A unique
constraint rejects a second fact at the same aggregate version. `snapshots` keeps
UUID, aggregate version, JSON state and capture time. SQLite, PostgreSQL and
MySQL/MariaDB use the same migration.

## Record and rebuild an aggregate

The attribute registers a stable event name. Renaming the Rust struct preserves
stored history. Declare a new schema version when the payload changes.

```rust
use suprnova::{aggregate, domain_event, Aggregate, AggregateRoot, FrameworkError, StoredEvent};
use suprnova::serde::{Deserialize, Serialize};

#[domain_event(name = "tally.added", version = 1)]
#[derive(Serialize, Deserialize)]
struct Added { amount: i64 }

#[aggregate]
#[derive(Clone, Default, Serialize, Deserialize)]
struct Tally { total: i64 }

impl AggregateRoot for Tally {
    const NAME: &'static str = "tally";
    fn apply(&mut self, fact: &StoredEvent) -> Result<(), FrameworkError> {
        if let Some(added) = fact.decode::<Added>()? {
            self.total += added.amount;
        }
        Ok(())
    }
}

async fn add() -> Result<(), FrameworkError> {
    let uuid = "7d200fce-d107-4a12-8b9b-7e642209aa01";
    let mut tally = Aggregate::<Tally>::retrieve(uuid).await?;
    tally.record(Added { amount: 1 })?;
    tally.persist().await?;
    let rebuilt = Aggregate::<Tally>::retrieve(uuid).await?;
    assert_eq!(rebuilt.state().total, tally.state().total);
    Ok(())
}
```

`record` applies to a copy before accepting the fact. `persist` writes every
recorded fact in one transaction. A stale writer gets an error with the aggregate
UUID and the expected and observed versions. Its recorded facts remain available.
Reload state and decide whether to issue the command again.

Persistence owns its transaction. Calling it from an outer `DB::transaction`
returns an error. This prevents an outer rollback from leaving the in-memory
version ahead of durable history. An after-commit error can occur after the write
succeeds. The instance then records the committed version and clears its pending
facts, so retrying the instance cannot duplicate them.

## Snapshots

Capture state after persistence, then load it with the later facts:

```rust
async fn capture(uuid: &str) -> Result<(), FrameworkError> {
    let tally = Aggregate::<Tally>::retrieve(uuid).await?;
    tally.snapshot().await?;
    let with_snapshot = Aggregate::<Tally>::retrieve(uuid).await?;
    let full = Aggregate::<Tally>::retrieve_without_snapshot(uuid).await?;
    assert_eq!(with_snapshot.state().total, full.state().total);
    Ok(())
}
```

A snapshot at a version replaces an earlier capture of that version. Older
versions remain available. A capture with pending facts or no history fails.
Snapshots are application state, so changes to that state's serialization need
an application migration or removal of the affected snapshots before deployment.

## Upgrade payloads

An upgrade function receives the stored version and returns the current payload.
Unknown event names and unsupported versions fail with the event name.

```rust
use suprnova::{domain_event, FrameworkError};
use suprnova::serde::{Deserialize, Serialize};
use suprnova::serde_json::Value;

fn upgrade(version: i32, mut payload: Value) -> Result<Value, FrameworkError> {
    if version == 1 {
        payload["amount"] = payload["old_amount"].take();
    }
    Ok(payload)
}

#[domain_event(name = "counter.added", version = 2, upgrade = "upgrade")]
#[derive(Serialize, Deserialize)]
struct Addition { amount: i64 }
```

### Why Suprnova diverges

Spatie's package finds event handlers and public aggregate state through PHP
reflection. Suprnova uses serde state, explicit `AggregateRoot::apply` and stable
names registered at link time. Every aggregate requires a UUID. Projectors share
the persistence transaction by default. This gives failed projections the same
rollback boundary as their facts. Snapshots contain the explicitly serialized
state rather than a reflection-selected set of public properties.

## Next

Read [Events](events.md) for in-process listeners and [Workflows](workflows.md)
for durable step recovery.
