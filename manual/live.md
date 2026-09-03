# Live

Suprnova Live is the framework's server-driven interaction engine. A Live
component is a Rust struct whose state lives on the server, whose view is an
Askama template, and whose actions run over a signed protocol from a small
browser runtime that morphs the re-rendered HTML in place. There is no
client-side state model to keep in sync, no build tool to install to use the
shipped runtime, and no inline JavaScript in your documents.

This chapter covers the application-facing surface: authoring a component,
registering it, serving documents and islands, the security boundaries every
Live request crosses, uploads, asynchronous updates, assets, testing,
diagnostics, and recovery. Everything here uses only `suprnova::live` and
`suprnova::view`.

## Quick start

A project created by `suprnova new` is Live-ready: it ships `src/live/mod.rs`
with an empty component registry and a `routes()` function, its bootstrap
binds the registry, and `cmd/main.rs` installs the routes. Scaffold a
component, then check it:

```bash
suprnova live:make Counter
suprnova live:check
```

`live:make` writes `src/live/counter.rs` and `templates/live/counter.html`,
registers the component in `src/live/mod.rs`, and prints the next steps.
`live:check` builds your application and proves every registered view against
the integrated checker.

## Authoring a component

```rust
use suprnova::live::{LiveComponent, live};

/// A counter rendered by `live/counter.html`.
#[derive(LiveComponent)]
#[live(name = "app.counter", view = "live/counter.html")]
pub struct Counter {
    /// Current count, exposed to the view.
    #[public]
    count: u64,
}

#[live]
impl Counter {
    /// Increments the counter in response to `live:click="increment"`.
    #[action]
    pub fn increment(&mut self) {
        self.count += 1;
    }
}
```

- `name` is the registered component name. Use a dotted, kebab-case name
  such as `app.counter`; the CLI derives `<package>.<kebab>`.
- `view` is the template identity, relative to the template root.
- `#[public]` fields are rendered and carried in the signed snapshot. `#[model]`
  fields additionally accept browser proposals through `live:model`.
- `#[action]` methods are the only entry points the browser can invoke. They
  receive validated arguments and may return typed outcomes such as a
  redirect or a flash.

Every field type must implement `Default`; a fresh island starts from those
defaults unless a mount hook says otherwise.

## Views

Views are Askama templates. The template root is `templates/` unless an
`askama.toml` names other directories, so `live/counter.html` lives at
`templates/live/counter.html`:

```html
<div>
<p>Count: {{ count }}</p>
<button type="button" live:click="increment">Increment</button>
</div>
```

Directives use the closed `live:` grammar: `live:click`, `live:submit`,
`live:model`, `live:upload`, `live:key`, `live:loading`, and the rest of the
documented set. The checker proves every directive against the component:
an unknown action, an unknown model field, a raw `safe` filter, or an
accessibility violation fails `live:check` with the file, line, and column.

Documents that place islands are ordinary views declared with
`#[suprnova::view]`; the only unescaped value they accept is `TrustedHtml`
through the `trusted_html` filter.

## Registration and bootstrap

`src/live/mod.rs` owns the registry and the routes:

```rust
use suprnova::live::{LiveRegistry, RegistryError};

pub mod counter;

/// Builds the registry of every Live component in this application.
pub fn registry() -> Result<LiveRegistry, RegistryError> {
    let registry = LiveRegistry::builder()
        .register::<counter::Counter>()?
        .build();
    Ok(registry)
}
```

Bind it during bootstrap so the server, the workers, and the `suprnova
live:*` commands see the same components:

```rust
suprnova::App::singleton(crate::live::registry().expect("Live component registry"));
```

The registry is immutable once the runtime assembles. A duplicate component
name or view, or a component whose actions need validation without a
validation port, fails registration with a typed `RegistryError`.

## Routes

`Router::try_live()` installs the reserved namespace exactly once:
`/__live/v1/action`, `/__live/v1/upload`, the `/__live/v1/async/*` control
routes and WebSocket handshake, and the immutable `/__live/v1/assets/*`
routes. Startup fails if an application route can claim `/__live`.

