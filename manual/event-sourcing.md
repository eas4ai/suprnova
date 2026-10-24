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

Rolling the framework migration back drops `stored_events`, `snapshots` and
`projector_checkpoints`, so back history up before a rollback.

## Record and rebuild an aggregate

The attribute registers a stable event name. Renaming the Rust struct preserves
stored history. Declare a new schema version when the payload changes.
Application and console startup validate every registered event name once. Two
Rust types declaring one name stop startup with both type names in the error.

```rust
use suprnova::{aggregate, domain_event, Aggregate, AggregateRoot, Fact, FrameworkError};
use suprnova::serde::{Deserialize, Serialize};

#[domain_event(name = "tally.added", version = 1)]
#[derive(Serialize, Deserialize)]
struct Added { amount: i64 }

#[aggregate]
#[derive(Clone, Default, Serialize, Deserialize)]
struct Tally { total: i64 }

impl AggregateRoot for Tally {
    const NAME: &'static str = "tally";
    fn apply(&mut self, fact: &Fact<'_>) -> Result<(), FrameworkError> {
        if let Some(added) = fact.decode::<Added>()? {
            self.total = self.total.checked_add(added.amount)
                .ok_or_else(|| FrameworkError::bad_request("tally total is too large"))?;
        }
        Ok(())
    }
}

async fn add() -> Result<(), FrameworkError> {
    let uuid = "7d200fce-d107-4a12-8b9b-7e642209aa01";
    let mut tally = Aggregate::<Tally>::retrieve(uuid).await?;
    tally.record(Added { amount: 1 }).await?;
    tally.persist().await?;
    let rebuilt = Aggregate::<Tally>::retrieve(uuid).await?;
    assert_eq!(rebuilt.state().total, tally.state().total);
    Ok(())
}
```

`AggregateRoot::apply` receives a `Fact` view with the stable event name, payload
schema version, aggregate version, payload and metadata. It excludes the storage
id and recording time, so recording and rebuilding receive identical values.
Projectors, reactors and event listings still receive `StoredEvent`, which exposes
this view through `fact()`.

`record` applies to a copy before accepting the fact. `persist` writes every
recorded fact in one transaction. A stale writer gets an error with the aggregate
UUID and the expected and observed versions, with HTTP status 409. Its recorded facts remain available.
Reload state and decide whether to issue the command again.

Persistence owns its transaction. Calling it from an outer `DB::transaction`
returns an internal error naming the aggregate. Storage failures remain database
errors. This prevents an outer rollback from leaving the in-memory
version ahead of durable history. An after-commit error can occur after the write
succeeds. The instance then records the committed version and clears its pending
facts, so retrying the instance cannot duplicate them. The instance reflects the
committed outcome as soon as COMMIT returns, before transaction listeners or
delivery callbacks. A cancelled persist settles on the next `record` or `persist`
call. These calls are async because they first await the owned transaction's
outcome. `is_settled()` remains false until that reconciliation completes.
Rollback retains pending facts and the prior durable version.

The wait uses the connection's `DatabaseConfig::outcome_settle_timeout` in seconds,
set with `.outcome_settle_timeout(seconds)` or `DB_OUTCOME_SETTLE_TIMEOUT` (default 30;
zero is rejected). This setting bounds the wait in seconds for the physical outcome
of a transaction whose caller was dropped during finalization; if the wait expires,
the outcome is reported as unknown. It never cancels or bounds a transaction, query
or commit. A timeout returns an internal error naming the aggregate and prevents
reuse. Retry settlement after the owned finalizer completes. Snapshots require a settled instance.
`recorded()` returns a copy of the pending facts at the time of the call.

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

## Projectors and reactors

A projector builds a read model with ordinary ORM writes. It shares the event
transaction by default. Its reset clears only the state it owns.

```rust
use suprnova::{Projector, Reactor, Model, FrameworkError, StoredEvent, attrs};

#[suprnova::model(
    table = "tally_totals", key_type = "String", timestamps = false,
    fillable = ["id", "total"]
)]
struct Total { pub id: String, pub total: i64 }

#[suprnova::projector]
#[derive(Default)]
struct Totals;

#[suprnova::async_trait]
impl Projector for Totals {
    fn name(&self) -> &'static str { "tally.totals" }
    fn handles(&self, fact: &StoredEvent) -> bool {
        fact.event_type == "tally.added"
    }
    async fn project(&self, fact: &StoredEvent) -> Result<(), FrameworkError> {
        if let Some(added) = fact.decode::<Added>()? {
            match Total::find(fact.aggregate_uuid.clone()).await? {
                Some(mut row) => {
                    row.total = row.total.checked_add(added.amount)
                        .ok_or_else(|| FrameworkError::bad_request("projected total is too large"))?;
                    row.save().await?;
                }
                None => {
                    Total::create(attrs! { id: fact.aggregate_uuid.clone(), total: added.amount }).await?;
                }
            }
        }
        Ok(())
    }
    async fn reset(&self) -> Result<(), FrameworkError> {
        Total::query().delete_all().await?;
        Ok(())
    }
}

#[suprnova::reactor]
#[derive(Default)]
struct Announce;

#[suprnova::async_trait]
impl Reactor for Announce {
    fn name(&self) -> &'static str { "tally.announce" }
    async fn react(&self, fact: &StoredEvent) -> Result<(), FrameworkError> {
        if fact.decode::<Added>()?.is_some() {
            println!("Committed tally event {}", fact.id);
        }
        Ok(())
    }
}
```

