# Seeding

Seeders populate the database with fixture data - the rows your app needs before
a real user has done anything. Think a default admin account, the canonical list
of countries, the demo posts on the staging environment, the 50 users + 200
posts your local dev iteration loop depends on. They are the runtime sibling of
[migrations](migrations.md): migrations build the empty schema, seeders fill it.

A seeder is a zero-sized type that implements the `Seeder` trait. The framework
keeps an ordered process-global registry; the per-project `console db:seed`
command runs the root you register, or one specific seeder via `--class=<Name>`.
You retain registration order when you register no root. You usually write a
few lines that call a [model factory](eloquent.md) to generate the rows.

```rust
use suprnova::{async_trait, Factory, FrameworkError, Seeder};
use crate::factories::UserFactory;

pub struct UsersSeeder;

#[async_trait]
impl Seeder for UsersSeeder {
    fn name() -> &'static str { "UsersSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        UserFactory::new().count(50).create_many().await?;
        Ok(())
    }
}
```

Register it once at boot:

```rust
// src/bootstrap.rs
suprnova::seed::register::<crate::seeders::UsersSeeder>();
```

Then:

```bash
cargo run --bin console -- db:seed
# (50 rows inserted)
```

That's the whole loop. The rest of this chapter covers the layout conventions,
the bigger registry composition patterns, the `--class` targeting flag, the
factory integration, the `without_events` escape hatch, and the
when-to-seed-vs-migrate-vs-factory call.

## Writing a seeder

A seeder is a unit type plus a `Seeder` impl. `name()` is the registry key
(also what `db:seed --class=<Name>` matches against), and `run()` is the
async fn that performs the inserts.

```rust
// src/seeders/users_seeder.rs
use suprnova::{async_trait, Factory, FrameworkError, Seeder};

use crate::factories::UserFactory;

pub struct UsersSeeder;

#[async_trait]
impl Seeder for UsersSeeder {
    fn name() -> &'static str { "UsersSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        UserFactory::new().count(50).create_many().await?;
        Ok(())
    }
}
```

`Seeder` is re-exported at the crate root, so `use suprnova::Seeder` is
enough - you do not need to reach into `suprnova::seed::Seeder`. `async_trait`
is also re-exported (`use suprnova::async_trait`) because `Seeder` is an
`#[async_trait]` trait: the seeder registry stores each run as a boxed `Send`
future, which `#[async_trait]` produces.

The `FrameworkError` return type is the same error envelope every other async
surface in the framework uses; bubbling the `?` out of a factory call or a
`Model::create` is the expected shape. See [Error Model](error-model.md) for
the full taxonomy.

### Layout convention

Mirror Laravel's `database/seeders/` directory, but at the source root:

```
src/
├── bootstrap.rs
├── factories/
│   ├── mod.rs
│   ├── user_factory.rs
│   └── post_factory.rs
├── seeders/
│   ├── mod.rs              // pub mod base_seeder; pub use base_seeder::BaseSeeder;
│   └── base_seeder.rs      // Seeder impl, registered in bootstrap.rs
└── …
```

Generate the file by hand - there is no `make:seeder` generator (this is a
file with about ten lines of boilerplate). The factories the seeder calls
into get the same treatment.

### A seeder that runs other seeders

The Laravel idiom of a single top-level `DatabaseSeeder::run` that orchestrates
the per-model seeds works here too. Instead of registering five small seeders
in bootstrap and trusting their registration order, register one composite
seeder and call the rest of them yourself:

```rust
use suprnova::{async_trait, Factory, FrameworkError, Seeder};

use crate::factories::{PostFactory, UserFactory};

pub struct BaseSeeder;

#[async_trait]
impl Seeder for BaseSeeder {
    fn name() -> &'static str { "BaseSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        // 50 users first - the post factory generates author_id in
        // 1..=50, so the references resolve.
        UserFactory::new().count(50).create_many().await?;

        // 200 posts referencing the user ids above.
        PostFactory::new().count(200).create_many().await?;

        Ok(())
    }
}
```

This is the recommended default. It keeps the dependency order
(`users` before `posts`) inside the seeder rather than scattered across the
bootstrap file, and `db:seed --class=BaseSeeder` is a single-target invocation
that runs the whole bundle.

If you want to chain seeders by name rather than by direct factory call, use
`seed::call` from inside your root seeder:

```rust
async fn run() -> Result<(), FrameworkError> {
    suprnova::seed::call(["UsersSeeder", "PostsSeeder", "CommentsSeeder"]).await?;
    Ok(())
}
```

You register each child with `seed::register` in `bootstrap.rs`. You select
the composite with `seed::register_root::<BaseSeeder>()?`. A bare `db:seed`
runs only that root, which chooses its children and order.

`call` runs names in the order you give and writes `RUNNING <name>` followed
by `DONE <name> (<ms> ms)` to the command's console output. You capture these
lines with `console::test(["db:seed"]).run().await`. A failed child stops the
list, returns its error, and has no DONE line. Earlier inserts remain.

You use `call_silent(names)` to suppress progress. You use `call_once(names)`
to skip each seeder that already completed in this invocation, including one
run through `call` or `call_silent`. You can retry a failed seeder. The next
`db:seed` invocation starts with an empty set. A recursive call to a running
seeder returns an error.

You pass named inputs with `call_with(name, params)`:

```rust
seed::call_with("UsersSeeder", suprnova::attrs! { count: 50 }).await?;
```

You read those inputs by overriding the default parameter path:

```rust
#[async_trait]
impl Seeder for UsersSeeder {
    fn name() -> &'static str { "UsersSeeder" }
    async fn run() -> Result<(), FrameworkError> {
        Self::run_with(suprnova::SeederParams::new()).await
    }
    async fn run_with(params: suprnova::SeederParams) -> Result<(), FrameworkError> {
        let count = match params.get("count") {
            None => 10,
            Some(value) => value.as_u64()
                .ok_or_else(|| FrameworkError::bad_request("count must be an integer"))?,
        };
        let count = usize::try_from(count)
            .map_err(|_| FrameworkError::bad_request("count is too large"))?;
        UserFactory::times(count).create().await?;
        Ok(())
    }
}
```

`SeederParams` is an `Attrs` map. Existing seeders keep their `run()` signature:
the default `run_with(params)` calls it. You resolve services with
`App::make::<Service>()` inside either method, as you do elsewhere in your app.

## The seeder registry

You register peers in a process-global ordered map. Each stored function takes
`SeederParams` and returns a boxed future. You register a root separately to
choose which seeder a bare `db:seed` runs.

### `register::<S>()`

Add a seeder to the registry under its `Seeder::name()`:

```rust
suprnova::seed::register::<crate::seeders::UsersSeeder>();
```

You select a root with `seed::register_root::<BaseSeeder>()?`. This registers
the root and replaces the previous root selection, if any.

Two things to know about the registry:

- **Order matters.** `run_all` visits seeders in the order they were
  registered. If `B` needs rows from `A`, register `A` first.
- **Re-registering a name replaces in place.** The slot keeps its original
  position, the function pointer changes. This is intentional - it lets a
  test bind a stub seeder over the real one without shifting the order. In
  production code, register each seeder exactly once at boot.

### `run_all()`

You run every registered seeder in registration order explicitly. A bare
`console db:seed` calls the registered root; without a root it calls `run_all`.

```rust
suprnova::seed::run_all().await?;
```

Stops on the first error. Seeders that already ran are not rolled back -
`run_all` does not wrap a transaction around the batch because most seeders
span multiple statements and many backends do not nest transactions cleanly.
If you need rollback semantics, open the transaction inside the seeder and
keep all its work inside that scope.

### `run_one(name)`

Run one named seeder without running the others. This is the engine for
`db:seed --class=<Name>` and is also useful from one-off scripts:

```rust
suprnova::seed::run_one("AdminAccountSeeder").await?;
```

Misses return ``FrameworkError::not_found("no seeder registered for `X`")``.
The console command propagates that to a non-zero exit and a stderr line -
no silent no-op.

### `count()` and `is_registered(name)`

Two read helpers, both useful in tests that assert "bootstrap wired up the
expected seeders":

```rust
assert_eq!(suprnova::seed::count(), 3);
assert!(suprnova::seed::is_registered("BaseSeeder"));
```

Both return zero / false on a poisoned registry lock (after logging an error),
which keeps tests deterministic in the face of an upstream panic.

## The `db:seed` command

`db:seed` is a framework-provided console command - it ships with the
framework and lands in your project's `console` binary automatically through
the same `inventory` registry that picks up your own `#[command]`s. See
[Console](console.md) for the binary's mechanics; this section covers the
seeder-specific surface.