The reserved request routes carry a strict policy: every request needs
session, origin, CSRF, principal, tenant, and rate-limit facts. The framework
records the session and the CSRF proof; your application attaches the rest
with the route guard:

```rust
use std::sync::Arc;
use std::time::Duration;

use suprnova::live::{LiveTenantMiddleware, LiveTenantResolver};
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::{AuthMiddleware, FrameworkError, RateLimitMiddleware, Request, Router, SlidingWindowConfig, async_trait};

pub fn routes(router: Router) -> Result<Router, FrameworkError> {
    let limiter = Arc::new(InMemoryRateLimiter::new());
    router.try_live_with(|guard| {
        guard
            .middleware(AuthMiddleware::optional())
            .middleware(LiveTenantMiddleware::new(Arc::new(SingleTenant)))
            .middleware(RateLimitMiddleware::new(
                limiter,
                SlidingWindowConfig { max_requests: 600, window: Duration::from_secs(60) },
                |request: &Request| format!("live:{}", request.ip().unwrap_or_else(|| "anon".into())),
            ))
    })
}

struct SingleTenant;

#[async_trait]
impl LiveTenantResolver for SingleTenant {
    async fn resolve(&self, _request: &Request) -> Result<Option<String>, FrameworkError> {
        Ok(None)
    }
}
```

Install the routes from the entry point so the runtime and the mount catalog
are ready before the first request:

```rust
Application::new()
    .bootstrap(bootstrap::register)
    .try_routes(|| live::routes(routes::register()))
    .run()
    .await;
```

## Documents and islands

A document route declares its islands once, renders them through
`LiveDocument`, and emits the bootstrap tags:

```rust
use std::collections::BTreeMap;

use suprnova::live::{CanonicalValue, LiveBootstrapOptions, LiveDocument, LiveMount, MountFlags};
use suprnova::view::{AssetSet, DocumentResponseIntent, TrustedHtml, ViewName};
use suprnova::{FrameworkError, HttpResponse, Request, Response, Router, StatusCode};

mod filters {
    pub use suprnova::view::filters::trusted_html;
}

#[suprnova::view(path = "live/page.html")]
struct Page<'a> {
    bootstrap: &'a TrustedHtml,
    counter: &'a TrustedHtml,
}

pub fn install(router: Router) -> Result<Router, FrameworkError> {
    let mount = LiveMount::<Counter>::identity_bound("/dashboard", "counter", "dashboard-counter")?;
    let handler_mount = mount.clone();
    let router: Router = router
        .get("/dashboard", move |request: Request| {
            let mount = handler_mount.clone();
            async move { render(request, &mount).await }
        })
        .middleware(AuthMiddleware::redirect_to("/login"))
        .into();
    router.try_live_mount(&mount)
}

async fn render(request: Request, mount: &LiveMount<Counter>) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let mut document = LiveDocument::from_request(&request)?;
        let counter = document
            .mount(mount, CanonicalValue::Object(BTreeMap::new()), MountFlags::empty())
            .await?;
        let bootstrap = document.bootstrap(LiveBootstrapOptions::esm())?;
        document
            .render(
                ViewName::parse("live/page.html").map_err(|_| FrameworkError::internal("view"))?,
                &Page { bootstrap: bootstrap.html(), counter: counter.html() },
                DocumentResponseIntent::html(StatusCode::OK).map_err(|_| FrameworkError::internal("intent"))?,
                AssetSet::empty(),
            )
            .map_err(FrameworkError::from)
    }
    .await;
    result.map_err(|_| HttpResponse::text("Live document failed").status(500))
}
```

- `LiveMount::public_seed` declares an island any visitor may render; its
  state is a reusable seed promoted to an instance on the first action.
- `LiveMount::identity_bound` declares an island that belongs to the current
  session and principal; the document route must authenticate.
