# Queue

The `Queue` facade dispatches background work to a driver and lets a separate
worker process drain it: HTTP handlers return fast, the heavy lifting runs
behind the scenes. Reach for it whenever a request would otherwise block on
something that can be done later — sending mail, hitting a webhook, generating
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
};
let shutdown = CancellationToken::new();
run_worker(driver, cfg, shutdown).await;
```

In a scaffolded app, the worker is started by the binary's `queue:work`
subcommand — `cargo run -- queue:work` — which runs the same bootstrap your
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
server boot path calls one of these for you — most apps only configure via
env.

### Environment configuration

```bash
QUEUE_DRIVER=redis
QUEUE_REDIS_URL=redis://127.0.0.1:6379
QUEUE_REDIS_STREAM=suprnova-queue
QUEUE_REDIS_GROUP=default
QUEUE_REDIS_CONSUMER=consumer-1
QUEUE_VISIBILITY_TIMEOUT_SECS=60

# Database driver — DB::init() must run first
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
process" hard to model otherwise. Tokio doesn't — explicit `Bus::dispatch`
vs `Queue::push` is clearer, faster, and surfaces the durability choice
at the call site. See [`bus.md`](bus.md) for the side-by-side.

## Push variants

Every push variant takes a typed `J: Job` value and returns when the
envelope is committed to the driver — not when the handler runs.

| Method | Behavior |
| --- | --- |
| `Queue::push(job)` | enqueue immediately |
| `Queue::push_later(job, at)` | available at a specific `DateTime<Utc>` |
| `Queue::later(delay, job)` | available after `delay` from now |
| `Queue::push_unique(job)` | dedupe by `J::unique_id` within `J::unique_for`, returns `Ok(true)` for fresh, `Ok(false)` for duplicate |
| `Queue::push_unique_later(job, at)` | unique + scheduled |
| `Queue::later_unique(delay, job)` | unique + delayed |
| `Queue::bulk(vec![job1, job2, ...])` | push every job (driver may use a native bulk path) |

`push_unique` requires the cache layer to be bootstrapped — the dedupe
lock lives in [`Cache`](cache.md) via
[`Idempotency::commit_on_success`](idempotency.md). A failed push releases
the dedupe key so the caller can retry; a successful push holds it for
`J::unique_for` seconds. The job must override `Job::unique_id(&self)` to
return `Some(id)` — `None` returns an internal error.

## Job configuration

Override `Job`'s associated functions to tune behavior per impl:

```rust
use std::time::Duration;
use suprnova::queue::{BackoffSchedule, JobMiddleware};

#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str { "SendWelcomeEmail" }

    async fn handle(self) -> Result<(), FrameworkError> { /* … */ Ok(()) }

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

1. a route registered with `Queue::route`
2. the job's own `Job::queue` / `Job::connection`
3. the driver / global default

Passing `None` for a field leaves that dimension alone, so routing a job's
connection does not disturb the queue it already declared.

The two dimensions run at different depths today. The **queue** is honored end
to end — stamped on the envelope, stored by the driver, filtered by `--queue`.
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

### Why Suprnova diverges

Laravel's `Queue::route(...)` takes a class string; Suprnova takes the job as a
type parameter, so a renamed or deleted job is a compile error rather than a
route that silently stops matching.

The larger divergence is what happens when a driver can't filter.
`QueueDriver::pop_from` **rejects** a queue filter it cannot honor instead of
falling back to draining everything. A worker told to drain only `billing` that
quietly drains all queues looks identical to a working deployment until the
wrong pool consumes the wrong jobs — so the misconfiguration is made loud at
the first poll. The memory and database drivers filter natively; a driver that
doesn't — the Redis driver is one, since a single stream consumer group has no
per-queue storage — will error rather than mislead.

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
it. Run the migration first, then roll binaries — older binaries list their
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

The default is `Exponential { base_secs: 2, cap_secs: 300, jitter_ratio: 0.25 }`
— 2 seconds to 5 minutes with ±25% jitter.

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
run — the cache backend blipped, the connection dropped — it logs at
`warn` and returns the handler's own outcome anyway. The lock then
lapses at `expire_after`.

That is deliberate. By the time the release runs, the handler has
already committed its side effects: rows written, mail sent, charges
made. Reporting the release failure as a job failure would make the
worker retry and do all of it a second time, which is a worse outcome
than a lock key held for its TTL. A handler that genuinely failed still
reports its failure — suppressing the release error does not suppress
the handler's.

### The release-without-burning-attempt contract

Middleware returns a `JobOutcome` rather than `Result<()>`. Four variants:

- `JobOutcome::Completed` — handler ran, ack.
- `JobOutcome::Released { delay }` — re-enqueue after `delay` **without**
  incrementing `attempts`. Used by `WithoutOverlapping`, `RateLimited`. The
  worker hands the whole operation to `QueueDriver::release`, and every
  in-tree driver requeues its own stored copy in place, so the message is
  never simultaneously reserved and visible, and never neither. The
  attempt count is preserved with no arithmetic in the worker for a driver
  to disagree with — the stored copy was never bumped for this run.
- `JobOutcome::Failed { reason }` — dead-letter now, persist to the
  failed-jobs store, do not retry.
- `JobOutcome::Deleted` — drop the reservation without dead-letter. Used
  by `Skip`. If the job belonged to a batch, the batch's `pending_jobs`
  decrements anyway so callbacks can fire.

This contract is what makes "throttled because the bucket was full" feel
different from "failed because the handler errored" in retry accounting,
metrics, and lifecycle events.

### What counts as an attempt

Two ways a job leaves a worker without finishing, and both consume an
attempt:

- **The handler failed** — returned `Err`, or panicked into the
  framework's boundary. The worker nacks; the driver requeues with
  `attempts + 1`.
- **The worker died** — OOM kill, `abort()`, a segfault, `docker kill`, or
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

`JobOutcome::Released` is the deliberate exception — see the contract
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
are already spent — it dead-letters it instead, before it takes another
worker down. Without this, counting the attempt would only make a number
climb while the job kept cycling.

**What this means for you.** `attempts` counts *deliveries to a worker*,
not *handler failures*. A worker lost for reasons unrelated to the job — a
host reboot, an OOM caused by a noisy neighbour — burns an attempt from
that job's budget too. Laravel behaves the same way. Size `max_tries` with
that in mind, and prefer idempotent handlers: at-least-once delivery was
always the contract, and this makes the redelivery path count honestly
rather than silently.

## Lifecycle events

Workers emit Laravel-shape lifecycle events through the
[`Event`](events.md) facade. Listeners get the envelope's identity (`id`,
`job_name`, `attempts`, `max_tries`, `connection`), not the typed job
instance — the worker is type-erased over JSON payloads. Errors travel
as a `String` since `FrameworkError` doesn't derive `Clone`.

| Event | Fires when |
| --- | --- |
| `JobQueueing` | before the envelope hits the driver |
| `JobQueued` | after the driver accepts |
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

Subscribe with the normal `Event::listen` API. Events are best-effort —
`Event::dispatch` with no listeners is a no-op `Ok(())`, so workers in
deployments without `Event::init()` pay nothing.

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

- `MemoryFailedJobStore` — in-process `Vec`, lost on restart.
- `DatabaseFailedJobStore` — persists to a `failed_jobs` table via SeaORM.
- `NullFailedJobStore` — discards every record. Mirrors Laravel's
  `NullFailedJobProvider`.

### When the store rejects a record

If the configured store returns an error, the worker logs at `error` and
**leaves the reservation intact** rather than acking. The job returns on
visibility expiry and is retried — it is not silently dropped.

That is deliberate. The alternative, acking anyway, discards a job that
already exhausted its attempts *and* failed to be recorded anywhere, which
is unrecoverable. A job that keeps coming back is recoverable: fix the
store and the next delivery lands.

The practical case is a `DatabaseFailedJobStore` pointed at an unmigrated
`failed_jobs` table. Until you migrate, dead-lettering jobs cycle at one
redelivery per visibility timeout, each logging the store's error. If you
genuinely want failures discarded, configure `NullFailedJobStore` — that
succeeds, so the job acks and is gone.

### Retrying

```rust
use uuid::Uuid;

