# Queue

The `Queue` facade dispatches background work to a driver and lets a separate
worker process drain it: HTTP handlers return fast, the heavy lifting runs
behind the scenes. Reach for it whenever a request would otherwise block on
something that can be done later - sending mail, hitting a webhook, generating
a report. Pair with [`Bus`](bus.md) when you want the work to run *now* in the
current task and return a typed result; pair with [`Events`](events.md) when
you want one signal to fan out to many listeners.

## Quick start

Define a job, register it once at boot, push it:

```rust
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use suprnova::{error::FrameworkError, queue::{Job, Queue}};

#[derive(Serialize, Deserialize)]
struct SendWelcomeEmail { user_id: i64 }

#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str { "SendWelcomeEmail" }

    async fn handle(self) -> Result<(), FrameworkError> {
        // … actually send the mail
        Ok(())
    }
}

// Boot once (the worker process and the dispatch process both need this).
Queue::set_driver(std::sync::Arc::new(suprnova::queue::MemoryQueueDriver::new()));
suprnova::queue::worker::register_job::<SendWelcomeEmail>();

// Push from a handler:
Queue::push(SendWelcomeEmail { user_id: 42 }).await?;
```

A worker process drains the configured driver until cancelled:

```rust
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use suprnova::queue::{Queue, worker::{WorkerConfig, run_worker}};

let driver = Queue::driver()?;
let cfg = WorkerConfig {
    visibility_timeout: Duration::from_secs(60),
    poll_interval: Duration::from_millis(100),
    max_jobs: None,
    queues: Vec::new(),
};
let shutdown = CancellationToken::new();
run_worker(driver, cfg, shutdown).await;
```

In a scaffolded app, the worker is started by the binary's `queue:work`
subcommand - `cargo run -- queue:work` - which runs the same bootstrap your
HTTP server does, so observers and listeners registered in `bootstrap()`
fire identically for inserts from a queue handler.

## Drivers

Five drivers ship in-tree. Configure via `QUEUE_DRIVER` env or by calling
`Queue::set_driver(...)` programmatically.

| Driver | Use for | Strengths |
| --- | --- | --- |
| `MemoryQueueDriver` | tests, single-process apps | `tokio::time::DelayQueue` for `available_at`, virtual-clock compatible |
| `RedisQueueDriver` | production fan-out | consumer groups + `XAUTOCLAIM` + ZSET-backed delayed jobs |
| `DatabaseQueueDriver` | single-DB apps | `FOR UPDATE SKIP LOCKED` on Postgres/MySQL, `BEGIN`-serialised on SQLite |
| `SyncQueueDriver` | dev, CI | runs the handler inline on `push`, no worker |
| `NullQueueDriver` | testing wrappers | drops every push without running |

`Queue::bootstrap_from_env()` reads `QUEUE_DRIVER` and wires the matching
driver; `Queue::bootstrap_default()` always wires the memory driver. The
server boot path calls one of these for you - most apps only configure via
env.

