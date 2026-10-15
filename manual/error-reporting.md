# Error Reporting

Suprnova reports unhandled errors through `Reporter`. The built-in `sentry`
driver speaks the Sentry envelope protocol, so one DSN reaches Rustrak,
Sentry or GlitchTip. No listener or vendor SDK setup is needed in your app.
Without a DSN, the driver sends nothing.

## Start with local Rustrak

In a scaffolded application, generate the optional service:

```bash
suprnova docker:compose --with-rustrak
export RUSTRAK_CREATE_SUPERUSER="admin@example.com:$(openssl rand -hex 24)"
docker compose up -d
```

Save the initial account credentials before closing that shell. Open
`http://127.0.0.1:8080`, sign in, create a project and copy its DSN. Put that
DSN in your application's configuration:

```dotenv
REPORTING_DRIVER=sentry
REPORTING_DSN=http://your-project-key@127.0.0.1:8080/1
```

Replace the example key and project id with the DSN Rustrak shows. The
generated service pins `rustrak/rustrak-server:v0.16.4`, publishes port 8080
on loopback only, persists `/data` in `rustrak_data`, generates its own
`SESSION_SECRET_KEY` and sets `RUSTRAK_TELEMETRY=off`. `RUSTRAK_PORT` changes
the published port and `PUBLIC_URL` together. Keep the volume and session key
across restarts. `RUSTRAK_CREATE_SUPERUSER` creates an account only while the
database has no users; changing it later does not reset a password.

For an application that already has a Compose file, plain Docker works too:

```bash
export SESSION_SECRET_KEY="$(openssl rand -hex 32)"
export CREATE_SUPERUSER="admin@example.com:$(openssl rand -hex 24)"
docker run -d --name rustrak \
  -p 127.0.0.1:8080:8080 \
  -v rustrak_data:/data \
  -e SESSION_SECRET_KEY -e CREATE_SUPERUSER \
  -e PUBLIC_URL=http://127.0.0.1:8080 \
  -e RUSTRAK_TELEMETRY=off \
  rustrak/rustrak-server:v0.16.4
```

Save both generated values in your secret store before recreating the
container. This follows Rustrak's [installation instructions](https://rustrak.github.io/rustrak/getting-started/installation)
with a pinned image, a loopback port and telemetry disabled. The dashboard
and API are in that single container. Rustrak runs as a separate service;
Suprnova includes none of its GPL-3.0 server code.

## Choose a driver

| Driver | Behaviour |
|---|---|
| `sentry` | Sends to `REPORTING_DSN` in the background. The default; inert without a DSN. |
| `log` | Writes the scrubbed event to the application's logs. |
| `null` | Discards events. Useful when another process owns reporting. |

Environment configuration is loaded at application startup. To select a
driver in Rust instead, configure it during bootstrap:

```rust
use suprnova::{FrameworkError, Reporting, ReportingConfig, ReportingDriver};

pub fn register_reporting() -> Result<(), FrameworkError> {
    Reporting::configure(
        ReportingConfig::default()
            .driver(ReportingDriver::Sentry)
            .dsn("http://your-project-key@127.0.0.1:8080/1"),
    )?;
    Ok(())
}
```

`ReportingDriver::Log` and `ReportingDriver::Null` select the other drivers.
A custom backend implements `Reporter::report(ReportingEvent)` and
`Reporter::flush(Duration)`. `ReportingConfig::build()` returns an
`Arc<dyn Reporter>` for applications that need an independent reporter.
`Reporter::report_error(&FrameworkError)` uses that driver's capture path;
the Sentry driver queues symbol resolution instead of doing it on the caller.
`ReportingEvent::from_error` uses the active application's configuration.
For an independent reporter, pass its configuration when capturing the
event so the same privacy settings apply:

```rust
use suprnova::{FrameworkError, ReportingConfig, ReportingEvent};

pub fn report_independent_failure() -> Result<(), FrameworkError> {
    let config = ReportingConfig::default()
        .dsn("http://your-project-key@127.0.0.1:8080/1")
        .send_pii(false);
    let reporter = config.build()?;
    let error = FrameworkError::internal("independent task failed");
    reporter.report(ReportingEvent::from_error_with_config(&error, &config))?;
    Ok(())
}
```

The default `reporting-sentry` Cargo feature includes the official Rust
Sentry SDK. Applications that disable default features must enable it to
send to a Sentry-compatible tracker.

The builder also offers `.release(...)`, `.queue_capacity(usize)`,
`.timeout(Duration)` and `.duplicate_window(Duration)`. The sender defaults
to 128 pending events, a two-second network timeout and a one-second burst
deduplication window. These delivery limits are programmatic settings.

## Settings

| Setting | Default | Purpose |
|---|---|---|
| `REPORTING_DRIVER` | `sentry` | `sentry`, `log` or `null`. |
| `REPORTING_DSN` | unset | Server-side Sentry-protocol DSN. An unset DSN sends nothing. |
| `REPORTING_RELEASE` | application's package version | Identifies the deployed release. Set a version or commit hash in deployment. |
| `APP_ENV` | `production` | Names the deployment environment in events and transactions. |
| `REPORTING_SEND_PII` | `false` | Allows the user's email and IP address. The user id is included without this opt-in. |
| `REPORTING_LOGS` | `off` | Sends structured Sentry logs at or above the selected level. |
| `REPORTING_TRACES_SAMPLE_RATE` | `0` | Fraction of request and job transactions to send, from `0` to `1`. |
| `REPORTING_PUBLIC_DSN` | unset | Public browser DSN for Live and the security policy's tracker origin. |
| `VITE_REPORTING_DSN` | unset | Public browser DSN for Svelte, React and Vue starter builds, and a tracker origin the security policy allows. |

