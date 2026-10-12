# HTTP Client

The `Http` facade is the outbound side of HTTP - the Rust equivalent of
Laravel's `Http::` helper. You reach for it when your handler, job, or
scheduled task needs to call somebody else's API: a payment gateway, a
geocoder, a webhook target, a Slack message. Fluent builder, JSON in
and out, retries with jitter, deterministic test fakes that record what
you sent. The same surface you used in Laravel, with task-local
isolation so parallel tests don't see each other's fakes.

```rust
use suprnova::Http;
use serde_json::json;

let resp = Http::post("https://api.stripe.com/v1/charges")
    .bearer_token(secret_key)
    .json(&json!({ "amount": 1000, "currency": "usd" }))
    .send()
    .await?;

let body: serde_json::Value = resp.json().await?;
```

That's the shape: `Http::<verb>(url)` returns a `RequestBuilder`; you
chain configuration onto it; `.send().await` returns a
`ClientResponse`. The backing client is one shared `reqwest::Client`
with rustls TLS, a 30s default timeout, and a `suprnova/<version>` user
agent - built lazily on first call.

## The verbs

```rust
Http::get("https://api.example.com/users/42")
Http::post("https://api.example.com/users")
Http::put("https://api.example.com/users/42")
Http::patch("https://api.example.com/users/42")
Http::delete("https://api.example.com/users/42")
Http::head("https://api.example.com/users/42")
```

Every verb returns a `RequestBuilder`. The URL can be any
`impl Into<String>` - a `&str`, a `String`, or a `Cow<str>`. `head`
sends `HEAD`: the response has the status and headers a `GET` would
have, and no body.

## Building the URL

Three builder methods put the URL together when the request is sent:

```rust
let resp = Http::get("users/{id}/orders")
    .base_url("https://api.example.com/v2")
    .url_parameters([("id", user_id.as_str())])
    .query(&[("page", "2"), ("status", "open")])
    .send()
    .await?;
// GET https://api.example.com/v2/users/42/orders?page=2&status=open
```

- `.base_url(url)` goes in front of a URL that does not start with
  `http://` or `https://`, with one slash between the two. An absolute
  URL is sent as it is, so one client setup can still call another host.
- `.url_parameters(params)` expands each `{name}` in the URL. Every
  character of a value except letters, digits, `-`, `.`, `_` and `~` is
  percent-encoded, so `a/b` becomes `a%2Fb`: a value cannot add a path
  segment, a query, a fragment or a host. A placeholder that no value names
  stays as it is.
- `.query(params)` merges the pairs into the URL's query. A name the URL
  already has takes the new value, and the other names stay. The URL has
  to be absolute by then, base URL included, or `send()` returns an error.

`query` takes a slice of pairs, `&[("name", "value")]`, and
`url_parameters` takes an array of pairs or any iterator of them. Calling
either again adds more.

## Bodies

Four ways to attach a body. Each one replaces any previously-set body.

### JSON

```rust
use serde::Serialize;

#[derive(Serialize)]
struct CreateUser {
    name: String,
    email: String,
}

Http::post("https://api.example.com/users")
    .json(&CreateUser {
        name: "Ada".into(),
        email: "ada@example.com".into(),
    })
    .send()
    .await?;
```

`.json(&value)` accepts anything that implements `serde::Serialize`.
The wire `Content-Type` is set to `application/json` automatically.
If serialization fails (e.g. a map with a non-string key), the
builder records the error and `send()` surfaces it instead of
silently sending a `null` body.

### Form

```rust
Http::post("https://login.example.com/oauth/token")
    .form(&serde_json::json!({
        "grant_type": "client_credentials",
        "client_id": id,
        "client_secret": secret,
    }))
    .send()
    .await?;
```

`.form(&value)` serializes the value as `application/x-www-form-urlencoded`.
The value must serialize to a JSON object; the keys become form fields.
Same body-error semantics as `.json` - a serialization failure surfaces
through `send().await?`, never as a silent empty body.

### Raw bytes

```rust
use bytes::Bytes;

let payload: Bytes = compress(report)?;
Http::post("https://collector.example.com/ingest")
    .header("Content-Type", "application/octet-stream")
    .body(payload)
    .send()
    .await?;
```