`FailoverQueueDriver` isn't a sixth backend. It wraps an ordered list of
the drivers above so a push one connection refuses falls through to the
next. See [Failover connections](#failover-connections).

### Environment configuration

```bash
QUEUE_DRIVER=redis
QUEUE_REDIS_URL=redis://127.0.0.1:6379
QUEUE_REDIS_STREAM=suprnova-queue
QUEUE_REDIS_GROUP=default
QUEUE_REDIS_CONSUMER=consumer-1
QUEUE_VISIBILITY_TIMEOUT_SECS=60

# Database driver - DB::init() must run first
QUEUE_DRIVER=database
QUEUE_DB_TABLE=jobs
```

The database driver validates `QUEUE_DB_TABLE` as a SQL identifier at
construction, so a malformed env value fails boot rather than reaching SQL
composition. Redis uses sea-streamer-redis under the hood with
`AutoCommit::Disabled`; the visibility timeout is fixed at consumer-group
construction time, so the per-pop `visibility_timeout` argument is ignored
on Redis (a documented divergence from the trait contract imposed by
Redis Streams).

### Why Suprnova diverges

Laravel routes every queueable through the Bus, distinguishing
`ShouldQueue` jobs at dispatch time. Suprnova splits the two: `Bus` for
synchronous work that returns a typed result, `Queue` for asynchronous
work that survives a process crash. PHP needs the implicit routing
because its request-per-process model makes "do this later, in another
process" hard to model otherwise. Tokio doesn't - explicit `Bus::dispatch`
vs `Queue::push` is clearer, faster, and surfaces the durability choice
at the call site. See [`bus.md`](bus.md) for the side-by-side.

## Failover connections

`FailoverQueueDriver` wraps an ordered list of connections. A push that
the first connection refuses is retried on the next, and so on down the
list, so a Redis outage doesn't turn every dispatch into a lost job.

Configure it from env:

```bash
QUEUE_DRIVER=failover
QUEUE_FAILOVER_CONNECTIONS=redis,database

# Each connection reads its own variables, exactly as it would if it
# were QUEUE_DRIVER on its own.
QUEUE_REDIS_URL=redis://127.0.0.1:6379
QUEUE_DB_TABLE=jobs
```

Or wire it yourself, when the connections need runtime configuration
that env can't express:

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::queue::{
    DatabaseQueueDriver, FailoverQueueDriver, Queue, QueueDriver, RedisQueueDriver,
};
use suprnova::{DB, FrameworkError};

pub async fn register() -> Result<(), FrameworkError> {
    let redis = RedisQueueDriver::connect(
        "redis://127.0.0.1:6379",
        "suprnova-queue",
        "default",
        "consumer-1",
        Duration::from_secs(60),
    )
    .await?;
    let database =
        DatabaseQueueDriver::new(DB::connection()?.inner().clone(), "jobs".to_string())?;

    let failover = FailoverQueueDriver::new(vec![
        ("redis".to_string(), Arc::new(redis) as Arc<dyn QueueDriver>),
        ("database".to_string(), Arc::new(database) as Arc<dyn QueueDriver>),
    ])?;
    Queue::set_driver(Arc::new(failover));
    Ok(())
}
```

The `String` on each entry is the connection label reported on the
`QueueFailedOver` event. It isn't derived from the driver type, because
two connections can run the same driver.

`QUEUE_FAILOVER_CONNECTIONS` is required when `QUEUE_DRIVER=failover`,
and the list can't contain `failover` itself. An entry naming a driver
that doesn't exist is a boot error rather than the warn-and-use-memory
fallback `QUEUE_DRIVER` applies to itself: inside a failover chain, a
typo that quietly became an in-memory connection would put an ephemeral
backend in a durable list.

### Writes fail over, reads don't

Only `push` and `bulk_push` walk the connection list. Every other
operation - `pop`, `ack`, `nack`, `release`, `settle`, `clear`, the four
counters and the three inspection listings - goes to the **first**
connection and no other.

That asymmetry is the contract, not an omission. A reservation token is
meaningful only to the driver that issued it, so acking against a
different connection would settle nothing and corrupt both. The counters
and listings follow the same rule so that what you inspect is what the
worker on this connection drains, rather than a sum across backends that
matches no worker's view.

**A worker on the failover connection drains the primary only.** Jobs
that failed over to a fallback need a worker running against that
fallback connection directly:

```bash
# Drains the primary of the failover chain.
QUEUE_DRIVER=failover QUEUE_FAILOVER_CONNECTIONS=redis,database ./app queue:work

# Drains what failed over to the database. Run this too.
QUEUE_DRIVER=database ./app queue:work
```

Laravel's documentation carries the same warning for the same reason.

This reaches chains, but only through one door. A worker settles a job and
enqueues the next link of a [queued chain](#queued-chains) in one call,
`settle`, and the decorator delegates that call to the primary alone. So
with a transactional primary such as the database driver, a primary that is
down fails the settle and nothing falls over: the worker leaves the
reservation intact and visibility expiry redelivers the job. The
fall-through happens when the primary answers `Settled::Unsupported`, which
the memory and Redis drivers do, because the worker then pushes the next
link through the bound driver like any other push - and that push falls
over. The rest of that chain then waits for a worker on the fallback
connection. Without one, the chain stalls - the link is durable and nothing
is lost, but nothing runs it either.

### The `QueueFailedOver` event

Each connection that refuses a push dispatches
`queue::events::QueueFailedOver { connection, job_name, exception }`, but
only on the push that moves that connection *into* failure. A connection
already known to be failing stays quiet until a later push succeeds on
it, which re-arms it. A four-hour outage produces one event, not one per
dispatch, which is what makes it usable as an alert.

`connection` is the label of the connection that failed, not the one that
accepted the job.

When every connection refuses a push, the push returns the last
connection's error. `bulk_push` pushes each envelope separately, so each
one falls through on its own: a batch the primary half-accepted is never
re-pushed wholesale onto the fallback, and each envelope keeps the
`available_at` it was built with. A batch is not atomic. If one envelope
is refused by every connection, `bulk_push` returns that envelope's error
with the earlier envelopes already enqueued.

Falling over is not deduplication. The decorator never re-attempts an
envelope a connection accepted, but a connection that writes the envelope
and *then* reports failure produces a duplicate on the next connection,
because "wrote it and lost the acknowledgement" is indistinguishable from
"never took it". Both copies carry the same job id. That is the
framework's at-least-once delivery contract, the same one that makes
handler idempotency a requirement everywhere else - see
[Idempotency is the contract between the worker and you](#idempotency-is-the-contract-between-the-worker-and-you).

### Why Suprnova diverges

Laravel's failover connection is a `connections` array in
`config/queue.php`, resolved through the connection registry. Suprnova
has no per-connection driver registry - one driver is bound
process-wide - so the labels come from `QUEUE_FAILOVER_CONNECTIONS` (or
from the `String` you pass to `FailoverQueueDriver::new`) and reads
delegate to the first *driver* rather than to a named connection.

Laravel's `FailoverQueue::bulk` loops the jobs individually so each one's
delay survives. Suprnova resolves the delay onto the envelope before any
driver sees it, so the per-envelope loop preserves it for free - but the
loop is still what keeps a half-landed batch from being double-pushed, so
it stays.

## Push variants

Every push variant takes a typed `J: Job` value and returns when the
envelope is committed to the driver - not when the handler runs.

| Method | Behavior |
| --- | --- |
| `Queue::push(job)` | enqueue immediately |
| `Queue::push_later(job, at)` | available at a specific `DateTime<Utc>` |
| `Queue::later(delay, job)` | available after `delay` from now |
| `Queue::push_with(job, overrides)` | enqueue immediately with per-push `EnvelopeOverrides` |
| `Queue::push_after_commit(job)` | enqueue when the surrounding `DB::transaction` commits |
| `Queue::later_with(delay, job, overrides)` | available after `delay` from now, with per-push `EnvelopeOverrides` |
| `Queue::push_unique(job)` | dedupe by `J::unique_id` within `J::unique_for`, returns `Ok(true)` when the envelope was pushed, `Ok(false)` when a live dedupe key suppressed it |
| `Queue::push_unique_later(job, at)` | unique + scheduled |
| `Queue::later_unique(delay, job)` | unique + delayed |
| `Queue::bulk(vec![job1, job2, ...])` | push every job (driver may use a native bulk path) |

`push_unique` requires the cache layer to be bootstrapped - the dedupe
lock lives in [`Cache`](cache.md) via
[`Idempotency::commit_on_success`](idempotency.md). A failed push releases
the dedupe key so the caller can retry; a successful push holds it for
`J::unique_for` seconds. The job must override `Job::unique_id(&self)` to
return `Some(id)` - `None` returns an internal error.

The boolean answers one question - "is this job on the queue?" - and there
is a third case behind it. If the dedupe lock's lease is lost while the push
is in flight, the push still completes (the idempotency layer never cancels a
body that may already have had an effect) and you still get `Ok(true)`, with
a `warn`-level log naming the job and its unique key. The job is queued; what
is unproven is that nobody else queued the same one concurrently. Your
handler already has to tolerate redelivery, so this needs no extra handling -
but the log is there because a burst of them means the cache backing your
dedupe lock is struggling.

### Unique until processing

A uniqueness lock normally lasts the whole `unique_for` window, even after the
job has run. When the lock exists to coalesce *queued* duplicates rather than
to serialize execution, opt in to releasing it the moment processing begins:

```rust
use std::time::Duration;
use suprnova::{FrameworkError, Job, async_trait};

#[derive(serde::Serialize, serde::Deserialize)]
struct RebuildSearchIndex {
    index: String,
}

#[async_trait]
impl Job for RebuildSearchIndex {
    fn job_name() -> &'static str { "rebuild-search-index" }
    fn unique_id(&self) -> Option<String> { Some(self.index.clone()) }
    fn unique_until_processing() -> bool { true }
    fn unique_for() -> Duration { Duration::from_secs(3600) }

    async fn handle(self) -> Result<(), FrameworkError> {
        // A rebuild that runs for 20 minutes no longer swallows the
        // re-dispatch that arrives at minute 2.
        Ok(())
    }
}
```

The worker releases the lock after the job's middleware pass and immediately
before the handler runs. Four consequences follow:

- A job that a middleware releases back onto the queue keeps its lock. It has
  not started processing, so nothing has changed for a duplicate.
- A job that a middleware short-circuits any other way gives up its lock,
  because it is never going to process at all. That covers deleting the job,
  dead-lettering it, and reporting it complete without ever calling the
  handler.
- A job that fails releases its lock and is still retried. The lock went the
  moment processing began, so a duplicate can enqueue while the failed attempt
  waits out its backoff, and you end up with two envelopes for the same unique
  id. That is the trade this opt-in makes. If a retry has to keep holding the
  slot, leave `unique_until_processing` off and let the `unique_for` TTL cover
  the whole attempt chain.
- The release is owner-scoped. `push_unique` records the lock's owner token on
  the envelope, and the worker releases with that token, so a redelivered
  attempt can never release a lock that a newer dispatch has since acquired.

`unique_until_processing` needs the same two things `push_unique` needs: a
`unique_id` that returns `Some(id)`, and a bootstrapped cache layer.

Under the `sync` driver the handler runs inline inside the `push_unique` call
that took the lock, so the job releases a lock its own caller is still
nominally holding. If that handler runs for longer than a third of
`unique_for`, the dedupe lease renewer notices the lock is gone and logs a
lost-lease warning, and `push_unique` logs its own "exclusivity could not be
proven" warning on top. Both are expected here rather than a fault: the job
ran, the push returns `Ok(true)`, and the lock is gone because the job itself
released it.

### Why Suprnova diverges

Laravel releases an *ordinary* unique job's lock once the handler returns.
Suprnova lets that lock expire with the `unique_for` TTL instead, which keeps
the dedupe window honest when a worker dies mid-job: the window you configured
is the window you get, whether or not the handler ever returned.
`unique_until_processing` behaves the same in both frameworks.

Suprnova also never force-releases a uniqueness lock. Laravel falls back to a
forced release for a first attempt that carries no owner token. The only
envelopes that reach a Suprnova worker without one are envelopes queued before
the token existed, and those keep TTL expiry rather than risking a release that
deletes a newer dispatch's lock.

### Debouncing - keep the last dispatch, not the first

`push_unique` suppresses a duplicate and keeps the **first** dispatch.
Debouncing is the opposite: it keeps the **last**. A burst of twenty "this
order changed" events becomes one reindex, one window after the twentieth,
carrying the newest payload.

```rust
use std::time::Duration;
use suprnova::{FrameworkError, Job, async_trait};

#[derive(serde::Serialize, serde::Deserialize)]
struct ReindexOrder {
    order_id: u32,
}

#[async_trait]
impl Job for ReindexOrder {
    fn job_name() -> &'static str { "reindex-order" }
    fn debounce_for() -> Option<Duration> { Some(Duration::from_secs(30)) }
    fn max_debounce_wait() -> Option<Duration> { Some(Duration::from_secs(300)) }
    fn debounce_id(&self) -> Option<String> { Some(self.order_id.to_string()) }

    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}
```

- `debounce_for` is the window: each dispatch re-arms it, so the run happens
  30 seconds after the *most recent* one.
- `max_debounce_wait` stops a continuous burst from deferring the work forever.
  Once the burst has been deferring for five minutes, the next dispatch is
  queued with no delay. The window then restarts, so each burst measures its
  maximum wait from its own first dispatch.
- `debounce_id` scopes the window. Twenty updates to order 7 become one run;
  an update to order 8 is untouched by them. Omit it and every dispatch of the
  job shares one window.

Every dispatch is still enqueued. The collapse is settled at the worker: each
push overwrites a cache token, and the worker drops any envelope whose token a
newer dispatch has replaced, acknowledging it and emitting `JobDebounced`. That
is what makes the surviving run carry the newest payload rather than the oldest.
If the token has expired or been evicted, the job runs - debouncing fails open,
because a lost token is not evidence that somebody else owns the window.

The [`sync` driver](#drivers) has no worker, so it runs every dispatch inline
and nothing is ever collapsed. Laravel's sync driver behaves the same way.
`Queue::bulk` pushes at the driver level and does not arm a window either, so a
debounced job pushed in bulk runs every copy. Laravel's `Queue::bulk` skips its
own debounce acquisition for the same reason.

Set the window at the call site instead when it belongs to the caller:

```rust
use suprnova::queue::DebounceOptions;

Queue::push_debounced(
    ReindexOrder { order_id: 7 },
    DebounceOptions::new(Duration::from_secs(30))
        .max_wait(Duration::from_secs(300))
        .id("7"),
)
.await?;
```

A job cannot declare both `debounce_for` and `unique_id`: uniqueness keeps the
first dispatch of a burst and debouncing keeps the last, so the push returns an
error naming both. Chains and batches refuse a debounced job for a related
reason - a superseded link is dropped, which would strand the rest of a chain,
and a dropped batch job leaves the batch's pending count above zero so its
callbacks never fire.

### Per-push overrides with `EnvelopeOverrides`

`Queue::push_with` and `Queue::later_with` take an `EnvelopeOverrides`
alongside the job, for the one dispatch that needs different queue,
connection, timeout, or retry behavior than the job's own defaults:

```rust
use std::time::Duration;
use suprnova::queue::{EnvelopeOverrides, Queue};

let overrides = EnvelopeOverrides {
    queue: Some("priority".into()),
    timeout: Some(Duration::from_secs(10)),
    max_tries: Some(1),
    ..Default::default()
};

Queue::push_with(SendWelcomeEmail { user_id: 42 }, overrides.clone()).await?;

// The delayed counterpart, mirroring `Queue::later`'s relationship to `Queue::push`.
Queue::later_with(Duration::from_secs(60), SendWelcomeEmail { user_id: 42 }, overrides).await?;
```

Every field defaults to `None` and defers to the normal resolution
`Queue::push` already runs; a `Some` field wins over all of it for this one
push, outranking both a route registered with [`Queue::route`](#queue-routing)
and the job's own `Job::*` declaration for that field:

| Field | Outranks |
| --- | --- |
| `queue` | `Queue::route`, `Job::queue()` |
| `connection` | `Queue::route`, `Job::connection()` |
| `timeout` | `Job::timeout()` |
| `fail_on_timeout` | `Job::fail_on_timeout()` |
| `max_tries` | `Job::max_tries()` |
| `backoff` | `Job::backoff()` |
| `after_commit` | `Job::after_commit()` |

`EnvelopeOverrides` is the primitive `Mail::on_queue`/`.on_connection()` and
`Notify::queue`'s per-notification queue tuning are both built on - see
[Mail](mail.md#queueing) and [Notifications](notifications.md).

### Job-declared delay

A job can carry its own default delay instead of every call site repeating
`Queue::later(Duration::from_secs(60), job)`:

```rust
impl Job for SendDigest {
    // ...
    fn delay() -> Option<Duration> { Some(Duration::from_secs(60)) }
}
```

`Queue::push(job)`, `Queue::push_with(job, overrides)`, `Queue::push_unique(job)`,
and `Queue::bulk(vec![job1, job2])` all honor it - `available_at` becomes
`now + J::delay()` instead of `now`. `Queue::bulk` resolves the delay once
per call, since every job in the vector shares the same concrete `J` and
therefore the same `Job::delay()`.

An explicit call-site delay always wins: `Queue::push_later(job, at)`,
`Queue::later(delay, job)`, `Queue::later_with(delay, job, overrides)`,
`Queue::push_unique_later(job, at)`, and `Queue::later_unique(delay, job)`
all use the timestamp or delay the caller passed, verbatim - `Job::delay()`
isn't consulted for any of them. Reach for the trait method when every
dispatch of a job type should start delayed by default; reach for one of
the `later`/`push_later` variants for a delay one specific dispatch needs
but the type doesn't otherwise declare.

Batches and chains don't consult it either: `Queue::batch()...add(job)` and
`Queue::chain()...add(job)?` both build their envelopes with `available_at`
set to the moment you called `add`, so a job with a declared `Job::delay()`
dispatches immediately as part of a batch or a chain even though a bare
`Queue::push(job)` of the same job would wait. Give the job an explicit
delay some other way - a field on the job itself, applied in `handle()` - if
a batched or chained step needs one.

### Why Suprnova diverges

Laravel's `$job->delay` is an instance property, set per dispatch
(`SendDigest::dispatch($user)->delay(60)`), so two dispatches of the same
class can carry different delays. `Job::delay()` here is a class-level
default instead, like `Job::queue()` or `Job::max_tries()` - a dispatch
needing a delay computed from its own data uses `Queue::later`/`push_later`,
which already outranks the declared default.

### After-commit dispatch

A job pushed inside a [`DB::transaction`](database.md#transactions) is racing
that transaction. A worker on another process can pop the envelope, look for
the row the transaction is still holding open, and fail - or worse, the
transaction rolls back and the job runs against data that no longer exists.

Opt the job into waiting for the commit:

```rust
use suprnova::{DB, FrameworkError, Job, Queue, async_trait};

#[derive(serde::Serialize, serde::Deserialize)]
struct SendReceipt {
    order_id: i64,
}

#[async_trait]
impl Job for SendReceipt {
    fn job_name() -> &'static str { "send-receipt" }
    fn after_commit() -> bool { true }

    async fn handle(self) -> Result<(), FrameworkError> {
        // The order row is guaranteed to be durable by the time this runs.
        Ok(())
    }
}

DB::transaction(|_tx| {
    Box::pin(async move {
        let order = Order::create(suprnova::attrs! { total: 4999i64 }).await?;
        // Nothing reaches the driver here.
        Queue::push(SendReceipt { order_id: order.id }).await?;
        Ok::<(), FrameworkError>(())
    })
})
.await?;
// The envelope is on the queue now, and only now.
```

Three rules cover every case:

- **Inside a transaction, the whole push waits for the commit.** Not just the
  driver write: the envelope build, the `JobQueueing` event and the
  `JobQueued` event all happen at commit time too, so a listener is never told
  about a job that a rollback then discards.
- **A rollback discards it.** The push simply never happens. If it took a
  uniqueness lock, the rollback gives that lock back.
- **Outside a transaction the push happens immediately.** That is what makes
  the opt-in safe to declare on the job type: a dispatch site does not have to
  know whether the code path it sits on is transactional.

A [savepoint](database.md#savepoints) rollback counts as a rollback for
everything registered inside it. `tx.rollback_to("name")` discards the pushes
deferred since `tx.savepoint("name")` and releases the locks they took, right
then, so a re-dispatch inside the same transaction wins the key again. Pushes
made before the savepoint are untouched, and a savepoint you never roll back
keeps everything registered inside it.

Per dispatch rather than per job type, use `EnvelopeOverrides::after_commit`.
`Some(true)` is Laravel's `afterCommit()` and has the shorthand
`Queue::push_after_commit(job)`; `Some(false)` is Laravel's `beforeCommit()`,
for the one dispatch that has to be visible to a worker before the commit
lands:

```rust
use suprnova::queue::{EnvelopeOverrides, Queue};

// Defer a job whose type does not opt in.
Queue::push_after_commit(SendWelcomeEmail { user_id: 42 }).await?;

// Push immediately even though the job type opts in.
Queue::push_with(
    SendReceipt { order_id: 7 },
    EnvelopeOverrides { after_commit: Some(false), ..Default::default() },
)
.await?;
```

A deferred `Queue::push` re-resolves [`Job::delay()`](#job-declared-delay)
against the commit, not against the push, because the delay means "wait this
long after dispatch" and for a deferred job dispatch *is* the commit. An
explicit timestamp is the caller's intent about a moment in time, so
`Queue::push_later`, `Queue::later` and `Queue::later_with` carry theirs
through the deferral unchanged.

`Queue::push_unique` defers with one deliberate asymmetry: the dedupe lock is
taken immediately, so a second `push_unique` for the same unique id inside the
same transaction is still suppressed and still reports `Ok(false)`. Only the
envelope waits. The winner reports `Ok(true)` even though its push is pending,
because the push is going to happen. A rollback releases the lock it took,
owner-scoped, so the `unique_for` window is never blocked by a dispatch that
never happened - and so does any other ending where the commit does not land,
including a refused `COMMIT`. The one bound on that guarantee is the TTL
itself: a transaction that stays open longer than `unique_for` can have its
lock expire and be re-taken by another dispatch mid-flight, so give
`unique_for` room above your longest transaction if the dedupe matters. The
`push_unique*` family takes no `EnvelopeOverrides`, so `Job::after_commit()` is
the only thing that decides whether a unique push defers - there is no per-push
override for it.

Batches and chains do not defer, the same way they do not consult
`Job::delay()`: `Queue::batch()` and `Queue::chain()` build and push their
envelopes directly. Wrap the `.dispatch()` call so it runs after the
transaction returns if a batch has to wait for a commit.

Queued [mail](mail.md#queueing) and [notifications](notifications.md) do not
defer either. Each rides a single shared job type (`SendMailJob` /
`SendNotificationJob`), and there is no
`ShouldQueueAfterCommit` equivalent on `Mailable` or `Notification` yet, so a
`Mail::queue` or `Notify::queue` call inside a transaction reaches the driver
immediately. Send those after the transaction returns.

Under `Queue::fake()` a push is recorded immediately, deferral and all, so a
test can assert on it without committing anything. This matches Laravel's
`Bus::fake`, and it is what lets a test drive one transactional handler and
assert its dispatches in the same breath.

### Why Suprnova diverges

`Queue::bulk` is monomorphic - every element shares one concrete `J` - so its
after-commit partition is all or nothing for the call. Laravel partitions a
heterogeneous array into deferred and immediate halves; there is nothing here
to partition.

Deferral is tied to the closure form. A push inside a manual
[`DB::begin_transaction`](database.md#manual-form) happens **immediately**,
because manual mode installs no ambient transaction and therefore has no
commit to hang a callback on. Deferring there would queue a callback that
nothing ever runs, and a dispatch that silently disappears is worse than one
that happens too early. Reach for `DB::transaction` when a dispatch has to
wait for the commit.

Laravel also reads a connection-level `after_commit` config key as the last
fallback in its precedence chain. Suprnova stops at the per-push override and
then the job's own `Job::after_commit()`: queue connections here do not carry
their own dispatch policy.

## Job configuration

Override `Job`'s associated functions to tune behavior per impl:

```rust
use std::time::Duration;
use suprnova::queue::{BackoffSchedule, JobMiddleware};

#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str { "SendWelcomeEmail" }

    async fn handle(self) -> Result<(), FrameworkError> { /* … */ Ok(()) }

    fn delay() -> Option<Duration> { None }                // default: no delay
    fn max_tries() -> u32 { 5 }                            // default: 3
    fn timeout() -> Option<Duration> { Some(Duration::from_secs(30)) }
    fn fail_on_timeout() -> bool { false }                 // default: false (timeout retries)
    fn backoff() -> BackoffSchedule {
        BackoffSchedule::Sequence { secs: vec![5, 15, 60, 300] }
    }
    fn unique_id(&self) -> Option<String> {
        Some(format!("welcome:{}", self.user_id))
    }
    fn unique_for() -> Duration { Duration::from_secs(600) }  // default: 5 minutes
    fn unique_until_processing() -> bool { true }          // default: false (TTL is the window)
    fn middleware() -> Vec<std::sync::Arc<dyn JobMiddleware>> {
        vec![/* see "Job middleware" below */]
    }
}
```

## Queue routing

By default every job goes to one queue and every worker drains all of it. Once
some jobs are slower or more important than others, you want dedicated worker
pools: a long-running export shouldn't sit behind a thousand welcome emails.

A job can state where it belongs:

```rust
#[async_trait]
impl Job for GenerateExport {
    fn job_name() -> &'static str { "GenerateExport" }
    async fn handle(self) -> Result<(), FrameworkError> { Ok(()) }

