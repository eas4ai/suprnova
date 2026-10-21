# Database Tests

The DB-specific companion to [Testing](testing.md). Where that chapter
covers the test harness - `#[suprnova_test]`, `describe!` / `test!`,
`expect!`, and the in-process fakes - this one covers what changes
when your test needs a database: how `TestDatabase` builds one for
you, how isolation actually works, where factories and seeders plug
in, and when an in-memory SQLite is and isn't enough.

## The two constructors

Every database test starts by building a `TestDatabase`. Two
constructors build an in-memory SQLite database, with two intents.
Three more run the test on the database `DATABASE_URL` names; see
[Tests on the configured database](#tests-on-the-configured-database).

### `TestDatabase::fresh::<Migrator>()`

Builds an in-memory SQLite database, loads your SQLite schema dump
(`database/schema/sqlite-schema.sql`) when you have one, runs your
migrator end-to-end,
and registers the connection in the test container so any code
calling `DB::connection()` or `App::resolve::<DbConnection>()`
resolves to it. This is the right default for everything that touches
real schema.

```rust
use suprnova::testing::TestDatabase;
use crate::migrations::Migrator;

#[tokio::test]
async fn user_lifecycle_end_to_end() {
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();

    let alice = User::create(attrs! {
        name: "Alice", email: "alice@example.com",
    })
    .await
    .unwrap();

    assert!(alice.id > 0);
    // Query directly when you want to bypass the model surface:
    let row = users::Entity::find_by_id(alice.id)
        .one(db.conn())
        .await
        .unwrap();
    assert!(row.is_some());
}
```

`Migrator` is your application's `MigratorTrait` implementation -
the same type the production `suprnova migrate` command runs. By
threading the real migrator through the test schema you make schema
drift impossible: a column the migrator forgot to add cannot be
silently present in the test DB.

The `test_database!()` macro is sugar for the common case (`crate::migrations::Migrator`):

```rust
use suprnova::test_database;

#[tokio::test]
async fn shortcut() {
    let db = test_database!();          // == TestDatabase::fresh::<crate::migrations::Migrator>()
    // ...
}

// Or with a custom migrator path:
let db = test_database!(my_crate::CustomMigrator);
```

### `TestDatabase::sqlite_memory()`

Same container and registry wiring, but **does not run any
migrator**. Use this when the test wants precise column-shape
control - typically cast round-trips, query-builder SQL surface
tests, or driver-level edge cases where a full migrator is overkill
or noise:

```rust
let db = TestDatabase::sqlite_memory().await.unwrap();
db.execute_unprepared(
    "CREATE TABLE casts_t (id INTEGER PRIMARY KEY, payload BLOB)",
)
.await
.unwrap();

// Then write directly and read back with the typed helpers:
let row = db.fetch_one(
    "INSERT INTO casts_t (payload) VALUES (?) RETURNING id, payload",
    vec![sea_orm::Value::Bytes(Some(Box::new(b"hello".to_vec())))],
).await.unwrap();
```

`sqlite_memory()` is the foundation `fresh()` is built on - `fresh`
calls it and then runs your migrator. Anything you can do with
`fresh` you can do here; you just bring your own DDL.

### `execute_unprepared`, `fetch_one`, `fetch_all`

`TestDatabase` re-exports the three SeaORM execution shapes you reach
for most in tests, so test files don't have to pull in
`ConnectionTrait`:

| Method | Use for |
| --- | --- |
| `execute_unprepared(sql)` | DDL or DML with no placeholders. Returns `Result<(), FrameworkError>` |
| `fetch_one(sql, bindings)` | One-row SELECT. Errors if zero rows |
| `fetch_all(sql, bindings)` | All-row SELECT |

The bindings are `Vec<sea_orm::Value>` - the same shape the
production query path uses. The connection's backend (SQLite for
both in-memory constructors) is supplied for you, so a `?` placeholder
is correct there.

## Tests on the configured database

`fresh` gives every test a private in-memory SQLite database. When
your suite must run on the engine you deploy to, three constructors
run the test on the database `DATABASE_URL` names instead. They are
Laravel's three database traits:

| Constructor | Laravel trait | What it does |
| --- | --- | --- |
| `TestDatabase::refresh::<M>()` | `RefreshDatabase` | Migrates the database once per test process, then holds each test in a transaction that it rolls back when the helper drops |
| `TestDatabase::refresh_lazily::<M>()` | `LazilyRefreshDatabase` | Does what `refresh` does on the first query, so a test that never touches the database never migrates it |
| `TestDatabase::migrate::<M>()` | `DatabaseMigrations` | Runs the migrations with no transaction around the test, and rolls them back when the helper drops |