`.body(bytes)` takes anything `impl Into<Bytes>`. You're responsible
for the `Content-Type` header - `.body` doesn't set one.

### Multipart

```rust
let avatar: Vec<u8> = std::fs::read("storage/avatar.png")?;
Http::post("https://api.example.com/profile")
    .attach("avatar", avatar, Some("avatar.png"))
    .attach("caption", "Me, in 2026", None)
    .send()
    .await?;
```

`.attach(name, contents, filename)` adds a part to a
`multipart/form-data` body: the bytes under the field `name`, with a file
name when you give one. Each call adds a part. The body is encoded when the
request is sent, with a random boundary in its `Content-Type`.

## Headers and auth

```rust
Http::get("https://api.example.com/private")
    .header("X-Request-Id", request_id)
    .header("Accept", "application/vnd.api+json")
    .bearer_token(api_key)
    .send()
    .await?;
```

`.header(name, value)` appends; the framework doesn't dedupe, so two
calls with the same name send two headers and reqwest joins them per
HTTP semantics. Two shortcuts for the common auth schemes:

- `.bearer_token(token)` - sets `Authorization: Bearer <token>`
- `.basic_auth(user, password)` - sets `Authorization: Basic <b64>`;
  `password` is `Option<&str>` so `.basic_auth("api-key", None)`
  encodes the `api-key:` form some providers want

## Timeouts

The shared client has a 30-second default timeout. Override per-request
when you need to:

```rust
use std::time::Duration;

Http::get("https://slow.example.com/report")
    .timeout(Duration::from_secs(120))
    .send()
    .await?;
```

`.timeout(dur)` sets the total time the request may take, connecting
included. The shared client also gives up on a connection after 10
seconds. `.connect_timeout(dur)` changes that for one request:

```rust
Http::get("https://flaky.example.com/health")
    .connect_timeout(Duration::from_secs(2))
    .timeout(Duration::from_secs(5))
    .send()
    .await?;
```

reqwest fixes the connect timeout when it builds a client, so each distinct
connect timeout gets a client of its own, built on first use and kept. Use a
few fixed values rather than one you compute per request.

## Redirects

The shared client follows redirects by default (up to reqwest's cap of
10) - the right behavior when you're calling a trusted endpoint that
answers `http → https` or hands you a CDN URL.

When the request URL is influenced by untrusted input, that default
becomes a server-side request forgery (SSRF) vector: a hostile endpoint
can answer with a `3xx` whose `Location` points at an internal service or
a cloud-metadata address (`http://169.254.169.254/…`), and a following
client would chase it. Disable redirect-following for those requests with
`.no_redirects()`:

```rust
let resp = Http::get(user_supplied_url)
    .no_redirects()
    .send()
    .await?;

// The 3xx is returned as-is instead of being followed - inspect it and
// reject rather than letting the client chase the Location header.
if (300..400).contains(&resp.status()) {
    return Err(AppError::bad_request("refusing to follow a redirect"));
}
```

`.no_redirects()` routes the request through a separate non-following
client; the default client - and every request that doesn't call it - is
unchanged. This is the general-client analogue of the redirect lockdown
the web-push sender already applies to attacker-controlled push endpoints.

## Global middleware and options

Some settings belong on every request: an `X-App` header, a token that
signs each call, a log line for each response. Register them once, at
boot:

```rust
use std::time::Duration;
use suprnova::{ClientResponse, Http, RequestBuilder};

Http::global_options(|request: RequestBuilder| {
    request.timeout(Duration::from_secs(10))
});
Http::global_request_middleware(|request: RequestBuilder| {
    request.header("X-App", "billing")
});
Http::global_response_middleware(|response: ClientResponse| {
    tracing::debug!(status = response.status(), "outbound response");
    response
});
```

- `Http::global_options(f)` runs `f` on every request as it is created, so
  what the request sets itself comes after and wins where a setting
  replaces, such as a timeout. Headers are appended, so a header set both
  ways is sent twice, the global one first. Calling it again replaces the
  options.