    fn queue() -> Option<&'static str> { Some("exports") }
    fn connection() -> Option<&'static str> { None }   // default connection
}
```

…and an operator can override that centrally, without touching the job:

```rust
// bootstrap::register()
use suprnova::Queue;

Queue::route::<GenerateExport>(None, Some("heavy"));
Queue::route::<SendInvoice>(Some("redis"), Some("billing"));
```

Resolution runs highest-priority first:

1. a per-push override passed to `Queue::push_with` / `Queue::later_with` (see
   [Per-push overrides with `EnvelopeOverrides`](#per-push-overrides-with-envelopeoverrides))
2. a route registered with `Queue::route`
3. the job's own `Job::queue` / `Job::connection`
4. the driver / global default

Passing `None` for a field leaves that dimension alone, so routing a job's
connection does not disturb the queue it already declared.

The two dimensions run at different depths today. The **queue** is honored end
to end - stamped on the envelope, stored by the driver, filtered by `--queue`.
The **connection** resolves the connection *name* carried on the `JobQueueing`
/ `JobQueued` lifecycle events, which is what listeners and dashboards see;
one process-global driver still receives every push, so routing a job's
connection does not yet select a different driver. Declaring connections now
is forward-compatible for when per-connection drivers land, not behavioral.

Then dedicate a worker to it:

```bash
./app queue:work --queue=billing
./app queue:work --queue=exports,heavy
./app queue:work                       # drains every queue, as before
```

A job with no route belongs to `default`, so `--queue=default` drains
unrouted work rather than stranding it.

### Forwarding a whole queue

`Queue::route` is keyed by job type. When you want to drain one pool through
another - retiring a queue, absorbing a backlog, moving work off a pool you are
about to take down - key the redirect by queue name instead:

```rust
// bootstrap::register()
use suprnova::Queue;