```rust
use suprnova::testing::TestDatabase;
use crate::migrations::Migrator;

// DATABASE_URL=postgres://app:secret@127.0.0.1/app_test cargo test
#[tokio::test]
async fn orders_are_numbered_per_customer() {
    let db = TestDatabase::refresh::<Migrator>().await.unwrap();

    let order = Order::create(attrs! { customer_id: 1 }).await.unwrap();
    assert_eq!(order.number, 1);
    // The row is gone when `db` drops, so the next test starts empty.
}
```

`DATABASE_URL` is required. When it's unset, the three constructors
return an error instead of falling back to the development database
`./database.db`. When it names an in-memory SQLite database, they
return an error that points you to `fresh`, which is the in-memory
helper.

### How `refresh` holds a test

The first `refresh` of a test process for a database and a migrator
migrates that database the way your application's `migrate` command
does: it loads the schema dump into a database that has run no
migration, then runs the pending migrations. The other `refresh` calls
of the process don't migrate again. A test that starts while that
migration runs waits for it, so parallel tests share one migration. A
migration that fails is an error of `refresh` and isn't remembered, so
the next `refresh` tries again.

Then the helper opens a pool of one connection, begins a transaction
on it, and registers it in the test container. Every query of the test
runs in that transaction: through `DB::connection()`, a model, a
factory, a seeder, `db.conn()`, the database session driver, and a
database queue driver. The queue driver takes its connection when it's
built, so build it after the helper. The `jobs`, `failed_jobs`, and
`sessions` rows these drivers write roll back with the test.

A `DB::transaction` inside the test is a savepoint of the test
transaction. It commits or rolls back its own work, and its
after-commit callbacks, `Job::after_commit()` pushes included, run
when it commits. When the helper drops, its pool closes and the
database discards the transaction, so a row one test writes is gone
for the next.

Keep these limits in mind:

- The pool holds one connection, as under `fresh`. Code that holds a
  transaction of its own (`DB::begin_transaction`) and runs a query
  outside it waits for the connection until the acquire timeout.
- A statement that ends the transaction itself defeats the rollback:
  DDL on MySQL commits implicitly, and so does a raw `COMMIT`. Create
  tables in migrations, not in the test body.
- Test processes that share a Postgres or MySQL database don't see
  each other's rows, but two tests that insert the same unique key
  wait for each other. SQLite allows one writer, so tests that share a
  SQLite file wait for each other's writes, up to the busy timeout.
  Give parallel test processes their own file.
- A transaction that asks for an isolation level, as the render cache
  asks for `REPEATABLE READ`, runs at the level of the test
  transaction, since a savepoint can't change it.

### `refresh_lazily`

`refresh_lazily` registers its connection at once and does the work of
`refresh` on the first query, the first transaction, or the first
seeder: it begins the transaction, migrates, and then runs the query.

A migration that fails on that first query can't fail the query from
inside the pool. The query runs on the database as it is, which usually
fails on a missing table, and dropping the helper fails the test with
the migration's error.

### `migrate`

`migrate` runs your pending migrations on the connection it registers,
each through its `up`. There's no transaction around the test, so
another connection sees its rows. Use it for a test whose code must see
committed rows from a second connection.

When the helper drops, it rolls back the migrations it ran and no
others, and drops the migration table when it created it, so the
database holds no table the helper created. The next `refresh` of the
process migrates again.

`migrate` never loads a schema dump, even into an empty database: a
table that a dump creates has no `down` to remove it. A migrator whose
migrations you pruned into a dump can't run them, so `migrate` returns
an error that names the dump. Test that migrator with `refresh`, which
loads the dump.

When a migration fails, `migrate` rolls back the migrations that ran
before it and returns the error. When the rollback on drop fails, the
drop panics, so a test can't pass while it leaves its tables behind.
Two `migrate` tests on one database must not run at once.

### Seed before the body

`seed::<S>()` runs the seeder `S` on the test database and hands the
helper back, so it chains after any constructor. `seed_root()` runs
the root seeder, as a bare `db:seed` does: the seeder registered with
`seed::register_root`, or every registered seeder in order when there
is no root. `seed_root()` returns an error when no seeder is
registered, instead of seeding nothing.