- Mount every island before `bootstrap`, and call `bootstrap` once. The
  bootstrap emits the inert configuration element and the script tags for
  the ESM or classic strategy, adding the upload and asynchronous roles when
  a mounted component needs them and the Stimulus bridge on request.
- The document template places `{{ bootstrap|trusted_html }}` in `<head>` and
  each island where it belongs.

## Security boundaries

Live never bypasses the framework's middleware. What each request needs:

| Fact | Recorded by |
|---|---|
| Session | `SessionMiddleware` |
| Origin and CSRF | `CsrfMiddleware` with origin verification enabled |
| Principal | `AuthMiddleware` on its authenticated branch |
| Tenant | `LiveTenantMiddleware` with your resolver |
| Rate limit | `RateLimitMiddleware` on its allowed branch |

The shipped runtime sends the Live media type and the browser's own
`Sec-Fetch-Site` header; it carries no session token. The CSRF middleware
verifies that proof for every Live request on its own, whatever origin policy
you configure, so a same-origin Live request passes with the stateless CSRF
disposition while a cross-site or header-less request falls back to token
validation and is refused. Ordinary routes keep token validation under the
default policy; using Live relaxes nothing else:

```rust
global_middleware!(CsrfMiddleware::new());
```

Anonymous visitors render public seeds, and they can act on them when the
guard uses `AuthMiddleware::optional()`: a signed-in principal is recorded, an
anonymous visitor continues, and the mount kind decides. A public seed then
promotes for the visitor's own session on the first action, while an
identity-bound island still refuses a request without principal evidence.
With `AuthMiddleware::new()` the guard answers `401` for every anonymous
request before any engine work. Identity-bound islands require a session and
a principal; the tenant is bound into the island's scope whenever your
resolver names one, and a resolver that cannot determine the tenant must
return an error rather than `None`. Every rejection is closed:
a `409` for a stale or tampered snapshot carries no body, and production
messages never include snapshots, tokens, cookies, or rendered HTML.

## Uploads

Declare an upload policy on a model field:

```rust
use suprnova::live::{LiveComponent, UploadPolicy, UploadReplacement, UploadScan, UploadType, live};

fn avatar_policy() -> UploadPolicy {
    UploadPolicy::builder()
        .maximum_files(1)
        .maximum_file_bytes(512 * 1024)
        .replacement(UploadReplacement::RetirePrevious)
        .accept(UploadType::Png)
        .scan(UploadScan::Disabled)
        .finalize_action("save_avatar")
        .build()
}

#[derive(LiveComponent)]
#[live(name = "app.avatar-uploader", view = "live/avatar-uploader.html")]
pub struct AvatarUploader {
    #[model]
    #[upload(policy = avatar_policy)]
    avatar: String,
}

#[live]
impl AvatarUploader {
    #[action]
    pub fn save_avatar(&mut self) {}
}
```

The view binds the field with `<input type="file" live:upload="avatar">`. The
runtime creates, transfers, and completes the upload through
`/__live/v1/upload`; the file waits in quarantine until the declared finalize
action runs, when the framework hands it to your `UploadFinalizer`. Bind the
finalizer, and any scanner or validator, before the runtime assembles:

```rust
App::singleton(LiveUploadHost::new().with_finalizer(Arc::new(AppUploadFinalizer::default())));
```

Uploads are authorized per field and control through the gate. Define the
abilities `live:<component>.upload.<field>.<Control>` for `Create`,
`Reacquire`, `Status`, `Queue`, `BeginTransfer`, `PutChunk`, `Complete`,
`Accept`, `BeginFinalize`, `CommitFinalize`, `Cancel`, `Reject`, `Expire`,
and `Fail`.

A browser that lost its transfer grant reacquires it through a route your
application owns outside the reserved namespace:

```rust
let router: Router = router
    .try_live_upload_reacquisition("/account/uploads/{handle}/reacquire")?
    .middleware(AuthMiddleware::new())
    .into();
```

The route requires the same facts as an action, answers only the session and
principal that created the upload, and returns a fresh grant with the current
transfer state.

