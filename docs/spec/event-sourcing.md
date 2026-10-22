# Event sourcing

Status: Agreed 2026-10-21
Prefix: ES

Drafted 2026-10-21 from the developer's "YES" to taking event sourcing
itself into the next commitment (aggregates, stored events, projectors,
reactors, replay), modelled on spatie/laravel-event-sourcing
(`reference/laravel-event-sourcing-7.15.2`), the projection source the
inertia-live model depends on. The subsystem persists an aggregate's
history as events and rebuilds state and read models from it; it is one
source of live signals (SIGNAL), not a requirement for them.

## Observed at bfcbd61fe

Status: Observed

- Events are typed in-process pub/sub: `Event`, `Listener`, `Subscriber`
  and `EventDispatcher`, with fail-fast and best-effort delivery and
  deferred dispatch; nothing requires a serializable history
  (`framework/src/events/mod.rs:53,73,121`,
  `framework/src/events/dispatcher.rs:286,401,794`).
- `QueuedListener` turns an event into a durable job with retries and
  backoff (`framework/src/events/queued_listener.rs:1,79`); `Bus`
  dispatches typed commands (`framework/src/bus/mod.rs:85,162`).
- Transactions, savepoints and after-commit callbacks exist
  (`framework/src/database/after_commit.rs:12,123`); workflow storage
  persists runs and step outputs and replays recorded steps, which is
  workflow recovery, not aggregate reconstruction
  (`framework/src/workflow/entities.rs:14,68`,
  `framework/src/workflow/context.rs:115`).
- No `AggregateRoot`, stored event, `Projector`, `Reactor` or event store
  exists in the framework, the macros or the CLI.

## Requirements

[ES-001] The framework MUST provide aggregate roots: an aggregate records
domain events, applies them to its own state, and persists them in one
transaction with an expected-version check, so two concurrent persists of
one aggregate cannot both succeed; retrieval MUST rebuild the aggregate
from its stored events, from a snapshot plus the events after it when a
snapshot exists, and the framework MUST let an application make and read
snapshots.
Falsifier: two concurrent persists of one aggregate both succeed; a rebuilt aggregate differs from the one that recorded the events; or a rebuild from a snapshot differs from a full replay.
Mechanism: `event-sourcing`.
Rationale: spatie `src/AggregateRoots/AggregateRoot.php:107,135,241,263` and the snapshots table.
Status: Agreed 2026-10-21

[ES-002] A stored event MUST be a row with the aggregate uuid, the
aggregate version, the event version, a stable event type name the
application declares, the serialized payload, metadata and the recorded
time, under a unique constraint on aggregate uuid and aggregate version,
on every database the framework supports. Payloads MUST be versioned and
an application MUST be able to upgrade an older version on read; reading
a row whose type name no application type declares MUST fail with an
error naming the type, never skip it.
Falsifier: a second row with one aggregate uuid and version inserts; renaming a Rust type breaks reading rows written under its declared name; an unknown type name is skipped silently; or a supported database lacks the tables.
Mechanism: `event-sourcing`.
Rationale: spatie `database/migrations/create_stored_events_table.php.stub:11,15`.
Status: Agreed 2026-10-21

[ES-003] Projectors and reactors MUST be distinct: a projector builds read
models and MUST be replayable from the first stored event after its state
is reset; a reactor causes side effects and MUST NOT run during a replay.
Projectors MUST run in the persisting transaction by default, and the
framework MUST offer queued projectors with an ordered checkpoint per
projector, retries and idempotent handling. A replay MUST have an explicit
start and finish, ordered and bounded reads, MUST advance RenderCache
dependency generations for the rows it writes as CACHE-009 requires, and
MUST suppress live signals until it finishes as SIGNAL-003 requires.
Falsifier: a reactor runs during a replay; a replayed projector double-counts; a queued projector applies events out of order; a replay leaves a rendered page stale; or a replay signals once per event.
Mechanism: `event-sourcing`.
Rationale: spatie `src/Projectionist.php:293,377`.
Status: Agreed 2026-10-21

[ES-004] A projector's committed write MUST be able to signal a Live topic
through the binding SIGNAL-003 describes, and an application that does
not event-source MUST be able to signal from ordinary model events and
from an explicit call, so event sourcing is one source of signals and
never a requirement for them.
Falsifier: a projector write cannot signal a topic; or signalling a topic requires an aggregate.
Mechanism: `event-sourcing`.
Status: Agreed 2026-10-21

[ES-005] The framework MUST ship console commands to replay projectors
(all, or named, from a position), to list an aggregate's stored events and
to make a snapshot; the manual MUST gain an event sourcing chapter linked
from `manual/documentation.md`; and the dogfood application MUST use one
aggregate with a projector and a reactor end to end, with a bound Inertia
or Live page reloading from the projector's write.
Falsifier: a command is missing; the chapter is missing or unlinked; or the dogfood application has no aggregate, projector, reactor and bound page.
Mechanism: `event-sourcing`.
Status: Agreed 2026-10-21