```rust
let db = TestDatabase::refresh::<Migrator>()
    .await
    .unwrap()
    .seed::<UsersSeeder>()
    .await
    .unwrap();

assert_eq!(User::query().count().await.unwrap(), 50);
```

`S` runs whether or not it's registered; the seeders it calls are
found in the registry. Under `refresh`, the seeded rows belong to the
test transaction and are gone for the next test.
[`#[suprnova_test]`](testing.md#suprnova_test---when-you-want-the-sugar)
takes the same choices as keys: `refresh`, `seed`, and `seed = Path`.

### Why Suprnova diverges

- Laravel's `RefreshDatabase` runs `migrate:fresh` once per process,
  which drops every table first. `refresh` runs the pending migrations
  only. Dropping tables isn't safe when several test processes share
  one database, and a migrated database stays valid between runs. To
  start from empty tables, run `migrate:fresh` on the test database
  before the run.
- Laravel's `DatabaseMigrations` also runs `migrate:fresh`, which loads
  the schema dump. `migrate` runs the pending migrations only and never
  loads the dump, so every table it creates has a `down` that removes
  it when the helper drops.
- Laravel's `#[Seed]` seeds once per process, inside `migrate:fresh`,
  and the rows stay committed. `seed` runs in each test, inside its
  transaction, so a seeder that inserts rows doesn't add them again on
  every run of a database that keeps its tables.
- Laravel keeps one in-memory PDO per process, so `RefreshDatabase`
  works with `:memory:`. A Suprnova in-memory database belongs to one
  pool, so `refresh` needs a database that outlives a connection, and
  `fresh` is the in-memory helper.
- Laravel counts the test transaction in `DB::transactionLevel()`.
  `DB::transaction_level()` doesn't, because the test transaction sits
  below the framework's own transactions.

## How isolation actually works

For `fresh` and `sqlite_memory`, the fresh-database-per-test model is
the isolation mechanism. Each call opens a new `sqlite::memory:`
connection, which under SQLite is an entirely separate database
instance - no shared schema, no shared rows, no other test can see
into it. There is no transaction wrapper and no rollback to remember:
the *next* test gets a clean empty DB because it builds its own. The
constructors that run on `DATABASE_URL` isolate tests with a
transaction instead; see
[How `refresh` holds a test](#how-refresh-holds-a-test).

When the `TestDatabase` value drops, three things happen, in this
order:

1. The held `TestContainerGuard` clears the thread-local test
   container, so any subsequent `App::get::<DbConnection>()` no longer
   finds the test connection.
2. If this was the *last* live `TestContainerGuard` in the process,
   the named [`ConnectionRegistry`](database.md#named-connections)
   is wiped. (A refcount over live guards guarantees an inner
   test's drop cannot erase a connection name a concurrent outer
   test still depends on - the standing trap that prompted the
   refcount.)
3. The SQLite connection itself drops, which destroys the in-memory
   database. For `refresh` and `refresh_lazily`, this step closes the
   test's connection and the database discards its transaction; for
   `migrate`, it rolls the migrations back and drops the migration
   table when it created it.

Because state is rebuilt rather than rolled back, the isolation is
stronger than `BEGIN`/`ROLLBACK` wrapping: there is no committed
state to mistakenly survive, no nested transaction quirks, no
sequence-counter drift between tests. The cost is that you pay for
running the migrator once per test (negligible for SQLite with most
schemas; if it becomes a real cost, `refresh` migrates a file or
server database once per process - see
[Tests on the configured database](#tests-on-the-configured-database)).

## Why the pool is pinned to one connection

Both constructors build the database with `max_connections(1)` and
`min_connections(1)`. This is load-bearing for `sqlite::memory:`,
not a generic policy.

`sqlite::memory:` is a per-connection database - each *new*
connection in the pool would be a separate, empty SQLite instance.
A pool of size 2 would mean half your queries see the migrated
database and half see an empty one. Pinning the pool to one
connection makes every query in the test land on the same in-memory
database that the migrator ran against.

The consequence: a test that exercises true connection concurrency
(two transactions racing, replica routing, a queue worker hitting
the DB while a request handler does) needs a real database. See
"When SQLite in-memory isn't enough" below.

## Factories in tests

Factories produce randomized model instances and (optionally) persist
them. The persistence path resolves the bound test connection
automatically - there is no factory-side wiring for tests.

```rust
use crate::factories::UserFactory;

#[tokio::test]
async fn factory_round_trip() {
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();

    // In-memory only: fastest, no DB round trip.
    let alice = UserFactory::new()
        .with(|u| u.email = "alice@example.com".into())
        .make();
    assert_eq!(alice.email, "alice@example.com");

    // Persist one + return the post-insert model (id assigned).
    let bob = UserFactory::new().create().await.unwrap();
    assert!(bob.id > 0);

    // Bulk: persist 50 in sequence.
    let many = UserFactory::times(50).create_many().await.unwrap();
    assert_eq!(many.len(), 50);
}
```

Two patterns worth knowing:

**Factory inserts fire model events.** For a `#[suprnova::model]`
struct, `create()` / `create_many()` take the same insert as
`Model::create`, which dispatches `Creating` / `Saving` / `Created` /
`Saved`. A test that asserts "the `Created` observer fired" can build its
fixture with a factory. To build a fixture that no observer sees, use
`create_quietly()` / `create_many_quietly()`.

**`create_many` does not transact.** Inserts are sequential. If a
later row fails the prior rows are not rolled back. Wrap the call
in your own `DB::transaction` if a test requires atomicity:

```rust
DB::transaction(|tx| async move {
    UserFactory::times(50).create_many().await?;
    PostFactory::times(200).create_many().await?;
    Ok::<_, FrameworkError>(())
}).await.unwrap();
```

See [Eloquent → Factories](eloquent-factories.md) for the full
factory surface (states, sequences, `with`-relations, `count`,
`times`, `make_one` / `create_one`).

## Seeders in tests

The shortest form is `seed::<S>()` on the test database, which runs
`S` before the test body; see [Seed before the body](#seed-before-the-body).
Seeders are also functions you've registered with the framework's
seeder registry under a stable name. Two patterns for driving them
from tests by name, one for each axis of intent.

### Run a single seeder by name

```rust
use suprnova::seed;
use my_app::seeders::UsersSeeder;

#[tokio::test]
async fn users_seeder_populates_fixtures() {
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();

    seed::register::<UsersSeeder>();
    seed::run_one("UsersSeeder").await.unwrap();

    let count = User::query().count().await.unwrap();
    assert!(count > 0);
}
```

### Run the full bootstrap seeder set

```rust
use serial_test::serial;
use suprnova::seed;

#[tokio::test]
#[serial]
async fn full_seed_lands_expected_row_counts() {
    seed::clear();                              // start from a known-empty registry
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();

    seed::register::<my_app::seeders::UsersSeeder>();
    seed::register::<my_app::seeders::PostsSeeder>();
    seed::run_all().await.unwrap();

    let users = User::query().count().await.unwrap();
    let posts = Post::query().count().await.unwrap();
    assert_eq!(users, 50);
    assert_eq!(posts, 200);

    seed::clear();
}
```

Two important contract details:

**The seeder registry is process-global.** `seed::register::<S>()`
inserts into a `RwLock<IndexMap>` keyed by `S::name()`. A test that
mutates the registry should call `seed::clear()` at entry, register
the seeders it needs, run, and `clear()` again at exit - and the
test itself should be `#[serial_test::serial]` so two parallel tests
don't fight over the registry. `seed::clear()` is compiled with the
`testing` feature, which is a default feature. `#[suprnova_test]` does **not** auto-
register seeders; only the explicit `seed::register::<>()` call in
your own `bootstrap.rs` or in the test body puts them in the
registry.

**Model-driven seeds vs factory-driven seeds.** A seeder that loops
`User::create(...)` in a `for` fires `Creating` / `Saving` /
`Created` / `Saved` per row and invokes every registered observer.
For bulk seeding where that fanout is unwanted, wrap the loop in
`seed::without_events`:

```rust
seed::without_events(async {
    for i in 0..50 {
        User::create(attrs! { name: format!("user{i}"), email: format!("user{i}@example.com") }).await?;
    }
    Ok::<_, FrameworkError>(())
}).await?;
```

The mute is **task-scoped** - only the work performed inside the
future is silenced; concurrent request handlers and queue workers
continue to fire events normally. Factories fire the same events, so
`without_events` mutes them as well; `create_quietly()` and
`create_many_quietly()` are the shorter form for a factory.

See [Seeding](seeding.md) for the seeder authoring surface and
[Eloquent → Factories](eloquent-factories.md) for the relationship
between the two.

## Parallel-safe database tests

`cargo test` runs tests in parallel by thread. The default
`#[suprnova_test]` expansion (which is `#[tokio::test]`, i.e. a
`current_thread` runtime per test) interacts safely with this for
two reasons:

- **Each test gets its own `sqlite::memory:` connection.** Tests do
  not share DB state. Under `refresh`, each test gets its own
  connection and transaction on the shared database instead.
- **The bound connection lives in the thread-local
  `TestContainer`.** Tests do not share container bindings.

What you don't have to think about: `DB::connection()`, `App::resolve`,
factory persistence, model trait writes - these all transparently
land on the right per-test database. `DB::listen` callbacks and the
query log belong to the test container too, so a test that counts
queries counts only its own.

What you *do* need to think about:

| Surface | Why it's process-global | Mitigation |
| --- | --- | --- |
| `ConnectionRegistry` (`DB::register_named`, `__read_replica__`) | Single `RwLock<HashMap>` shared by the process | `#[serial_test::serial]` for any test that registers or reads named connections |
| The seeder registry | Single `RwLock<IndexMap>` | `#[serial_test::serial]` + `seed::clear()` at entry and exit |
| The Eloquent observer / scope registries | Keyed by `TypeId::<M>()` | Each test should use a unique model struct, or be `#[serial]` and call the registry's `clear()` helper |

The connection-registry refcount makes this safer than it sounds: a
test holding a `TestContainerGuard` keeps the registry alive even
when a *sibling* test's guard drops. You still want `#[serial]` for
the tests that actually mutate the registry, so their reads and
writes can't interleave.

### Multi-thread runtime caveat

`#[suprnova_test]` expands to `#[tokio::test]` with the default
`current_thread` runtime, so the thread-local container path always
works. If you explicitly opt a test into the multi-thread runtime:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parallel_io_test() {
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();
    // PROBLEM: tasks spawned with `tokio::spawn` may run on a
    // worker thread different from the one that built the
    // TestDatabase. They will not see the thread-local
    // TestContainer binding, and DB::connection() will return the
    // global (production) container's value or error.
}
```

Two fixes, depending on what the test does:

1. **Direct connection access** - `db.conn()` still returns the
   right `&DatabaseConnection` regardless of which worker thread
   reads it. If the test only ever talks to the DB through the
   `db` handle (not through `DB::connection()`), the multi-thread
   runtime is fine.

2. **`TestContainer::scope`** - wrap the test body in
   `TestContainer::scope(async { ... }).await` and bind your fakes
   (and the DB connection) inside it. The scope binds the container
   to the task-local layer, which is preserved across awaits even
   when the runtime hops the future between worker threads. For
   spawned sub-tasks, use `TestContainer::spawn` (not bare
   `tokio::spawn`) so the task-local container is captured and
   reinstalled inside the spawned future.

See [Service Container → Lookup order](container.md) for the full
task-local / thread-local / global layering.

## SQLite in-memory vs a real Postgres / MySQL / MariaDB

`fresh` and `sqlite_memory` are SQLite-only: the driver is
`sqlite::memory:`. For the overwhelming majority of test surface - model CRUD, query builder shape, cast
round-trips, relationship loading, observer firing order, soft-delete
semantics - SQLite in-memory is the right tool: zero setup, zero
network, milliseconds per test, perfect isolation, no external
service to keep alive in CI.

There are four cases where SQLite in-memory isn't enough:

1. **Driver-specific SQL.** A query that uses Postgres `LATERAL`,
   `JSONB` operators, `ON CONFLICT ... WHERE`, MySQL window
   functions, or any other dialect-specific surface won't run on
   SQLite. The model+builder path tries to stay generic, but a
   raw-SQL test asserting Postgres-shaped output needs Postgres.
2. **Concurrency under real connection contention.** SQLite
   in-memory is single-connection (see "Why the pool is pinned to
   one connection"). Tests that race two transactions, exercise
   read-replica routing under load, or measure deadlock retry need
   a multi-connection server.
3. **Vector / NoSQL / temporal surfaces.** Suprnova's MariaDB
   `VECTOR` driver, Qdrant integration, Pinecone integration, and
   similar non-SQL drivers cannot be modelled in SQLite at all.
4. **Production parity smoke tests.** A handful of "does this
   actually work on the real DB we deploy to?" tests, gated to
   CI, are worth keeping even when the unit-test layer is SQLite.

For all four cases, point `DATABASE_URL` at that engine and build the
test database with `refresh`, or with `migrate` when the code under
test must see committed rows from a second connection; see
[Tests on the configured database](#tests-on-the-configured-database).
For a driver-level test that needs no migrator, or a second database
beside the primary, build a `DbConnection` against an
operator-supplied `DATABASE_URL`-style env var, env-gate the test
so it skips when the var is absent, and mark it `#[serial]` so two
of them don't fight over the shared real database. The
`MARIADB_URL` pattern in `framework/tests/vector/mariadb.rs` is the
canonical example:

```rust
use serial_test::serial;
use suprnova::database::{DatabaseConfig, DbConnection};

async fn maybe_real_db(test_name: &str) -> Option<DbConnection> {
    let url = match std::env::var("POSTGRES_TEST_URL") {
        Ok(u) if !u.is_empty() => u,
        _ => {
            eprintln!("[{test_name}] skipping: POSTGRES_TEST_URL not set");
            return None;
        }
    };
    let config = DatabaseConfig::builder().url(&url).build();
    Some(DbConnection::connect(&config).await.expect("real DB connects"))
}

#[tokio::test]
#[serial]
async fn jsonb_operator_works_against_postgres() {
    let Some(conn) = maybe_real_db("jsonb_operator_works_against_postgres").await else {
        return;
    };
    // Drive Postgres-specific SQL directly against `conn`.
}
```

The standing convention: name the env var after the target driver
(`POSTGRES_TEST_URL`, `MYSQL_TEST_URL`, `MARIADB_URL`), print a
skip line so a developer running the suite locally sees the test
was skipped (not silently passed), and document the env var in the
test module's leading doc-comment so CI can wire it up.

`MARIADB_URL` is also the variable `MariaDbVectorDriver::from_env()` reads
(with `DATABASE_URL` as its fallback), and `QDRANT_URL` is the one
`QdrantVectorDriver::from_env()` reads. A test that builds either driver
with `from_env` needs no extra wiring. See [Vector](vector.md).

## A worked example

The full app dogfood pattern, combining everything in this chapter:

```rust
use app::migrations::Migrator;
use app::models::posts::Post;
use app::models::users::User;
use serial_test::serial;
use suprnova::testing::TestDatabase;
use suprnova::{Model, attrs, seed, FrameworkError};

#[tokio::test]
#[serial]
async fn users_and_posts_full_seed_round_trip() {
    // 1. Empty seeder registry.
    seed::clear();

    // 2. Fresh in-memory DB with the app's migrator.
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();

    // 3. Register the seeders the test cares about.
    seed::register::<app::seeders::UsersSeeder>();
    seed::register::<app::seeders::PostsSeeder>();

    // 4. Drive the seed inside without_events so observer fanout
    //    doesn't try to enqueue jobs (no queue is running here).
    seed::without_events(async {
        seed::run_all().await
    }).await.unwrap();

    // 5. Read back via the model surface and the raw connection.
    let user_count = User::query().count().await.unwrap();
    assert_eq!(user_count, 50);

    let raw_post_count = db.fetch_one(
        "SELECT COUNT(*) AS n FROM posts",
        vec![],
    ).await.unwrap();
    let n: i64 = raw_post_count.try_get("", "n").unwrap();
    assert_eq!(n, 200);

    // 6. Exercise the cancellable observer path on a fresh model.
    let alice = User::create(attrs! {
        name: "Alice", email: "alice@example.com",
    }).await.unwrap();
    assert!(alice.id > 0);

    seed::clear();
}
```

Step 5 is the part that proves the wiring: the model query and the
raw `fetch_one` are both reading the same in-memory database - the
model surface because the `DB::connection()` lookup found the
`TestContainer` binding, the raw `fetch_one` because `db.conn()`
returns that same connection directly.

## Cross-references

- [Testing](testing.md) - the test harness, `expect!`, `describe!`,
  `test!`, fakes.
- [Database](database.md#testing) - the surface-level testing
  section that introduces `TestDatabase`.
- [Migrations](migrations.md) - the migrations that `refresh` and
  `migrate` run, and the schema dumps that `refresh` loads.
- [Eloquent → Factories](eloquent-factories.md) - factory definition
  syntax, states, sequences, relations.
- [Seeding](seeding.md) - seeder authoring, ordering, idempotency.
- [Service Container](container.md) - task-local vs thread-local
  vs global lookup, which decides what `DB::connection()` resolves
  to inside a test.
- [Mocking & Fakes](mocking.md) - `Storage::fake`, `Mail::fake`,
  `Queue::fake`, `Notify::fake`, and the trait-bind pattern
  for swapping in fake HTTP clients and other external surfaces.
- [HTTP Tests](http-tests.md) - driving handlers through the
  routing stack with a `TestDatabase` bound.