- `Http::global_request_middleware(f)` runs `f` on every request just
  before it is sent, once per `send()`. The request it receives has its
  final URL, which `request.url()` returns; `request.method()` returns the
  method.
- `Http::global_response_middleware(f)` runs `f` on every response before
  your code sees it, each attempt of a retried request included.

Middleware runs in the order you register it. A request takes the global
configuration in force when it is created. `Http::without_global_configuration`
creates requests without any of it:

```rust
let resp = Http::without_global_configuration(|| async {
    Http::get("https://status.example.com/ping").send().await
})
.await?;
```

Only the requests created on the current task inside the closure are left
out; other requests of the process keep their middleware. The mail and
vector drivers of the framework call their providers without the facade,
so global middleware never reaches them.

Registered inside an `Http::fake` scope, global middleware and options
belong to that scope and end with it, so tests running in parallel do not
see each other's. A request inside the scope takes the process-wide
configuration first, then the scope's.

## Retries

`Http` ships exponential-backoff retries with full jitter - the AWS
recipe, the same one Laravel uses. One rule decides every retry, for a
transport failure and for a received 5xx response alike. The two retry
modes differ in the methods they retry: `.retry(...)` retries `GET`, `PUT`
and `DELETE`, and `.retry_non_idempotent(...)` retries `POST` and `PATCH`
as well.

### `.retry(max_attempts, base_backoff)` - retries for idempotent methods

```rust
use std::time::Duration;

let resp = Http::get("https://flaky.example.com/health")
    .retry(4, Duration::from_millis(200))
    .send()
    .await?;
```

`max_attempts` includes the first try, so `retry(4, ...)` retries up
to three times after the initial attempt. The delay before attempt
`n+1` is a uniform random duration in `[0, base_backoff * 2^(n-1)]`,
capped at 30 seconds. Full jitter, not exponential-backoff-plus-fixed-
sleep, so many workers retrying the same outage don't synchronize into
a thundering herd.

`.retry()` retries only idempotent methods (`GET`, `PUT`, `DELETE`): a
transport failure (connect, timeout, DNS) or a 5xx status triggers another
attempt. 4xx and 2xx/3xx responses are returned as-is. After exhausting
retries, the last response or transport error is returned to the caller.

`POST` and `PATCH` are sent once, whatever the failure. A transport failure
on a write can mean the server committed it but the response was lost, so
retrying could apply it twice. Opt in with `.retry_non_idempotent(...)`
when the upstream is protected by an idempotency key.

### `.retry_non_idempotent(...)` - opt-in for POST/PATCH

```rust
Http::post("https://api.example.com/charges")
    .header("Idempotency-Key", idem_key)
    .retry_non_idempotent(3, Duration::from_millis(200))
    .send()
    .await?;
```

When you've supplied an idempotency key the upstream honors, or you've
otherwise made the request safe to replay, switch to
`.retry_non_idempotent(...)`. It retries `GET`, `PUT` and `DELETE` as
`.retry()` does, and it also retries `POST` and `PATCH`, after a transport
failure and after a 5xx response. It still returns 4xx and 2xx/3xx
responses as-is.

### Retry-After is honored on 503

For a `503 Service Unavailable`, the framework respects a `Retry-After`
header - in either delta-seconds (`Retry-After: 30`) or HTTP-date
(`Retry-After: Tue, 15 Nov 1994 08:12:31 GMT`) form. The actual wait
is the larger of the jittered backoff and the `Retry-After` hint,
still capped at 30 seconds. A hostile or misconfigured server returning
`Retry-After: 86400` won't park your task for a day.

### `.retry_when(predicate)` - narrow the policy further

```rust
use std::time::Duration;

let resp = Http::get("https://flaky.example.com/health")
    .retry(4, Duration::from_millis(200))
    .retry_when(|ctx| ctx.method == "GET")
    .send()
    .await?;
```

`retry_when` registers a predicate consulted before every retry the
policy above would otherwise make. It can veto an otherwise eligible
retry, but it cannot manufacture one. In particular, it cannot turn a
2xx, 3xx, or 4xx response into a retry, and it cannot make a received
5xx response retryable for `POST` or `PATCH` without
`.retry_non_idempotent(...)`. It is consulted for transport-error retries
and for 5xx retries by the same rule, and only for an attempt that the
policy would retry: not after the last attempt, and not for a `POST` or
`PATCH` under plain `.retry()`. Without a `.retry(...)` or
`.retry_non_idempotent(...)` policy, a lone `retry_when` has nothing to
veto.

