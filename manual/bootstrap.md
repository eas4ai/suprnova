# Application Bootstrap

`bootstrap.rs` is the one place where your application wires itself up
at startup. Container bindings, event listeners, observers, supervisors,
global middleware - anything that should exist before the first request
hits the server (or the first job pops off the queue) is registered
here. There is no service-provider scaffold to assemble.

There are two hooks, not one. `register` is process-wide: every
subcommand runs it, including `queue:work`, `schedule:work`,
`workflow:work`, and your console binary, not only the server. Register
the database connection, container bindings, event listeners, observers,
supervisors, and worker job registration there. `register_http_stack`,
wired through `.http_bootstrap`, runs only on the server path (`serve` /
`web:run`) - global middleware and `Inertia::install` belong there. The
"Where bootstrap sits in the boot order" section below explains why the
split exists.

## The shape

A scaffolded app's entry point builds an [`Application`](lifecycle.md)
fluently and runs it. Bootstrap is two methods on the builder:

```rust
// cmd/main.rs
use app::{bootstrap, config, migrations, routes};
use suprnova::Application;

#[suprnova::main]
async fn main() {
    Application::new()
        .config(config::register_all)
        .bootstrap(bootstrap::register)
        .http_bootstrap(|| async { bootstrap::register_http_stack() })
        .routes(routes::register)
        .migrations::<migrations::Migrator>()
        .run()
        .await;
}
```

### `#[suprnova::main]`, not `#[tokio::main]`

The attribute is not cosmetic, and swapping it back breaks the boot with
a message explaining why.

Loading `.env` writes to the process environment, and `set_var` is sound
only while the process is single-threaded. `#[tokio::main]` builds the
runtime *around* the whole of `main`, so every worker thread already
exists before your first statement runs - and any of them can call
`getenv` indirectly through DNS resolution, time formatting, or a C
dependency. The race is silent when it goes wrong, which is the worst
property a race can have.

`#[suprnova::main]` keeps the same `async fn main` you would write
anyway, and simply reorders two things: it loads the environment, then
builds the runtime, then runs your body on it. It accepts the same
`flavor` and `worker_threads` arguments as `#[tokio::main]`.

If `Application::run` finds the environment was never loaded from a
single-threaded context, it refuses to boot rather than warning - an app
that starts "fine" under `#[tokio::main]` is precisely the one that
corrupts an unrelated environment read weeks later.

The framework calls your `bootstrap_fn` once during the boot sequence,
after the environment is loaded and after the runtime drivers (Cache, Queue,
RateLimit, Mail) are up but before the router is built. The same call
runs for background workers (`queue:work`, `workflow:work`,
`schedule:work`) so an observer or listener registered here fires
identically for an insert from a queue job and an insert from an HTTP
handler. `http_bootstrap_fn` runs immediately after `bootstrap_fn`, but
only on the server path - background workers and the console binary
never call it. [Lifecycle](lifecycle.md) walks the full sequence.

Both functions' signatures are fixed by `Application::bootstrap` and
`Application::http_bootstrap`:

```rust
// src/bootstrap.rs
pub async fn register() {
    // database, bindings, observers, listeners, supervisors, worker job registration
}

pub fn register_http_stack() {
    // global middleware, Inertia::install
}
```

`register` returns `()`; `register_http_stack` is synchronous, not
`async` - both are wired as async closures at the call site
(`.http_bootstrap(|| async { bootstrap::register_http_stack() })`)
because a plain function pointer can also serve as a test harness
entry point without pulling `async` into the test. Fallible setup uses
`.expect("…")` with a message that explains the remediation - boot is
the right time to fail loudly. The example app's call is
`DB::init().await.expect("Failed to connect to database");` so a
missing `DATABASE_URL` aborts the process at boot with the actual
error printed, instead of surfacing as a confusing "connection
refused" on the first request.

## What goes in bootstrap

A real `bootstrap` function does a small number of distinct things.
Each subsection below is one of them. The example app's
`app/src/bootstrap.rs` exercises all of them and is the working
reference.

### Database connection

```rust
use suprnova::DB;

pub async fn register() {
    DB::init().await.expect("Failed to connect to database");
}
```

`DB::init` reads `DatabaseConfig` (registered by your `config_fn`) and
opens the pool. The connection is stored in the [container](container.md)
as a singleton - `DB::connection()` / `DB::get()` resolves it
anywhere. `DB::init_with(config)` is the test-and-tooling escape
hatch when you want to point at something other than the env-derived
URL.