Logs and transactions use the same DSN as errors. Both are off by default;
error reporting alone does not opt in to either. For example:

```dotenv
REPORTING_LOGS=warn
REPORTING_TRACES_SAMPLE_RATE=0.1
REPORTING_RELEASE=2026.10.15-abc123
```

See [Environment Variables](env-vars.md) for parsing and the complete list.
The `#[suprnova::main]` entry point records your application's package
version for the release fallback. Applications with a custom entry point
can set `.release(...)` on the builder or call
`suprnova::boot::set_default_build_id(env!("CARGO_PKG_VERSION"))` at startup.

## Report an error you handle

The framework already reports unhandled 5xx errors, caught panics, queue
failures, console failures, scheduled tasks, workflow and supervisor
failures, WebSocket sessions and rescued Inertia props. A 4xx, validation
failure, authentication or authorization refusal, or silent error is not
reported. Use the helpers when your code handles a failure and continues:

```rust
use suprnova::{FrameworkError, report, report_if, report_unless, rescue};

pub async fn load_settings() -> Result<String, FrameworkError> {
    let error = FrameworkError::internal("optional settings unavailable");
    report(&error)?;
    report_if(false, &error)?;
    report_unless(true, &error)?;

    let settings = rescue(
        async {
            std::fs::read_to_string("storage/settings.json")
                .map_err(FrameworkError::from_external)
        },
        |_error| String::from("{}"),
    )
    .await;
    Ok(settings)
}
```

`rescue` reports an error before calling the fallback and returns the
fallback's value. A reporting failure does not prevent the fallback.
`Exceptions::report` remains available for existing applications.

## Control which errors leave the app

`Exceptions::reportable` callbacks run before the reporter. A callback's
`.stop()` prevents the reporter from sending that error. Register callbacks
and exclusions once in bootstrap, rather than on each request:

```rust
use suprnova::{Exceptions, FrameworkError};

pub fn register_exclusions() {
    Exceptions::dont_report::<std::io::Error>();
    Exceptions::dont_report_when(|error: &FrameworkError| error.is_silent());
    Exceptions::dont_report_duplicates();
}
```

Type exclusions match wrapped external errors. A predicate can use your
own application rule. `dont_report_duplicates` suppresses repeated reports
of the same error. The sender also coalesces bursts within a bounded window.

Events include the error type and chain, stack trace, known error or panic
location, release, environment and host name. Request events add the method,
route pattern, URL, headers, request id and user id. Recent log records from
the same request or job become breadcrumbs.

The reporter and debug page share scrubbing. Authorization, Cookie and
Set-Cookie headers, CSRF and session identifiers, passwords and fields whose
names contain password, token, secret, key or signature are removed. So are
model `hidden` fields, `SecretString` values and the context's hidden bag.
The reporter never sends request bodies. Keep secrets out of error text and
ordinary log messages as well.

The sender uses a bounded queue and a network timeout. Reporting enqueues
work without waiting on the tracker. A full queue or unreachable server
drops and counts events; it does not fail the request or job. The default
Sentry driver captures the reporting stack immediately and resolves its
symbols in the sender. While the resolver is busy, new captures are
dropped and counted to keep requests and panic hooks from waiting on it.
Server and daemon shutdown flush the queue within a deadline.
`Reporting::metrics()` returns queued, delivered, dropped and duplicate
counts without exposing event data or tracker credentials.

## Browser reports

Set the public DSN for the kit you use: `VITE_REPORTING_DSN` for a Svelte,
React or Vue starter, `REPORTING_PUBLIC_DSN` for Live. It is a browser
ingestion address; never put a tracker account password or API token there.

```dotenv
# Svelte, React or Vue
VITE_REPORTING_DSN=http://your-project-key@127.0.0.1:8080/1
# Live
REPORTING_PUBLIC_DSN=http://your-project-key@127.0.0.1:8080/1
```

Svelte, React and Vue starters load Sentry's browser package only when
`VITE_REPORTING_DSN` is set. They report uncaught errors, unhandled
rejections and framework render or hydration failures. The package loads in
its own chunk while the application mounts, so the first page does not wait
for it; an error raised before it is ready, such as a hydration failure on
the first render, is held (up to 32) and sent once it loads. Rebuild the
frontend after changing the Vite setting. With no value the reporter is not
loaded.

Live uses its built-in runtime reporter, configured by
`REPORTING_PUBLIC_DSN`, without a third-party script. It covers uncaught
errors, unhandled rejections and Live runtime failures.

`SecurityHeaders` allows the tracker origin of each public DSN it finds in
the environment, `REPORTING_PUBLIC_DSN` and `VITE_REPORTING_DSN`, in
`connect-src`, so the scaffold's own policy does not block the reports. A
value that is not a public DSN stops the application at boot with an error
naming the setting. The server reads `VITE_REPORTING_DSN` from the same
`.env` the build reads; a deployment that passes it to the build alone must
give the server the same value.

### Why Suprnova diverges

Laravel reports through log channels and uses `sentry/sentry-laravel` for
Sentry integration. Suprnova includes a `Reporter` trait and selectable
drivers so HTTP requests, background work and the starter kits use one
reporting path. Rust returns errors as values, so the helpers accept
`FrameworkError` and `rescue` accepts a future returning `Result`.

## Next

- [Error Handling](errors.md) covers returned errors and callbacks.
- [Error Model](error-model.md) explains conversion and response sanitisation.
- [Logging](logging.md) covers application log channels.
- [Security Headers](security-headers.md) covers CSP and browser connections.