The predicate receives `RetryContext { attempt, method, url, outcome }`,
where `outcome` is `RetryOutcome::TransportError` (send failed before a
response arrived) or `RetryOutcome::Status(n)` (an eligible 5xx
response).

## Reading the response

`ClientResponse` exposes status, headers, and three body-reading
methods. Each body method consumes the response.

```rust
let resp = Http::get("https://api.example.com/users/42").send().await?;

let status: u16 = resp.status();
let etag: Option<String> = resp.header("ETag");

// Pick one - each consumes the response.
let user: User = resp.json().await?;
// let text: String = resp.text().await?;
// let bytes: Bytes = resp.bytes().await?;
```

`.header(name)` is case-insensitive. `.json::<T>()` returns
`Result<T, FrameworkError>` and uses `serde_json` for decoding.
`.text()` enforces UTF-8 and surfaces a `FrameworkError` if the body
isn't valid UTF-8.

### Response body cap

A slow or hostile upstream can otherwise stream an unbounded body into
memory. To protect that, every buffered body read is capped - 25 MiB
by default. Override globally at boot:

```rust
use suprnova::Http;

// Once, somewhere in bootstrap.
Http::set_max_response_bytes(100 * 1024 * 1024); // 100 MiB
```

Or per-request when one call legitimately handles a larger payload:

```rust
let bytes = Http::get("https://example.com/big-export.json")
    .max_response_bytes(500 * 1024 * 1024) // 500 MiB
    .send()
    .await?
    .bytes()
    .await?;
```

A response that declares a `Content-Length` over the cap is rejected
before any body is read; the streaming loop also enforces the cap
against the actual bytes, in case `Content-Length` is absent or lies.

## Escape hatch - raw reqwest

The framework covers the common cases. When you need something we don't
expose - streaming bodies, redirect policy inspection, websocket
upgrades - call `.into_inner()` to unwrap the underlying
`reqwest::Response`:

```rust
let resp = Http::get("https://example.com/big-stream").send().await?;
let raw: reqwest::Response = resp.into_inner()?;
let mut stream = raw.bytes_stream();
while let Some(chunk) = stream.next().await {
    process(chunk?);
}
```

`into_inner()` returns `Err(FrameworkError::internal(...))` when called
on a fake response - there's no underlying `reqwest::Response` in that
case. The response-body cap also no longer applies once you take the
raw response; you own the read from there.

## Testing with `Http::fake`

This is the part you'll use every day. `Http::fake` runs your test body
inside a `tokio::task_local!` scope where every outbound call is
intercepted, captured, and answered with whatever you've queued.

```rust
use suprnova::{Http, fake_response, assert_sent};

#[tokio::test]
async fn creates_a_user_via_api() {
    Http::fake(|| async {
        fake_response(
            "POST",
            "/api/users",
            201,
            serde_json::json!({ "id": 42, "name": "Ada" }),
        );

        let resp = Http::post("https://example.com/api/users")
            .json(&serde_json::json!({ "name": "Ada" }))
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), 201);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["id"], 42);

        assert_sent(|r| r.method == "POST" && r.url.contains("/api/users"));
    })
    .await;
}
```

### Matching canned responses

`fake_response(method, url_substring, status, body)` queues a canned
response. The first outbound request whose method matches
(case-insensitive) and whose URL contains `url_substring` consumes the
canned entry and returns that response. Use method `"*"` to match any
method.

Subsequent matching requests fall through to the next canned entry of
the same shape, or - if none match - return an empty `200 {}`. Queue
one canned response per expected call:

```rust
fake_response("GET", "/v1/customer", 200, json!({ "id": "cus_1" }));
fake_response("GET", "/v1/customer", 200, json!({ "id": "cus_2" }));
// Two GETs to /v1/customer get distinct responses; a third gets 200 {}.
```

