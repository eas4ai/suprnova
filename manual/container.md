# Service Container

The container is where Suprnova holds your application's services -
the DB connection pool, the mail driver, your `Arc<MyService>`. You
bind values into it at boot time and resolve them in handlers and
workers. It's the Suprnova equivalent of Laravel's service container,
with one important difference: lookup is task-local first, so tests
running concurrently don't see each other's bindings.

## The two pieces

| Type | Role |
|---|---|
| `Container` | The underlying registry: holds bindings, factories, and singletons |
| `App` | The global facade you actually call - `App::bind`, `App::get`, etc. |

You almost always call `App::*` rather than constructing a
`Container` directly. The container is plumbing; the `App` facade
is the API.

## Lookup order

Every `App::get` / `App::make` call checks **three layers** in order:

```
        task-local
            │
            ▼  (miss)
       thread-local
            │
            ▼  (miss)
          global
            │
            ▼  (miss)
          None
```

The first two layers hold test overrides. The third holds your
application's bindings. This matters because:

- **Tests use task-local or thread-local** - `let _g = TestContainer::fake();`
  followed by `TestContainer::bind(...)` binds inside one thread
  without touching the global container, so parallel tests don't
  bleed services into each other. The guard clears the test
  container when it drops. `TestContainer::scope` does the same for one
  async task, and survives a move between worker threads.
- **App-wide services go through global** - bound once at boot,
  resolved everywhere. `App::bind`, `App::singleton`, `App::factory`
  and `App::scoped` all write to this layer.
- **A test override wins** - if a test binds a type in the first two
  layers, that binding answers, whatever the global layer holds for
  the same type.