Queue::forward("default", "high");
Queue::forward_on("exports", "heavy", "redis");   // only on the `redis` connection
```

The connection in `forward_on` is a gate, and it is compared against this
process's connection name - `Queue::set_connection_name` if you set one, the
driver's own name otherwise. It is not compared against the job's
`Job::connection`, a `Queue::route`'s connection, or a per-push
`EnvelopeOverrides` connection: those name what the lifecycle events report, and
a worker has only the process name to gate its claim list on. Both halves of the
redirect gate on that one value, so a forward can never move the push without
moving the claim.

The redirect applies on both sides, which is what keeps it from stranding work:

- **On the push side**, the name is rewritten after routing and the job's own
  `Job::queue` have had their say, and after a per-push `EnvelopeOverrides`
  queue if you passed one.
- **On the pop side**, a worker started with `--queue=default` drains `high`.
  Without that half, the destination queue would collect jobs no worker claims.

A worker started with no `--queue` at all already drains everything, so a
forward changes nothing for it. Forwarding `default` catches jobs that named no
queue, because an unrouted job belongs to `default`.

A forward is a single lookup, never a chain. With `a -> b` and `b -> c`
registered, a push that resolved to `a` lands on `b`. Registering `b -> a` on
top of an existing `a -> b` is therefore a coherent pool swap, not a loop: a
push to `a` still lands on `b`, a push to `b` now lands on `a`, and a worker
started on either name claims the other - nothing chains, so nothing strands. A
longer rotation among more queue names resolves the same way, one independent
hop at a time. Laravel's `Queue::forward` has no cycle check either, for the
same reason: its resolver is this same single lookup. Forwarding a queue onto
its own name is the identity - no redirect at all - which is how you neutralize
a forward you already registered.

Only future pushes move. Envelopes already sitting on the source queue stay
there, and the worker that used to drain them is now claiming the destination,
so drain the source pool before you forward it. The same applies to
`queue:retry`: a failed job is re-enqueued onto the queue it died on.

Pausing is evaluated before the redirect, on the names the worker was started
with. `Queue::pause(&connection, "default")` still stops a worker started on
`--queue=default`, even while `default` is forwarded to `high`. The converse
also holds: pausing the forward's *destination* - `Queue::pause(&connection,
"high")` - does not stop a worker started on `--queue=default`, because that
worker is reached through its source name, not the rewritten one. The
`WorkerQueuePaused` event this transition raises carries `queue: default`,
the configured name, never `high` - Laravel orders and reports it the same way.

The inspection calls are deliberately not forwarded: `Queue::pending_jobs(
Some("default"))` lists what is literally on `default`, not what is on `high`,
which is how you see the backlog stranded on a source queue you have just
forwarded. Laravel resolves the forward there too; see the divergence note below.

Read a registered forward back with `Queue::forward_for("default")`, which
returns the destination in `queue` and the connection gate in `connection`.

### Why Suprnova diverges

Laravel's `Queue::route(...)` takes a class string; Suprnova takes the job as a
type parameter, so a renamed or deleted job is a compile error rather than a
route that silently stops matching.

The larger divergence is what happens when a driver can't filter.
`QueueDriver::pop_from` **rejects** a queue filter it cannot honor instead of
falling back to draining everything. A worker told to drain only `billing` that
quietly drains all queues looks identical to a working deployment until the
wrong pool consumes the wrong jobs - so the misconfiguration is made loud at
the first poll. The memory and database drivers filter natively; a driver that
doesn't - the Redis driver is one, since a single stream consumer group has no
per-queue storage - will error rather than mislead.

`Queue::forward` ports the queue-to-queue half of Laravel's `Queue::forward`
in full, and only that half. Laravel's third argument can move a forwarded queue
onto a different *connection*, because its queue manager resolves a driver per
connection name. Suprnova has one process-global driver and a connection name
only labels lifecycle events, so `Queue::forward_on(from, to, connection)`
treats the connection as a **gate** - it decides whether the queue-name redirect
applies - and never as a destination. For the same reason `to` is required here,
while Laravel's is optional: an omitted `to` in Laravel means "move only the
connection", which is precisely the dimension Suprnova cannot honor, so a
`forward(from, None)` would be a no-op dressed as a configuration change.

Laravel's inspection calls follow a forward, because `pendingJobs($queue)` and
its siblings run through the same driver-level `getQueue()` the push and the pop
do. Suprnova's `Queue::pending_jobs` / `delayed_jobs` / `reserved_jobs` report
the literal queue you name instead. With one process-global driver, the literal
view is the only way to see the envelopes that stayed behind on a queue you have
just forwarded away - the backlog this section tells you to drain first. Ask for
the destination queue by name to see where new work is landing.

### The `jobs` table

`DatabaseQueueDriver` expects this schema. The `queue` column is what makes
`--queue` filtering possible:

```sql
CREATE TABLE jobs (
    id              TEXT PRIMARY KEY,
    job_name        TEXT NOT NULL,
    queue           TEXT NULL,
    envelope_json   TEXT NOT NULL,
    available_at    BIGINT NOT NULL,
    reserved_until  BIGINT NULL,
    reserved_token  TEXT NULL,
    attempts        INTEGER NOT NULL DEFAULT 0,
    created_at      BIGINT NOT NULL
);
CREATE INDEX idx_jobs_available_at ON jobs(available_at);
CREATE INDEX idx_jobs_queue ON jobs(queue);
```

`queue` is nullable, and an unrouted job stores `NULL` rather than `'default'`.
That is deliberate: a row written by an older binary is indistinguishable from
an unrouted row written by a new one, so a mixed-version fleet drains the same
work during a rolling upgrade.

Adding the column to an existing table is **required**, not just for
filtering: `push` names the `queue` column in its `INSERT` whether or not the
job is routed, so a 0.7.0+ binary fails every push against a table that lacks
it. Run the migration first, then roll binaries - older binaries list their
columns explicitly and ignore the new one, so that order is safe:

```sql
ALTER TABLE jobs ADD COLUMN queue TEXT NULL;
CREATE INDEX idx_jobs_queue ON jobs(queue);
```

### Backoff schedules

| Variant | Behavior |
| --- | --- |
| `Fixed { secs }` | constant per-attempt delay |
| `Exponential { base_secs, cap_secs, jitter_ratio }` | `min(base * 2^(attempts-1), cap)` × random in `[1±jitter]` |
| `Sequence { secs }` | one entry per attempt; the last entry repeats once exhausted |

The default is `Exponential { base_secs: 2, cap_secs: 300, jitter_ratio: 0.25 }` -
2 seconds to 5 minutes with ±25% jitter.

## Job middleware

Six middleware ship in-tree, all mirroring `Illuminate\Queue\Middleware\*`:

| Middleware | Behavior |
| --- | --- |
| `WithoutOverlapping` | hold a `Cache::lock` for the duration; release-with-delay on contention |
| `RateLimited` | gate on `RateLimiter` budget; release until the window resets |
| `ThrottlesExceptions` | rate-limit on consecutive *failures*, not requests |
| `Skip::when(cond)` / `Skip::unless(cond)` | drop the job when the condition is met |
| `FailOnException` | promote matching errors to permanent failures (no retry) |
| `SkipIfBatchCancelled` | drop the job if its owning batch was cancelled |

Wire them on the `Job` impl:

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::queue::{JobMiddleware, RateLimited, WithoutOverlapping};

fn middleware() -> Vec<Arc<dyn JobMiddleware>> {
    vec![
        Arc::new(
            WithoutOverlapping::new("user-42")
                .expire_after(Duration::from_secs(120))
        ),
        Arc::new(
            RateLimited::new(10, Duration::from_secs(60))
                .by("send-mail")
        ),
    ]
}
```