### Global middleware

Global middleware is HTTP-only, so it belongs in `register_http_stack`,
not `register`:

```rust
use suprnova::{global_middleware, SessionMiddleware, SessionConfig, TimeoutMiddleware};
use crate::middleware;

pub fn register_http_stack() {
    global_middleware!(middleware::LoggingMiddleware);
    global_middleware!(TimeoutMiddleware::default());
    global_middleware!(SessionMiddleware::new(SessionConfig::from_env()));
}
```

`global_middleware!` registers a layer that runs on every request,
including unrouted ones (404s, OPTIONS preflight). The order you
register in is the order the chain runs - outside-in. The framework
slots its own `RequestIdMiddleware` outermost; everything you add sits
inside it. [Middleware](middleware.md) explains the full chain shape,
including the per-route layer.

### Container bindings

The container takes whatever you put in it; the macros are sugar over
the [`App`](container.md) facade.

```rust
use std::sync::Arc;
use suprnova::{App, bind, singleton, factory};
use crate::providers::DatabaseUserProvider;

pub async fn register() {
    // Trait → singleton (wraps in Arc):
    bind!(dyn UserProvider, DatabaseUserProvider);

    // Concrete singleton:
    singleton!(MyConfig { max_uploads_per_user: 100 });

    // Factory (constructed per resolve):
    factory!(|| RequestLogger::new());

    // Or call the facade directly for finer control:
    let hub: Arc<dyn BroadcastHub> = Arc::new(InMemoryBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(hub);
}
```

Trait-object bindings are the most common shape - bind an interface,
let handlers and tests substitute the implementation. The
[Container](container.md) chapter has the full binding API including
`bind_factory!`, the `_if_absent` variants, and the three-layer
lookup model.

### Event listeners and observers

The dispatcher is alive as soon as bootstrap runs - listeners
registered here see every subsequent dispatch.

```rust
use std::sync::Arc;
use suprnova::EventFacade;
use crate::events::UserRegistered;
use crate::listeners::SendWelcomeEmailListener;

pub async fn register() {
    EventFacade::listen::<UserRegistered, _>(
        Arc::new(SendWelcomeEmailListener),
    ).await;
}
```

Eloquent observers (`#[suprnova::observer(M)]`) collect themselves via
`inventory::submit!` at compile time. One call drains the inventory
into the dispatcher:

```rust
suprnova::eloquent::observers::bootstrap_observers()
    .await
    .expect("observer install failed");
```

The call is idempotent - re-running bootstrap (a worker that boots a
second time) does not double-register the listener adapters.
[Events](events.md) covers dispatch and listener authoring;
[Eloquent](eloquent.md) covers observers.

### Supervisors

Long-running background tasks declared via the `Supervisor` trait and
`inventory::submit!` start through one call:

```rust
use suprnova::SupervisorRegistry;

pub async fn register() {
    SupervisorRegistry::start_all().await;
}
```

Each supervisor runs in its own restart-loop task with a panic
boundary; a panicked supervisor is logged and restarted, not allowed
to take the process down. See [Supervisors](supervisors.md) for the
trait and the restart policy.

### Worker job registration

Queue jobs and mailables that workers need to dispatch by name register
themselves at boot:

```rust
use suprnova::queue::worker::register_job;

pub async fn register() {
    register_job::<crate::jobs::welcome_log::WelcomeLog>();

    suprnova::mail::register_mailable_factory::<crate::mail::welcome::WelcomeEmail>()
        .expect("register at boot");
    register_job::<suprnova::mail::send_job::SendMailJob>();
}
```

Without this, the worker has no way to map a queued envelope back to
the type that handles it.

## The post-boot hook: `booted()`

Bootstrap *registers*; `booted()` *resolves*. The builder takes a
second callback that fires after the server has finished its own
service boot but before it begins accepting connections. Use it when
you need to read something the framework itself bound during boot:

```rust
Application::new()
    .config(config::register_all)
    .bootstrap(bootstrap::register)
    .http_bootstrap(|| async { bootstrap::register_http_stack() })
    .routes(routes::register)
    .booted(|| {
        let cfg: MyConfig = suprnova::App::get().unwrap();
        tracing::info!(?cfg, "services booted");
    })
    .run()
    .await;
```

