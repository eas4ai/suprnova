# Logging

Suprnova logs through [`tracing`](https://docs.rs/tracing) - every log
line is a structured event with fields, not a formatted string. A
subscriber is installed at boot that reads `LOG_LEVEL` and `LOG_FORMAT`
from the environment, emits pretty multi-line output in dev and one
JSON object per line in production, and propagates a per-request id
into every event a handler emits.

This chapter covers the log surface itself: the subscriber, the
formats, the levels, the channels the lines go to, and the request-id
correlation that makes a production log searchable. For the OpenTelemetry bridge and query
logging see [Observability](observability.md); for the request
`Context` bag that emitters can read alongside the id see
[Context](context.md).

## What gets logged where

`tracing`'s events go to the default channel, `stdout` unless `LOG_CHANNEL`
names another (see [Channels](#channels)). On `stdout`, two formats:

| Where | Format | When |
|---|---|---|
| `stdout` | `LogFormat::Pretty` - multi-line, coloured, human-friendly | dev (`APP_ENV` is `local`, `dev`, `testing`, …) |
| `stdout` | `LogFormat::Json` - one JSON object per line | production (`APP_ENV=production` / `prod`) |

The dev/prod default is computed from `APP_ENV` via
`Environment::detect()`. Override with `LOG_FORMAT=pretty` or
`LOG_FORMAT=json` to force one explicitly.

```env
# .env (dev)
LOG_LEVEL=info,sqlx=warn
LOG_FORMAT=pretty   # optional; this is the dev default

# .env.production
LOG_LEVEL=info,sqlx=warn,suprnova::queue=debug
LOG_FORMAT=json     # optional; this is the prod default
```

By default the framework writes to `stdout`. In a container, point the
runtime, the systemd journal or a log aggregator at it (`docker logs`,
`kubectl logs`, `journalctl -u my-app`, a Loki or Vector agent). Where
there is no platform to keep the logs, write them to files with a
[channel](#channels).

## Channels

A channel is where log lines go. `LOG_CHANNEL` names the default channel,
the one `tracing`'s events go to:

| Channel | Writes to |
|---|---|
| `stdout` | standard output, in the `LOG_FORMAT`; the default |
| `stderr` (also `errorlog`) | standard error |
| `single` | `storage/logs/suprnova.log`, appended to |
| `daily` | `storage/logs/suprnova-2026-10-02.log`, a file a day, keeping the newest `LOG_DAILY_DAYS` (14; 0 keeps every file) |
| `monthly` | `storage/logs/suprnova-2026-10.log`, a file a month, keeping the newest 3 |
| `syslog` | the local syslog socket, with the facility `LOG_SYSLOG_FACILITY` (`user`) |
| `null` | nowhere |
| `stack` | every channel `LOG_STACK` lists, `single` unless set |

```env
LOG_CHANNEL=stack
LOG_STACK=daily,stdout
LOG_DAILY_DAYS=30
```

A channel that does not exist stops the server and the workers at boot,
once the bootstrap has run, with an error that names it; so does a name in
`LOG_STACK`, a `LOG_SYSLOG_FACILITY` or a `LOG_DAILY_DAYS` that is wrong,
whichever channel is the default. Dates are those of the framework clock,
in UTC. File lines are text, one line a record, or JSON objects when
`LOG_FORMAT=json`, and carry the fields of the spans the event is in, the
request's `request_id` among them. A field an inner span shares a name with
replaces the outer span's, and an event's own field replaces both, in the
message's placeholders and in the context alike.

Define your own channels in the bootstrap, and reach any channel with
`Log`:

```rust
use serde_json::json;
use suprnova::{Log, LogChannel, LogLevel};

// bootstrap.rs
Log::define("audit", LogChannel::daily("storage/logs/audit.log").days(90));
Log::define("alerts", LogChannel::syslog().facility("local0").level(LogLevel::Error));

// anywhere
Log::channel("audit")?.info_with("user {id} signed in", json!({ "id": 42 }));
Log::stack(&["audit", "alerts"])?.critical("payments are failing");
Log::build(LogChannel::single("storage/logs/import.log"))?.notice("import done");
```

`Log::channel`, `Log::stack` and `Log::build` write to their channels
only; `tracing`'s events go to the default channel. A logger has
Laravel's eight levels, from `emergency` to `debug`, and the `*_with`
methods take a JSON context: each `{key}` in the message is replaced with
its value, and the context is written beside the message. A channel
`.level(...)` keeps only the records at that level or above. A stack's
`.level(...)` applies to every channel it lists, on top of each channel's
own, and a default stack that names `stdout` more than once writes an event
there when any of those channels keeps its level.

A stack writes each record to every channel it lists, and a channel that
cannot write, such as a file it cannot open, stops none of the others;
the failure is reported once on stderr, and the code that logged never
sees an error. That covers a driver's sink that returns an error from
`write` or `flush`, and buffered lines a file cannot flush, such as on a
full disk. `Log::channels()` lists the channels in use,
`Log::forget_channel(name)` closes one, which is resolved again on its
next use, reopening a file rotated away; every stack that lists it, the
default channel included, reopens it too. `Log::default_channel()` and
`Log::set_default_channel(name)` read and move the default.

`Log::extend` adds a driver for anything else, such as Slack or a log
service, with a `LogSink` that receives each `LogRecord`:

```rust
use std::sync::Arc;
use suprnova::{Log, LogChannel, LogRecord, LogSink};

struct Webhook;

impl LogSink for Webhook {
    fn write(&self, record: &LogRecord) -> std::io::Result<()> {
        // send record.level, record.message and record.context
        Ok(())
    }
}

Log::extend("webhook", |_channel| Ok(Arc::new(Webhook) as Arc<dyn LogSink>));
Log::define("ops", LogChannel::driver("webhook").option("url", "https://..."));
```

`MAIL_LOG_CHANNEL` names the channel the `log` mail transport writes to.

### Flushing

File channels buffer their lines. A record at `error` or above is written
out at once, the rest within a second, and everything on `Log::flush()`,
which the server, the workers and every console command call when they
end. A record written before a clean exit is never lost. A driver added
with `Log::extend` is flushed with the files.

## Emitting events

Use the `tracing` macros in handlers, jobs, middleware, anywhere:

```rust
use suprnova::{json_response, session, Request, Response};
use tracing::{debug, info, warn, error, instrument};

pub async fn checkout(_req: Request) -> Response {
    let user_id: i64 = session()
        .and_then(|s| s.get::<i64>("user_id"))
        .unwrap_or(0);

    info!(user_id, "checkout starting");

    let order = place_order(user_id).await.map_err(|e| {
        error!(user_id, error = %e, "checkout failed");
        e
    })?;

    info!(user_id, order_id = order.id, total = order.total_cents, "checkout succeeded");

    json_response!(order)
}
```

Each field becomes a top-level key in JSON output and a coloured
`field=value` pair in pretty output. Prefer fields over interpolation -
they're searchable in JSON logs and the formatter handles type-aware
rendering.

To wrap a function in a span and stamp every event inside it with
shared fields, use `#[instrument]`:

```rust
#[instrument(skip(db), fields(user_id = %user_id))]
pub async fn load_dashboard(
    db: &suprnova::DatabaseConnection,
    user_id: i64,
) -> Result<Dashboard, FrameworkError> {
    info!("loading"); // automatically carries user_id from the span
    // … queries …
}
```

The same `#[instrument]` becomes an OpenTelemetry span when the `otel`
feature is enabled - see [Observability](observability.md#opentelemetry).

## Log levels

`LOG_LEVEL` is a [`tracing-subscriber` env-filter
directive](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html),
not a single level. The grammar is comma-separated `target=level`
pairs, where bare values set the default:

```env
LOG_LEVEL=info                                  # everything at info+
LOG_LEVEL=debug                                 # everything at debug+
LOG_LEVEL=info,sqlx=warn                        # info default, sqlx quieter
LOG_LEVEL=warn,suprnova::queue=debug,my_app=info  # warn default, two targets verbose
```

Targets are usually the emitting crate or module path
(`suprnova::queue`, `hyper::server`, `my_app::services::checkout`).
Find a target by reading the JSON log line - the `target` field on
every event is its filter key.

Levels in increasing verbosity: `error` < `warn` < `info` (default) <
`debug` < `trace`. The wire-format error response is always sanitised
to `{"message": "Internal Server Error"}` regardless of level - the
detail goes only to the structured log.

### Invalid directives don't crash boot

A malformed `LOG_LEVEL` (e.g. `LOG_LEVEL=app=notalevel`) falls back to
`"info"` and writes a one-line warning to `stderr`:

```text
suprnova: invalid LOG_LEVEL directive "app=notalevel" (...); falling back to "info". Fix LOG_LEVEL to silence this.
```

This is `stderr` rather than `tracing::warn!` because the subscriber
hasn't been installed yet - a `warn!` would be silently dropped. Fix
the directive and the warning goes away.

## Pretty vs JSON output

The same `info!(user_id = 42, "saved")` renders differently per format.

**Pretty (dev):**

```text
  2026-05-30T22:14:08.221341Z  INFO request{request_id=78a9...} my_app::handlers::checkout: saved
    at src/handlers/checkout.rs:48
    in checkout
    in request with request_id: 78a9..., method: POST, path: /checkout
```

**JSON (prod):**

```json
{
  "timestamp": "2026-05-30T22:14:08.221341Z",
  "level": "INFO",
  "fields": { "message": "saved", "user_id": 42 },
  "target": "my_app::handlers::checkout",
  "span": { "name": "checkout" },
  "spans": [
    { "name": "request", "request_id": "78a9...", "method": "POST", "path": "/checkout" }
  ]
}
```

The JSON shape is what production aggregators (Datadog, Loki,
Honeycomb, CloudWatch, …) parse out of the box. `span.request_id` is
the correlation key - see below.

## Per-request id correlation

Every HTTP request gets a `RequestId` from `RequestIdMiddleware`, the
outermost middleware on every chain. The id is:

- **Reused** from a safe inbound `X-Request-Id` header (alphanumerics
  plus `- _ . :`, up to 128 bytes), or **freshly minted** as a UUID v4
  if absent / unsafe.
- **Echoed** back on the response as `X-Request-Id` (both 2xx and
  5xx variants).
- **Scoped** into a `request` `tracing` span so every event from any
  middleware, handler, or downstream library carries `request_id` in
  its `spans` array automatically.
- **Seeded** into the request `Context` bag as `_request_id`, so
  emitters that want the bare string (jobs, broadcast payloads, error
  reports) can read it by name.

Read it in code with `current_request_id()`:

```rust
use suprnova::current_request_id;
use tracing::info;

if let Some(id) = current_request_id() {
    info!(request_id = %id, "checkpoint reached");
}
```

`current_request_id()` returns `Option<RequestId>` because background
work (jobs, scheduled tasks, tests that didn't install the middleware)
runs outside any request scope.

### Background tasks: spawn with the id

`tokio::spawn` starts a fresh task with empty task-locals - a handler
that spawns side-effect work loses `current_request_id()` and its log
events become orphaned. Use `spawn_with_request_id` instead:

```rust
use suprnova::spawn_with_request_id;
use tracing::info;

pub async fn checkout(req: suprnova::Request) -> suprnova::Response {
    let order = place_order().await?;

    spawn_with_request_id(async move {
        // This task still observes current_request_id().
        // Its log events carry the same request_id as the handler's.
        info!(order_id = order.id, "post-checkout fanout running");
        send_receipt(order.id).await;
        update_analytics(order.id).await;
    });

    Ok(suprnova::HttpResponse::ok().json(&order))
}
```

The helper propagates both the `RequestId` task-local and the current
`tracing::Span`, so the spawned future's events nest under the same
`request` span in the log. Outside an active request scope it falls
through to a bare `tokio::spawn` - safe to use unconditionally.

Only the request id and tracing span follow the task - the request
`Context` bag deliberately does not, because background work isn't
serving the originating HTTP request.

## The subscriber

The framework installs a global `tracing` subscriber at boot from
`Server::run()`. You almost never call this yourself; it's documented
because tests, embedders, and unusual entry points sometimes need to.

```rust
use suprnova::{LogConfig, init_subscriber};

// Read LOG_LEVEL / LOG_FORMAT from the environment:
init_subscriber(LogConfig::from_env());

// Or programmatic:
init_subscriber(LogConfig {
    level: "info,sqlx=warn".to_string(),
    format: suprnova::LogFormat::Json,
});
```

`init_subscriber` is **idempotent**. A second call leaves the existing
subscriber in place, with its default channel and the format of its file
lines, and emits a `tracing::warn!` so an operator can see that the new
`LogConfig` was not applied. This is what lets tests
that each call `init_subscriber` not race each other - the first wins,
the rest are no-ops.

For the OTel-aware variant (the same `LogConfig`, plus
distributed-tracing export), use
[`init_telemetry`](observability.md#opentelemetry).

### The daemons

`queue:work`, `schedule:work`, `schedule:run` and `workflow:work` are
subcommands of your app binary and do not boot through `Server::run()`, so
they install their own subscriber on the way up. They read the same
`LOG_LEVEL` and `LOG_FORMAT` as the server, and you call nothing yourself:

```bash
LOG_LEVEL=info,suprnova::queue=debug cargo run --bin my-app -- queue:work

# …or, in a container, against the built binary:
LOG_LEVEL=info my-app queue:work
```

Before 0.9.1 that path installed nothing at all. Every `tracing::` line the
daemons emit went nowhere and `LOG_LEVEL` was inert for them, which in a
container left the startup banner as the only output - a worker
dead-lettering jobs, a scheduler skipping a tick it lost the election for,
and a lock it could not release all looked identical to an idle process. If
you are running a pinned build older than 0.9.1 and wondering why a worker
says nothing, that is why, and the fix is the upgrade rather than a
configuration change.

Most of what a worker has to say it says at `warn!` and `error!` - a job
exhausting its attempts, a dead-letter it could not persist, a lock it
could not release - so the default `info` level is enough to see trouble.
Drop to `debug` when you need the quieter decisions as well.

## Tests

Tests don't need to install a subscriber - the `#[suprnova_test]`
attribute and `TestContainer::fake` set up enough machinery for
handler events to flow. If you want to assert on log output, capture
via `tracing-subscriber`'s
[`tracing_subscriber::fmt::TestWriter`](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/fmt/struct.TestWriter.html)
or a custom layer; the framework deliberately does not ship a "capture
all logs in this test" fake because the standard `tracing-subscriber`
test patterns work cleanly.

## Why Suprnova diverges

Laravel uses [Monolog](https://github.com/Seldaek/monolog) - message
strings with optional context arrays, log channels, and per-channel
handlers (file, syslog, Slack, …). PHP's request-per-process model
means a single global static logger is safe: each request gets its
own process and its own context.

Rust's process model is the opposite - one process serves many
concurrent requests on many threads. A global string-formatter would
race on context and require explicit `request_id` plumbing through
every call site. `tracing` solves both with structured fields and
task-local spans: no plumbing, fields stay typed, and correlation is
automatic because the request span is in scope for every event the
chain emits.

Laravel's channels are here as `Log`, with three differences. The
default channel is `stdout`, not a stack of files, because a container's
runtime keeps its logs. Channels are configured from the environment and
`Log::define` in the bootstrap, not a config file. And file channels
buffer, flushing at once for errors, within a second otherwise, and on
shutdown, where Monolog writes each record straight through.

## Next

- [Observability](observability.md) - OpenTelemetry, query log, the
  full operator surface
- [Context](context.md) - the per-request bag where `_request_id` and
  other contextual fields live
- [Error Handling](errors.md) - how the framework's panic boundary
  and 5xx path emit their own structured events
- [Environment Variables](env-vars.md) - `LOG_LEVEL`, `LOG_FORMAT`,
  `LOG_CHANNEL` and the channel variables
