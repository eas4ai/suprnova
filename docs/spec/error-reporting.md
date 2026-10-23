# Error reporting

Status: Agreed 2026-10-15
Prefix: REPORT

Drafted 2026-10-15 from the developer's direction "https://github.com/rustrak/rustrak
add this next before finishing search and include it with the security
headers fix in the next release", with Rustrak's installation page and its
`docker run` command as the setup he named. Rustrak is a GPL-3.0 server that
speaks the Sentry protocol; Suprnova talks to it over the network and takes
none of its code, so the framework's MIT license is unaffected. The same
protocol reaches Sentry and GlitchTip. The Observed section describes
framework `16d607aad`, checked by an independent reader the same day. The
developer agreed the requirements on 2026-10-15 ("ok").

## Observed at 16d607aad

Status: Observed

- `Exceptions` (`framework/src/error/reporting.rs`, exported at
  `framework/src/lib.rs:349`) is the framework's report choke point:
  `reportable<E>` callbacks (`reporting.rs:118-136`), `report`, which runs
  them and logs "framework error" with status, chain and request id
  (`reporting.rs:153-164`), `report_logged` (`178-180`), silent errors
  skipped (`184-186`), panicking callbacks caught (`188-196`). Callbacks
  receive only the error: no request, user, route, `ErrorReport` or frames.
  There is no `dont_report`, throttle, context callback, `report()` helper
  or `rescue()` helper.
- Reported today: a 5xx from a handler error (`framework/src/http/response.rs:1812-1832`),
  a panic in the chain (`framework/src/server.rs:1663-1727`), a 5xx the
  framework builds (`response.rs:465-488`), Live endpoint and upload 5xx
  (`framework/src/live/action.rs:349-379`, `framework/src/live/upload.rs:241-257`),
  queue job failures and timeouts (`framework/src/queue/worker.rs:1281-1285,1364-1366,1435-1437`),
  console command and bootstrap errors (`framework/src/console/mod.rs:348-389`).
  A 4xx is only logged at warn (`response.rs:1834-1840`).
- Not reported: scheduled task failures and panics, which reach only
  `eprintln!` (`framework/src/app/mod.rs:667-679,784-790`;
  `framework/src/schedule/mod.rs:713-728,763-765`); workflow step panics
  (`framework/src/workflow/mod.rs:526-588`); supervisor task errors and
  panics (`framework/src/supervisor/mod.rs:459-490`); WebSocket session
  errors (`server.rs:2138-2195`); rescued Inertia props
  (`framework/src/inertia/response.rs:2656-2678`); Live engine panics,
  whose payload is discarded (`crates/suprnova-live/src/isolation.rs:33-35`);
  panics off a request's task (`framework/src/error/report.rs:356-362`).
- `ErrorReport` keeps the chain, type name and panic location in the
  response's extensions (`report.rs:1-14`); stack frames are recorded only
  while serving a request with debug on (`framework/src/error/frames.rs:3-19`),
  so a production report has none.
- The app knows `APP_NAME`, `APP_ENV` and `APP_URL`
  (`framework/src/config/providers/app.rs:6-74`); `suprnova::VERSION` is the
  framework's version (`lib.rs:145`); nothing records the application's own
  version, its git commit or the host name.
- Redaction exists only on the debug page (Authorization, Cookie and
  Set-Cookie, and any header or query parameter whose name holds token,
  secret, password, key or signature; `framework/src/error/debug_page.rs:801-930`)
  and in the queue's log line (`worker.rs:338-356`). `ErrorOccurred`
  carries the unsanitized chain on purpose (`framework/src/events/builtins.rs:18-23`).
- The manual shows reporting to Sentry by hand from an `ErrorOccurred`
  listener (`manual/errors.md:530-553`). No Sentry dependency exists.
- The starter kits capture no browser error; the Live runtime writes its
  diagnostics to the console only (`crates/suprnova-live/browser/src/runtime/diagnostics.ts:5-18`).
- `suprnova docker:compose` writes Postgres and Redis, with Mailpit and MinIO
  optional (`suprnova-cli/src/commands/docker_compose.rs:11-40`).
- Laravel reports through `Handler::report` with `reportable`,
  `dontReport`, `dontReportWhen`, `dontReportDuplicates`, `context` and
  `dontFlash`, and the `report()`, `report_if`, `report_unless` and
  `rescue()` helpers (`reference/framework-13.35.0/src/Illuminate/Foundation/Exceptions/Handler.php:72-193,425-545,652-694`;
  `Foundation/helpers.php:723-810`); Sentry support is the
  `sentry/sentry-laravel` package.

## Requirements

[REPORT-001] The framework MUST report errors to an error tracker through a
`Reporter` trait with drivers chosen by `REPORTING_DRIVER` or a builder:
`sentry`, which sends the Sentry envelope protocol to `REPORTING_DSN` and so
reaches Rustrak, Sentry and GlitchTip; `log`; and `null`. With no DSN the
`sentry` driver sends nothing. Every error `Exceptions::report` handles
reaches the reporter after the application's `reportable` callbacks, which
can still stop it.
Falsifier: an application cannot send its errors to a Sentry-protocol server without writing its own listener; a stopping `reportable` callback does not keep an error from the reporter; or anything is sent with no DSN.
Mechanism: `error-reporting`.
Rationale: The `sentry` driver uses the official `sentry` crate (MIT), on by default and inert without a DSN, as the developer agreed on 2026-10-15 ("ok").
Status: Agreed 2026-10-15