## Asynchronous updates

A component declares the streams it listens to; the browser runtime
subscribes over SSE or WebSocket and falls back to polling:

```rust
use suprnova::live::{EventPayloadMetadata, LiveComponent, live};

pub struct ActivityPosted;

impl EventPayloadMetadata for ActivityPosted {
    const NAME: &'static str = "activity.posted";
    const VERSION: u16 = 1;
}

#[derive(LiveComponent)]
#[live(
    name = "app.activity-feed",
    view = "live/activity-feed.html",
    minimum_protocol_version = 2,
    streams(stream(name = "activity", topics("activity"), events(ActivityPosted)))
)]
pub struct ActivityFeed {
    #[public]
    headline: String,
}
```

Define the ability `live:<component>.stream.<name>` for subscribers, then
publish from anywhere in the application:

```rust
let streams = LiveStreams::resolve()?;
streams.event::<ActivityPosted>("activity", LiveEventTarget::Island, payload).await?;
streams.refresh("activity").await?;
```

A refresh tells subscribed islands to fresh-render; an event is delivered to
the island's registered handlers. Polling is the ordinary fresh render: the
island's state catches up whenever a transport is unavailable, but event
payloads published in between are not replayed to their handlers, which the
runtime reports as a degraded stream rather than a current one. A component
that declares exactly one stream gets its island root subscribed for it; a
component with several streams subscribes each through the runtime's
registered calls.

## Assets and no-build use

The framework serves the exact reviewed runtime artifacts at
`/__live/v1/assets/<identity>/<file>` with immutable caching, strong
validators, and integrity attributes in the bootstrap tags. A strict
`script-src 'self'` policy holds because documents contain no inline script.
To publish the same bytes to a CDN or a static directory:

```bash
suprnova live:assets --out public/__live
```

The publication is atomic and refuses to replace a directory whose bytes
differ unless you pass `--replace`.

## Testing

`suprnova::live::testing` prepares a router's runtime and mount catalog for
in-process tests. The application tests in `app/tests/live_*.rs` show the
complete pattern: an in-memory database, a seeded session cookie, the real
global middleware stack, and requests through `handle_request`:

```rust
let router = app::live::routes(app::routes::register())?;
let runtime = prepare_live_router_for_test(&router)?;
App::singleton(runtime.clone());
```

Decode an island's snapshot from its `data-suprnova-live-snapshot` attribute,
post an action with the session cookie and `Sec-Fetch-Site: same-origin`, and
assert on the accepted render. A stale snapshot answers `409` with an empty
body; a missing principal answers `401`.

## Diagnostics and operations

- `suprnova live:check` proves every registered view; `--allow-unproved`
  accepts dynamic structures the checker deliberately makes no claim about.
- `suprnova live:inspect` reports the bound registry, configuration limits,
  installed upload capabilities, assembled runtime services, and the asset
  identity without exposing state or secrets.
- `LiveConfig` bounds request and response bytes and the trusted context
  lifetime; bind a custom one before the runtime assembles.
- Errors carry closed kinds such as `live_document_context_rejected` and
  `invalid_live_bootstrap`; telemetry labels are closed enumerations.

## Recovery

- A `409` tells the runtime to fresh-render the island; the operation is not
  replayed.
- A closed asynchronous transport is retired and the runtime reconnects with
  a new transport generation; a stale generation is refused.
- A session that expires or rotates invalidates identity-bound work; the
  application exposes its sign-in path and the visitor continues from a fresh
  document.

Live runs complete without RenderCache; caching Live documents is a separate
feature with its own chapter when it lands.

## CLI reference

| Command | Purpose |
|---|---|
| `suprnova live:make <name>` | Scaffold a component and its view and register it |
| `suprnova live:check` | Prove every registered view with the integrated checker |
| `suprnova live:inspect` | Report safe runtime, registry, provider, and artifact state |
| `suprnova live:assets --out <dir>` | Publish the reviewed runtime artifacts atomically |