`WithoutOverlapping` and `RateLimited` need the cache subsystem booted
(`Cache::init` or `App::bind::<dyn CacheStore>(...)` at startup).

### A lock that will not release does not fail the job

If `WithoutOverlapping` cannot release its lock after the handler has
run - the cache backend blipped, the connection dropped - it logs at
`warn` and returns the handler's own outcome anyway. The lock then
lapses at `expire_after`.

That is deliberate. By the time the release runs, the handler has
already committed its side effects: rows written, mail sent, charges
made. Reporting the release failure as a job failure would make the
worker retry and do all of it a second time, which is a worse outcome
than a lock key held for its TTL. A handler that genuinely failed still
reports its failure - suppressing the release error does not suppress
the handler's.

### The release-without-burning-attempt contract

Middleware returns a `JobOutcome` rather than `Result<()>`. Four variants:

- `JobOutcome::Completed` - handler ran, ack.
- `JobOutcome::Released { delay }` - re-enqueue after `delay` **without**
  incrementing `attempts`. Used by `WithoutOverlapping`, `RateLimited`. The
  worker hands the whole operation to `QueueDriver::release`, and every
  in-tree driver requeues its own stored copy in place, so the message is
  never simultaneously reserved and visible, and never neither. The
  attempt count is preserved with no arithmetic in the worker for a driver
  to disagree with - the stored copy was never bumped for this run.
- `JobOutcome::Failed { reason }` - dead-letter now, persist to the
  failed-jobs store, do not retry.
- `JobOutcome::Deleted` - drop the reservation without dead-letter. Used
  by `Skip`. If the job belonged to a batch, the batch's `pending_jobs`
  decrements anyway so callbacks can fire.

This contract is what makes "throttled because the bucket was full" feel
different from "failed because the handler errored" in retry accounting,
metrics, and lifecycle events.

### What counts as an attempt

Two ways a job leaves a worker without finishing, and both consume an
attempt:

- **The handler failed** - returned `Err`, or panicked into the
  framework's boundary. The worker nacks; the driver requeues with
  `attempts + 1`.
- **The worker died** - OOM kill, `abort()`, a segfault, `docker kill`, or
  the SIGKILL a supervisor sends when a stop times out. Nothing settles
  anything; the reservation simply lapses. Whichever worker reclaims the
  job charges the attempt at that point.

The second case used to be free, and that was a hole rather than a
kindness: a job that reliably kills its worker could never exhaust
`max_tries` and so could never be dead-lettered. It would kill each worker
that claimed it, come back byte-identical, and kill the next one, for as
long as anything kept restarting workers.

All three in-tree drivers charge it, because swapping `QUEUE_DRIVER` must
not change whether a poison job can be stopped. `database` detects a
lapsed `reserved_until`; `memory` charges it when the reaper moves the
reservation back to visible; `redis` reads the entry's delivery count from
`XPENDING`, since a Redis stream entry is immutable and its own counter is
the only record.

`JobOutcome::Released` is the deliberate exception - see the contract
above. A job throttled by `RateLimited` never ran, so it owes nothing.

**On Redis, reclaim has two clocks.** `--visibility-timeout` sets how long
an entry must sit unacked before it qualifies for reclaim; a second
interval governs how often a consumer looks. The driver ties the second to
the first, so a lost job comes back within roughly twice the configured
timeout rather than the timeout plus a fixed 30 seconds.

**The budget is checked before the handler runs, not only when settling.**
Every other dead-letter decision happens after a handler returns, which
assumes the handler returns. A job that kills its worker cannot reach
that check, so the worker also refuses to dispatch a job whose attempts
are already spent - it dead-letters it instead, before it takes another
worker down. Without this, counting the attempt would only make a number
climb while the job kept cycling.