### Run the root

```bash
cargo run --bin console -- db:seed
```

The `suprnova` CLI runs the same command through your project's console
binary. `suprnova db:seed` runs your root, `suprnova db:seed UsersSeeder`
and `suprnova db:seed --class=UsersSeeder` run one. The CLI checks no name;
the console binary does.

You run the root registered with `seed::register_root`. Without one you run
every registered peer in order. On an empty registry you receive a
warning to stderr (`db:seed: no seeders registered - nothing to run`) and
exits zero - that's correct behavior for "someone ran the command before
registering anything" and keeps test suites that haven't seeded anything
specific from failing.

### Run one seeder

Three accepted forms, in increasing order of how Laravel-shaped they feel:

```bash
cargo run --bin console -- db:seed --class=UsersSeeder
cargo run --bin console -- db:seed --class UsersSeeder
cargo run --bin console -- db:seed UsersSeeder
```

All three look the seeder up in the registry by exact name and run it.
A name the registry doesn't hold fails with
``no seeder registered for `<Name>` `` and a non-zero exit, including on
an empty registry.

A targeted run reports its progress:

```text
  UsersSeeder .......................................................... RUNNING
  UsersSeeder ...................................................... 812 ms DONE

```

You receive these outer progress lines on stdout for a named run. A bare
`db:seed` leaves outer progress to your root's `call` methods. You keep the
`tracing` record per seeder for machine logs.

An unknown name fails fast:

```bash
cargo run --bin console -- db:seed --class=NotARealSeeder
# Error: no seeder registered for `NotARealSeeder`
# (exit 1)
```

A malformed flag (`--class` with no following value, `--class=` with empty
value, `--class --force`) fails fast too, with a diagnostic that names the
expected shape.

### Production and connection selection

You pass `--force` in production. Without it the command returns an error
before any seeder runs. You select another registered connection with
`--database reporting` or `--database=reporting`:

```bash
./console db:seed --force
./console db:seed --database reporting --class=UsersSeeder
```

You use the selected connection for the invocation's default queries and
factories. Explicit model and query connections keep their precedence.
The enclosing default returns after success or failure. An unknown connection
fails before your seeder runs. `Config::is_production()` uses your loaded app
configuration, or detects `APP_ENV=production` when none is loaded.

### From a built binary

In a containerized or systemd-managed deployment, the console binary lives at
`target/release/console` (or wherever your release artifact lands). Same
syntax, no `cargo` in front:

```bash
./console db:seed
./console db:seed --class=BaseSeeder
```

The console binary calls `suprnova::console::dispatch_argv(std::env::args())`,
which routes through the same registry as `cargo run --bin console --`. There
is no separate dispatch path for built artifacts.

## Composing with factories

Seeders almost always end up calling [factories](eloquent.md). The factory
trait knows how to build a randomized instance of one model; the seeder
sequences the factory calls and any non-randomizable wiring (deterministic
admin credentials, joined-table rows, file uploads).

The minimal factory + seeder pair:

```rust
// src/factories/user_factory.rs
use suprnova::Factory;
use crate::models::users::User;

pub struct UserFactory;

impl Factory for UserFactory {
    type Model = User;

    fn definition() -> User {
        User {
            id: 0,                              // persist_via_seaorm flips PK to NotSet
            name: "Factory User".into(),
            email: "factory@example.suprnova.app".into(),
            password: "factory-placeholder".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            ..Default::default()
        }
    }
}
```

```rust
// src/seeders/users_seeder.rs
use suprnova::{async_trait, Factory, FrameworkError, Seeder};
use crate::factories::UserFactory;

pub struct UsersSeeder;

#[async_trait]
impl Seeder for UsersSeeder {
    fn name() -> &'static str { "UsersSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        UserFactory::new().count(50).create_many().await?;
        Ok(())
    }
}
```

The fluent builder lives on `FactoryBuilder<M>`; what you can chain before
`create_many` matches Laravel:

```rust
// Build one persisted row with overrides:
let admin = UserFactory::new()
    .with(|u| u.email = "admin@example.com".into())
    .with(|u| u.role = "admin".into())
    .create()
    .await?;

// Build N persisted rows, all admins:
UserFactory::times(5)
    .with(|u| u.role = "admin".into())
    .create_many()
    .await?;

// Conditional state - applies the closure only when the flag is set:
UserFactory::times(10)
    .when(seed_admins, |b| b.with(|u| u.role = "admin".into()))
    .create_many()
    .await?;
```

