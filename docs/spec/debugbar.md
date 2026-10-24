# Debug bar

Status: Draft
Prefix: DBG

Drafted 2026-10-23 from the developer's direction on
`reference/php-debugbar-3.8.0`: "I am not sure if something like a
debugbar can be made in Rust or not ... add these as commitments." It
can: the framework already collects most of what the panels show, and
this spec puts it in front of the developer during a request.

## Observed at d448ed10c

Status: Observed

- The database layer dispatches `QueryExecuted` with the SQL, the
  bindings, the duration and the connection, and the transaction events
  (`framework/src/database/events.rs`).
- Every response built from an error carries its report in process
  (PAR-010), and the development error page shows the request's method,
  path, query, headers, route pattern and request id with credentials and
  secret-named values redacted (`framework/src/error/debug_page.rs`).
- Cache, events, mail, notifications, queue and the HTTP client have
  fakes that record what went through them; no live collector records
  them during an ordinary request.
- The security headers middleware issues a CSP nonce per response
  (SEC-001); RenderCache reports a hit, a miss or a store per request;
  Live records component renders and actions in its own log.
- No toolbar, no per-request record and no debug endpoint exist.

## Requirements

[DBG-001] With debug on, a middleware the application installs MUST
record, for each request: the method, path, query, route pattern and
parameters, the request id, the status, the time spent in each middleware
and in the handler, the peak memory the request added, the session id,
the authenticated principal and the active locale; every `QueryExecuted`
with its SQL, bindings, duration and connection, and every transaction
event; every cache read with hit or miss and every cache write; every
event dispatched with its listeners; every mail and notification sent;
every queue job pushed; RenderCache's hit, miss or store; every Live
component render and action; and the response's error report when one
exists. Values MUST be shown through their `Debug` output; the record
MUST redact what the development error page redacts.
Falsifier: a request whose handler ran a query, read the cache, dispatched an event, sent a mail or pushed a job yields a record missing that entry; a record carries a credential header's value or a secret-named parameter in clear; or a middleware that took 50 ms shows no time.
Mechanism: `debugbar`.
Rationale: php-debugbar's collectors (`src/DataCollector/`: PDO, Messages, Exceptions, TimeData, Memory, RequestData, Config, Localization) and Laravel Debugbar's cache, events, mail, queue and views panels; the framework's events and fakes already observe each source.
Status: Draft

[DBG-002] With debug on, a response whose `Accept` lists `text/html` and
that is not an Inertia visit MUST carry the toolbar before `</body>`, its
script and style under the response's CSP nonce when the response has
one, showing the record and letting the developer open each panel; every
response MUST carry the record's id in a response header, and the
framework MUST serve the record as JSON at a debug route by that id, so
the toolbar already on the page shows the records of the Inertia visits
and XHR requests that follow. Records MUST be kept in a bounded in-memory
store of recent requests, configurable in size, and never written to
disk or to a database.
Falsifier: a browser response with debug on has no toolbar, or the toolbar's script is refused by the page's CSP; an Inertia visit's record cannot be fetched by the id its response named; a JSON, file or redirect response carries toolbar markup; or the store grows past its configured size.
Mechanism: `debugbar`.
Rationale: php-debugbar renders into the page (`JavascriptRenderer`) and fetches Ajax requests' data through its open handler by request id; Laravel Debugbar keeps the same shape.
Status: Draft

[DBG-003] With debug off, nothing of the debug bar MUST reach a
response or a route: the middleware MUST pass requests through
unchanged, no collector MUST run, the debug route MUST answer 404, and
no record MUST be kept. With debug on, the bar MUST be on, and
`DEBUGBAR=false` MUST turn it off. The scaffolds MUST install the
middleware in their bootstrap, and the dogfood application MUST run it.
Falsifier: with debug off, a response carries the record header or toolbar markup, the debug route answers other than 404, or a request leaves a record; with debug on and `DEBUGBAR=false` a toolbar appears; or a fresh scaffold's bootstrap does not install the middleware.
Mechanism: `debugbar`.
Rationale: Laravel Debugbar is enabled by `APP_DEBUG` and `DEBUGBAR_ENABLED`; the production risk of a debug bar is the data it shows, so the gate is on the debug flag the framework already keys every other disclosure on (PAR-012).
Status: Draft