### Stubs that stay

Three helpers register a stub that answers for as long as the fake lives.
Each answers with a `FakeResponse`: `FakeResponse::new(status)`,
`FakeResponse::json(status, value)` or `FakeResponse::text(status, body)`,
with `.header(name, value)` and `.body(bytes)` to add to it.

```rust
use suprnova::{FakeResponse, Http};

Http::fake(|| async {
    // Every matching URL, every time.
    Http::fake_url("api.example.com/users/*", FakeResponse::json(200, json!({ "id": 7 })));

    // Decide in code; `None` lets the next stub answer.
    Http::fake_using(|request| {
        request
            .has_header("X-Tenant", "acme")
            .then(|| FakeResponse::new(403))
    });

    // Answered in turn, then the `when_empty` response.
    Http::fake_sequence("api.example.com/jobs*")
        .push(FakeResponse::new(201))
        .push_status(202)
        .when_empty(FakeResponse::new(204));

    // ... code under test ...
})
.await;
```

In a URL pattern `*` matches any run of characters, and a leading `*` is
implied, so `api.example.com/users/*` matches
`https://api.example.com/users/7`. A sequence that runs out without a
`when_empty` response fails the request; `.dont_fail_when_empty()` answers
an empty `200` instead.

A request is answered by a `fake_response` entry that matches first, then
by the stubs in the order you registered them, then by the default
`200 {}`.

### Assertions

```rust
// Pass if at least one recorded request matches.
assert_sent(|r| r.method == "POST" && r.url.contains("/charges"));

// Pass if no recorded request matches.
assert_not_sent(|r| r.url.contains("/refunds"));
```

`RecordedRequest` exposes `method: String`, `url: String`,
`headers: Vec<(String, String)>`, and `body: Option<Vec<u8>>`. The URL is
the one the request is sent to, base URL, URL parameters and query
applied, and the headers are the ones it is sent with: yours, the ones
global middleware added, the `Content-Type` a JSON, form or multipart body
sets, and the user agent. Five helpers read them, names compared without
regard to case:

```rust
assert_sent(|r| {
    r.is_json()
        && r.has_header("X-App", "billing")
        && r.header("authorization").is_some_and(|v| v.starts_with("Bearer "))
});
assert_sent(|r| r.is_form() || r.is_multipart());
```

The predicate runs against every recorded request; assertion failures
print the recorded list with header values and bodies redacted (a
small allowlist of `Content-Type`, `Accept`, and `User-Agent` is shown
in full; everything else is `<redacted>`). That keeps bearer tokens
and webhook payloads out of CI logs even when an assertion blows up.

### Tests run in parallel safely

The fake state lives in a `tokio::task_local!` - every fake scope is
scoped to the task running the test, not the process. Two tests
running concurrently on different tasks each get their own
recorded-requests vec and their own canned-response queue. No shared
mutex, no test ordering, no `#[serial]`.

```rust
#[tokio::test]
async fn first_test() {
    Http::fake(|| async {
        fake_response("GET", "/a", 200, json!({"who": "first"}));
        let _ = Http::get("https://x.test/a").send().await.unwrap();
        assert_sent(|r| r.url.contains("/a"));
        // Sibling test's request to /b is invisible here.
    })
    .await;
}

#[tokio::test]
async fn second_test() {
    Http::fake(|| async {
        fake_response("GET", "/b", 200, json!({"who": "second"}));
        let _ = Http::get("https://x.test/b").send().await.unwrap();
        assert_sent(|r| r.url.contains("/b"));
    })
    .await;
}
```

## The spawned-task gotcha

`tokio::task_local!` is scoped to the current task. Work that goes
through `tokio::spawn` lands on a fresh task and does NOT inherit
the fake - by default, outbound calls from the spawned future hit the
real network. Two helpers address this.

### `Http::fail_on_real_calls()` and `FailOnRealCallsGuard`

Flips a process-global flag that turns any unmatched outbound call
into a `FrameworkError::internal(...)` instead of letting it hit the
network. This is Suprnova's analogue of Laravel's
`Http::preventStrayRequests()` - it catches the exact bug the gotcha
creates.

