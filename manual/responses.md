# Responses

Every Suprnova handler returns a `Response`, which is an alias for
`Result<HttpResponse, HttpResponse>`. The `Ok` arm carries the success
response, the `Err` arm carries an already-rendered error response, and
the `?` operator collapses any error type that has a `From` into
`HttpResponse` along the way. This chapter is the practical reference
for building the `Ok` side - the `HttpResponse` builders, the
`Redirect` builder, the cookie API, and the `abort_*` short-circuits.
For the error story see [Error Model](error-model.md) and
[Error Handling](errors.md).

## `HttpResponse` builders

`HttpResponse` is the wire-shaped response type. The constructors set
sensible defaults; the chainable setters override them.

### Body constructors

```rust
use suprnova::{HttpResponse, Response};
use serde_json::json;

pub async fn examples() -> Response {
    // text/plain
    let _ = HttpResponse::text("OK");

    // application/json (any serde_json::Value)
    let _ = HttpResponse::json(json!({ "ok": true }));

    // text/html; charset=utf-8
    let _ = HttpResponse::html("<h1>Hello</h1>");

    // Raw bytes with an explicit content type - used by JSON:API
    // serialization and any other non-JSON byte body.
    let _ = HttpResponse::bytes_body(b"PNG...".to_vec(), "image/png");

    Ok(HttpResponse::text("done"))
}
```

Two streaming constructors exist for long-lived responses:

- `HttpResponse::sse(stream)` - Server-Sent Events. Wraps a `Stream` of
  `SseEvent` values, sets the four required headers
  (`Content-Type: text/event-stream`, `Cache-Control: no-cache`,
  `Connection: keep-alive`, `X-Accel-Buffering: no`), and keeps the
  connection open until the producing stream ends. See
  [Server-Sent Events](sse.md).
- `HttpResponse::stream_bytes(stream)` - generic chunked response.
  Takes a `Stream<Item = Result<Bytes, Infallible>>`. The error type is
  `Infallible` by design: every producer in the framework turns its own
  errors into a terminal stream message before the stream ends, because
  there is no way to surface a transport-level error to the client
  mid-response.
- `HttpResponse::event_stream(stream, end)` - Laravel's `ResponseFactory::eventStream`.
  Wraps a `Stream` of `sse::StreamedEvent` values, framing each as `event: update` (or its
  own name) plus a configurable terminal frame. See [Server-Sent Events](sse.md).