[REPORT-002] Every unhandled error MUST be reported: a 5xx from a handler,
a panic anywhere the framework catches one (the request chain, Live's
engine with its payload, scheduled tasks, workflow steps, supervisor tasks,
WebSocket sessions, the panic hook for tasks outside a request), a queue job
failure, a console command error, a scheduled task error and a rescued
Inertia prop. A 4xx, a validation failure, an authentication or
authorization refusal and a silent error MUST NOT be. Applications MUST get
Laravel's `report`, `report_if`, `report_unless` and `rescue` helpers and
`Exceptions::dont_report`, `dont_report_when` and `dont_report_duplicates`.
Falsifier: one of those failures reaches no reporter; a 4xx, validation, auth or silent error is reported; or one of the helpers or settings is missing.
Mechanism: `error-reporting`.
Status: Agreed 2026-10-15

[REPORT-003] A reported event MUST carry the error type and chain, a stack
trace taken where it was reported (the panic location or the error's
recorded location when known), the release (`REPORTING_RELEASE`, else the
application's package version), the environment (`APP_ENV`), the host
name, and under a request its method, route pattern, scrubbed URL and
headers, request id and the user's id; recent log records of the same
request or job as breadcrumbs. The user's email and IP address are sent
only with `REPORTING_SEND_PII=true`.
Falsifier: an event lacks one of those fields in production; or an email or IP address is sent without the opt-in.
Mechanism: `error-reporting`.
Status: Agreed 2026-10-15

[REPORT-004] Nothing secret MUST leave the application in an event:
passwords, tokens, cookies, `Authorization` and `Set-Cookie` headers, CSRF
and session identifiers, any header, query or form field whose name holds
password, token, secret, key or signature, a model's `hidden` fields,
`SecretString` values and the context's hidden bag. Request bodies are not
sent. The debug page and the reporter MUST share one redaction function.
Falsifier: any of those values appears in a sent event, or the debug page and the reporter redact differently.
Mechanism: `error-reporting`.
Status: Agreed 2026-10-15

[REPORT-005] Reporting MUST never slow or fail a request, job or command:
events go into a bounded queue drained by a background sender with a
timeout; a full queue or an unreachable server drops the event and counts
it; duplicates of one error inside a window are sent once; the server and
every daemon flush the queue on shutdown within a deadline.
Falsifier: a request waits on the tracker; a down tracker fails or slows a request beyond the queue push; the queue grows without bound; a duplicate burst sends every copy; or a clean shutdown loses queued events.
Mechanism: `error-reporting`.
Status: Agreed 2026-10-15

[REPORT-006] Opt-in, through the same DSN: log records at or above
`REPORTING_LOGS` sent as Sentry logs, and request and job transactions
sampled at `REPORTING_TRACES_SAMPLE_RATE` from the spans the framework
already opens. Both are off by default.
Falsifier: logs or traces are sent without the opt-in, or not sent with it.
Mechanism: `error-reporting`.
Rationale: The developer included logs and traces on 2026-10-15 ("yes"), so Rustrak's log and performance views work for an application that turns them on.
Status: Agreed 2026-10-15

[REPORT-007] The starter kits MUST report browser errors (uncaught errors,
unhandled rejections and framework render or hydration errors) to the same
tracker when the application sets a public DSN, and load nothing when it is
unset: the Inertia kits through Sentry's own browser package, loaded only
when `VITE_REPORTING_DSN` is set; the Live kit through a reporter built into
the Live runtime, configured from the server, with no third-party script.
With a public DSN, `SecurityHeaders` MUST allow the tracker's origin in
`connect-src`, so the scaffold's own policy does not block the reports.
Falsifier: a kit with a public DSN does not report an uncaught browser error or a hydration error; a kit without one loads a reporter; or the scaffold's CSP blocks the report request.
Mechanism: `error-reporting`.
Rationale: The developer included browser reporting on 2026-10-15 ("yes"); issue #150's hydration error would have reached the tracker on first run.
Status: Agreed 2026-10-15

[REPORT-008] `suprnova docker:compose` MUST offer a Rustrak service
(`rustrak/rustrak-server` at a pinned version, its data volume, a generated
`SESSION_SECRET_KEY`, `PUBLIC_URL`, loopback-bound port), and the env
templates MUST carry the `REPORTING_*` rows, so a new application reports to
a local Rustrak by setting one DSN.
Falsifier: `docker:compose` cannot add Rustrak, writes an unpinned image or a fixed secret, or the env templates lack the rows.
Mechanism: `error-reporting`.
Status: Agreed 2026-10-15

[REPORT-009] The manual MUST document reporting in its own chapter with a
"Why Suprnova diverges" section (Laravel reports to log channels and
reaches Sentry through `sentry/sentry-laravel`), and replace the hand-made
Sentry listener in `manual/errors.md`.
Falsifier: the chapter omits a driver, a setting, the helpers, scrubbing, the kits or the divergence, or `errors.md` still shows the hand-made listener.
Mechanism: `error-reporting`.
Status: Agreed 2026-10-15

[REPORT-010] The release gate MUST exercise a real Rustrak: a standing
`suprnova-gate-rustrak` container created once, and engine-named ignored
tests that send an error, a panic and a log through the `sentry` driver and
read each back through Rustrak's API.
Falsifier: no gate step sends to a running Rustrak, or an event Rustrak never received passes.
Mechanism: `error-reporting`.
Status: Agreed 2026-10-15