// Single record — false if the id wasn't in the store.
Queue::retry_failed(some_id).await?;

// Bulk — optional cutoff (only retry records older than `before`).
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

Two tables, which the framework does not create — add them to your
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
derived from the settlement rows on every read —

```text
pending_jobs = max(0, total_jobs - COUNT(settlements))
failed_jobs  = COUNT(settlements WHERE failed)
```

— because queues are at-least-once, so the same job settles more than once
whenever a redelivery happens, an ack is duplicated, or a worker dies
between doing the work and recording it. A counter decremented per
settlement drifts on every one of those, and the drift is not cosmetic:
`pending_jobs` gates the callbacks, so an early zero fires `then` while
other jobs in the batch are still running. With the counts derived and the
primary key on `(batch_id, job_id)`, a repeat settlement inserts nothing and
there is no counter to get wrong — across processes, not just within one.

### When a dispatch fails halfway

If a `driver.push` fails partway through `dispatch()`, the jobs that
already reached the queue are real and already stamped with the batch id.
So the batch is settled rather than removed: every envelope that was *not*
pushed is recorded as a failed job, and the batch is cancelled.

`total_jobs` still counts what you asked for, `failed_job_ids` names
exactly the jobs that never made it, the ones already queued settle
normally, and `SkipIfBatchCancelled` drops the rest — so `pending_jobs`
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
            None    => format!("Batch {} done — {} jobs", batch.name, batch.total_jobs),
        };
        // … send mail
        Ok(())
    }
}
```

Register at boot with `batch::register_callback(Arc::new(SendSummary))`.
Callbacks are keyed by `name()` — the batch's options store callback
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
chain — subsequent links are never enqueued.

### Terminal settlement

Finishing a chained job means two things: enqueue the successor, and
release the job just finished. As two separate operations there is no safe
order. Ack first, and a crash in the gap loses the rest of the chain
permanently — nothing is left in the queue to retry from. Push first, and
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
`Stale` — having enqueued nothing. Two-step settlement cannot express that
at all: your push succeeds, the new owner's push succeeds, and the chain
forks.

Redis and the in-memory driver answer `Unsupported` and keep the
push-before-ack ordering, which trades permanent loss for an at-least-once
duplicate. That is the framework's documented contract, and it is why
chained envelope ids are derived from their predecessor rather than random
— a redelivered step re-pushes the id it pushed before, so the duplicate is
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
`reserved_size` / `delayed_size` / `clear`; `MemoryQueueDriver` and
`DatabaseQueueDriver` implement them natively. `RedisQueueDriver`
returns an "unsupported" error for `size` / `clear` — use the admin
redis-cli for those.

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
successful side effect or lose attempt accounting — alert on it
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
captures `(payload, available_at)` per push and clears on `Drop`. In
fake mode, `push_unique` always records the push as fresh — dedupe is
irrelevant when no driver is wired.

## Idempotency is the worker's contract with you

Redis-backed queue drivers can't make `nack` atomic — `XADD` and `XACK`
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

- [Bus](bus.md) — synchronous dispatcher with typed results
- [Events](events.md) — pub/sub fan-out
- [Idempotency](idempotency.md) — the contract handlers honour for at-least-once delivery
- [Cache](cache.md) — backs `push_unique`, `WithoutOverlapping`, `RateLimited`
- [Mocking](mocking.md) — every fake guard, including `Queue::fake`