`booted` is synchronous and runs after `Server::from_config` - drivers
are up, encryption keys are loaded, your bindings exist. Most apps do
not need this hook; reach for it when a one-shot post-boot side effect
needs to see a fully-constructed container.

## A complete `bootstrap.rs`

A trimmed but representative shape, drawn from the example app. Two
functions, not one: `register` is process-wide, `register_http_stack`
is HTTP-only.

```rust
//! Application bootstrap - register services, listeners, global
//! middleware, and the Inertia layer.

use std::sync::Arc;
use std::time::Duration;

use suprnova::broadcasting::{BroadcastHub, ChannelRegistry, InMemoryBroadcastHub};
use suprnova::features::{FeatureMiddleware, bootstrap_database_cached};
use suprnova::queue::worker::register_job;
use suprnova::{
    App, DB, EventFacade, FrameworkError, Inertia, InertiaConfig,
    SessionConfig, SessionMiddleware, Storage, SupervisorRegistry,
    UserProvider, bind, global_middleware,
};

use crate::broadcasting::ChatChannel;
use crate::events::UserRegistered;
use crate::listeners::SendWelcomeEmailListener;
use crate::middleware;
use crate::providers::DatabaseUserProvider;

pub async fn register() {
    // ── Database
    DB::init().await.expect("Failed to connect to database");

    // ── Auth provider
    bind!(dyn UserProvider, DatabaseUserProvider);

    // ── Broadcasting hub + channel registry
    let hub: Arc<dyn BroadcastHub> = Arc::new(InMemoryBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(Arc::clone(&hub));

    let mut registry = ChannelRegistry::new();
    registry.register(ChatChannel);
    App::singleton(Arc::new(registry));

    // ── Event listeners + bridges
    EventFacade::listen::<UserRegistered, _>(
        Arc::new(SendWelcomeEmailListener),
    ).await;
    EventFacade::broadcast::<UserRegistered>(Arc::clone(&hub)).await;

    // ── Storage disks (env-gated S3 in production)
    Storage::register_fs("public", "./storage/public")
        .expect("register public disk");

    // ── Worker job registration
    register_job::<crate::jobs::welcome_log::WelcomeLog>();
    suprnova::mail::register_mailable_factory::<crate::mail::welcome::WelcomeEmail>()
        .expect("register at boot");
    register_job::<suprnova::mail::send_job::SendMailJob>();

    // ── Observers + supervisors
    suprnova::eloquent::observers::bootstrap_observers()
        .await
        .expect("observer install failed");
    SupervisorRegistry::start_all().await;

    // ── Feature flags
    bootstrap_database_cached(Duration::from_secs(60))
        .await
        .expect("feature-flag chain wired");
}

pub fn register_http_stack() {
    // ── Global middleware (outside-in in registration order)
    global_middleware!(middleware::LoggingMiddleware);
    global_middleware!(suprnova::TimeoutMiddleware::default());
    global_middleware!(SessionMiddleware::new(SessionConfig::from_env()));

    // ── Inertia protocol layer (no version pin: the default hashes the
    // Vite build manifest, so a frontend build bumps the asset version
    // on its own - see "Version detection" in frontend-inertia-responses.md)
    Inertia::install(&InertiaConfig::new()).expect("Inertia install failed");

    global_middleware!(FeatureMiddleware::new());
}
```

Notice the rhythm: each block does one thing, calls one or two APIs,
and either succeeds or fails with a clear message. Nothing here is
clever; the functions are long because the app has a lot of moving
parts, not because the bootstrap pattern is complicated.

## When to bootstrap vs `#[injectable]`

`#[injectable]` is a macro that auto-registers a singleton in the
container's `inventory` at compile time. It is the right choice for
services that need nothing more than their `#[inject]` dependencies to
construct:

```rust
use suprnova::injectable;

#[injectable]
pub struct UserService;

#[injectable]
pub struct OrderService {
    #[inject]
    user_service: UserService,
}
```

These resolve themselves; bootstrap does not need to touch them.

Bootstrap is the right place when construction needs anything else -
an environment variable, a constructed config struct, a `dyn Trait`
binding, a runtime decision, an async setup call, or registration of
something that is not itself a service (a listener, an observer, a
queue job mapping, a global middleware layer).

| Use `#[injectable]` for | Use `bootstrap` for |
|---|---|
| Concrete singletons with no runtime config | Anything `dyn Trait` |
| Services constructed from other injectables | Anything async at boot |
| Default DI graph | Environment-driven values |
| | Event listeners, observers, supervisors |
| | Global middleware |
| | Worker job + mailable registration |