Use the RAII guard so the flag resets when the test ends, even on
panic:

```rust
use suprnova::FailOnRealCallsGuard;

#[tokio::test]
async fn no_test_makes_a_real_call() {
    let _guard = FailOnRealCallsGuard::install();

    // Any unfaked outbound HTTP call from anywhere inside this test
    // - including from a `tokio::spawn`-ed task - errors with a
    // message naming the URL. No network IO actually happens.
}
```

Nested guards compose correctly: the inner guard's `Drop` restores
the PREVIOUS state, not unconditionally "allowed". So an inner test
helper that installs its own guard inside an outer guarded scope
doesn't disarm the outer guard on the way out.

The flag is process-global by design. The point is catching a
`tokio::spawn`-ed future silently escaping a fake scope and pinging a
real third party from CI. A per-task flag would miss that.

Laravel's names move the same flag. `Http::prevent_stray_requests(true)`
arms it as `fail_on_real_calls()` does, `Http::prevent_stray_requests(false)`
releases it as `allow_real_calls()` does, and
`Http::preventing_stray_requests()` answers whether it is armed.

Inside a fake, `Http::allow_stray_requests(&patterns)` lets a request that
no stub answers reach the network when its URL matches one of the patterns,
even while stray requests are refused. Here `*` matches any run of
characters and no leading `*` is implied:

```rust
Http::prevent_stray_requests(true);

Http::fake(|| async {
    // The local test server answers for real; anything else must be faked.
    Http::allow_stray_requests(&["http://127.0.0.1:*"]);
    // ...
})
.await;

Http::prevent_stray_requests(false);
```

Such a request is still recorded, and goes out with its global middleware.

### `Http::spawn_with_fake_inheritance(future)`

When code under test legitimately spawns a task - a queue worker, a
background syncer, a sub-task - and you want its outbound calls to go
through the parent's fake, swap `tokio::spawn` for
`Http::spawn_with_fake_inheritance`:

```rust
Http::fake(|| async {
    fake_response("GET", "/child", 204, json!({}));

    let handle = Http::spawn_with_fake_inheritance(async {
        // Runs on a NEW task, but the parent's fake state is
        // re-installed in this task's task-local scope. The send
        // is intercepted; the response is the 204 above.
        Http::get("https://child.example.com/child").send().await
    });

    let response = handle.await.unwrap().unwrap();
    assert_eq!(response.status(), 204);

    // Recorded requests from the child show up here - the
    // Arc<Mutex<FakeState>> is shared, not snapshotted.
    assert_sent(|r| r.url.contains("/child"));
})
.await;
```

If no fake scope is active when you call
`spawn_with_fake_inheritance`, it's equivalent to `tokio::spawn` - the
child runs without any fake context. So you can use it
unconditionally in code that's sometimes tested with `Http::fake` and
sometimes not.

### Belt-and-braces in test setup

The two combine. A test that wants to be loudly safe pairs them:

```rust
#[tokio::test]
async fn pays_the_invoice() {
    let _guard = FailOnRealCallsGuard::install();

    Http::fake(|| async {
        fake_response("POST", "/v1/charges", 200, json!({ "id": "ch_1" }));

        // If a typo on the URL or method drifts away from the fake,
        // the request falls through to the guard, which errors out
        // with a message naming the URL - instead of silently
        // returning an empty 200 that hides the mismatch.
        pay_invoice(&invoice).await.unwrap();

        assert_sent(|r| r.url.contains("/v1/charges"));
    })
    .await;
}
```

Without the guard, an URL or method that drifts from the fake silently
falls through to a default `200 {}`, and your test passes despite the
production code calling a different endpoint. With the guard, you
fail loudly on the first mismatch.

## OpenTelemetry trace propagation

When the framework is built with the `otel` feature and a W3C
TraceContext propagator is installed, every outbound `Http::*` request
injects `traceparent` (and `tracestate` when non-empty) into its
headers - so downstream services can continue the trace. No
configuration on the call site; the propagator reads
`opentelemetry::Context::current()` at send time.

Without an active OTel context, no headers are injected and outbound
requests look exactly like they did before. See
[Observability](observability.md) for the propagator setup.