- `HttpResponse::stream_json(stream)` - Laravel's `ResponseFactory::streamJson`. Wraps a
  `Stream` of any `Serialize` value and flushes it as one incrementally-built JSON array
  instead of buffering the whole collection first. See [Server-Sent Events](sse.md#event_stream-and-stream_json).

### Status, headers, cookies

Every builder returns `Self`, so chain freely:

```rust
use suprnova::{Cookie, HttpResponse, Response};
use serde_json::json;

pub async fn created() -> Response {
    Ok(HttpResponse::json(json!({ "id": 42 }))
        .status(201)
        .header("X-Resource-Id", "42")
        .cookie(Cookie::new("last_id", "42")))
}
```

| Method | Behavior |
|---|---|
| `.status(code)` | Set the HTTP status. Codes outside `100..=599` downgrade to 500 at the wire boundary with a warning log. |
| `.header(name, value)` | Append a header. Duplicates allowed (matches `Set-Cookie` semantics). |
| `.replace_header(name, value)` | Drop any prior occurrences and set one. |
| `.with_headers([(k, v), ...])` | Append many at once. Accepts any `IntoIterator<Item = (K, V)>`. |
| `.without_header(name)` | Remove every occurrence (case-insensitive). |
| `.header_value(name)` | Read back the first-set value. Useful in tests. |
| `.cookie(Cookie)` | Attach one cookie as `Set-Cookie`. |
| `.with_cookies([Cookie, ...])` | Attach many. |
| `.without_cookie(name)` | Schedule a deletion (equivalent to `Cookie::forget(name)`). |

The same chainable setters are available on a `Response` (the
`Result`) through the `ResponseExt` trait, so the macros stay
ergonomic:

```rust
use suprnova::{json_response, Cookie, Response, ResponseExt};

pub async fn list() -> Response {
    json_response!({ "ok": true })
        .status(200)
        .header("X-Total-Count", "42")
        .cookie(Cookie::new("last_query", "list"))
}
```

`ResponseExt` exposes `.status`, `.header`, `.with_headers`,
`.without_header`, `.cookie`, `.with_cookies`, and `.without_cookie`.

### Wire-boundary validation

`HttpResponse::into_hyper` runs two safety filters before handing the
response to hyper:

- **Status range.** Anything outside `100..=599` downgrades to 500 with
  a `tracing::warn!`. This catches `AppError::status(700)` typos at the
  boundary instead of letting non-conformant codes reach the wire.
- **Header CRLF injection.** Every header name and value is validated
  via hyper's own `HeaderName::try_from` / `HeaderValue::try_from`. Any
  rejected header is dropped with a warn log and the response is built
  without it. Attacker-controlled values that get reflected into a
  header (CORS allow-headers, `X-Forwarded-*`, custom debug headers)
  cannot split the response.

Both filters are silent in the success path - you only see them in
logs when something tried to slip through.

## Response macros

Two `Response`-shaped macros exist for the common cases:

```rust
use suprnova::{json_response, text_response, Response};

pub async fn json_handler() -> Response {
    json_response!({ "users": [{ "id": 1, "name": "Alice" }] })
}

pub async fn text_handler() -> Response {
    text_response!("OK")
}
```

Both expand to `Ok(HttpResponse::...)`. Chain `ResponseExt` setters on
either to adjust status, headers, or cookies.

## Cookies

`Cookie::new(name, value)` produces a cookie with secure defaults -
`HttpOnly`, `Secure`, `SameSite=Lax`, `Path=/`. Override per cookie:

```rust
use suprnova::Cookie;
use std::time::Duration;

let session = Cookie::new("session_id", "abc123")
    .http_only(true)
    .secure(true)
    .same_site(suprnova::SameSite::Strict)
    .path("/")
    .domain("example.com")
    .max_age(Duration::from_secs(3600))
    .partitioned(true);
```

Four convenience constructors cover common patterns:

- `Cookie::forget(name)` - empty value, `Max-Age=0`, path `/`, no
  domain. Use this on logout to instruct the browser to drop the cookie.
- `Cookie::forget_with(name, path, domain)` - the scoped form. A browser
  only drops a cookie when the deletion cookie's `Path` and `Domain`
  match the ones it was set with, so a cookie set with `Path=/admin` or
  `Domain=.example.com` survives a plain `forget`. Pass `None` for
  either argument to keep the default.
- `Cookie::forever(name, value)` - five-year `Max-Age`.
- `Cookie::encrypted(name, plaintext)` - writes AES-256-GCM ciphertext
  whose AAD is bound to the cookie's logical name. Read it with
  `Cookie::read_encrypted_for(name, wire)` using the same name.
  `Cookie::read_encrypted(wire)` is the deprecated, un-contexted v1
  reader; it cannot decrypt current `Cookie::encrypted` output and is
  scheduled for removal in 1.4.0 together with the v1 fallback. Requires
  `APP_KEY` to be set at boot. See [Encryption](encryption.md).

Removing several cookies at once - the usual logout shape - is
`without_cookies`, available on `HttpResponse`, on `Response` through
`ResponseExt`, and on both redirect builders:

```rust
use suprnova::{HttpResponse, Redirect};

let _ = HttpResponse::text("bye").without_cookies(["session", "remember"]);
let _: suprnova::Response = Redirect::to("/login")
    .without_cookies(["session", "remember"])
    .into();
```

On a redirect the deletions ride the 302 itself, not the destination, so
the browser has already dropped them by the time it follows the
`Location`.

Header serialization percent-encodes every byte that isn't a valid
cookie-octet per RFC 6265, including all control characters. CRLF in
a cookie name or value gets encoded, not propagated - header injection
through cookies is closed at the serializer.

### Queueing a cookie for later

Sometimes code that isn't building the response still needs to set a
cookie - a listener reacting to an event, a piece of middleware that
runs ahead of the handler, an `App::bind` service with no `HttpResponse`
in scope. `Cookie::queue` is Laravel's `Cookie::queue()`: it stashes the
cookie in a per-request jar that `SessionMiddleware` drains onto the
outgoing response, right after the session cookie.

```rust
use suprnova::Cookie;

Cookie::queue(Cookie::new("theme", "dark"));

// Look up what's queued.
let queued = Cookie::queued("theme");

// Remove it before the response goes out.
Cookie::unqueue("theme");

// Queue a deletion instead of a value - composes with `forget_with`.
Cookie::expire("theme", Some("/app"), None);
```

The jar is task-local and freshly empty for every request - nothing
queued on one request is visible on the next, and a value queued but
never drained (no `SessionMiddleware` in the route's chain) is dropped
rather than panicking. Queued cookies attach to whatever the handler
returns, including a redirect: a handler that queues a cookie and then
returns `Redirect::to(...)` still carries the `Set-Cookie` header on
the 3xx response. They also attach to a 500 that `SessionMiddleware`
builds itself for an internal failure partway through the request - an
existing session that can't be read, a session write that fails, or
session-cookie encryption failing - because a queued cookie can already
represent a side effect committed elsewhere (a remember-me token row
already written, for instance), so the response reporting the failure
still carries it. They do **not** survive a panic - `SessionMiddleware`'s
draining code runs after the handler returns normally, and a caught
panic is converted to a 500 outside the whole middleware chain, the
same point where Laravel's own queued cookies are lost to an uncaught
exception.

### Why Suprnova diverges

Laravel's `CookieJar` keys the queue by name *and* path, so two cookies
with the same name at different paths can be queued independently.
Suprnova keys the jar by name only: queuing a second cookie under a
name already queued replaces the first rather than adding a second
`Set-Cookie` line for it. That covers the common case - one call site
owns a given cookie name - without the extra path-keyed lookup
Laravel's version needs.

## Redirects

`Redirect` covers the full Laravel redirector surface. Every variant
implements `From<Redirect> for Response`, so the idiomatic form is
`Redirect::...().into()`.

### Targets

```rust
use suprnova::{Redirect, redirect_to};

// Explicit URL or path
let _ = Redirect::to("/dashboard");

// Same thing, slightly shorter free function
let _ = redirect_to("/dashboard");

// Named route (returns RedirectRouteBuilder)
let _ = Redirect::route("users.show").with("id", "42");

// Explicit external URL - same as `to`, but the name signals
// "this is going off-site" for open-redirect audits
let _ = Redirect::away("https://external.example.com");

// Refresh the page (reads previous URL from the session; falls back
// to "/" if no session scope is active)
let _ = Redirect::refresh();

// Same, but taking an explicit Request when no scope is active
// let _ = Redirect::refresh_for(&request);

// Session previous_url, with fallback when no session is in scope
let _ = Redirect::back("/login");

// Session-stored intended URL, consumed on read, with fallback
let _ = Redirect::intended("/home");

// Guest redirect: stashes the current request URL as "intended" and
// sends the user to a login page
// let _ = Redirect::guest(&request, "/login");
```

`Redirect::back`, `Redirect::intended`, `Redirect::guest`, and
`Redirect::refresh` all integrate with the session. Without a session
scope they fall through to their defaults silently - handy for
partial test setups. See [Session](session.md).

`Redirect::back`'s target - the session's recorded previous URL - is
never trusted verbatim. The session middleware only records a
root-relative, same-origin URL in the first place (a path starting
with `//` or `/\`, or carrying an ASCII control byte anywhere in it, is
never stored), and the same check runs again on every read, so `back`
can't be steered off-origin either by a request that reaches your app
with an unusual path or by a session cookie written before this guard
existed. See [Session](session.md#other-operations) for the full rule.

### Named-route validation

The `redirect!` proc-macro validates the route name at compile time
and expands to `Redirect::route(name)`:

```rust
use suprnova::{redirect, Response};

pub async fn store() -> Response {
    // Compile fails if "users.index" is not a registered route name;
    // the error message lists available routes and suggests close matches.
    redirect!("users.index").into()
}
```

### Status codes

```rust
use suprnova::Redirect;

let _ = Redirect::to("/x").permanent();      // 301
let _ = Redirect::to("/x").status(303);      // 303, 307, 308, ...
```

The default is 302.

### Flash data

Redirect builders carry their own flash bag. On conversion to a
`Response` the bag drains into the live session, surviving exactly
one more request:

```rust
use suprnova::Redirect;

let _ = Redirect::back("/users/new")
    .with("status", "User created")            // single key/value
    .with_input([                              // repopulate form
        ("email", "shawn@example.com"),
        ("name", "Shawn"),
    ])
    .with_errors([                             // default error bag
        ("email", "Must be unique"),
    ])
    .with_errors_bag("login", [                // named error bag
        ("password", "Required"),
    ]);
```

The receiving page reads these back through `session.get(...)` (for
`with`), `session.get_old_input(...)` (for `with_input`), and the
bag map drained by `session.pull_errors_flash()` (for
`with_errors` / `with_errors_bag`). The Inertia layer consumes the
errors-flash automatically - every Inertia response's `errors` prop
is seeded from the session, so `Redirect::back().with_errors(...)`
surfaces messages on the destination without extra wiring. The
`X-Inertia-Error-Bag` request header scopes the prop under a named
bag for multi-form pages.

Note that on `RedirectRouteBuilder` (what `Redirect::route` and
`redirect!` return), `.with(key, value)` sets a **route parameter**,
not a flash entry - use `.flash(key, value)` there:

```rust
use suprnova::redirect;

let _ = redirect!("users.show")
    .with("id", "42")                          // route param
    .flash("status", "Updated");               // session flash
```

### Cookies, headers, fragments

```rust
use suprnova::{Cookie, Redirect};

let _ = Redirect::route("billing.show")
    .with_cookies([Cookie::new("welcome", "yes")])
    .with_headers([("X-Trace", "abc")])
    .with_fragment("invoices")                 // append #invoices
    .without_fragment();                       // OR strip any prior fragment
```

`with_fragment` accepts the fragment with or without a leading `#`.
Calling `with_fragment` after `without_fragment` re-attaches one.

### Preserve fragment across the redirect

For Inertia apps where the destination should preserve the
*originating* URL hash, use `preserve_fragment`:

```rust
use suprnova::Redirect;

let _ = Redirect::route("dashboard.index").preserve_fragment();
```

On conversion this flashes `_inertia.preserve_fragment = true` into
the session; the next Inertia response reads the flag and emits
`preserveFragment: true` in its page object. No session scope - flag
silently dropped.

### Signed redirects

Two builders wrap the URL-signing surface for one-shot redirects to
named routes (password reset, email verification, download links):

```rust
use suprnova::Redirect;

let r = Redirect::signed_route("downloads.show", &[("id", "42")])?;
let r = Redirect::temporary_signed_route(
    "downloads.show",
    &[("id", "42")],
    1_700_000_000, // expires_at_epoch_seconds
)?;
```

Both return `Result<Redirect, FrameworkError>` - `?`-propagate the
error since `Redirect` converts to a `Response` cleanly. See
[URLs](urls.md) for the signing surface.

### Storing the intended URL

`Redirect::set_intended_url` writes the session's intended target
without performing a redirect - typically called from auth middleware
before redirecting to `/login`, so a later `Redirect::intended` can
recover the originally-requested URL:

```rust
suprnova::Redirect::set_intended_url("/admin/users");
```

## Aborting from a handler

Three free functions short-circuit a handler at a given status. They
return `Result<(), FrameworkError>`; combine with `?`:

```rust
use suprnova::{abort_if, abort_unless, abort_with, json_response, Request, Response};

pub async fn show(req: Request) -> Response {
    abort_unless(Auth::user().await?.is_some(), 401, "must be logged in")?;
    abort_if(req.param("id")? == "0", 404, "User not found")?;
    abort_with(503, "scheduled maintenance")?;
    json_response!({ "ok": true })
}
```

The underlying error is `FrameworkError::Domain { message, status_code }`,
so it renders through the same JSON envelope and 5xx sanitisation rules
as every other error path. Out-of-range status codes are coerced to
500 by the response renderer. See [Error Model](error-model.md) for
the full conversion contract.

## Returning errors directly

Because `Response` is `Result<HttpResponse, HttpResponse>`, you can
return an `Err` arm directly - useful when the response shape is
already a specific JSON body and you want it on the wire as-is:

```rust
use suprnova::{HttpResponse, Response};
use serde_json::json;

pub async fn legacy_lookup() -> Response {
    Err(HttpResponse::json(json!({
        "error": "deprecated endpoint",
    })).status(410))
}
```

For anything richer - typed domain errors, validation, observability -
use the [Error Model](error-model.md) surface (`AppError`,
`FrameworkError`, `#[domain_error]`).

## Quick reference

| Need | Use |
|---|---|
| JSON response | `HttpResponse::json(v)` or `json_response!({...})` |
| Text response | `HttpResponse::text(s)` or `text_response!(s)` |
| HTML response | `HttpResponse::html(s)` |
| Raw bytes + content-type | `HttpResponse::bytes_body(b, "image/png")` |
| Server-Sent Events | `HttpResponse::sse(stream)` - see [SSE](sse.md) |
| Chunked stream | `HttpResponse::stream_bytes(stream)` |
| Set status | `.status(code)` |
| Add header | `.header(k, v)` / `.with_headers([...])` |
| Remove header | `.without_header(name)` |
| Attach cookie | `.cookie(c)` / `.with_cookies([...])` |
| Forget cookie | `.without_cookie(name)` / `.without_cookies([...])` |
| Forget a path/domain-scoped cookie | `Cookie::forget_with(name, Some("/admin"), Some("example.com"))` |
| Queue a cookie for the next response | `Cookie::queue(c)` |
| Look up a queued cookie | `Cookie::queued(name)` |
| Remove a cookie from the queue | `Cookie::unqueue(name)` |
| Queue a deletion cookie | `Cookie::expire(name, path, domain)` |
| Simple redirect | `Redirect::to(path).into()` or `redirect_to(path).into()` |
| Named-route redirect | `redirect!("name").into()` or `Redirect::route("name")` |
| Back redirect | `Redirect::back(fallback)` |
| Intended redirect | `Redirect::intended(default)` |
| Guest redirect (stash intended) | `Redirect::guest(&req, login)` |
| Set intended target | `Redirect::set_intended_url(url)` |
| External URL | `Redirect::away(url)` |
| Refresh current page | `Redirect::refresh()` / `Redirect::refresh_for(&req)` |
| Signed-route redirect | `Redirect::signed_route(name, &[(k, v)])?` |
| Route param on redirect | `.with("key", "value")` |
| Query param on redirect | `.query("key", "value")` |
| Flash data | `.with(key, value)` (or `.flash` on `RedirectRouteBuilder`) |
| Flash input | `.with_input([(k, v), ...])` |
| Flash errors | `.with_errors([(k, msg), ...])` |
| Named error bag | `.with_errors_bag(bag, [(k, msg)])` |
| Append fragment | `.with_fragment("section")` |
| Strip fragment | `.without_fragment()` |
| Preserve fragment (Inertia) | `.preserve_fragment()` |
| Permanent redirect | `.permanent()` (301) |
| Custom redirect status | `.status(303)` |
| Abort early | `abort_with(code, msg)?`, `abort_if(cond, code, msg)?`, `abort_unless(cond, code, msg)?` |

## Next

- [Error Model](error-model.md) - `FrameworkError`, `AppError`,
  `HttpError`, and the single conversion that renders every error to
  an `HttpResponse`
- [Error Handling](errors.md) - practical handler patterns for `?`,
  `AppError`, and custom domain errors
- [Server-Sent Events](sse.md) - building and consuming `sse(...)`
  responses
- [URLs](urls.md) - signed URLs, named-route resolution, the
  surface behind `Redirect::signed_route`
- [Session](session.md) - flash data, intended URLs, the bag
  `Redirect::with`/`with_input`/`with_errors` writes into