You can mix freely. `#[injectable]` services are visible in the
container by the time `bootstrap` runs, so a binding in bootstrap can
read them.

## Where bootstrap sits in the boot order

The full sequence (excerpted from [Lifecycle](lifecycle.md)):

1. `Config::init(".")` - load `.env`, detect environment
2. `init_policies()` - drain the `#[policy]` inventory
3. Your `config_fn` runs (typed config registration)
4. Migrations run (auto-migrate on `serve`)
5. **Your `bootstrap_fn` runs** ← `bootstrap::register`
6. **Your `http_bootstrap_fn` runs, server path only** ← `bootstrap::register_http_stack`
7. Routes assembled from your `routes_fn`
8. `Server::from_config` boots drivers + container
9. Your `booted_fn`s fire
10. Server begins accepting connections

Background workers (`queue:work`, `workflow:work`, `schedule:work`) and
the console binary share steps 1-5 and 8 - they run `bootstrap_fn`, but
never step 6, since only `serve` / `web:run` runs `http_bootstrap_fn`.
That is what lets a listener or observer you register in `register`
reach worker code paths exactly as it reaches HTTP handlers, while
`register_http_stack`'s global middleware and `Inertia::install` stay
off processes that never serve HTTP.

### Why Suprnova diverges

Laravel runs every service provider's `register()` and `boot()` for
`artisan` commands and queue workers too, not only for HTTP requests -
and gets away with it because its Vite integration resolves asset URLs
lazily, at render time, from whatever the `@vite` Blade directive is
asked to render. A worker that never renders a view never touches the
manifest, so a missing build simply never comes up.

Suprnova's `Inertia::install` resolves the manifest once, at boot, and
fails closed in production when it is missing - by design, so a
misconfigured deployment cannot serve asset URLs pointing at a Vite dev
server nobody runs. That design choice is exactly what breaks a worker
or console image that (correctly) ships no `public/assets`: the failure
Laravel defers to request time, Suprnova would otherwise hit at process
start, on every subcommand. Splitting the boot surface into `bootstrap`
and `http_bootstrap` keeps the fail-closed check, but only where it
belongs - the server path that will actually render an Inertia page.

Laravel also splits boot itself across multiple service providers:
each provider implements `register()` and `boot()`, they're collected
in `config/app.php`, and Laravel walks them in two passes (all
`register`, then all `boot`) so a service can depend on another
provider's bindings without ordering ceremony in user code. The
provider class gives you a unit of organisation when an app
accumulates dozens of distinct subsystems.

Suprnova collapses that to two functions - `register` and
`register_http_stack` - rather than a `register`/`boot` pair per
provider. The reasons:

- **The two-pass `register`/`boot` split solves an ordering problem
  Rust does not have.** `#[injectable]` and the container's
  `bootstrap_singletons` already resolve dependency graphs without
  user-visible ordering. Bindings register inline; the lookup machinery
  handles the rest.
- **Two functions are easier to read than ten.** A new contributor
  opens `bootstrap.rs` and sees every binding, every listener, every
  observer, every middleware layer in one of two places. Provider-style
  fragmentation hides what the app actually does.
- **Inventory-style auto-registration covers the rest.** Observers,
  supervisors, scheduled tasks, policies, and queue handlers all
  collect themselves at compile time via `inventory::submit!`.
  Bootstrap drains the inventories with single calls
  (`bootstrap_observers`, `SupervisorRegistry::start_all`) rather than
  enumerating each.

Where Laravel earns the provider split is library distribution: a
crate that ships its own bindings would want a registration entry
point that an app can opt into without editing its own bootstrap.
Suprnova's analogue is a public `pub async fn register()` in the
crate's root and a one-line call from the app's `bootstrap`. The
ergonomic cost is one line; the readability gain is everything in
one place.

## Next

- [Lifecycle](lifecycle.md) - full boot order and where `bootstrap_fn` fires
- [Container](container.md) - `App::bind` / `App::singleton` /
  `App::factory` and the three-layer lookup
- [Configuration](configuration.md) - typed config registration that
  runs before bootstrap
- [Middleware](middleware.md) - chain composition for layers
  registered with `global_middleware!`
- [Events](events.md) - the dispatcher that listeners and observers
  plug into