A scoped binding sits in the global layer, but its value does not. The
container builds the value inside the scope of the current unit of
work. See [Scoped bindings](#appscoped-and-appbind_scoped---one-value-per-unit-of-work).

You rarely think about which layer a binding lives in - `App::get`
finds it wherever it lives. The model only matters when something
behaves unexpectedly under concurrency, and then the
[Testing](testing.md) chapter has the detail.

## Binding a value

These are the ways to put something into the container, depending on
what you have:

### `App::singleton(value)` - owned, cloned at lookup

For any `T: Any + Send + Sync + 'static` value that should live
forever. The `Clone` bound is on the *getter* (`App::get`), not the
binding - the value is stored once inside an `Arc` and cloned out of
that `Arc` on each `get`:

```rust
use suprnova::App;

App::singleton(MyConfig {
    timeout_secs: 30,
    retries: 3,
});

let cfg = App::get::<MyConfig>().expect("registered at boot");
println!("{}", cfg.timeout_secs);
```

The value is stored once; `App::get::<MyConfig>()` returns a clone.
Use this for plain config-shaped data that's cheap to clone.

### `App::bind(Arc<T>)` - for traits and shared services

For trait objects or anything you want behind an `Arc`:

```rust
use std::sync::Arc;
use suprnova::App;

let store: Arc<dyn KeyValueStore> = Arc::new(RedisStore::connect(url)?);
App::bind(store);

let store = App::make::<dyn KeyValueStore>().expect("bound at boot");
store.put("hello", b"world").await?;
```

`App::make::<T>()` returns the `Arc<T>` clone (cheap atomic refcount
bump). Use this for any service shared across threads, especially
trait objects.

### `App::factory(|| { … })` - built on demand

When constructing the value should happen at first use (or every time):

```rust
App::factory(|| {
    HttpClient::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("http client config is hand-rolled and known-good")
});
```

`App::factory` registers a *concrete-type* factory (`Fn() -> T`);
`App::bind_factory` registers a *trait-object* factory
(`Fn() -> Arc<T>`). Neither closure returns `Result` - handle
construction failure inside the closure (panic at boot, or build a
sentinel value) or use a regular `App::singleton` / `App::bind` after
constructing the value yourself with `?`. Both invoke the closure
outside any container lock, so a factory that re-enters the container
won't deadlock and an expensive constructor won't block other bindings.

### `App::scoped` and `App::bind_scoped` - one value per unit of work

Some services belong to one request, and the next request must not
see them: the database handle of the current tenant, or an API client
bound to the caller. A singleton is one value for the whole process,
so it would leak between requests. A factory builds a new value on
every resolution, so it can't share one handle across a request.

A scoped binding sits between the two. You register a factory once.
The container runs it **at most once in a scope**, at the first
resolution, and returns the same value on every later resolution in
that scope. When the scope ends, the container drops the value. If
nothing resolves the binding in a scope, the factory never runs.

`App::scoped` registers a concrete-type factory (`Fn() -> T`). Resolve
it with `App::get` or `App::resolve`, which clone the value, so
register an `Arc<T>` when callers must share one instance.
`App::bind_scoped` registers a trait-object factory (`Fn() -> Arc<T>`).
Resolve it with `App::make` or `App::resolve_make`:

```rust
use std::sync::Arc;
use suprnova::{App, Context, FrameworkError};

// In bootstrap: the factory reads the tenant from the request's context.
App::scoped(|| {
    let tenant: String = Context::get("tenant_id").unwrap_or_default();
    Arc::new(TenantDb::connect_for(&tenant))
});

// Trait-object form, resolved with `App::make`:
App::bind_scoped::<dyn ApiClient, _>(|| {
    Arc::new(TenantApiClient::for_current_tenant()) as Arc<dyn ApiClient>
});

// In a handler or a service it calls: built on the first call, reused
// on the next, dropped when the request ends.
async fn load_orders() -> Result<Vec<Order>, FrameworkError> {
    let db = App::resolve::<Arc<TenantDb>>()?;
    db.orders().await
}
```

The factory runs with no container lock held, so it can resolve other
bindings, scoped ones included. If two tasks that share a scope resolve
the same binding at the same moment, the factory runs once. The second
task waits for the value on its own thread, which blocks that thread.
A factory must therefore be quick and must not block on I/O.

A cycle of scoped factories is an error. A factory that resolves its own
type, directly or through other scoped factories, gets an error instead of
recursing or waiting without end. This holds in one task and across the
tasks of one scope.

#### Which units of work get a scope

The framework opens a scope for each unit of work that it runs. A
unit of work has values of its own, so a job never sees the values of
the request that dispatched it.

| Unit of work | Scope |
|---|---|
| HTTP request, including a WebSocket upgrade request | One per request |
| WebSocket session | One per session, separate from the upgrade request |
| Attempt of a queued job | One per attempt, so a retry starts fresh |
| Queued event listener | One per attempt of the listener |
| Scheduled task run | One per run |
| Workflow run | One per claimed run. The steps of the run share it |
| Supervisor run | One per run, so a restart starts fresh |
| Console command | One per command |

Three kinds of work belong to a request but run after it. They share the
scope of the request, and the scope ends after they finish:

- An after-commit callback (see [Database](database.md)).
- A hook that runs after the response, such as a terminable middleware
  (see [Middleware](middleware.md)).
- The body of a streamed response, such as `HttpResponse::sse`,
  `stream_bytes` or `stream_json`. The stream resolves the scoped values of
  its request, and the body keeps the scope until it ends or is dropped.

#### Scopes in your own code

A task that you start with a bare `tokio::spawn` has no scope, even when
you spawn it from a request. Three helpers on `App` cover the cases:

- `App::run_scoped(future)` runs a future in a new scope of its own and
  returns its output. Use it for work the framework does not run, such
  as your own worker loop or a test. Inside another scope it opens a
  nested scope that does not see the outer values.
- `App::in_current_scope(future)` wraps a future so that it runs in the
  scope of the caller. The container captures the scope when you call
  it, so you can pass the result to any spawn helper.
- `App::spawn_scoped(future)` is `App::in_current_scope` followed by
  `tokio::spawn`.

A spawned task shares the values of the scope, so a binding that the task
resolves first is the value the request sees afterward. The scope ends
when the last future that holds it ends. Outside a scope, `in_current_scope`
and `spawn_scoped` behave like the plain future and `tokio::spawn`.

```rust
use suprnova::App;

// Inside a handler: the task sees the request's `dyn AuditLog`.
let handle = App::spawn_scoped(async {
    if let Some(log) = App::make::<dyn AuditLog>() {
        log.record("export finished");
    }
});

// Only the container scope follows the task. To carry the request id too,
// hand the wrapped future to `spawn_with_request_id`:
let handle = suprnova::spawn_with_request_id(App::in_current_scope(async {
    // ...
}));
```

#### Outside a scope

A scoped binding never builds a value that lives for the process. If
you resolve one where no scope is active:

- `App::resolve` and `App::resolve_make` return an error. Its message
  names the type and says that the binding is scoped.
- `App::get` and `App::make` log a warning and return `None`.

#### Order of resolution

The container looks up a type in this order:

1. A test override in the task-local layer.
2. A test override in the thread-local layer.
3. The global layer. A scoped binding resolves in the current scope.

A test override wins over a scoped binding of the same type. A type has
one registration in the global layer, so the last call wins: a
`singleton`, `factory` or `scoped` call for a type replaces an earlier
one for the same type.

### `App::*_if_absent(value)` - boot-order-friendly registration

Sometimes a default service is registered by a service crate, and the
app wants to override it only when present. The `_if_absent` variants
let you register a default that won't clobber an existing binding:

```rust
// Inside a starter or library crate:
App::singleton_if_absent(DefaultMailDriver::new());

// In your app's bootstrap.rs:
App::singleton(MyCustomMailDriver::new());  // wins because it ran later
```

`bind_if_absent`, `singleton_if_absent`, and the factory variants all
return `bool` - `true` if they actually inserted, `false` if there
was already a binding.

Boot registers every `#[injectable]` this way, so an instance you bind
by hand in `bootstrap.rs` is kept. Boot doesn't run the generated
constructor of a type that is already bound, either: a dependency that
only that constructor reads doesn't have to be registered.

## Resolving a value

Two read methods, plus their `Result`-returning siblings:

```rust
// Clone the bound value out:
let cfg: MyConfig = App::get::<MyConfig>().expect("bound at boot");

// Clone the Arc:
let store: Arc<dyn KeyValueStore> = App::make().expect("bound at boot");

// Same but Result, for the `?` idiom in fallible paths:
let cfg = App::resolve::<MyConfig>()?;
let store = App::resolve_make::<dyn KeyValueStore>()?;
```

`resolve` and `resolve_make` return
`Result<_, FrameworkError>` (specifically the `ServiceNotFound`
variant when the lookup misses) - useful in handler paths where a
missing service should surface as a 500 with a proper log, not a panic.
A [scoped binding](#appscoped-and-appbind_scoped---one-value-per-unit-of-work)
that cannot resolve is a different error: `resolve` returns
`FrameworkError::Internal`, and `get` and `make` log a warning and
return `None`.

Membership checks (rarely needed):

```rust
if App::has::<MyConfig>() { … }
if App::has_binding::<dyn KeyValueStore>() { … }
```

## Where binding happens

The standard place is `src/bootstrap.rs` - one function that runs
once at boot:

```rust
use std::sync::Arc;
use suprnova::App;
use crate::services::{MyService, RealEmailGateway};

pub async fn register() {
    // Plain singletons
    App::singleton(MyAppConfig {
        max_uploads_per_user: 100,
    });

    // Trait-object services
    let gateway: Arc<dyn EmailGateway> = Arc::new(RealEmailGateway::new());
    App::bind(gateway);

    // Lazy services (built on first use)
    App::bind_factory::<dyn HttpClient, _>(|| {
        Arc::new(ReqwestClient::with_timeout(30))
    });
}
```

The function name `register` matches the scaffold default (`src/bootstrap.rs::register`); the return type is `()`, not `Result`. Bind errors that happen during boot (e.g. driver connect failures) should propagate via the driver/service constructor, not from `register` itself - see [Application Bootstrap](bootstrap.md) for the full boot wiring.

The framework also calls into the container itself during boot:

- Your `bootstrap_fn` runs first, so a binding you install by hand is
  in place before the inventory boots
- `App::init()` then initialises the registry, and
  `App::boot_services()` registers the `#[injectable]` and `#[service]`
  inventory around your bindings
- The runtime drivers come up last

The server, the workers, the `queue:*` commands, `down` and `up`, and
the console binary all boot in this order.

See [Application Bootstrap](bootstrap.md) for the full boot order.

## Inertia shared data

The container is also where Inertia shared data lives. Three
convenience APIs make that explicit:

```rust
use suprnova::App;

// Eager value - serialised once and reused for every Inertia response.
App::inertia_share("appName", "Suprnova");

// Lazy value - resolver runs per response. Use for per-request data
// that needs async work.
App::inertia_share_lazy("locale", || async {
    Ok::<_, suprnova::FrameworkError>(detect_locale().await)
});

// Push a single flash entry onto the per-request flash bag.
App::flash("message", "Saved!");
```

These read from `Container::inertia()` which returns
`&Arc<InertiaRegistry>` - you can interact with it directly if you
need lower-level access. See [Inertia / Frontend](frontend.md) for
how the shared data ends up in the page response.

## Why three layers?

The task-local → thread-local → global cascade exists for one
reason: **isolation under concurrency**. Two things benefit:

**Per-test isolation.** A test that binds a fake mail driver should
not see a fake bound by a sibling test. `TestContainer::fake()`
returns a thread-local guard, and `TestContainer::bind` /
`TestContainer::singleton` route writes into the active scope.
Parallel tests stay hermetic:

```rust
use std::sync::Arc;
use suprnova::container::testing::TestContainer;
use suprnova::suprnova_test;

#[suprnova_test]
async fn one_test_binds_a_fake() {
    let _guard = TestContainer::fake();
    TestContainer::bind::<dyn Mailer>(Arc::new(FakeMailer::new()));

    // … this test uses FakeMailer
    // a sibling test running in parallel doesn't see it
}
```

For multi-thread tokio runtimes - where the future may migrate between
worker threads - use `TestContainer::scope(async { ... })` instead;
that installs a task-local override that survives the migration.

**Override-at-boot.** Application code can override defaults registered
by library crates. The `_if_absent` variants and the layered lookup
combine to give library crates clean default-registration without
fighting application overrides.

The layers do not isolate requests from each other. A binding in the
global layer is one value for the whole process. To keep a value
inside one request, use a [scoped binding](#appscoped-and-appbind_scoped---one-value-per-unit-of-work).

## Common patterns

### Bind a struct holding the DB pool

You almost never do this directly - the framework binds the DB pool
itself. But if you have your own subsystem with an expensive
shared resource:

```rust
let pool = MyResourcePool::connect(url).await?;
App::bind(Arc::new(pool));

// later:
let pool = App::resolve_make::<MyResourcePool>()?;
let conn = pool.checkout().await?;
```

`App::make` returns `Option<Arc<T>>` and pairs with `.expect(...)`; `App::resolve_make` returns `Result<Arc<T>, FrameworkError::ServiceNotFound>` and pairs with `?` in fallible code. Use the one that matches your caller's error story.

### Swap a default for a fake in tests

```rust
use std::sync::Arc;
use suprnova::container::testing::TestContainer;
use suprnova::suprnova_test;

#[suprnova_test]
async fn order_dispatches_email() {
    let fake = Arc::new(FakeEmailGateway::new());
    let fake_for_assert = Arc::clone(&fake);

    let _guard = TestContainer::fake();
    TestContainer::bind::<dyn EmailGateway>(fake);

    place_order(123).await.expect("place_order succeeds");

    assert_eq!(fake_for_assert.sent_count(), 1);
}
```

### Lazy expensive construction

```rust
// Builds the embedding model on first request, not at boot.
App::bind_factory::<dyn EmbeddingModel, _>(|| {
    Arc::new(
        OnnxEmbedding::load_from_disk("/models/all-mini-lm.onnx")
            .expect("embedding model must load"),
    )
});
```

For fallible construction that needs to surface a structured error to
the operator, build the value yourself in `bootstrap()` with `?` and
call `App::bind(...)` once it's ready.

## Why Suprnova diverges

Laravel's container has one global scope - bindings are global, and
isolating between tests requires `setUp` / `tearDown` discipline plus
the framework's per-test database transaction. PHP's request-per-process
model makes this safe-by-accident: a fresh process per request means
the container is reset every time.

Rust's process model is the opposite - one process serves many
concurrent requests on many threads. A global-only container would
mean a test in one thread can see a fake bound by another, or a
request could see another request's per-request data. Suprnova
answers both problems:

- The three-layer cascade keeps tests apart: a task-local layer and a
  thread-local layer hold test overrides, and the global layer holds
  app-wide services.
- Scoped bindings keep requests apart. `App::scoped` is the Suprnova
  form of Laravel Octane's `scoped()` binding: the container builds
  the value once for one unit of work and drops it when that unit ends.

The container API is the same as Laravel's; the lookup machinery
is different because the runtime is different.

## Next

- [Application Bootstrap](bootstrap.md) - where the binding code goes
- [Configuration](configuration.md) - typed config registration
  alongside services
- [Testing](testing.md) - `TestContainer::fake` and `#[suprnova_test]`
- [Lock Policy](lock-policy.md) - why poisoned-lock recovery matters
  in a container-backed application
