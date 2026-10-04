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

Register the jobs you write. The framework's own jobs - `SendMailJob` behind `Mail::queue` and `SendNotificationJob` behind `Notify::queue` - are registered before your code runs.

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
fire identically for inserts from a queue handler. `queue:work --connection
<name>` drains a named connection instead of the default one - see
[Connections](#connections).

## Drivers

Six drivers ship in-tree. Configure via `QUEUE_DRIVER` env or by calling
`Queue::set_driver(...)` programmatically.

| Driver | Use for | Strengths |
| --- | --- | --- |
| `MemoryQueueDriver` | tests, single-process apps | `tokio::time::DelayQueue` for `available_at`, virtual-clock compatible |
| `RedisQueueDriver` | production fan-out | consumer groups + `XAUTOCLAIM` + ZSET-backed delayed jobs |
| `DatabaseQueueDriver` | single-DB apps | `FOR UPDATE SKIP LOCKED` on Postgres/MySQL, `BEGIN`-serialised on SQLite |
| `SqsQueueDriver` | production on AWS | a managed queue outside your database and Redis; see [Amazon SQS](#amazon-sqs) |
| `SyncQueueDriver` | dev, CI | runs the handler inline on `push`, no worker |
| `NullQueueDriver` | testing wrappers | drops every push without running |

`suprnova::queue::bootstrap_from_env()` reads `QUEUE_DRIVER` and wires the matching
driver; `suprnova::queue::bootstrap_default()` always wires the memory driver. The
server boot path calls one of these for you - most apps only configure via
env.

`QUEUE_DRIVER` accepts `memory`, `sync`, `null`, `redis`, `database`, `sqs`
and `failover`, and defaults to `memory`. `sync` selects `SyncQueueDriver` and
`null` selects `NullQueueDriver`. A value that names no driver is a boot
error when `APP_ENV` is `production`, because an in-memory queue chosen by
mistake loses every job at the next restart. In any other environment the
boot logs a warning that lists the accepted names and uses the memory
driver.

`FailoverQueueDriver` isn't a seventh backend. It wraps an ordered list of
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
composition. Redis uses Streams consumer groups directly (`XREADGROUP`
for new work, `XAUTOCLAIM` to reclaim unacknowledged entries, Redis 6.2 or
newer). The visibility timeout is the `XAUTOCLAIM` idle threshold, set once
per connection, so the per-pop `visibility_timeout` argument is ignored on
Redis (a documented divergence from the trait contract imposed by Redis
Streams).

### Amazon SQS

`QUEUE_DRIVER=sqs` queues jobs on Amazon SQS standard queues. It reads the
variables Laravel's `sqs` connection reads:

```bash
QUEUE_DRIVER=sqs
SQS_PREFIX=https://sqs.us-east-1.amazonaws.com/123456789012
SQS_QUEUE=default          # the queue a job that names none goes to
SQS_SUFFIX=-production     # optional, appended to every queue name
AWS_DEFAULT_REGION=us-east-1
AWS_ACCESS_KEY_ID=...      # optional, see below
AWS_SECRET_ACCESS_KEY=...
```

A job goes to the queue it names, with `Job::queue` or a route, at
`SQS_PREFIX/emails-production` for `emails`, and to `SQS_QUEUE` when it
names none. A
queue name that is already a URL is used as it is, so `SQS_QUEUE` can be the
full URL of the queue and `SQS_PREFIX` left unset. `SQS_ENDPOINT` points the
driver at another endpoint: a service that is not AWS, such as LocalStack or
ElasticMQ, or a VPC endpoint. Without it the driver uses the SQS endpoint of
the region, in the China partition (`amazonaws.com.cn`) for a `cn-` region.

With `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` set (and
`AWS_SESSION_TOKEN` for temporary keys), the driver signs with them.
Without them it uses the default credential chain of AWS: the shared
profile, web identity, the ECS task role and the EC2 instance role.

The server does not boot when no region is set, when the queue is not a URL
and `SQS_PREFIX` is not set, or when the queue is a FIFO queue (its name
ends in `.fifo`): a FIFO queue needs a message group and a deduplication ID
on each job, which Suprnova jobs do not carry.

How the driver maps onto SQS:

- **A worker receives from the queues `--queue` names**, in order, and from
  `SQS_QUEUE` when it names none. SQS has no receive across queues, so a
  job sent to `emails` waits for a worker started with `--queue=emails`. A
  name there is an SQS queue name, as in Laravel: `--queue=default` receives
  from `SQS_PREFIX/default`, which holds the unrouted jobs only when
  `SQS_QUEUE` is `default`. Name `SQS_QUEUE` itself to drain them.
- **A receive waits up to `SQS_WAIT_TIME_SECONDS` for a message** (default
  1, at most 20). The wait keeps an idle worker to about one request a
  second for each queue it names, where SQS bills each request, and it makes
  SQS answer from all of its servers, so an empty answer means an empty
  queue. Setting it to `0` sends short polls, about ten a second.
- **The visibility timeout is the worker's.** A reservation that expires
  makes the job visible again, and its next delivery counts one more
  attempt.
- **A `nack` counts an attempt and a release does not**, as on the other
  drivers. A `nack` whose delay is longer than SQS can still hide the
  message, 12 hours from its receive, sends a copy that counts the attempt. The attempt count is the one in the job plus the receives SQS
  counted before this one (`ApproximateReceiveCount`), so a release sends a
  fresh copy that keeps the count and deletes the original.
- **Delays longer than 15 minutes hold.** SQS delays one message by 15
  minutes at most, so a job due later is sent on with what is left when it
  arrives early. Waiting out the delay is not an attempt.
- **`Queue::size` and its siblings** report the approximate counts SQS
  keeps for `SQS_QUEUE`, and `clear` purges it. SQS cannot list messages
  without receiving them, so `pending_jobs`, `delayed_jobs` and
  `reserved_jobs` return an error.

A release, a long `nack` and a delay sent on are each a send followed by a
delete, because SQS cannot change a message. If the delete fails after the
send, the job is on the queue twice until the original's visibility runs out:
the at-least-once delivery the queue documents. A request SQS throttles or
fails with a fault of its own is tried up to three times.

To build the driver in code, for a second connection in another region or
account, fill an `SqsConfig` and register the driver under a name:

```rust,ignore
use suprnova::{Queue, SqsConfig, SqsQueueDriver};
use std::sync::Arc;

let mut config = SqsConfig::new("eu-west-1", "reports");
config.prefix = Some("https://sqs.eu-west-1.amazonaws.com/123456789012".into());
Queue::register_connection("reports", Arc::new(SqsQueueDriver::new(config)?));
```

`SqsQueueDriver::call(action, body)` sends any other SQS action, signed as
the driver's own requests are, where Laravel hands out the AWS client with
`getSqs()`.

SQS takes at most 1 MiB in one message, and a larger job fails its push
with an error that says so. Turn on overflow to store large jobs on a disk
instead. SQS then carries a pointer to the file:

```bash
SQS_OVERFLOW_ENABLED=true
SQS_OVERFLOW_DISK=s3                      # optional; the default disk otherwise
SQS_OVERFLOW_ALWAYS=false                 # true stores every job on the disk
SQS_OVERFLOW_DELETE_AFTER_PROCESSING=true # delete the file when the job is done
SQS_OVERFLOW_FLUSH_ON_CLEAR=false         # true deletes the files on Queue::clear
```

The files go under `sqs-payloads/<queue>-<digest>/` on the disk, where the
digest is 16 hexadecimal digits of the SHA-256 of the full queue URL, so
two queues with one name in different accounts or regions never share a
directory and `SQS_OVERFLOW_FLUSH_ON_CLEAR` deletes only the cleared queue's
files. The server does not boot when overflow is on and the disk is not
registered. A file is deleted only when the driver knows no message points
at it any more, so a send that times out, a send that is refused after an
earlier try timed out or met a fault of the service (5xx), or a settlement
after its reservation expired, leaves the file on the disk rather than risk
a message that can no longer be read. A retry after a send that got no
answer, or met a fault of the service, carries its own copy of the file: if
SQS took both tries, each of the two messages owns a file, and the job's
duplicate stays readable after the first one is acknowledged.

`SqsQueueDriver` is behind the `queue-sqs` cargo feature, which is on by
default and brings `filesystem` with it.

#### Why Suprnova diverges

- **A release does not cost an attempt.** Laravel's SQS job counts every
  receive as an attempt, so releasing a job there spends one. Suprnova
  keeps the rule every other driver follows.
- **Long delays work.** Laravel passes SQS a delay over 900 seconds, which
  SQS refuses.
- **Overflow goes to a disk, not a cache store.** A cache can evict a
  payload before its job runs; a disk keeps it until the job is done.
- **No default region or prefix.** Laravel's configuration falls back to
  `us-east-1` and a placeholder account URL; a missing setting here stops
  the boot instead.

### Why Suprnova diverges

Laravel routes every queueable through the Bus, distinguishing
`ShouldQueue` jobs at dispatch time. Suprnova splits the two: `Bus` for
synchronous work that returns a typed result, `Queue` for asynchronous
work that survives a process crash. PHP needs the implicit routing
because its request-per-process model makes "do this later, in another
process" hard to model otherwise. Tokio doesn't - explicit `Bus::dispatch`
vs `Queue::push` is clearer, faster, and surfaces the durability choice
at the call site. See [`bus.md`](bus.md) for the side-by-side.

## Connections

A connection is a name bound to a driver. The driver you install with
`Queue::set_driver`, or that `QUEUE_DRIVER` selects, is the **default
connection**. Its name is `Queue::connection_name()`: the name you gave
`Queue::set_connection_name`, or else the driver's own name. Every other
connection you register by name:

```rust
use std::sync::Arc;
use suprnova::queue::{MemoryQueueDriver, Queue};

// bootstrap::register()
Queue::register_connection("reports", Arc::new(MemoryQueueDriver::new()));
```

A push goes to the connection it resolves to. The first of these that names
one wins:

1. `EnvelopeOverrides::connection` on `Queue::push_with` or
   `Queue::later_with`
2. a route registered with [`Queue::route`](#queue-routing)
3. the job's own `Job::connection()`
4. the default connection

`Queue::push`, `push_with`, `later`, `bulk` and `push_unique` all resolve it.
The jobs of a [batch](#queued-batches) each go to their own connection. A
failed job goes back to the connection it failed on when you retry it.

```rust
impl Job for BuildReport {
    fn job_name() -> &'static str { "BuildReport" }
    fn connection() -> Option<&'static str> { Some("reports") }
    // ...
}
```

### What a connection name means

While you register no connection, the process has one driver and every push
reaches it. A connection name is then a label on the lifecycle events and
selects nothing, so a job can name a connection before you configure it.

Once you register one connection, a name selects a driver. It is either a
registered connection or the default connection by its own name. A push to
any other name returns an error and pushes nothing. A push that waits for a
commit returns the error before the commit, while you can still abandon the
transaction.

`Queue::connection(name)` returns the driver of one connection, and
`Queue::connection_names()` lists the registered names. `Queue::size()` and
the other counts and listings read the default connection. Read another
connection with `Queue::connection("reports")?.size().await`.

### Register connections from the environment

`QUEUE_CONNECTIONS` registers one connection per entry, next to the default
one:

```bash
QUEUE_DRIVER=memory
QUEUE_CONNECTIONS=redis,database
```

Each connection is named for its driver and reads that driver's own
variables, as it would if it were `QUEUE_DRIVER`. An entry takes the same
names as `QUEUE_DRIVER`. An unknown name is a boot error in every
environment.

One queue has one connection. An entry that names the driver `QUEUE_DRIVER`
selects is a second name for the default connection, so a pause or a
`Queue::forward_on` that you set under either name reaches the whole queue.
An entry that would put a second connection over a Redis stream or a jobs
table that another connection already uses is refused at boot.

### Workers, pauses and chains

Start a worker for one connection with `--connection`, and pause a queue on
it the same way:

```bash
./app queue:work --connection=reports
./app queue:pause billing --connection=reports
./app queue:resume billing --connection=reports
```

Without `--connection` each command acts on the default connection. A
`--connection` that names no connection stops `queue:work` and
`queue:pause` before they do anything. In code, `queue::worker::run_worker_on`
does what `queue:work --connection` does:

```rust
use suprnova::queue::worker::{WorkerConfig, run_worker_on};
use tokio_util::sync::CancellationToken;

let shutdown = CancellationToken::new();
run_worker_on("reports", WorkerConfig::default(), shutdown).await?;
```

The worker carries the name of its connection on its lifecycle events and on
the failed-job records it writes. A pause and a `Queue::forward_on` gate on
that name, so the push and the worker's claim agree on it.

A chain runs on the connection of its first job, because the worker enqueues
the next link in the step that settles the one before it. Dispatching a
chain whose links resolve to different connections returns an error and
pushes nothing.

### Why Suprnova diverges

Laravel fails on a connection name that `config/queue.php` does not define.
Suprnova accepts any name while you register no connection, because
applications with one driver often name a connection on their jobs to label
their events. The first `Queue::register_connection` turns names into
selectors, and from then on an unknown name is an error rather than a push
to the default connection, where no worker of the intended connection would
run it.

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

### A worker drains every connection

A push walks the list and stops at the first connection that accepts it. A
read covers the whole list:

- `pop` and `pop_from` begin at a different connection on each call and
  then try every connection in order, one after the other. A primary that
  is busy again after an outage cannot starve the jobs that went to a
  fallback during it.
- `ack`, `nack`, `release` and `settle` go to the connection that issued
  the reservation. A reservation token means something to the driver that
  issued it and to no other, and two backends can issue the same token.
  The failover driver gives each reservation a token of its own and keeps
  the origin until the visibility timeout ends. A token that it does not
  know, or whose timeout is over, is treated as stale and is sent to no
  connection.
- The four counters and the three inspection listings add up every
  connection, in the order of the list. `clear` clears every connection.

So one worker on the failover connection runs the jobs of the primary and
the jobs that went to a fallback:

```bash
QUEUE_DRIVER=failover QUEUE_FAILOVER_CONNECTIONS=redis,database ./app queue:work
```

A worker with `--queue` needs every connection of the list to support
queue filtering. When one connection does not, `pop_from` returns an error
that names it, and the worker claims nothing.

A [queued chain](#queued-chains) stays on the connection of the link that
ran. The worker settles a job and enqueues the next link in one call,
`settle`, and that call goes to the connection that issued the
reservation. The database driver and the Redis driver settle in one step,
so the next link is written to that connection with the acknowledgement.
When that connection is down, the settle fails and nothing falls over: the
worker leaves the reservation as it is and the visibility timeout
redelivers the job. A driver that cannot settle in one step, such as the
memory driver, answers `Settled::Unsupported`. The worker then pushes the
next link through the failover driver like any other push, so the link
goes to the first connection that accepts it.

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
`config/queue.php`, resolved through the connection registry. Suprnova's
failover driver takes the drivers themselves rather than names in the
[connection registry](#connections), so the labels come from
`QUEUE_FAILOVER_CONNECTIONS` (or from the `String` you pass to
`FailoverQueueDriver::new`) and reads go to the drivers of the list rather
than to named connections. The failover driver is one driver, and it is
the default connection or a registered connection like any other.

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
| `after_commit` | `Job::after_commit()`, `QUEUE_AFTER_COMMIT` |

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

A chain honors it: every link of `Queue::chain()...add(job)?` records the
job's `Job::delay()` when you add it. The head becomes available that long
after `dispatch()`, and each later link becomes available that long after
the link before it completes. See [Queued chains](#queued-chains).

A batch doesn't consult it: `Queue::batch()...add(job)` builds its envelopes
with `available_at` set to the moment you called `add`, so a job with a
declared `Job::delay()` dispatches immediately as part of a batch even though
a bare `Queue::push(job)` of the same job would wait. Give the job an
explicit delay some other way - a field on the job itself, applied in
`handle()` - if a batched step needs one.

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
`push_unique*` family takes no `EnvelopeOverrides`, so `Job::after_commit()` and
`QUEUE_AFTER_COMMIT` are the only things that decide whether a unique push
defers - there is no per-push override for it.

Batches and chains do not defer: `Queue::batch()` and `Queue::chain()` build
and push their envelopes directly. Wrap the `.dispatch()` call so it runs
after the transaction returns if a batch or a chain has to wait for a
commit.

Queued [mail](mail.md#queueing) and [notifications](notifications.md) defer
when the message asks. Each rides a single shared job type (`SendMailJob` /
`SendNotificationJob`), so the job's own `Job::after_commit()` cannot answer
for one message. Instead `Mailable::after_commit(&self)` and
`Notification::after_commit(&self)` return `false` by default. When one
returns `true`, `Mail::queue`, `Mail::later` and `Notify::queue` inside
`DB::transaction` push at the commit, and a rollback discards the push.
Outside a transaction they push at once. A message that returns `false`
keeps the default, so the per-push and process-wide settings below still
apply to it.

To defer every job push, queued mail and queued notification in the process,
whatever the job or the message declares, set `QUEUE_AFTER_COMMIT=true` (`1`
works too). Batches and chains still push at once. Suprnova reads the
variable at each push. It is the `after_commit` option of a Laravel queue
connection.
`Job::after_commit()` answers `false` both for a job that never chose and
for one that chose `false`, so a job cannot turn the process-wide setting
off. One push can go ahead of the commit with `EnvelopeOverrides {
after_commit: Some(false), .. }`.

Under `Queue::fake()` a push is recorded immediately, deferral and all, so a
test can assert on it without committing anything. This matches Laravel's
`Bus::fake`, and it is what lets a test drive one transactional handler and
assert its dispatches in the same breath.

### Why Suprnova diverges

`Queue::bulk` is monomorphic - every element shares one concrete `J` - so its
after-commit partition is all or nothing for the call. Laravel partitions a
heterogeneous array into deferred and immediate halves; there is nothing here
to partition.

Ambient deferral follows the closure form. A push inside a manual
[`DB::begin_transaction`](database.md#manual-form) that doesn't name the
handle happens **immediately**, because manual mode installs no ambient
transaction: code that doesn't name the handle runs outside it. To wait for
that transaction, push with `Queue::push_after_commit_with_tx(&tx, job)`,
which defers to `tx.commit()` and is discarded by a rollback or by dropping
the handle uncommitted. See
[After-commit callbacks](database.md#after-commit-callbacks).

Laravel also reads a connection-level `after_commit` config key as the last
fallback in its precedence chain. Suprnova reads one process-wide switch,
`QUEUE_AFTER_COMMIT`, in that place: the order is the per-push override, then
the job's own `Job::after_commit()` or the switch. Queue connections here do
not carry their own dispatch policy, so the switch applies to every
connection.

### Raw pushes

`Queue::push_raw(payload, queue)` pushes a payload that is already an
envelope, in the JSON form `Envelope::to_json` writes, onto the default
connection. It is Laravel's `Queue::pushRaw`: use it for a payload you did
not build from a typed job, such as one another service produced or one
replayed from an export.

```rust
use suprnova::{FrameworkError, Queue};

async fn replay(lines: Vec<String>) -> Result<(), FrameworkError> {
    for line in lines {
        // Each line is an envelope a producer wrote with `Envelope::to_json`.
        Queue::push_raw(&line, Some("replays")).await?;
    }
    Ok(())
}
```

A worker runs a raw-pushed envelope like any other, with the handler
registered for its `job_name`, so that job type has to be registered in the
process that drains the queue. The envelope goes on the queue as it is, apart
from its queue: `Some(queue)` replaces the envelope's own queue and is
redirected by [`Queue::forward`](#forwarding-a-whole-queue) like a per-push
override, and `None` keeps the queue the envelope carries.

A raw push skips everything a typed push resolves from the job type:
`Queue::route`, `Job::after_commit`, and the `JobQueueing` / `JobQueued`
events, which Laravel's `pushRaw` does not fire either. To push to another
connection, go through its driver: `Queue::connection("reports")?.push(envelope)`
with the decoded `Envelope`. Under `Queue::fake()` the payload is recorded,
see [Testing](#raw-pushes-under-the-fake).

#### Why Suprnova diverges

Laravel stores a raw payload as whatever string it is given, and nothing
checks it until a worker tries to run it. Here the payload has to decode as an
`Envelope` this build can read, and `push_raw` returns an error when it does
not, under `Queue::fake()` too: every driver stores envelopes, so a payload
that is not one could never reach a worker. Laravel's sync driver also drops a
raw push without running it; `SyncQueueDriver` runs it, as it runs every
other envelope.

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
4. the default: the default queue of the driver, and the default connection

Passing `None` for a field leaves that dimension alone, so routing a job's
connection does not disturb the queue it already declared.

Both dimensions are honored end to end. The **queue** is stamped on the
envelope, stored by the driver, and filtered by `--queue`. The
**connection** selects the driver a job is pushed to, as
[Connections](#connections) describes, and its name is carried on the
`JobQueueing` / `JobQueued` lifecycle events that listeners and dashboards
see.

Then dedicate a worker to the queue:

```bash
./app queue:work --queue=billing
./app queue:work --queue=exports,heavy
./app queue:work                       # drains every queue, as before
```

A job with no route belongs to `default`, so `--queue=default` drains
unrouted work rather than stranding it. The `sqs` driver is the exception:
there a name is an SQS queue, and unrouted jobs go to `SQS_QUEUE`. See
[Amazon SQS](#amazon-sqs).

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

The connection in `forward_on` is a gate. It is compared against the name of
the [connection](#connections) the push goes to, which is the name the worker
on that connection was started with (`queue:work --connection`, or the default
connection's name when you pass none). Both halves of the redirect gate on that
one value, so a forward can never move the push without moving the claim. On
any other connection the forward does nothing and the queue name passes
through.

While you register no connection, every push goes to the default connection.
The gate then compares with the default's name - `Queue::set_connection_name`
if you set one, the driver's own name otherwise - whatever connection the job,
a `Queue::route` or a per-push `EnvelopeOverrides` names.

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

The inspection calls are deliberately not forwarded:
`Queue::pending_jobs(Some("default"))` lists what is literally on `default`,
not what is on `high`, which is how you see the backlog stranded on a source
queue you have just forwarded. Laravel resolves the forward there too; see
the divergence note below.

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
onto a different *connection*. Suprnova selects the connection by job, route or
push, and a forward only renames the queue, so `Queue::forward_on(from, to,
connection)` treats the connection as a **gate** - it decides whether the
queue-name redirect applies - and never as a destination. To move a job to
another connection, route the job with `Queue::route`. For the same reason
`to` is required here, while Laravel's is optional: an omitted `to` in
Laravel means "move only the connection", which a forward here does not do,
so a `forward(from, None)` would be a no-op dressed as a configuration
change.

Laravel's inspection calls follow a forward, because `pendingJobs($queue)` and
its siblings run through the same driver-level `getQueue()` the push and the pop
do. Suprnova's `Queue::pending_jobs` / `delayed_jobs` / `reserved_jobs` report
the literal queue you name instead. The literal view is the only way to see the envelopes that stayed behind on a queue you have
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

## Context on queued work

A job runs after the request that queued it, often in another process.
Suprnova carries the [`Context`](context.md) to it. Every push takes a
`ContextSnapshot` of the caller's visible and hidden bags and stores it in
the `context` field of the `Envelope`. The field is `None` when the caller
had nothing to carry.

```rust
use suprnova::Context;

// In a handler:
Context::add("tenant_id", "acme");
Queue::push(SendInvoice { invoice_id: 7 }).await?;

// In SendInvoice::handle, on a worker:
let tenant: Option<String> = Context::get("tenant_id");
```

The worker restores one scope from the snapshot for each attempt. The job,
its middleware and the lifecycle events around it (`JobProcessing`,
`JobProcessed`, `JobFailed` and the rest) all run in that scope, so a
listener reads what the job read and what the job added. The rules:

- The snapshot is taken when you push. A push that waits for a
  [commit](#after-commit-dispatch) carries the snapshot of the code that
  called `push`.
- Every link of a chain and every job of a batch carries the snapshot of the
  code that built it. A queued event listener carries the snapshot of the
  dispatch.
- The job works on a copy. What it adds reaches neither the request nor the
  next job, and every retry starts from the snapshot again.
- The query bag does not travel. A job that runs inline under the `sync`
  driver does not read the request's query parameters.
- A job queued while a request is served carries the request's id as
  `_request_id`.
- Hidden values travel too, because the job needs them. They reach the queue
  store and the failed-job store. They do not reach a log: the envelope a
  worker logs when no failed-job store is bound carries no hidden context.

`Context::dehydrating` and `Context::hydrated` register callbacks for the
two ends of the trip. See [Context](context.md#queued-work).

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

`retry_failed` loads the envelope, resets `attempts`, `available_at`, the
`idempotency_key` and the uniqueness lock owner, pushes it to the
[connection](#connections) it failed on, then deletes the failed-job record.
Mirrors `php artisan queue:retry <id>` plus `queue:flush` semantics (each
retried envelope is pushed AND removed from the store).

`retry_all_failed` keeps a record whose connection is not registered, as it
keeps a record whose envelope does not decode, and logs a warning.
It carries on with the records after it. `retry_failed` returns the error
instead.

### Failed-job commands

The application binary has five commands over the failed-job store. Run them
as you run `queue:work`, with `./app` or `cargo run --bin app --`:

| Command | Effect |
| --- | --- |
| `queue:failed` | list every failed job: id, connection, queue, job, failure time and the first line of the error |
| `queue:retry <id>...` | push the failed jobs with these ids back onto the queue |
| `queue:retry all` | push every failed job back onto the queue |
| `queue:forget <id>` | delete one failed job |
| `queue:flush [--hours N]` | delete every failed job, or only those that failed more than `N` hours ago |
| `queue:prune-failed [--hours N]` | delete the failed jobs older than `N` hours; `N` defaults to `24` |

```bash
./app queue:failed
./app queue:retry 3f2a9c1e-5b7d-4c8e-9a10-2d6e4b8f7a11
./app queue:retry all
./app queue:flush --hours 168
```

`queue:retry` calls `Queue::retry_failed` for each id, or
`Queue::retry_all_failed` for `all`. A command exits non-zero when part of
the request fails: an id that names no failed job, or an id that is not a
UUID. The `suprnova` CLI forwards the same five commands to your
application's binary, so `suprnova queue:failed` from the project directory
does the same as `./app queue:failed`.

The commands run the application's bootstrap, as `queue:work` does, and use
the failed-job store that boot bound. `QUEUE_DRIVER=database` binds one. With
any other driver, call `Queue::set_failed_store(...)` in `bootstrap::register()`.
With no store bound, a command exits non-zero and says so. A retry pushes to
the queue that boot configured, so with the `memory` driver the retried job
goes into the memory of the command's own process, not into the process that runs
your workers.

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
    created_at    BIGINT NOT NULL,
    cancelled_at  BIGINT NULL,
    finished_at   BIGINT NULL
);

CREATE TABLE job_batch_settlements (
    batch_id   TEXT NOT NULL,
    job_id     TEXT NOT NULL,
    failed     INTEGER NOT NULL,
    settled_at BIGINT NOT NULL,
    PRIMARY KEY (batch_id, job_id)
);
```

The epoch columns are `BIGINT` so they outlive 2038. Tables created with
`INTEGER` epoch columns from an earlier version of this schema keep
working: the repository reads every integer column at either width.

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

The first envelope is pushed at dispatch; the rest travel on its
`chain_remaining` payload field. On every successful settlement the
worker pops the next entry and dispatches it. A failure breaks the
chain - subsequent links are never enqueued.

The [`sync` driver](#drivers) has no worker, so it runs the whole chain
inline inside `dispatch()`, link by link. A link whose handler fails returns
its error from `dispatch()`, and the links after it do not run.

Each link applies its job's own `Job::delay()`, as a direct push does. A
delay of 30 seconds on the head makes the head available 30 seconds after
`dispatch()`. A delay on any later link makes that link available that long
after the link before it completes. A chain runs on one connection, the one
its first job resolves to - see [Workers, pauses and chains](#workers-pauses-and-chains).

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
./app queue:pause billing --connection reports
```

A pause belongs to one connection. `queue:pause` and `queue:resume` take
`--connection <name>` and act on the default connection without it. See
[Connections](#connections).

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

Workers tag every lifecycle event with a connection name. A worker on the
default connection uses that connection's name, which is by default the
driver's `name()` (e.g. `"memory"`, `"redis"`, `"database"`). Override it
with:

```rust
Queue::set_connection_name("orders-redis");
```

A worker started with `--connection reports` tags its events, and the
failed-job records it writes, with the name of that connection. See
[Connections](#connections).

## Testing

`Queue::fake()` installs the fake, and the assertions live in `queue::testing`.
`queue::testing::install_fake()` is the same call and returns the same guard:

```rust
let _guard = suprnova::Queue::fake();
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
[Mocking](mocking.md#queue---queuefake) for
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

### Batches, chains and failed-job retries

Every path that would write to the driver records in the fake instead:
`Queue::batch()...dispatch()`, `Queue::chain()...dispatch()`,
`Queue::retry_failed` and `Queue::retry_all_failed`. None of them needs a
driver under the fake, and none writes to one that is installed, unless
[`except`](#letting-some-jobs-through) names the job.

- A batch records as a `FakedBatch` with its `id`, `name` and `jobs`. The
  repository still stores the batch, so the id you receive names a batch.
  No job runs, so the batch stays pending.
- A chain records as a `FakedChain` whose `links` run head first. A chain
  whose links resolve to different connections is refused under the fake, as
  it is in production.
- A retry records the retried envelope as a push. The failed-job record still
  leaves the store.

The jobs of a batch, the head of a chain and every retried job also record as
ordinary pushes, so `assert_pushed` sees them whichever path queued them.
Assert on the batch or the chain itself with these functions of
`queue::testing`:

```rust
use suprnova::queue::testing::{
    assert_batch_count, assert_batched, assert_chained, assert_nothing_chained,
    batched, chained,
};

let _guard = suprnova::Queue::fake();
import_users(vec![1, 2, 3]).await?;

assert_batch_count(1);
assert_batched(|batch| batch.name == "import-users" && batch.jobs.len() == 3);
assert_eq!(batched()[0].jobs_of::<ImportUser>().len(), 3);

assert_chained(&["GenerateReport", "UploadToBucket"]);
let report: Option<GenerateReport> = chained()[0].link::<GenerateReport>(0);
```

`assert_chained` takes the `Job::job_name()` of every link, head first, and
matches a chain made of exactly those jobs. `assert_nothing_batched` and
`assert_nothing_chained` assert the opposite. See
[Mocking](mocking.md#queue---queuefake) for the whole table.

`assert_pushed_without_chain::<J>()` asserts the reverse of a chain: at least
one push of `J` carried no chain. A job pushed on its own passes, and so does
a batch member or a chain of one job. The head of a longer chain carries the
rest of the chain, so it does not:

```rust
use suprnova::queue::testing::assert_pushed_without_chain;

let _guard = suprnova::Queue::fake();
send_receipt(order_id).await?;

// The receipt went out on its own, not as the first step of a chain.
assert_pushed_without_chain::<SendReceipt>();
```

### Letting some jobs through

`Queue::fake_except(&[...])` fakes every job except the ones named, which
reach the real queue as they would without the fake. Each name is a
`Job::job_name()`. `Queue::fake().except(&[...])` is the same call on the
guard, and calling `except` again adds to the names already excepted:

```rust
use std::sync::Arc;
use suprnova::Queue;
use suprnova::queue::SyncQueueDriver;
use suprnova::queue::testing::assert_pushed;

Queue::set_driver(Arc::new(SyncQueueDriver::new()));
let _guard = Queue::fake_except(&["ProvisionAccount"]);

activate_account(42).await?;

// ProvisionAccount ran on the sync driver; the welcome email was recorded.
assert_pushed::<SendWelcomeEmail>(|j| j.user_id == 42);
```

An excepted job takes the real path from end to end: it resolves its
connection, reaches the driver, emits `JobQueueing` and `JobQueued`, and fails
where a real push fails. A batch is still recorded with all of its jobs, and
only its excepted jobs reach the real queue. A chain whose first job is
excepted reaches the real queue, and the worker that runs that job dispatches
each later link through the fake, as Laravel does: a link `except` names
reaches the real queue, and any other link is recorded and does not run. A
raw push is always recorded, whatever `except` names, because a raw payload is
not a job type.

### Raw pushes under the fake

Under the fake, [`Queue::push_raw`](#raw-pushes) records its payload and writes
no driver. Read the records back with `raw_pushes()`, every `RawPush` in push
order, or `pushed_raw(...)`, the ones a predicate accepts:

```rust
use suprnova::queue::testing::{pushed_raw, raw_pushes};

let _guard = suprnova::Queue::fake();
replay(lines).await?;

assert_eq!(raw_pushes().len(), 3);
let invoices = pushed_raw(|raw| raw.envelope().is_ok_and(|env| env.job_name == "SendInvoice"));
assert_eq!(invoices[0].queue.as_deref(), Some("replays"));
```

A `RawPush` holds the `payload` and the `queue` exactly as they were passed,
and `envelope()` decodes the payload. Raw pushes are kept apart from typed
pushes, as in Laravel: `pushed` and `assert_pushed` do not see them.

### Why Suprnova diverges

Laravel's `except` and `assertPushedWithoutChain` take class names. Rust has
no class name to pass at run time, so `except` takes `Job::job_name()`s, as
`assert_chained` and `EventFacade::fake_except` do, and
`assert_pushed_without_chain` takes the job type. It takes no callback; to
narrow to some pushes of `J`, read them with `pushed::<J>()`. Laravel's
`QueueFake` does not record batches at all, so it has no rule for a batch
that mixes excepted and faked jobs; here the batch is recorded and each job
goes where `except` sends it.

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