## Why Suprnova diverges

A few divergences from Laravel's `Http::` facade are worth calling
out.

**Task-local fakes instead of a process-global mock store.** Laravel's
`Http::fake()` mutates a process-wide registry; tests serialize on it,
or you accept that parallel runners can race. Suprnova's `Http::fake`
uses `tokio::task_local!` so two tests on two tasks each see their own
fake - no test ordering, no shared mutex. The price is that
`tokio::spawn`-ed work doesn't inherit the fake by default, which is
why `Http::spawn_with_fake_inheritance` and
`FailOnRealCallsGuard` exist. Together they give you the same
"can't accidentally hit production" guarantee that
`Http::preventStrayRequests()` does in Laravel, with stricter scoping.

**Retries default to refusing POST/PATCH.** Laravel's HTTP client
retries any method by default. Suprnova's `.retry(...)` sends `POST` and
`PATCH` once, after a transport failure and after a 5xx response alike.
Use `.retry_non_idempotent(...)` to opt into retries for those methods
only after making the write safe to replay, typically with an idempotency
key the upstream honors.

**Global configuration registered inside a fake belongs to the fake.**
Laravel's tests each get a fresh application, so a test's global
middleware ends with it. Suprnova's process outlives a test, so
`Http::global_*` called inside an `Http::fake` scope register for that
scope only, and `without_global_configuration` leaves out the requests of
the current task rather than emptying the process-wide lists while other
requests run.

**`query` merges into the URL's query.** Guzzle replaces the query string
of the URL with the `query` option. Suprnova keeps the URL's other names,
so `query` adds to a URL that already has one. `url_parameters` expands
only the simple `{name}` form of a URI template.

**Stubs take a `FakeResponse`, and use-once entries answer first.**
Laravel's `Http::fake([...])` takes a response, a status, a string or a
closure in one array. Suprnova has one helper for each:
`fake_url`, `fake_using` and `fake_sequence`. A `fake_response` entry, which
is used up as it answers, is asked before them.

**`retry_when` can only narrow, never widen.** Laravel's `retry()`
`$when` callback fully replaces the "should retry" decision, so it can
retry statuses the framework wouldn't otherwise touch (a 404, say).
Suprnova's `retry_when` only vetoes a retry `.retry(...)` or
`.retry_non_idempotent(...)` already decided to make. It is consulted
for every retry the policy would make, but cannot turn a 2xx, 3xx, or
4xx response into a retry or make a `POST` or `PATCH` eligible under plain
`.retry()`.

## Edge cases and small print

- **`Http::*` is closed.** We deliberately don't expose the
  underlying `reqwest::Client`. To grow the surface, add a method to
  the facade rather than reaching for `reqwest` directly - except via
  the documented `into_inner()` escape hatch on a real response.
- **The shared client is built once and lives forever.** Built lazily
  on first call to any `Http::*` verb, kept in a `OnceLock`. The
  rustls TLS stack and the 30s default timeout are baked in. A request
  with its own `connect_timeout` uses a client kept for that timeout.
- **JSON/form serialization failures fail loudly.** A
  `.json(&unserializable)` builder records the error and `send()`
  returns it as `FrameworkError::internal(...)`. The request never
  goes out - we don't degrade to a `null` body.
- **The 30s retry ceiling is hard.** The backoff math caps at 30
  seconds; the `Retry-After` interpretation caps at 30 seconds; no
  single retry sleep parks a task for longer.
- **Process-global cap is one-shot.** `Http::set_max_response_bytes`
  is a write to a process-global atomic - set it once at boot, then
  override per-request as needed. There's no "reset to default" call.

## Next

- [Mail](mail.md) - outbound email, which uses similar fake / driver
  patterns for tests
- [Notifications](notifications.md) - notification channels including
  web push, all share the same test-fake philosophy
- [Queues](queues.md) - jobs that make outbound HTTP calls, plus the
  `spawn_with_fake_inheritance` pattern for testing workers
- [Testing](testing.md) - `#[suprnova_test]`, `TestContainer`, and the
  rest of the fakes surface
- [Observability](observability.md) - OTel propagator setup that makes
  `traceparent` injection light up