**What this means for you.** `attempts` counts *deliveries to a worker*,
not *handler failures*. A worker lost for reasons unrelated to the job - a
host reboot, an OOM caused by a noisy neighbour - burns an attempt from
that job's budget too. Laravel behaves the same way. Size `max_tries` with
that in mind, and prefer idempotent handlers: at-least-once delivery was
always the contract, and this makes the redelivery path count honestly
rather than silently.

## Lifecycle events

Workers emit Laravel-shape lifecycle events through the
[`Event`](events.md) facade. Listeners get the envelope's identity (`id`,
`job_name`, `attempts`, `max_tries`, `connection`), not the typed job
instance - the worker is type-erased over JSON payloads. Errors travel
as a `String` since `FrameworkError` doesn't derive `Clone`.

| Event | Fires when |
| --- | --- |
| `JobQueueing` | before the envelope hits the driver |
| `JobQueued` | after the driver accepts |
| `UniqueJobSkipped` | `push_unique` suppressed a duplicate inside the `unique_for` window |
| `JobDebounced` | the worker dropped an envelope a newer debounced dispatch superseded |
| `JobProcessing` | worker popped, about to dispatch |
| `JobProcessed` | handler returned `Ok` |
| `JobAttempted` | every terminal settlement (success, fail, timeout) |
| `JobExceptionOccurred` | handler returned `Err`, will retry |
| `JobReleasedAfterException` | retry-after-error re-enqueue happened |
| `JobReleased` | middleware-driven release (no failure) |
| `JobFailed` | dead-lettered |
| `JobTimedOut` | per-attempt timeout exceeded |
| `Looping` | every loop iteration (before the pop) |
| `WorkerStarting` / `WorkerStopping` | once per worker lifetime |
| `WorkerInterrupted` | `Queue::restart()` signal observed |
| `QueuePaused` | `Queue::pause` set one queue's own switch |
| `QueueResumed` | `Queue::resume` cleared one queue's own switch |
| `QueuesPaused` | `Queue::pause_all` set the global switch |
| `QueuesResumed` | `Queue::resume_all` cleared the global switch |
| `WorkerQueuePaused` | a running worker first observed a queue as paused |
| `WorkerQueueResumed` | a running worker saw a paused queue become claimable |

Subscribe with the normal `Event::listen` API. Events are best-effort -
`Event::dispatch` with no listeners is a no-op `Ok(())`, so workers in
deployments without `Event::init()` pay nothing.

`UniqueJobSkipped` is the one event that fires on the *push* side rather
than the worker side, and the one that reports a non-failure. It carries
`job_name`, `unique_id`, and `connection` - the dedupe decision happens
before an envelope exists, so there is no envelope id to report. The
push still returns `Ok(false)`; the event is what makes an otherwise
invisible suppression observable.

`QueuePaused` / `QueueResumed` / `QueuesPaused` / `QueuesResumed` fire the
same way - from `Queue::pause` / `resume` / `pause_all` / `resume_all`
themselves, not from the worker loop. They carry no envelope identity
either; see "Pausing queues" below for the full contract.

`WorkerQueuePaused` / `WorkerQueueResumed` are the worker-side pair, and they
are the ones that tell you *why a particular worker went quiet*. They fire once
per transition from inside the worker loop, carry the connection the worker is
draining, and carry the queue name - or `None`, when an unfiltered worker is
idle on a global pause and has no queue names to report.

## Failed-jobs storage

Dead-lettered jobs land in the configured `FailedJobStore`:

```rust
use std::sync::Arc;
use suprnova::queue::{Queue, MemoryFailedJobStore};

Queue::set_failed_store(Arc::new(MemoryFailedJobStore::new()));

// In admin tooling:
let store = Queue::failed_store().unwrap();
for record in store.all().await? {
    println!("{} failed: {}", record.job_name, record.exception);
}
store.forget(some_id).await?;
store.flush(None).await?;
```

Three backends:

- `MemoryFailedJobStore` - in-process `Vec`, lost on restart.
- `DatabaseFailedJobStore` - persists to a `failed_jobs` table via SeaORM.
- `NullFailedJobStore` - discards every record. Mirrors Laravel's
  `NullFailedJobProvider`.

### When the store rejects a record

If the configured store returns an error, the worker logs at `error` and
**leaves the reservation intact** rather than acking. The job returns on
visibility expiry and is retried - it is not silently dropped.

That is deliberate. The alternative, acking anyway, discards a job that
already exhausted its attempts *and* failed to be recorded anywhere, which
is unrecoverable. A job that keeps coming back is recoverable: fix the
store and the next delivery lands.

The practical case is a `DatabaseFailedJobStore` pointed at an unmigrated
`failed_jobs` table. Until you migrate, dead-lettering jobs cycle at one
redelivery per visibility timeout, each logging the store's error. If you
genuinely want failures discarded, configure `NullFailedJobStore` - that
succeeds, so the job acks and is gone.

### Retrying

```rust
use uuid::Uuid;

// Single record - false if the id wasn't in the store.
Queue::retry_failed(some_id).await?;

// Bulk - optional cutoff (only retry records older than `before`).
let count = Queue::retry_all_failed(None).await?;
```

`retry_failed` loads the envelope, resets `attempts`, `available_at`, and
the `idempotency_key`, pushes through the configured driver, then deletes
the failed-job record. Mirrors `php artisan queue:retry <id>` plus
`queue:flush` semantics (each retried envelope is pushed AND removed
from the store).

### `failed_jobs` schema

The `DatabaseFailedJobStore` expects this table (managed by your
migrations):

```sql
CREATE TABLE failed_jobs (
    id              TEXT PRIMARY KEY,
    connection      TEXT NOT NULL,
    queue           TEXT NOT NULL,
    job_name        TEXT NOT NULL,
    envelope_json   TEXT NOT NULL,
    exception       TEXT NOT NULL,
    failed_at       BIGINT NOT NULL
);
CREATE INDEX idx_failed_jobs_failed_at ON failed_jobs(failed_at);
```

The `table` argument to `DatabaseFailedJobStore::new` is validated as a
SQL identifier at construction.

## Queued batches

Dispatch a group of jobs with progress tracking and completion callbacks:

```rust
use std::sync::Arc;
use suprnova::queue::{Queue, MemoryBatchRepository, batch::register_callback};

Queue::set_batch_repository(Arc::new(MemoryBatchRepository::new()));

// Register named callbacks at boot.
register_callback(Arc::new(SendSummary));
register_callback(Arc::new(PageOnFail));

let id = Queue::batch()
    .name("import-users")
    .add(ImportUser { id: 1 })
    .add(ImportUser { id: 2 })
    .add(ImportUser { id: 3 })
    .then("send-summary-email")
    .catch("page-on-fail")
    .finally("cleanup-temp-tables")
    .dispatch()
    .await?;

// Inspect progress later:
let repo = Queue::batch_repository().unwrap();
let snap = repo.find(&id).await?.unwrap();
println!("{}/{} jobs done ({}%)", snap.processed_jobs(), snap.total_jobs, snap.progress());
```

Each worker settles its job against the batch, and when `pending_jobs`
hits zero the worker fires the registered `then`/`catch`/`finally`
callbacks. By default the first failure cancels the batch;
`.allow_failures()` keeps remaining jobs going.

### Durable batches

`MemoryBatchRepository` is lost on restart, which strands every in-flight
batch: its counters are gone, `pending_jobs` can never reach zero again,
and the callbacks never fire. Use `DatabaseBatchRepository` in production:

```rust
use std::sync::Arc;
use suprnova::queue::{Queue, DatabaseBatchRepository};

Queue::set_batch_repository(Arc::new(DatabaseBatchRepository::new(db.clone())));
```

Two tables, which the framework does not create - add them to your
migrations, the same way `jobs` and `failed_jobs` work:

```sql
CREATE TABLE job_batches (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    total_jobs    INTEGER NOT NULL,
    options_json  TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    cancelled_at  INTEGER NULL,
    finished_at   INTEGER NULL
);

CREATE TABLE job_batch_settlements (
    batch_id   TEXT NOT NULL,
    job_id     TEXT NOT NULL,
    failed     INTEGER NOT NULL,
    settled_at INTEGER NOT NULL,
    PRIMARY KEY (batch_id, job_id)
);
```

`DatabaseBatchRepository::with_tables(db, batches, settlements)` names them
yourself; both names are validated as SQL identifiers at construction.