Create `tally_totals` in an application migration before persisting:

```sql
CREATE TABLE tally_totals (id VARCHAR(36) PRIMARY KEY, total BIGINT NOT NULL);
```

The attributes register `Default` handlers through inventory. Names are stable
checkpoint identities. Names must be ASCII, at most 191 bytes and cannot start
with `__`. Leading or trailing whitespace and control characters are rejected,
so checkpoint names stay distinct on every engine. Duplicate handler names fail. Handlers run in name order.

A reactor runs after commit. It never runs during replay. Use the stored event id
as an idempotency key for external effects. A reactor failure reports an error
without undoing the committed history. Reactors are direct after-commit effects;
queue a separate job when delivery needs durable retries.

## Queued projectors

Queued projectors must not read synchronous projectors' read models. Build each
projection from its own state and stored facts. Live persistence completes
synchronous writes before queue delivery. Replay follows that order within each
page: synchronous handlers receive every fact first, then queued handlers receive
every fact. Replay pages need not match the original persistence batches.

Replace `#[suprnova::projector]` on `Totals` with
`#[suprnova::projector(queued)]`. Run a [queue worker](queues.md). Each persistence
queues a `QueuedProjector` wake-up after commit. The worker type is registered by
the framework and retries failures up to five attempts with the queue backoff.

The job reads the stored history after its checkpoint in pages of 256 facts.
Writes and checkpoint advancement share a transaction. A duplicate wake-up skips
facts at or below that checkpoint. Shuffled wake-ups still apply facts in global
id order. `handles` avoids queueing irrelevant facts.

A queue push can fail after the history commits. The durable facts remain the
recovery source. Requeue a catch-up wake-up after restoring the queue:

```rust
use suprnova::{EventStore, FrameworkError, Queue, QueuedProjector};
async fn catch_up() -> Result<(), FrameworkError> {
    let job = QueuedProjector::new("tally.totals", EventStore::last_position().await?)?;
    Queue::push(job).await?;
    Ok(())
}
```

The queue wake-up and the database commit are separate operations. A process
failure between them requires this catch-up or a projector replay. Monitor queue
push errors and run catch-up after recovering a failed writer.

## Replay

A full replay calls `replay_started`, resets each selected read model and its
checkpoint, reads ordered pages, then calls `replay_finished`. A failed replay
rolls back the reset, writes and checkpoints. A successful replay emits
`ReplayFinished` after commit. Reactors are excluded.

```rust
use suprnova::{EventReplay, FrameworkError};
async fn rebuild() -> Result<(), FrameworkError> {
    EventReplay::projectors(["tally.totals"]).page_size(128).run().await?;
    EventReplay::all().run().await?;
    Ok(())
}
```

`EventReplay` is also `suprnova::event_sourcing::Replay`. `from(position)` resumes
strictly after that global event id and does not reset state. Use a checkpoint
from a prior run when resuming. A repeated resume skips facts already committed.

Replay holds the append lock for its transaction. Live persistence and queued
projection wait until it completes. Use replay as maintenance work on large
histories. Reads remain bounded by the page size, from one to ten thousand facts.
ORM writes advance [RenderCache generations](render-cache-generations.md) inside
the same transaction. Suppressing notifications does not suppress invalidation.

## Live signals from projection writes and ordinary models

Declare a [Live](live.md) stream on the page's component. A component named
`app.tally` can use a stream named `changes` and the topic `tally`.
Its rendering hook reloads the read model after a refresh. The dogfood page at
`/live/event-tally` shows this flow.

Declare the private topic in the application registry and authorize its audience
before resolving the publisher. A signed-in principal can observe this shared tally:

```rust
use suprnova::{FrameworkError, Gate, live::LiveRegistry};

fn registry() -> Result<LiveRegistry, FrameworkError> {
    Gate::define::<String, String>("live:topic.tally", |principal, _| {
        !principal.is_empty()
    });
    Ok(LiveRegistry::builder().topic("tally")?.build())
}
```