`make` / `make_one` / `make_many` are the in-memory siblings (no insert) for
unit tests that don't want a database round-trip. See the
[Eloquent](eloquent.md) chapter for the full factory surface (including
`prepend`, `Sequence`, and the `#[derive(Factory)]` macro, with `#[factory(name = "…")]` to rename
the generated factory).

### Idempotency is the seeder's responsibility

`run_all` does not snapshot or wrap a transaction; if a seeder inserts
unconditionally, re-running it produces duplicates. The two standard ways to
make a seeder safe to re-run:

- **Reset first.** Local dev's "wipe and reseed" loop usually does
  `suprnova migrate:fresh && cargo run --bin console -- db:seed` -
  `migrate:fresh` drops and rebuilds every table, so the seeder always
  starts from empty. This is the shape most projects use day to day.
- **Upsert / check-first.** For a seeder that must coexist with existing
  data (a default admin account in production, the canonical list of
  countries), guard the insert with a lookup or use an upsert query.

```rust
async fn run() -> Result<(), FrameworkError> {
    let exists = User::query()
        .db_where("email", "admin@example.com")
        .exists()
        .await?;

    if !exists {
        let password_hash = suprnova::hashing::hash("change-me-on-first-login")?;
        User::create(attrs!{
            email: "admin@example.com",
            name: "Admin",
            password: password_hash,
        }).await?;
    }
    Ok(())
}
```

## Muting model events with `without_events`

A seeder that calls `Model::create` in a loop fires every lifecycle event -
`Creating`, `Saving`, `Created`, `Saved` - on every row. That wakes any
registered `Observer<M>`, runs any queued broadcast listeners, and can
incidentally enqueue a hundred background jobs you don't actually want.
`seed::without_events` is the Laravel-`WithoutModelEvents` analogue:

```rust
use suprnova::{async_trait, FrameworkError, Seeder, seed};
use crate::models::users::User;

pub struct UsersSeeder;

#[async_trait]
impl Seeder for UsersSeeder {
    fn name() -> &'static str { "UsersSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        seed::without_events(async {
            for i in 0..50 {
                User::create(attrs!{
                    name: format!("user{i}"),
                    email: format!("user{i}@example.com"),
                }).await?;
            }
            Ok(())
        }).await
    }
}
```

While the inner future is awaiting, both the cancellable veto path
(`dispatch_cancellable`) and the after-event fanout (`dispatch_after`)
short-circuit to `Ok(())`. Observers are silent, the broadcaster doesn't
wake, downstream jobs don't enqueue.

The effect is task-scoped - only work performed inside `fut` is muted.
Concurrent work on other tasks (HTTP request handlers, queue workers running
in the background, other seeders) continues to fire events normally. Nested
calls compose: an inner `without_events` block inherits the outer flag.

### Factories fire model events

A factory insert of a `#[suprnova::model]` struct takes the same insert
as `Model::create`. It dispatches `Creating`, `Saving`, `Created`, and
`Saved`, so an observer sees a factory's rows as it sees the
application's. `seed::without_events` mutes a factory path too. For a
single factory call, `create_quietly()` and `create_many_quietly()` insert
with the events muted.

In practice: if a seeder should not wake observers, wrap its inserts in
`without_events`, whether they are factory calls or a hand-rolled loop of
`User::create(attrs)`.

## Using seeders in tests

The same registry the console binary drives is callable from a
`#[tokio::test]` - handy when you want a known fixture set in front of an
integration test:

```rust
use serial_test::serial;
use suprnova::container::testing::TestContainer;
use suprnova::{DbConnection, seed};

use app::seeders::BaseSeeder;

#[tokio::test]
#[serial]
async fn dashboard_renders_seeded_posts() {
    // Reset the registry so a prior test's registrations don't leak.
    seed::clear();

    let _guard = TestContainer::fake();
    let conn = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
    app::migrations::Migrator::up(&conn, None).await.unwrap();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));

    // Register the seeder you want, run it, and assert against the
    // fresh database.
    seed::register::<BaseSeeder>();
    seed::run_all().await.unwrap();

    // …controller test against the seeded data…

    seed::clear();
}
```

Two notes on the test shape:

- `#[serial]` is required when the test mutates the process-global registry -
  parallel tests sharing the same registry will race. Add `serial_test`
  as a dev-dependency in your project's `Cargo.toml` to get the attribute.
- `seed::clear()` forgets every registered seeder. It is compiled with the
  `testing` feature, which is a default feature, and it is meant for tests:
  the registry is built once at boot and needs no reset in production. The
  guard of `TestContainer` does not reset the registry, so a test that
  registers a seeder calls `clear()` first and again when it finishes.

See [Testing](testing.md) for the broader test-harness conventions
(`#[suprnova_test]`, `TestContainer`, `TestDatabase::fresh::<Migrator>()`,
the fakes for every external surface).

## When to seed, migrate, or factory

These three patterns all put rows into tables. The decision is usually
straightforward, but it's worth naming the dividing lines explicitly because
PHP teams often blur them.

| You want… | Use |
|---|---|
| A column to exist | [Migration](migrations.md) |
| A row that must exist for the app to boot (the default admin, the singleton site-config row, the canonical list of currencies) | **Seeder** - idempotent, runs in every environment, including production |
| A randomized set of rows for local dev or staging (50 users, 200 posts, 1000 events) | Seeder that calls a factory |
| A row a unit test needs | [Factory](eloquent.md) called directly inside the test |
| The shape of a row | [Factory](eloquent.md) |

The mistakes to avoid:

- **Don't insert data from a migration.** Migrations describe schema, not
  state. A migration that inserts a default row will run once on the
  production database and then never again - the moment a column changes, you
  have a forked source of truth between migration history and the seeder.
  Put the insert in a seeder; if production needs the row, run
  `console db:seed --force --class=DefaultsSeeder` as part of deploy.
- **Don't write fixture data into your test by hand.** Reach for a factory.
  Five `User::create(attrs!{ … })` blocks in a test are five rewrites the
  moment you add a NOT NULL column. One `UserFactory::new().create()` survives.
- **Don't put production data in a seeder.** A seeder is for the rows the
  application requires to function, not for "here are the 8,000 historical
  records we're importing." Imports are one-off scripts (write a `#[command]`
  for them; see [Console](console.md)).

### Why Suprnova diverges

You explicitly register your root with `seed::register_root::<S>()?`, because
Rust has no class autoloader to discover `DatabaseSeeder`. You keep a flat
registry of named peers and compose them with `call`, `call_silent`,
`call_once` and `call_with`. A project that registers no root retains the
previous registration-order default.

You receive plain progress lines from nested calls. A named command keeps
its outer dot leader at 80 columns. Laravel sizes that leader to the terminal
and colors the status word. You receive whole milliseconds without a
thousands separator, so you can capture progress without terminal escape codes.

## Bootstrap registration

Every seeder needs a `seed::register` call in `bootstrap.rs`, alongside the
other process-global wiring (config, observers, supervisors, queue jobs).
The pattern is the same shape used elsewhere in the bootstrap file:

```rust
// src/bootstrap.rs
pub async fn register() -> Result<(), FrameworkError> {
    // …config + container bindings + auth wiring…

    // Seeders. Order matters - run_all visits in registration order.
    suprnova::seed::register_root::<crate::seeders::BaseSeeder>()?;
    suprnova::seed::register::<crate::seeders::DemoContentSeeder>();

    // …observers, supervisors, queue jobs…
    Ok(())
}
```

If you forget to register a seeder, `console db:seed --class=X` fails with
"no seeder registered for `X`" - a clear signal rather than a silent skip.
The `seed::count()` and `seed::is_registered("…")` helpers exist precisely
so a test can assert the bootstrap registered every seeder you expected.

See [Bootstrap](bootstrap.md) for the full file's structure and the order
the framework expects each subsystem to be wired in.

## Next

- [Migrations](migrations.md) - the schema half of the seed/migrate pair
- [Eloquent](eloquent.md) - models, factories, and the `Persistable` machinery
  every seeder calls into
- [Console](console.md) - the per-project `console` binary that hosts
  `db:seed` alongside your own `#[command]`s
- [Testing](testing.md) - `TestContainer`, `TestDatabase::fresh`, and the
  `#[serial]` pattern for tests that touch the seeder registry
- [Error Model](error-model.md) - what `FrameworkError` is and how
  `run`'s `Result<(), _>` shape composes with the rest of the framework