Note what `pending_jobs` and `failed_jobs` are **not**: columns. They are
derived from the settlement rows on every read -

```text
pending_jobs = max(0, total_jobs - COUNT(settlements))
failed_jobs  = COUNT(settlements WHERE failed)
```
 -
because queues are at-least-once, so the same job settles more than once
whenever a redelivery happens, an ack is duplicated, or a worker dies
between doing the work and recording it. A counter decremented per
settlement drifts on every one of those, and the drift is not cosmetic:
`pending_jobs` gates the callbacks, so an early zero fires `then` while
other jobs in the batch are still running. With the counts derived and the
primary key on `(batch_id, job_id)`, a repeat settlement inserts nothing and
there is no counter to get wrong - across processes, not just within one.

### When a dispatch fails halfway

If a `driver.push` fails partway through `dispatch()`, the jobs that
already reached the queue are real and already stamped with the batch id.
So the batch is settled rather than removed: every envelope that was *not*
pushed is recorded as a failed job, and the batch is cancelled.

`total_jobs` still counts what you asked for, `failed_job_ids` names
exactly the jobs that never made it, the ones already queued settle
normally, and `SkipIfBatchCancelled` drops the rest - so `pending_jobs`
still reaches zero and your `catch`/`finally` callbacks still run. If
nothing was pushed at all, `dispatch` fires them itself, because no worker
is left to. You get the original push error back either way.

### Batch options

| Option | Builder method | Effect |
| --- | --- | --- |
| Allow failures | `.allow_failures()` | continue scheduling after a job fails |
| Then callback | `.then(name)` | runs on every-job-success |
| Catch callback | `.catch(name)` | runs on first failure |
| Finally callback | `.finally(name)` | runs after batch settles either way |
| Skip cancelled | `SkipIfBatchCancelled` middleware on the job | drop remaining jobs when batch is cancelled |

### `BatchCallback` impl

```rust
use async_trait::async_trait;
use suprnova::queue::{Batch, BatchCallback};
use suprnova::error::FrameworkError;

pub struct SendSummary;

#[async_trait]
impl BatchCallback for SendSummary {
    fn name(&self) -> &'static str { "send-summary-email" }

    async fn handle(&self, batch: Batch, error: Option<String>) -> Result<(), FrameworkError> {
        let subject = match error {
            Some(_) => format!("Batch {} failed", batch.name),
            None    => format!("Batch {} done - {} jobs", batch.name, batch.total_jobs),
        };
        // … send mail
        Ok(())
    }
}
```

Register at boot with `batch::register_callback(Arc::new(SendSummary))`.
Callbacks are keyed by `name()` - the batch's options store callback
names, so a process restart picks up registered callbacks by lookup
instead of trying to deserialize a closure (Rust closures don't
serialize).

## Queued chains

Sequential workflows where each link runs only after the previous one's
handler acks:

```rust
Queue::chain()
    .add(GenerateReport { id: 99 })?
    .add(UploadToBucket { id: 99 })?
    .add(NotifyOwner { id: 99 })?
    .dispatch()
    .await?;
```

The first envelope is pushed immediately; the rest travel on its
`chain_remaining` payload field. On every successful settlement the
worker pops the next entry and dispatches it. A failure breaks the
chain - subsequent links are never enqueued.

### Terminal settlement

Finishing a chained job means two things: enqueue the successor, and
release the job just finished. As two separate operations there is no safe
order. Ack first, and a crash in the gap loses the rest of the chain
permanently - nothing is left in the queue to retry from. Push first, and
the same crash redelivers the finished job, so its handler runs again and
the successor is enqueued twice.

So the worker hands both to the driver at once, via
`QueueDriver::settle(token, follow_ups)`:

| Outcome | Meaning |
| --- | --- |
| `Settled::Atomically` | successor enqueued and reservation dropped in one transaction |
| `Settled::Stale` | the reservation was reclaimed by another consumer; **nothing** was enqueued or dropped |
| `Settled::Unsupported` | this driver cannot settle transactionally |

`DatabaseQueueDriver` implements it: both effects are one transaction, and
the reservation-keyed `DELETE` doubles as a fence. If your visibility
timeout expired while the handler was running and another worker picked the
job up, the delete matches nothing, the transaction rolls back, and you get
`Stale` - having enqueued nothing. Two-step settlement cannot express that
at all: your push succeeds, the new owner's push succeeds, and the chain
forks.

Redis and the in-memory driver answer `Unsupported` and keep the
push-before-ack ordering, which trades permanent loss for an at-least-once
duplicate. That is the framework's documented contract, and it is why
chained envelope ids are derived from their predecessor rather than random -
a redelivered step re-pushes the id it pushed before, so the duplicate is
recognisable as the same logical step.

If you write a driver whose follow-up write and acknowledgement share a
transaction domain, implement `settle`. Its default returns `Unsupported`,
so drivers written before this existed keep working unchanged.

## Introspection

```rust
Queue::size().await?;            // total
Queue::pending_size().await?;    // available_at <= now, not reserved
Queue::delayed_size().await?;    // available_at > now
Queue::reserved_size().await?;   // currently popped, not yet acked
Queue::clear().await?;           // drop every envelope, returns the count
Queue::driver_name()?;           // configured driver name for logs / admin
```

The `QueueDriver` trait declares defaults for `size` / `pending_size` /
`reserved_size` / `delayed_size` / `clear`; `MemoryQueueDriver`,
`DatabaseQueueDriver`, and `RedisQueueDriver` all implement them natively.

### Inspecting queues

Counts tell you how much is queued; sometimes you need to see the actual
envelopes - an admin dashboard, a debugging session, a "what exactly is
stuck" question. `Queue::pending_jobs` / `delayed_jobs` / `reserved_jobs`
return the same information the size counters count, as a listing of
`InspectedJob` DTOs:

```rust
use suprnova::queue::{InspectedJob, Queue};

let pending: Vec<InspectedJob> = Queue::pending_jobs(None).await?;
let billing_only: Vec<InspectedJob> = Queue::pending_jobs(Some("billing")).await?;
let delayed = Queue::delayed_jobs(None).await?;
let reserved = Queue::reserved_jobs(None).await?;

for job in &pending {
    println!(
        "{} attempts={} queue={:?} payload={}",
        job.name, job.attempts, job.queue, job.payload
    );
}
```

`InspectedJob` carries `id`, `queue`, `name`, `attempts`, `payload`, and
`created_at`. `id` and `created_at` are `Option`: the database driver's
listings still report a row whose `envelope_json` failed to decode - as
`id: None` and `payload: {"unparseable": true}` - rather than dropping it
and hiding a poison job from whoever is looking; `Queue::fake()`'s
projection never records a dispatch timestamp separate from
`available_at`, so `created_at` is always `None` there.

On the memory driver, `delayed_size()` reads the delayed store's length
directly, while `delayed_jobs()` and `pending_jobs()` first promote any
entry whose `available_at` has already passed. In the narrow window
between a job coming due and the background reaper's next 50ms tick,
`delayed_size()` can still count a job that `delayed_jobs()` has already
promoted into `pending_jobs()` - the listings are the more current view;
a mismatch there is expected, not a bug.

A reservation whose visibility timeout has lapsed keeps appearing in
`reserved_jobs()` until a `pop` or the background reaper reclaims it. Only
those two reclaim, and reclaiming is what spends an attempt, so a listing call
never changes a job's attempt count however often you call it.

#### Why Suprnova diverges

- **One method with `Option<&str>`, not a pair per listing.** Laravel ships
  `pendingJobs($queue)` alongside a separate `allPendingJobs()`; here
  `queue: None` collapses the two into one call. Same shape for
  `delayedJobs`/`allDelayedJobs` and `reservedJobs`/`allReservedJobs`.
- **The trait default is an honest `Err`, not an empty collection.**
  Laravel's Beanstalkd and SQS drivers return `[]` from these methods even
  for a queue that plainly has jobs - a lie of omission a third-party
  driver author could copy without noticing. A Suprnova driver that has
  not implemented inspection says so; `sync` and `null` override with
  `Ok(vec![])` because for them "there is never anything to list" is the
  literal truth, not an unimplemented method.
- **Redis's `reserved_jobs` is per-consumer.** The driver only knows the
  reservations it has personally handed out in-process; another
  consumer's in-flight entries are visible only through Redis's own
  `XPENDING`, not through this call.