Include the page's component registration in the same builder. Bind the registry
at bootstrap as described in [Live topic signals](live.md#topic-signals).

Register a normal saved listener to signal a model write:

```rust
use std::sync::Arc;
use suprnova::{EventFacade, FrameworkError, Listener, LiveTopics};

struct TotalChanged;
#[suprnova::async_trait]
impl Listener<total::events::Saved> for TotalChanged {
    async fn handle(&self, _: &total::events::Saved) -> Result<(), FrameworkError> {
        LiveTopics::resolve()?.changed("tally", &["total"]).await
    }
}
async fn bind_total() {
    EventFacade::listen::<total::events::Saved, _>(Arc::new(TotalChanged)).await;
}
```

The same listener works when a projector saves `Total` or when a controller saves
it directly. It does not need an aggregate, a stored event or the event store
migration. An explicit call also signals without a model event. For example,
notify after an ordinary bulk write:

```rust
async fn clear_total() -> Result<(), FrameworkError> {
    suprnova::DB::transaction(|_| Box::pin(async {
        Total::query().update_all(suprnova::attrs! { total: 0i64 }).await?;
        LiveTopics::resolve()?.changed("tally", &["total"]).await?;
        Ok(())
    })).await
}
```

`changed` waits for the owning transaction's commit and discards rolled-back
changes. Requests and job attempts coalesce changes per topic with the union of
prop names. `batch(future)` coalesces another explicit unit of work.
`suppress(|| future)` keeps one final union per touched topic through replay.
Replay resolves the application's publisher and uses this shared suppression
scope, including in a worker. Register its topics and abilities during bootstrap.
It preserves ORM cache invalidation and carries no model values to the browser.
Live streams reload their server view after a topic change.

Once replay commits, an owned task keeps its final topic unions in publication
order. Cancelling the caller stops waiting and does not discard those changes.
A caller that waits receives the first publication error after every remaining
topic is attempted.

A manual transaction is not ambient state. Await identity capture when registering
a change on its handle:

```rust
async fn signal_manual_write() -> Result<(), FrameworkError> {
    let topics = LiveTopics::resolve()?;
    let tx = suprnova::DB::begin_transaction().await?;
    topics.changed_with_tx(&tx, "tally", &["total"]).await?;
    tx.commit().await
}
```

## Console commands

Run these through the [console binary](console.md) from the application
directory after migrating its tables:

```sh
cargo run --bin console -- event-sourcing:replay
cargo run --bin console -- event-sourcing:replay --projector tally.totals
cargo run --bin console -- event-sourcing:replay --projector tally.totals --from 120
cargo run --bin console -- event-sourcing:events 7d200fce-d107-4a12-8b9b-7e642209aa01
cargo run --bin console -- event-sourcing:snapshot 7d200fce-d107-4a12-8b9b-7e642209aa01
```

`replay` defaults to all registered projectors. Repeat `--projector` to select
several. `--from` is an exclusive global event position. Omitting it performs a
full reset and replay. `events` prints JSON lines with the original payload,
metadata and schema version. `snapshot` finds the aggregate by its stable name
in history, reconstructs it and writes a snapshot. Unknown names and invalid
positions fail before replay changes a read model.

## The dogfood tally

The application registers the framework migration and two application tables:
`event_tally_totals` for the read model and `event_tally_activity` for the reactor.
Its aggregate is `app::event_sourcing::TallyState`, and its stable fact is
`dogfood.tally.added`. The demo UUID is
`a9c9bce3-8b35-4530-b215-14cbb0d46f01`.

Sign in and open `/live/event-tally` in two windows. **Add one** sends a protected
POST to `/live/event-tally/add`. The aggregate records a fact. The projector saves
the new total and calls `LiveTopics::changed("event-tally", &["total"])`. The
reactor records activity once per stored event. The private `event-tally` topic
requires a signed-in principal. Both pages subscribe to the Live stream, and a
topic change runs the component's rendering hook to read the new total.
The form uses an ordinary POST because aggregate persistence owns its transaction.

The example uses a Live page. It needs no Inertia live binding. Rebuild its read
model and inspect the same history with:

```sh
cargo run --bin console -- event-sourcing:replay --projector dogfood.tally-total
cargo run --bin console -- event-sourcing:events a9c9bce3-8b35-4530-b215-14cbb0d46f01
cargo run --bin console -- event-sourcing:snapshot a9c9bce3-8b35-4530-b215-14cbb0d46f01
```

Replay rebuilds the total and sends one final topic change. It leaves reactor
activity in place and never records that activity again.

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
