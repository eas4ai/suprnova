# HTTP Tests

This chapter shows how to test your HTTP surface - routes, middleware,
auth flows, error responses - by driving the framework's request
pipeline through `suprnova::handle_request`. If you've written Laravel
feature tests with `$this->get('/users')` and asserted on
`$response->status()`, this is the Suprnova equivalent: the same
`Router` you mount in production runs in the test, every middleware
fires, the panic boundary still catches, and the response is
byte-for-byte what a real client sees. `suprnova::testing::TestClient`
drives that pipeline for you, with no port and no harness to copy; see
[The test client](#the-test-client).

## The test surface

There are exactly three building blocks:

| Piece | Role |
|---|---|
| `Router` | The routes under test - built the same way as in production |
| `MiddlewareRegistry` | The global middleware stack - also built the same way |
| `handle_request(router, registry, req) -> hyper::Response<…>` | The in-process driver - runs one request end-to-end |

`handle_request` is the same function `Server::run` calls per
request, exposed for tests and embedders. Anything that works in
production works here - the panic-recovery wrapper, the request-id
scope, the Inertia flash-bag scope, the auth request state scope, the
HEAD-body strip, post-response termination. There is no "test mode"
that swaps a quieter pipeline in.

`handle_request_with_peer` is the same call with an explicit
`Option<std::net::IpAddr>` for the connecting peer - useful when you
want to assert on `Request::ip()` resolution without setting up proxy
headers.

## The test client

`suprnova::testing::TestClient` is the one client every HTTP test can
reach for, as Laravel's `$this->get('/users')` is. Build it from the
same router and middleware registry you would pass to `handle_request`,
send requests, and assert on the `TestResponse` each one returns:

```rust
use serde_json::json;
use suprnova::testing::TestClient;
use suprnova::{HttpResponse, MiddlewareRegistry, Request, Router};

#[tokio::test]
async fn greets_and_creates_a_user() {
    let router = Router::new()
        .get("/", |_req: Request| async { suprnova::http::text("hello") })
        .post("/users", |req: Request| async move {
            let body: serde_json::Value = req.json().await.map_err(HttpResponse::from)?;
            Ok(HttpResponse::json(json!({ "created": body["name"] })))
        });
    let client = TestClient::new(router, MiddlewareRegistry::new());

    client.get("/").send().await.assert_ok().assert_see("hello");

    client
        .post("/users")
        .json(&json!({ "name": "Ada" }))
        .send()
        .await
        .assert_ok()
        .assert_json(json!({ "created": "Ada" }));
}
```

`get`, `post`, `put`, `patch`, `delete`, and `send(Method, path)` for any
other method start a `TestRequest`. Before `.send().await`, add to it
with `header(name, value)`, which replaces a header of the same name,
`json(&body)`, `form(&pairs)`, `inertia()`, which sends an Inertia
visit's headers (see [Testing Inertia responses](#testing-inertia-responses)),
and `inertia_version(version)`. A request gets `Host: localhost` unless
you set one.

The client takes care of what a hand-written harness does by hand:

- **No port.** Each request opens an in-memory connection
  (`tokio::io::duplex`): the server half hands the parsed request to
  `handle_request`, and the client half sends it. Both halves run in the
  task that awaits `send()`, so a `TestContainer::fake()` or
  `TestContainer::scope` the test set up is the container the request
  sees.
- **Cookies.** The client keeps every cookie a response sets and sends
  them on its next request, and it drops a cookie a response expires, so
  a session one request starts is the next request's session. A clone of
  the client shares its cookies.
- **The error report.** Each response keeps the `ErrorReport` the
  framework attached, as `TestResponse::from_response` does; see
  [See why a request failed](#see-why-a-request-failed).
- **The session store.** `with_session_store(store, cookie_name)` gives
  every response the store, so `assert_session_has` reads the session
  with nothing attached per response. A request that leaves the session
  unchanged sets no cookie; the lookup then uses the cookie the client
  carries.
- **A timeout.** A request that doesn't answer within 10 seconds panics
  naming its method and path. `timeout(duration)` changes the limit.

A session carried across requests:

```rust
use suprnova::session::{SessionConfig, SessionMiddleware};

let session = SessionMiddleware::new(SessionConfig::default());
let store = session.store();
let client = TestClient::new(router, MiddlewareRegistry::new().append(session))
    .with_session_store(store, "suprnova_session");

client.post("/cart").form(&[("sku", "A-1")]).send().await.assert_redirect(Some("/cart"));
client
    .get("/cart")
    .send()
    .await
    .assert_ok()
    .assert_session_has("cart_sku", "A-1")
    .await;
```

To test the whole application, build the registry the way it does:
call its bootstrap's middleware registration, then pass
`MiddlewareRegistry::from_global()`. The dogfood app's
`app/tests/inertia_test_client.rs` does exactly that.

### Why Suprnova diverges

Laravel's test client hands the request object to the kernel in the
same process. Suprnova's request path takes hyper's `Incoming` body,
which only a hyper connection produces, so the client still speaks
HTTP/1.1, over memory instead of a socket. A request is a builder you
finish with `.send().await` rather than a call that takes the headers as
arguments, because it is asynchronous.

## The hyper body problem

The one wrinkle worth knowing about up front: `handle_request` takes a
`hyper::Request<hyper::body::Incoming>`. `Incoming` is hyper's
internal streaming body type; you cannot construct one with
`Full::new(bytes)` or any of the in-memory body types. It only comes
out of a hyper connection.

There are three clean ways around it:

1. **An in-memory connection** - what `TestClient` does: a
   `tokio::io::duplex` pipe with a hyper server on one end and a hyper
   client on the other. Reach for this first.
2. **TCP loopback** - bind a `127.0.0.1:0` listener, serve one
   accept inside a `service_fn`, send the request through a hyper
   client, and let `Incoming` be produced naturally on the server
   side. Many of the framework's own integration tests still do this.
3. **In-process Request building** - for tests that only need to
   inspect `Request` accessors (headers, route params, IP, JSON
   parsing) without going through routing, use the same TCP-loopback
   capture pattern but with a service that pulls the `Request` out
   into a `oneshot::channel` instead of running it. The
   `framework/tests/http/request_accessors.rs` file has this
   `build_request()` helper verbatim.

All three produce real `Incoming` bodies. The loopback is local,
synchronous in test wall-clock terms (microseconds), and never touches
the network outside `lo`. There is no slower or simpler way that
preserves the contract.

### Why Suprnova diverges

Laravel's `$this->get('/users')` works because PHP's request lifecycle
is "build a `Request` object, dispatch it through the kernel". The
kernel takes the in-memory object directly; there is no body type that
forces a transport. Suprnova's server is built on hyper, and hyper's
body type is opinionated for good reasons (streaming, backpressure,
zero-copy). The test surface inherits that constraint.

What you trade for the constraint is fidelity. Every detail of the
production request path - header parsing, body limits, connection
upgrades - runs the same way in tests. You will never have a test
pass because the test harness skipped a layer the real server runs.

## A first end-to-end test

Here is a complete, working test that mounts a single route, sends a
GET against it, and asserts on the status and body.

```rust
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;

use suprnova::http::text;
use suprnova::{MiddlewareRegistry, Request, Router, handle_request};

async fn spawn_server(
    router: Router,
    middleware: MiddlewareRegistry,
    accepts: usize,
) -> SocketAddr {
    let router = Arc::new(router);
    let middleware = Arc::new(middleware);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");

    tokio::spawn(async move {
        for _ in 0..accepts {
            let Ok((stream, _)) = listener.accept().await else { return };
            let io = TokioIo::new(stream);
            let router = router.clone();
            let middleware = middleware.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req: hyper::Request<Incoming>| {
                    let router = router.clone();
                    let middleware = middleware.clone();
                    async move {
                        Ok::<_, Infallible>(handle_request(router, middleware, req).await)
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, svc)
                    .await;
            });
        }
    });

    addr
}

async fn send_get(addr: SocketAddr, path: &str) -> (u16, Bytes) {
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let io = TokioIo::new(stream);
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(io).await.unwrap();
    tokio::spawn(async move { let _ = conn.await; });

    let req = hyper::Request::builder()
        .method("GET")
        .uri(path)
        .header("Host", "localhost")
        .header("Content-Length", "0")
        .body(Full::new(Bytes::new()))
        .unwrap();

    let resp = tokio::time::timeout(Duration::from_secs(5), sender.send_request(req))
        .await
        .expect("send_get timeout")
        .expect("hyper send_request");
    let (parts, body) = resp.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes();
    (parts.status.as_u16(), bytes)
}

#[tokio::test]
async fn get_root_returns_hello() {
    let router = Router::new().get("/", |_req: Request| async { text("hello") });
    let addr = spawn_server(router.into(), MiddlewareRegistry::new(), 1).await;

    let (status, body) = send_get(addr, "/").await;
    assert_eq!(status, 200);
    assert_eq!(&body[..], b"hello");
}
```

That's the entire shape, and `TestClient` is the same test in two
lines. When a suite needs the socket itself, copy the two helpers per
crate and tune them (multiple accepts, header capture, body capture). The
framework itself uses near-identical helpers in
`framework/tests/cors/middleware.rs`,
`framework/tests/middleware/panic_safety.rs`, and
`framework/tests/auth_flows/email_verified_middleware.rs`.

The `accepts` argument bounds how many connections the accept loop
serves before exiting. One is enough for a single request; bump to
two-or-more when a test exercises post-panic recovery (see
[Testing the panic boundary](#testing-the-panic-boundary)).

## Building a request

Inside `send_get` you saw:

```rust
let req = hyper::Request::builder()
    .method("GET")
    .uri("/users/42")
    .header("Host", "localhost")
    .header("Content-Length", "0")
    .body(Full::new(Bytes::new()))
    .unwrap();
```

That's the canonical shape. A few things worth knowing:

- **`Host` header**. Hyper rejects HTTP/1.1 requests without one. Always
  include it; the value doesn't matter unless your handler keys on it.
- **`Content-Length: 0`**. Match the body. Hyper computes this for you
  with `Full::new(Bytes::new())`, but being explicit reads cleaner in
  tests.
- **Body types**. The client side sends `Full<Bytes>`. The server side
  receives `Incoming`. You only ever build `Full<Bytes>` requests in
  tests; the framework receives them as `Incoming` after hyper's
  per-connection conversion.

A POST with a JSON body:

```rust
let body_bytes = serde_json::to_vec(&serde_json::json!({
    "name": "Alice",
    "email": "alice@example.com"
})).unwrap();

let req = hyper::Request::builder()
    .method("POST")
    .uri("/users")
    .header("Host", "localhost")
    .header("content-type", "application/json")
    .header("content-length", body_bytes.len())
    .body(Full::new(Bytes::from(body_bytes)))
    .unwrap();
```

## Asserting on the response

The response that comes back from `handle_request` is a
`hyper::Response<BoxBody<Bytes, Infallible>>`. Three things you'll
read off it:

```rust
let (parts, body) = resp.into_parts();

// 1. Status.
assert_eq!(parts.status.as_u16(), 200);

// 2. Headers - case-insensitive lookup.
let location = parts.headers.get("location").and_then(|v| v.to_str().ok());
assert_eq!(location, Some("/login"));

// 3. Body - collect into bytes, then parse.
use http_body_util::BodyExt;
let bytes = body.collect().await.unwrap().to_bytes();

// As text:
let text = String::from_utf8_lossy(&bytes);

// As JSON:
let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
assert_eq!(value["message"], "ok");
```

For ordinary error responses that reach the common renderer, the body
shape documented in [Error Model](error-model.md) includes `message`,
optional `errors`, `request_id`, and an optional `debug_message`.
`request_id` is `null` outside a request scope. Three special variants
return before request-id injection: `PrecognitionSuccess` is a bodyless
204, `PrecognitionFailure` is the validation body plus Precognition
headers, and an accidentally HTTP-rendered `AlreadyReported` sentinel
is a generic 500 containing only `message`. Use an ordinary error
response when asserting that request-id middleware ran.

## Fluent response assertions with TestResponse

Building the `(status, headers, body)` triple by hand and asserting on it
piece by piece, as above, is the foundation every harness in this crate
uses. `suprnova::testing::TestResponse` wraps that same triple in a
fluent, Laravel-shaped API, so a test reads like an assertion instead of
a header lookup:

```rust
use suprnova::testing::TestResponse;

let (parts, body) = resp.into_parts();
let bytes = body.collect().await.unwrap().to_bytes();
let headers = parts.headers.iter().map(|(k, v)| {
    (k.as_str().to_string(), v.to_str().unwrap_or_default().to_string())
});

TestResponse::new(parts.status.as_u16(), headers, bytes)
    .assert_ok()
    .assert_header("content-type", "application/json")
    .assert_json(serde_json::json!({ "message": "ok" }));
```

`new()` accepts anything iterable as `(String, String)` header pairs -
a `HashMap<String, String>` (what several existing harnesses already
collect into), a `Vec<(String, String)>`, or `HeaderMap::iter()` mapped
to owned strings - so no harness has to change how it drives a request.

Every assertion returns `&Self`, so they chain: `assert_status`,
`assert_ok`, `assert_redirect(target: Option<&str>)`, `assert_json`
(subset match - extra keys in the body are fine), `assert_json_path`
(dot notation, a numeric segment indexes an array), `assert_json_count`,
`assert_see`, `assert_header`, `assert_cookie`. Assertion failures
panic with an expected/actual excerpt, the same contract as `expect!`
([Testing](testing.md)) - this is a testing surface, not library code,
so the no-panic house rule doesn't apply.

### See why a request failed

When a handler or middleware returns an error, or panics, the client
gets a sanitized body. With debug off, a 5xx says only
`{"message": "Internal Server Error", ...}`. That's the right answer for
a client and no help when a test fails. So the framework attaches an
`ErrorReport` to the response it builds from a failure. The report
holds the error and each of its sources, or a panic's message and the
location it was raised at. It lives in the response's in-process
extensions, and it never adds anything to a header or the body.

A response carries a report when one of these built it:

- A handler or middleware returned a `FrameworkError`, other than
  `PrecognitionSuccess`, which answers a passing dry run.
- The panic boundary caught a panic in a handler or a middleware, on an
  HTTP route or in the middleware of a WebSocket route's upgrade.
- A framework middleware answered a failure with a 5xx of its own:
  `SessionMiddleware` when it can't store the session safely and fails
  closed, or when the cache can't take the session's lock;
  `ThrottleRequestsMiddleware` when its cache fails or it names a
  limiter that nobody defined; `RateLimitMiddleware` and
  `LoginThrottleMiddleware` when they fail closed on a backend error;
  `TimeoutMiddleware` when the request runs past its deadline; and the
  RenderCache middleware's `503` when a provider fails on a route whose
  `FailurePolicy` is `Closed`, and its `500` for a render whose
  transaction the database refused.
- A route the framework ships answered a failure with a 5xx: the payment
  webhook route when its database, the render cache, hydration, or the
  body read fails, and the Live endpoints when the endpoint itself holds
  the error: binding the Live runtime, reading the request body,
  completing an action's response, or loading the asset catalog.

A refusal that a middleware answers on purpose carries no report,
because nothing failed: the `429` a throttle or rate limiter answers a
client over its limit with, the `503` while another request holds the
session's lock, or the `503` of maintenance mode.

With debug on, which is the default when `APP_DEBUG` is unset in the
local, development, and testing environments, a 5xx body built from a
`FrameworkError` also carries
`debug_message`: the error chain as `render_error_chain` renders it.
That's the body debug mode gave before the report existed, and the
report doesn't change it. The field comes from the debug setting; the
report adds nothing to the body in either mode. A test request that is
an Inertia visit, or whose `Accept` header lists `text/html`, gets the
development error page in place of that body, with the report still
attached; see [Error Model](error-model.md#the-development-error-page).

To keep the report, build the `TestResponse` from the response
`handle_request` returns, with `TestResponse::from_response`. It
collects the body for you:

```rust
use std::sync::Arc;

use suprnova::testing::TestResponse;
use suprnova::{MiddlewareRegistry, handle_request};

// `incoming_get_request` builds a real `Incoming` request over an
// in-memory pipe. Copy it from `framework/tests/support/common.rs`.
let req = incoming_get_request("/invoices/42", &[]).await;
let resp = handle_request(Arc::new(router), Arc::new(MiddlewareRegistry::new()), req).await;

TestResponse::from_response(resp).await.assert_ok();
```

When an assertion fails on a response that carries a report, the
failure message ends with it. With debug off, it reads:

```text
assert_ok()
  Expected: 200
  Received: 500
  body: {"message":"Internal Server Error","request_id":"9f1c..."}
  error report:
    posting the invoice failed
    caused by: writing ledger entry 42 failed
    caused by: disk /var/ledger is full
```

For a panic, the report names the panic and where it was raised:
`panicked at src/controllers/ledger.rs:31:9: ledger index page 7 is
unreadable`. To capture the location, the framework wraps the process
panic hook once, and the wrapper calls the hook it replaced. An app
that installs its own hook later, without calling the previous one,
still gets the panic message in the report but not the location.

To inspect the report in a test, call `error_report()`. It returns
`None` for a response that wasn't built from an error:

```rust
let response = TestResponse::from_response(resp).await;
let report = response.error_report().expect("the request failed");

assert!(!report.is_panic());
assert_eq!(report.chain()[0], "posting the invoice failed");
```

`chain()` lists the error's own message and then each source's. Only
an error that keeps its source has more than one link:
`FrameworkError::from_external` and `from_external_with` keep the
wrapped error and its sources, while most other variants, including the
`Domain` error that `FrameworkError::from_http_error` and an `AppError`
become, hold a message and a status only, so their report is that one
line. `is_panic()` and `panic_location()` describe a caught panic.
Middleware can read the same report off an `HttpResponse` with
`HttpResponse::error_report()`.

The Inertia error page and the Inertia validation redirect keep the
report of the response they replace, so an Inertia visit that failed
with a `500` and came back as the `Error` page still says why. The
assertions of `assert_inertia()`, and of an `AssertableInertia` built
with `AssertableInertia::from_response` from an `HttpResponse` that
carries a report, end their failure messages with it too:

```text
AssertableInertia::component("Posts/Index")
  Expected: "Posts/Index"
  Received: "Error"
  error report:
    posting the invoice failed
    caused by: writing ledger entry 42 failed
    caused by: disk /var/ledger is full
```

Ways to end up without a report:

- `TestResponse::new` builds from a `(status, headers, body)` triple,
  which has nowhere to carry one.
- A response read back over the TCP loopback has crossed the wire,
  and the report stays on the server side by design.
- A middleware of your own that builds a new `HttpResponse` in place of
  an error response drops the report, which stays on the response it
  replaced. Change the response you were given, with `header` or
  `status`, and the report stays on it.
- A Live 5xx whose cause the Live stack reduces to a closed error kind
  before the endpoint answers carries none: the action endpoint's
  answers from the Live engine itself, the `unavailable` answers of the
  async publisher, and the `503` kinds of the upload service. Those
  error types carry only the kind, by design, so the cause is in the log
  alone.

### Why Suprnova diverges

Laravel records a test request's exceptions in a process-wide
`LoggedExceptionCollection`, and `TestResponse` appends them to a
failing assertion. `cargo test` runs tests on several threads in one
process, so a process-wide list would hand one test another test's
error. The report rides on the response instead, so each failure
message names only its own request's error.

### `assert_session_has` needs a session store

Every other assertion reads only the wire-level response.
`assert_session_has` can't: server-side session state lives in the
`SessionStore`, not in the response, and by the time a response comes
back over the loopback socket there is no in-process session left to
read. Attach the same store the test's `SessionMiddleware` was built
with, plus its cookie name, and the assertion decrypts the response's
session cookie to find the row itself:

```rust
let response = TestResponse::new(status, headers, body)
    .with_session_store(middleware.store(), "suprnova_session");

response
    .assert_session_has("flash.success", serde_json::json!("Saved!"))
    .await;
```

It's the only `async` assertion, since it's the only one that does I/O;
it still returns `&Self`, so `.await` sits inline and the chain
continues after it.

### Why Suprnova diverges

Laravel's `TestResponse` lives in the same PHP process as the app under
test, so `assertSessionHas` reads `$this->session()` directly - no wire
boundary to cross. Suprnova's tests drive a real hyper connection, so
the session is exactly as opaque to the test as it is to a real
browser: a cookie. `assert_session_has` earns that honesty back with an
explicit store handle instead of pretending the in-process shortcut
exists.

## Testing Inertia responses

`suprnova::testing::AssertableInertia` wraps an Inertia page object in
the same fluent, panic-on-failure style as `TestResponse`. It is
Laravel's `Inertia\Testing\AssertableInertia`.

`TestResponse::assert_inertia()` reads the page from either shape a page
response takes: the JSON page object of an Inertia visit, which carries
`X-Inertia: true`, or the HTML document of a first visit, whose
`<script>` element with `type="application/json"` and `data-page` holds
the page whatever id `InertiaConfig::mount_id` gave it (`app` by
default). The two attributes can come in either order, with others
between them, so the document of a server-rendered first visit reads the
same way: Inertia's `buildSSRBody` writes it as
`<script data-page="app" type="application/json">`. The
`inertia()` request method sends the headers an Inertia visit sends:
`X-Inertia: true`, its `Accept`, and `X-Inertia-Version` set to the
installed configuration's asset version, or the empty string with none
installed.

```rust
use suprnova::testing::TestClient;

let client = TestClient::new(router, MiddlewareRegistry::new());

// An Inertia visit: the JSON page object.
client
    .get("/users")
    .inertia()
    .send()
    .await
    .assert_inertia()
    .component("Users/Index")
    .url("/users")
    .has("users")
    .where_("users.0.name", "Ada")
    .count("users", 1)
    .missing("admin_only_field");

// A first visit: the HTML document.
client.get("/users").send().await.assert_inertia().component("Users/Index");
```

`assert_inertia_with(callback)` runs the callback over the page and
returns the response, so response assertions chain after the page's:

```rust
client
    .get("/users")
    .inertia()
    .send()
    .await
    .assert_inertia_with(|page| {
        page.component("Users/Index").has("users");
    })
    .assert_ok();
```

For a test that drives the response pipeline without a request,
`AssertableInertia::from_response` reads the same two shapes from an
`HttpResponse`, the type `InertiaResponse::resolve` returns:

```rust
use suprnova::testing::AssertableInertia;

let response = InertiaResponse::new("Users/Index")
    .with("users", users_json)
    .resolve(&req)
    .await?;

AssertableInertia::from_response(&response)
    .component("Users/Index")
    .where_("users.0.name", "Ada");
```

`version()` checks the page's asset version. The default resolver
hashes the configured asset URL or the Vite manifest, and the version is
the empty string when neither exists, as in a test that hasn't built a
frontend:

```rust
response.assert_inertia().version("");
```

### The page file behind a component

With an Inertia configuration installed (`Inertia::install`),
`component(name)` also checks that `name` has a page file under the
configuration's `pages_dir` with one of its `page_extensions`, the
lookup `InertiaConfig::ensure_pages_exist` does at render time. A test
asserting a component nobody built fails with
`Inertia page component file [Name] does not exist.`, the directory, and
the extensions it tried, instead of passing while the browser shows a
blank page. With no configuration installed there is no directory to
look in, and nothing is checked.

`InertiaConfig::testing_ensure_pages_exist(false)` turns the check off,
and `component_exists(name, should_exist)` decides it for one
assertion: `false` compares the name only, and `true` checks the file
even when the configuration turned the check off.

```rust
page.component_exists("Reports/Draft", false); // no page file yet, on purpose
```

### Asserting on props

Every prop assertion takes a dot path, where a numeric segment indexes
an array (`"users.0.name"`), and returns `&Self`:

| Assertion | Passes when |
|---|---|
| `has(path)`, `has_all(paths)`, `has_any(paths)` | The prop exists; every one exists; at least one exists |
| `missing(path)`, `missing_all(paths)` | No prop exists there; at none of them |
| `where_(path, value)`, `where_not(path, value)`, `where_all(pairs)` | The prop equals the value; exists and differs; every pair matches |
| `where_null(path)`, `where_not_null(path)` | The prop exists and is `null`; exists and is not |
| `where_type(path, types)`, `where_all_type(pairs)` | The prop has one of the types, joined by `\|` (`"integer\|null"`) |
| `where_contains(path, value)` | An array prop holds the value, or each value of an array; any other prop equals it |
| `count(path, n)`, `count_between(path, min, max)` | The array or object has `n` elements; from `min` to `max` |

`where_type` takes the names Laravel compares PHP's `gettype` against:
`string`, `integer` (a number without a fraction), `double` (a number
with one), `boolean`, `array` (a JSON array or object), and `null`. Any
other name fails the assertion rather than matching nothing.

`prop(path)` reads a value without asserting, `Null` for a path that
resolves to nothing.

### Scoping into nested props

`scope(path, callback)` runs the callback over the object or array at
`path` as its own `AssertableInertia`. `has_with(path, callback)` asserts
the prop exists first. `first(callback)` and `each(callback)` scope onto
the first element and onto every element of the current level, and fail
when there is none. `has_count_with(path, n, callback)` asserts the
count, then scopes onto the first element:

```rust
response.assert_inertia_with(|page| {
    page.component("Users/Show")
        .scope("user", |user| {
            user.where_("name", "Ada")
                .where_type("id", "integer")
                .missing("password")
                .etc();
        })
        .has_count_with("posts", 2, |post| {
            post.where_type("id", "integer").has("title");
        });
});
```

A scope fails when its callback returns with a prop no assertion
touched, and names the props. Every assertion touches the first segment
of its path. That is how a test notices a page that starts sending a
field nobody asserted on, such as a user's email. Call `etc()` in a
scope to allow the props you didn't name. The page's top level never
checks. A failure inside a scope names the full path (`user.name`), and
a scope keeps the page's component, url, version, and flash.

### Flash data, the whole page, and big integers

`has_flash(key, expected)` reads the page's flash data the same dot-path
way `has` and `where_` read props. `expected` is an `Option`, so pass
`None::<serde_json::Value>` to check presence only. `missing_flash(key)`
asserts a key is absent.

```rust
let page = response.assert_inertia();
page.has_flash("toast.message", Some(serde_json::json!("Saved!")))
    .has_flash("toast", None::<serde_json::Value>)
    .missing_flash("error");
```

A handler that flashes and redirects (`Inertia::flash`) leaves the data
in the session for the page after the redirect, and the redirect itself
carries no page. `assert_inertia_flash(key, expected)` and
`assert_inertia_flash_missing(key)` read it from the session through the
store the client was given, as `assert_session_has` does:

```rust
let saved = client.post("/posts").form(&[("title", "Hello")]).send().await;

saved
    .assert_redirect(Some("/posts"))
    .assert_inertia_flash("toast", Some("Saved"))
    .await
    .assert_inertia_flash_missing("error")
    .await;
```

`TestResponse::inertia_page()` returns the whole page as a
`serde_json::Value` (`component`, `props`, `url`, `version`, `flash`,
and `encryptHistory` and `clearHistory` only when the page set them),
which `AssertableInertia::to_page()` returns too. `inertia_props(None)`
returns the props, and `inertia_props(Some("user.name"))` returns one
value, `Null` when the path resolves to nothing.
`encrypt_history()` and `clear_history()` read the two flags.

A page rendered with `preserve_big_integers(true)` sends every integer
beyond JavaScript's safe range (2^53 - 1) as `{"$bigint": "<digits>"}`.
The assertions decode the markers first, so you compare against the
integer the handler rendered:

```rust
response.assert_inertia().where_("id", 9007199254740993_i64);
```

A marker whose digits fit no 64-bit integer (`i64` or `u64`) fails the
assertion, naming the marker's path and digits, such as `props.user.id`
and `18446744073709551616`. The framework marks only integers it holds in
64 bits, so such a marker comes from a page object built by hand.

### Reloading for partial-reload and deferred-props assertions

A page from a `TestClient` response reloads through that client, with
its cookies, the way the Inertia client reloads after the first visit.
Each reload asserts it landed on the same component, url, and version,
and returns the reloaded page, which reloads again the same way:

```rust
let page = client.get("/users").inertia().send().await.assert_inertia();

// A full reload: no partial-reload header.
page.reload().await.has("users");

// Requests only `users`, and asserts it came back.
page.reload_only(["users"]).await;

// Requests everything except `stats`, and asserts it is absent.
page.reload_except(["stats"]).await;

// Requests the props of the `stats` deferred group only.
page.load_deferred_props_of(["stats"]).await.has("stats");

// Requests every deferred group in one partial reload.
page.load_deferred_props().await;
```

`reload_with`, `reload_only_with`, `reload_except_with`, and
`load_deferred_props_with` take a callback over the reloaded page:

```rust
page.load_deferred_props_with(["stats"], |reloaded| {
    reloaded.where_("stats.total", 25);
})
.await;
```

`load_deferred_props_of` fails naming a group the page doesn't defer, so
a typo can't request nothing and pass.

Under a public path prefix, such as `APP_URL=https://example.test/billing`,
the page's url is the public one, `/billing/users`, while the router
matches the path the request arrived on, `/users`. The client keeps the
root each request was served under, and a reload replays the internal
path, `/users` with the page's query, so it reaches the route the first
visit did. A reload also sends again the `X-Forwarded-Prefix` header the
page's request sent, if any. A `ReloadRequest` carries the public url, so
a `with_reload` harness removes the root itself.

A test that drives requests through its own harness attaches the replay
with `with_reload`, a closure from a `ReloadRequest` (the url,
component, version, and partial-reload keys to send) to a future that
produces the reloaded page. It replaces the client's own on a client
response too:

```rust
let page = TestResponse::new(status, headers, body)
    .assert_inertia()
    .with_reload(move |reload| async move {
        let header_pairs = reload.headers();
        let headers: Vec<(&str, &str)> = header_pairs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let (status, headers, body) = request(addr, "GET", &reload.url, &headers).await;
        TestResponse::new(status, headers, body).assert_inertia()
    });

page.reload_only(["users"]).await;
```

`ReloadRequest::headers()` sends `X-Inertia` and `X-Inertia-Version`
always, and `X-Inertia-Partial-Component` with `X-Inertia-Partial-Data`
or `X-Inertia-Partial-Except` only when the reload names keys. A page
with neither a client nor `with_reload` panics on a reload with that
instruction.

### Why Suprnova diverges

- Laravel's optional callback arguments are `_with` siblings here:
  `assert_inertia_with`, `reload_with`, `reload_only_with`,
  `reload_except_with`, `load_deferred_props_with`, `has_with`, and
  `has_count_with`. Laravel's `component($name, $shouldExist)` is
  `component_exists`, and `loadDeferredProps($groups)` is
  `load_deferred_props_of`. Rust has no optional arguments.
- `where` is a Rust keyword, so the assertion is `where_`. The
  assertions drop the `assert_` prefix of `TestResponse`'s, as Laravel's
  `AssertableInertia` does; the panic-on-failure contract is the same.
- The page's top level never checks that every prop was touched.
  Laravel's `assertInertia` doesn't either, where `AssertableJson` used
  through `assertJson(fn)` does; scopes check in both.
- A reload returns the reloaded page, where Laravel's returns the
  original, so you can chain assertions and further reloads off it.
- `load_deferred_props_of` fails for a group the page doesn't defer,
  where Laravel requests no props and passes.
- `assert_inertia_flash` is `async` and reads the session through the
  attached store, since the session lives behind the server, not in the
  test's process memory.

## Testing middleware

Middleware tests look identical to route tests; the only difference
is what you `.append()` to the registry before spawning.

### Testing global middleware

Pass the middleware to `MiddlewareRegistry::new().append(...)` and
use that registry - multiple middlewares run in append order,
`prepend` puts a new one at the front.

```rust
use suprnova::{CorsConfig, CorsMiddleware, MiddlewareRegistry};

fn cors_registry() -> MiddlewareRegistry {
    MiddlewareRegistry::new().append(CorsMiddleware::new(
        CorsConfig::allow_origins(["https://app.example"])
            .allow_credentials(true)
            .max_age(std::time::Duration::from_secs(600)),
    ))
}

#[tokio::test]
async fn cors_preflight_returns_204_with_headers() {
    let router = Router::new();
    let addr = spawn_server(router, cors_registry(), 1).await;

    let (status, headers, _) = options(
        addr,
        "/anything",
        &[
            ("Origin", "https://app.example"),
            ("Access-Control-Request-Method", "POST"),
        ],
    ).await;

    assert_eq!(status, 204);
    assert_eq!(
        headers.get("access-control-allow-origin").map(String::as_str),
        Some("https://app.example"),
    );
}
```

This test proves more than the CORS logic itself: it proves that
global middleware runs on **unrouted** requests too, which is the
contract the framework guarantees (otherwise an OPTIONS preflight that
never matches a route would skip CORS). See `framework/tests/cors/middleware.rs`
for the full suite.

### Testing route-specific middleware

Attach with `.middleware(...)` on the route builder, exactly like
production. Then test the route as normal - the middleware chain is
built off the same registration.

```rust
let router = Router::new()
    .get("/admin/dashboard", |_req| async { text("admin") })
    .middleware(RequireRole::new("admin"));

let (status, _) = send_get(addr, "/admin/dashboard").await;
assert_eq!(status, 403); // unauthenticated request
```

### Stubbing the authenticated user

Real auth-flow tests need a logged-in user. The cleanest pattern is a
tiny one-off middleware that calls `Auth::set_user` ahead of the
middleware under test. The framework's own
`framework/tests/auth_flows/email_verified_middleware.rs` uses this:

```rust
use std::any::Any;
use std::sync::Arc;
use suprnova::{Auth, Authenticatable, Middleware, Next, Request, Response};

struct UserById(String);

impl Authenticatable for UserById {
    fn get_auth_identifier(&self) -> String { self.0.clone() }
    fn as_any(&self) -> &dyn Any { self }
}

struct LoginAs(String);

#[async_trait::async_trait]
impl Middleware for LoginAs {
    async fn handle(&self, request: Request, next: Next) -> Response {
        Auth::set_user(Arc::new(UserById(self.0.clone())));
        next(request).await
    }
}
```

Then in the test:

```rust
let registry = MiddlewareRegistry::new()
    .append(LoginAs("user-id-123".to_string()))
    .append(EnsureEmailVerifiedMiddleware::new());
```

`LoginAs` runs first, installs the user into the per-request auth
state, and the middleware under test sees `Auth::id() == Some(...)`
without ever issuing a real login. The auth state scope is set up by
`handle_request` itself - the same one that runs in production - so
the user is visible to every later middleware and the handler.

## Testing route model binding

A `#[suprnova::model]` struct binds from the route parameter its argument
names. Drive the binding through a router, so the router's checks and the
handler's extraction both run:

```rust
use suprnova::{Response, handler};

#[suprnova::model(table = "users")]
pub struct User {
    pub id: i64,
    pub email: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[handler]
async fn show(user: User) -> Response {
    suprnova::http::json(serde_json::json!({ "email": user.email }))
}

#[tokio::test]
async fn show_user_binds_from_the_route() {
    // Insert a test user via the model. Database setup omitted -
    // see the testing chapter for `TestDatabase` patterns.
    let user = User::create(suprnova::attrs! {
        email: "bound@example.com"
    }).await.unwrap();

    // The placeholder carries the argument's name.
    let router: Router = Router::new()
        .get("/users/{user}", show)
        .into();
    router.prepare_bindings().expect("the route declares `user`");

    let addr = spawn_server(router, MiddlewareRegistry::new(), 1).await;
    let (status, body) = send_get(addr, &format!("/users/{}", user.id)).await;

    assert_eq!(status, 200);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["email"], "bound@example.com");
}
```

`router.prepare_bindings()` runs the router's startup checks and returns
the refusal as an error, so a test can assert that a route whose handler
reads an undeclared parameter, such as `/users/{id}` with `user: User`,
is refused. A router driven through `handle_request` runs the same checks
before its first request and answers 500 when they fail.

For binding-in-isolation tests, call
`<User as suprnova::RouteBinding>::resolve_route_binding(value, None)`
directly. It returns `Ok(None)` for a value that does not parse or
matches no row, and checks the lookup without a router.

## Testing auth flows end-to-end

To test a login session end to end, pass a registry containing
`SessionMiddleware` to the loopback server and protect `/dashboard`
with `AuthMiddleware` or the application's web-auth middleware. First
prove the route rejects a cookieless request, then log in, replay the
returned session cookie, and prove the protected route succeeds:

```rust
#[tokio::test]
async fn login_flow_issues_session_cookie() {
    // 1. Bootstrap: create the user.
    Auth::password()
        .register("alice@example.com", "longpassword123")
        .await.expect("register");

    // 2. Mount a protected route and the stateful session middleware.
    let router: Router = Router::new()
        .post("/login", login_handler)
        .get("/dashboard", |_req: Request| async { text("dashboard") })
        .middleware(AuthMiddleware::new())
        .into();
    let registry = MiddlewareRegistry::new()
        .append(SessionMiddleware::new(SessionConfig::from_env()));
    let addr = spawn_server(router, registry, 3).await;

    // 3. Prove the route is protected before authenticating.
    let (guest_status, _) = send_get(addr, "/dashboard").await;
    assert_eq!(guest_status, 401);

    // 4. Drive login and capture the Set-Cookie header.
    let login = post_json(addr, "/login", serde_json::json!({
        "email": "alice@example.com",
        "password": "longpassword123",
    })).await;
    assert_eq!(login.status, 200);
    let cookie = extract_session_cookie(&login.headers);

    // 5. Replay the cookie against the protected route.
    let (status, body) = get_with_cookie(addr, "/dashboard", &cookie).await;
    assert_eq!(status, 200);
    assert_eq!(&body[..], b"dashboard");
}
```

The abbreviated router without those middlewares demonstrates cookie
plumbing only; it is not an authentication-flow test.
`framework/tests/auth/http_middleware.rs` tests authentication
middleware behavior with explicit registries, but it does not install a
real `SessionMiddleware`. A stateful login-flow test must install both
the session middleware and the authentication gate as shown above.

## Testing the panic boundary

A panic inside a handler must not crash the server. The
panic-recovery wrapper (`execute_chain_safely`) catches it and
converts to a 500 through the same path returned errors flow through.
You can verify this without any special test infrastructure - set
`accepts >= 2` so the listener survives the panic:

```rust
#[tokio::test]
async fn panicking_handler_yields_500_and_server_survives() {
    let router = Router::new()
        .get("/panic", |_req: Request| async {
            panic!("intentional test panic");
            #[allow(unreachable_code)] text("unreachable")
        })
        .get("/ok", |_req: Request| async { text("ok") });

    let addr = spawn_server(router.into(), MiddlewareRegistry::new(), 4).await;

    // First: the panic translates to a sanitised 500.
    let (s1, body) = send_get(addr, "/panic").await;
    assert_eq!(s1, 500);
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed["message"], "Internal Server Error");
    assert!(parsed.get("request_id").is_some());

    // Second: the listener survives. The next request is normal.
    let (s2, body2) = send_get(addr, "/ok").await;
    assert_eq!(s2, 200);
    assert_eq!(&body2[..], b"ok");
}
```

## Testing accessors without going through routing

Sometimes you want to test a `Request` accessor (`bearer_token`,
`is_method`, `ip`, `is_json`, etc.) without spinning up a router at
all. The trick is a tiny harness that runs a hyper service whose only
job is to construct the `Request` and ship it back through a
`tokio::sync::oneshot::channel`:

```rust
let (req_tx, req_rx) = tokio::sync::oneshot::channel::<suprnova::Request>();
// ... loopback hyper service whose service_fn does:
//     let req = suprnova::Request::new(hyper_req);
//     let _  = req_tx.send(req);
//     return a 200 with an empty body
let req = req_rx.await.unwrap();
```

`framework/tests/http/request_accessors.rs` has the full
`build_request(builder, body) -> Request` helper. Copy it once per
crate and every accessor test reads cleanly:

```rust
#[tokio::test]
async fn bearer_token_extracts_simple_token() {
    let req = build_request(
        hyper::Request::builder()
            .method("GET")
            .uri("/api/users")
            .header("Authorization", "Bearer secret-token-123"),
        "",
    ).await;
    assert_eq!(req.bearer_token().as_deref(), Some("secret-token-123"));
}
```

The Request is real (produced by hyper from a real wire exchange), but
no routing or middleware ran - exactly what you want when the unit
under test is the accessor itself.

## Builder hooks on `Request`

When you have a `Request` in hand and need to fake one piece of the
routing layer, three builder methods help:

```rust
impl Request {
    pub fn with_params(mut self, params: HashMap<String, String>) -> Self;
    pub fn with_route_pattern(mut self, pattern: String) -> Self;
    pub fn with_peer_addr(mut self, addr: std::net::IpAddr) -> Self;
}
```

These are the same methods the server calls when it dispatches a
matched route - `Router` calls `with_params` after `matchit`
returns, `with_route_pattern` so `req.route_pattern()` resolves, and
`with_peer_addr` once it knows the accepted-TCP socket's IP. In
tests you call them yourself to short-circuit the same setup.

```rust
let req = Request::new(hyper_req)
    .with_params(HashMap::from([("id".into(), "42".into())]))
    .with_route_pattern("/users/{id}".into())
    .with_peer_addr("192.168.1.10".parse().unwrap());

assert_eq!(req.param("id").unwrap(), "42");
assert_eq!(req.ip(), Some("192.168.1.10".parse().unwrap()));
```

## Things to know

A short list of footguns that catch first-time authors:

- **`Incoming` is server-side only.** You cannot build one in your test.
  A hyper connection (`TestClient`'s in-memory one, the TCP loopback,
  or in-process service capture) is the only path - there is no "build
  a `Request` from a `Vec<u8>` body" constructor.
- **Don't share state between tests.** Each `#[tokio::test]` gets its
  own runtime; cross-test pollution usually means you're sharing a
  global (`once_cell`, `lazy_static`, env var). For DB state see
  `TestDatabase` in [Testing](testing.md).
- **Cookies need a client that keeps them.** `TestClient` carries them
  from one request to the next. A hand-written harness has no cookie
  jar - thread `Set-Cookie` from one response into `Cookie` on the
  next. See `framework/tests/auth/http_middleware.rs` for the pattern.
- **The post-response termination spawn is non-blocking.** If you
  want to assert on side effects that run via `Terminable`, poll
  for them - the response returns to the client before the hook runs.

## Where each piece lives

| Piece | File |
|---|---|
| `handle_request`, `handle_request_with_peer` | `framework/src/server.rs` |
| `Request::new`, `with_params`, `with_route_pattern`, `with_peer_addr` | `framework/src/http/request.rs` |
| `MiddlewareRegistry::new`, `append`, `prepend` | `framework/src/middleware/registry.rs` |
| `TestClient`, `TestRequest` (in-memory connection, cookies, reloads) | `framework/src/testing/client.rs` |
| Loopback test harness (for a test that needs the socket) | `framework/tests/cors/middleware.rs` |
| `TestResponse` (fluent assertions over the triple above) | `framework/src/testing/response.rs` |
| `ErrorReport` (what went wrong, kept in process) | `framework/src/error/report.rs` |
| `AssertableInertia`, `ReloadRequest` (fluent Inertia page-object assertions) | `framework/src/testing/inertia.rs` |
| In-process `Request` capture harness | `framework/tests/http/request_accessors.rs` |
| Panic-boundary test pattern | `framework/tests/middleware/panic_safety.rs` |
| Auth + middleware end-to-end pattern | `framework/tests/auth_flows/email_verified_middleware.rs` |

## Next

- [Testing](testing.md) - `#[suprnova_test]`, `TestDatabase`, the
  `describe!`/`test!`/`expect!` macros, and the unit-level surface
- [Error Model](error-model.md) - the JSON shape every error response
  uses, the 5xx sanitisation rule, and what `request_id` means in a
  test body
- [Middleware](middleware.md) - writing the middleware you test here,
  and the global-vs-route lifecycle
- [Routing](routing.md) - the `Router` you mount in both production
  and tests, route params, route names, signed URLs
- [Authentication](authentication.md) - the `Auth` facade,
  `Authenticatable`, guards, and how `Auth::set_user` interacts with
  the request scope `handle_request` installs