- **Redis's `pending_jobs` means "never delivered to any consumer in this
  group."** It scans `XRANGE (<last-delivered-id> +` - everything past the
  group's delivery cursor (`XINFO GROUPS`) - rather than the whole stream,
  because `ack` only `XACK`s an entry (this driver never `XDEL`/`XTRIM`s
  the stream), so a scan that merely excluded one consumer's in-memory
  reservations would report every acked job as pending forever. A
  released or nacked job is re-published under a fresh id above the
  cursor, so it reappears once its retry is live. Same "upper bound"
  register as `pending_size`: the cursor is read once, so a concurrent
  `pop` can claim an entry between that read and the scan. In practice, a
  running consumer's background read-ahead task tends to claim a newly
  pushed entry within milliseconds of the push, well before an
  application ever calls `pop` - so `pending_jobs` mostly reflects work
  pushed while no consumer for that stream is actively polling, not "any
  envelope nobody has explicitly popped yet".

## Worker restart signal

`php artisan queue:restart` translates to:

```rust
Queue::restart().await?;
```

The signal lives in `Cache` as a millisecond timestamp. Workers poll
once per loop and exit cleanly when the timestamp is newer than their
start time. Pair with a supervisor (systemd, Kubernetes, the
`supervisor` module) so a fresh worker picks up where the previous one
stopped.

## Pausing queues

`php artisan queue:pause` / `queue:resume` translate to:

```rust
Queue::pause(&connection, "billing").await?;
Queue::resume(&connection, "billing").await?;
Queue::pause_all().await?;
Queue::resume_all().await?;
```

or from the CLI:

```bash
./app queue:pause billing
./app queue:pause --all
./app queue:resume billing
./app queue:resume --all      # alias: queue:continue
```

A paused worker finishes whatever it already popped - pausing never
interrupts a job in flight - then stops claiming new work until resumed.
`pause_all` / `resume_all` are the global switch; pausing (or resuming) a
named queue only affects that queue. **`resume_all` does not clear a
per-queue pause** - a queue paused individually stays paused after a
global resume, matching Laravel. Clear it explicitly with
`Queue::resume(&connection, "billing")`.

A paused worker also says so. `queue:work` prints one line per transition:

```text
  2026-08-25 14:03:11 Queue billing PAUSED
  2026-08-25 14:07:44 Queue billing RESUMED
```

A worker started without `--queue` has no queue names to report, so a global
pause prints `All queues PAUSED` instead. Both lines come from the
`WorkerQueuePaused` / `WorkerQueueResumed` events, so you can listen for them
yourself and route them wherever your alerting lives.

Both signals live in `Cache`, next to the restart signal above:

| Key | Meaning |
| --- | --- |
| `suprnova:queues:paused` | global switch, set by `pause_all` |
| `suprnova:queue:paused:{connection}:{queue}` | one queue's switch, set by `pause` |

Check state with `Queue::is_paused(&connection, "billing").await?` (true if
either key is set) or `Queue::paused_queues(&connection, &queues).await?`
(which of `queues` are currently paused).

### Per-queue pausing needs a named `--queue`

A worker started with `--queue=billing,exports` only claims from those two
queues, so pausing `billing` narrows that list to `exports` for as long as
the pause holds. A worker started with no `--queue` at all drains every
queue the driver holds, and there is no way to ask "pause just `billing`"
against that - `QueueDriver::pop_from` never reports which queue names
exist, so there's nothing to check a per-queue pause key against.
`pause_all` still stops an unfiltered worker completely; a named
per-queue pause only takes effect once you also name that worker's
queues.

### Disabling pause polling

Set `QUEUE_PAUSABLE=false` and every worker in that process ignores pause
signals entirely, at no extra cache-read cost per loop. `queue:pause` (not
`queue:resume`) also refuses to run and exits non-zero, so an operator who
disabled pausing finds out immediately rather than issuing a pause that
quietly does nothing. Mirrors Laravel's `Worker::$pausable`.

### Why Suprnova diverges

An unreachable cache fails **open**: a worker that can't read the pause
keys behaves as "not paused" and keeps draining - the same fail-open
contract the worker restart signal above already uses. A transient cache
outage should degrade a worker fleet to "ignoring pause," never to "every
worker silently freezes" - the pause state is an explicit opt-in signal,
and its own unavailability should not become a hidden kill switch.

## Graceful shutdown

The worker's `CancellationToken` fires at the next pop boundary, never
mid-dispatch. A handler that's already been popped runs to completion
(bounded by its own `Job::timeout()` if set) before the worker exits.
That means in-flight side effects don't get torn mid-stride, but a
SIGTERM can take up to the per-job timeout to drain. Set
`WorkerConfig::max_jobs` for a periodic-restart strategy on long-lived
workers; the worker exits cleanly after that many settlements regardless
of outcome.

## Settlement metrics

The worker emits a `queue.settlement.failures` counter via [`Metrics`](observability.md) on every ack/nack failure. Attributes: `operation`
(`"ack"` | `"nack"`), `driver` (the configured driver's name), `job`
(the job_name), `outcome` (`"success"`, `"dead_letter"`, `"retry"`,
`"deleted"`, `"timeout_dead_letter"`, `"timeout_retry"`, `"released"`).

A non-zero rate here means at-least-once delivery may re-deliver a
successful side effect or lose attempt accounting - alert on it
explicitly.

## Typed errors

`MaxAttemptsExceeded`, `TimeoutExceeded`, and `ManuallyFailed` mirror
Laravel's `MaxAttemptsExceededException` / `TimeoutExceededException` /
`ManuallyFailedException`. The worker attaches the relevant cause to
the dead-letter `JobFailed` event so listeners can pattern-match instead
of substring-searching the error message.

## Connection naming

Workers tag every lifecycle event with a connection name. By default
this is the driver's `name()` (e.g. `"memory"`, `"redis"`, `"database"`).
Apps that run multiple connections at once can override:

```rust
Queue::set_connection_name("orders-redis");
```

## Testing

`Queue::fake()` semantics live in `queue::testing`:

```rust
let _guard = suprnova::queue::testing::install_fake();
my_code_that_dispatches_jobs().await;

suprnova::queue::testing::assert_pushed::<SendWelcomeEmail>(|j| j.user_id == 42);

// For delayed dispatches, pin the scheduled timestamp:
suprnova::queue::testing::assert_pushed_later::<SendWelcomeEmail>(|j, at| {
    j.user_id == 42 && at > chrono::Utc::now()
});
```

The fake guard serialises parallel tests via a process-wide mutex; it
captures `(payload, available_at, overrides)` per push and clears on
`Drop`. The `overrides` field is `EnvelopeOverrides::default()` for
every entry point except `push_with`/`later_with` - see
[Mocking](mocking.md#queue---queuetestinginstall_fake) for
`assert_pushed_on_queue`/`assert_pushed_on_connection` and
`pushed_with_overrides`, the assertions over it. In fake mode,
`push_unique` always records the push as fresh - dedupe is irrelevant
when no driver is wired.

A debounced push behaves the same way: the fake writes nothing to the
cache, so no window is armed and the recorded `available_at` carries no
debounce delay. `assert_pushed_later` sees it as undelayed. What the
fake does still catch is a job declaring both `debounce_for` and
`unique_id` - that pair cannot hold whatever the environment is, so the
push returns an error under `Queue::fake()` exactly as it would in
production.

## Idempotency is the contract between the worker and you

Redis-backed queue drivers can't make `nack` atomic - `XADD` and `XACK`
are separate commands. A crash between them re-delivers the message via
`XAUTOCLAIM`. In-memory and database drivers are exactly-once-per-attempt,
but the worker loop doesn't distinguish drivers, so **every job handler
in a production deployment must be idempotent**.

For typical command-style jobs, wrap the handler body in
[`Idempotency::once`](idempotency.md) or
[`Idempotency::commit_on_success`](idempotency.md) keyed by a stable
per-operation key (entity id, caller-supplied request id, etc.). When a
retry must return the *original* outcome rather than skip re-execution,
use `Idempotency::remember`, which records the success value and
replays it on later deliveries.

## Next

- [Bus](bus.md) - synchronous dispatcher with typed results
- [Events](events.md) - pub/sub fan-out
- [Idempotency](idempotency.md) - the contract handlers honour for at-least-once delivery
- [Cache](cache.md) - backs `push_unique`, `WithoutOverlapping`, `RateLimited`
- [Mocking](mocking.md) - every fake guard, including `Queue::fake`
