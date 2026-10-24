# Debug Bar

The debug bar shows what one request did. It keeps recent records in memory
and adds a compact toolbar to browser HTML. Each tab opens a panel, and the
request selector switches between the initial page and later fetch, XHR and
Inertia responses.

## Install

The dogfood application and the Inertia, API and Live scaffolds install it in
their HTTP bootstrap. To install it in an existing application:

```rust
use suprnova::{DebugBar, FrameworkError, SecurityHeaders, global_middleware};

pub fn register_http_stack() -> Result<(), FrameworkError> {
    global_middleware!(SecurityHeaders::from_env()?);
    global_middleware!(DebugBar::middleware()?);
    Ok(())
}
```

Register it globally before constructing the server. The registration carries
the store; cloned registrations share it. Separate registrations have separate
stores. It preserves the application's middleware order. The server opens the
collector around routing, panic recovery and error-page rendering so the
record describes the final response.

## The gate

`Config::is_debug()` must be true. With debug on, the installed bar is enabled
by default. `DEBUGBAR=false` disables it; `0`, `off` and `no` also disable it,
without regard to letter case. `DEBUGBAR=true` cannot override debug off.

With the gate closed, the middleware passes application responses through
unchanged. No collector runs, no record header or toolbar is added, and
`/_debugbar/{id}` answers 404. A request that observes the gate closing also
clears that registration's previous records.

```bash
APP_ENV=local APP_DEBUG=true DEBUGBAR=true DEBUGBAR_KEEP=100 suprnova serve
```

`DEBUGBAR_KEEP` is the number of recent completed requests to retain. Its
default is 100. Zero, negative numbers and invalid integers fail bootstrap.
Records are never written to disk or a database. Each server process retains
its own records; a lookup must reach the process that handled the request.

## Panels

- Request: method, path, decoded query, headers, matched route and parameters,
  request id, session id, authenticated principal identifier and active locale.
  A cached named guard's principal is prefixed with its guard name.
- Timings: each middleware's own elapsed time, its inclusive time through
  `Next`, and the handler's elapsed time. A short-circuit has no handler span.
- Memory: the peak sampled process RSS growth above the request's starting
  RSS, in bytes. Linux RSS is sampled every 10 ms while the request runs and
  at completion. It includes concurrent process activity, and can miss shorter
  allocation spikes. On a platform without `/proc/self/status`, the value is
  `null` rather than a guessed allocation count.
- Queries and transactions: SQL, bindings, duration, connection and outcome
  from `QueryExecuted`, plus transaction begin, commit and rollback events.
- Cache: reads with hit or miss, writes, tagged writes, conditional writes,
  deletion, counters and lock operations through the facade's resolved stores.
- Events: dispatched payloads and the listeners selected for them.
- Mail, notifications and queue: sent messages, per-channel notification
  deliveries and pushed jobs. Their framework fakes are observed too.
- Render cache: hit, miss, store, bypass and decline outcomes.
- Live: generated component renders, action names and arguments, and execution
  phases. Nested component renders use the same observation point.
- Error: the response's error report, including a recovered panic.

Values use Rust's `Debug` output. Cache and queue interfaces do not require a
`Debug` bound on application types, so those panels show the stored value or
serialized envelope through its `Debug` implementation. The collector does
not resolve a user or load additional application data to populate a panel.
Framework-owned transaction finalization retains the request's collector.
Independent application tasks spawned with `tokio::spawn` do not inherit task
locals and belong to their own lifetime.

## HTML and later requests

A request whose `Accept` lists `text/html`, without a truthy `X-Inertia`, gets
the toolbar before the closing body tag. JSON, redirects, downloads, streams,
static files and HEAD responses keep their bodies. HTML fragments without a
closing body receive the toolbar at the end. Inline CSS and JavaScript ship
inside the crate; there are no toolbar assets to build or serve.

The toolbar uses the response's CSP nonce for its style and script elements.
When a policy has no nonce and forbids inline elements, the bar adds a nonce
source for its elements while preserving the policy's other sources. The
isolated development error page also permits same-origin diagnostic lookups.
Responses carrying records use `Cache-Control: no-store`. Rewriting an HTML
body removes its old length and validators. Injection happens after RenderCache,
so a shared representation never stores a request's diagnostic markup.

Every response has an `X-Debugbar-Id` while the gate is open. Read its JSON
record with a GET, using the same application path prefix:

```bash
curl -H 'Accept: application/json' http://localhost:8765/_debugbar/RECORD_ID
```

Replace `RECORD_ID` with the header value. An unknown or evicted id answers
404. Diagnostic lookups are requests too: they receive their own response id
and count toward the same retention limit. Their JSON body is the record
requested in the URL. The toolbar ignores lookup responses, so its fetch and
XMLHttpRequest observers do not recurse. Its first record is embedded as
escaped JSON in the document; later records are fetched by response id.

### Why Suprnova diverges

There is no arbitrary dump API. The bar observes framework operations and
shows their `Debug` output rather than reflecting over arbitrary application
objects. It shares the development error page's redaction rules: credential
headers, secret-named parameters and fields, and passwords in URLs are
redacted. SQL that names a secret, its positional bindings and any query error
are hidden together because a positional binding has no field name to redact.
Credential values captured from the request are also removed when
they appear elsewhere in the record. Request bodies, session contents,
environment variables and configuration values are not collected.

Nothing of the debug bar exists on the response or route with debug off.
The debug flag remains the authority even if `DEBUGBAR=true` is set.

## Next

- [Middleware](middleware.md) - register the HTTP stack
- [Security Headers](security-headers.md) - nonce policies
- [Error Model](error-model.md) - response error reports
- [Environment Variables](env-vars.md) - runtime settings
