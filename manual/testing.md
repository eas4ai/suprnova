# Testing

This is the hub chapter for Suprnova's testing surface - the macros, the
in-process database, the container fakes, and the encryption key
helpers your test binaries reach for. The depth-first chapters live
alongside it: [HTTP Tests](http-tests.md) for routes + middleware,
[Database Tests](database-testing.md) for everything around
`TestDatabase`, [Mocking and Fakes](mocking.md) for the seven external
surfaces (Mail, Notify, Queue, Bus, Events, Storage, HTTP client). Read
this one to learn what's in the box; jump to a sibling when you need
the long form.

## The pieces

| Piece | Role |
|---|---|
| `#[tokio::test]` + `TestDatabase::fresh::<Migrator>()` | The default workhorse - every real test in the framework uses this |
| `#[suprnova_test]` | Attribute macro sugar - runs `App::init()` + `App::boot_services()` and builds a `TestDatabase` for you |
| `describe!` + `test!` | Jest-shaped grouping macros, paired with `expect!` for named failure output |
| `expect!` | Fluent assertion macro with typed matchers (equality, option, result, string, vec, ordering) |
| `TestDatabase::fresh` / `sqlite_memory` | In-memory SQLite + container registration, with or without your migrator |
| `TestContainer::fake` / `scope` / `spawn` | Thread-local or task-local DI overrides, hermetic across parallel tests |
| `install_test_encryption_key[ring]` | Deterministic `APP_KEY` for tests that touch encrypted casts or signed payloads |
| `TestClock` | Freeze and move the clock the framework reads, without a sleep - see [Moving the clock](#moving-the-clock) |
| `crypto::testing::encrypt_string_under` | A value encrypted under an old key, for a rotation test |
| `console::test` | Run a console command, read what it printed, and answer what it asks - see [Console](console.md#testing-a-command) |
| Per-surface `fake()` helpers | Mail, Notify, Queue, Bus, Events, Storage, HTTP - see [Mocking](mocking.md) |
| `TestResponse` | Fluent assertions over an HTTP test's `(status, headers, body)` triple - see [HTTP Tests](http-tests.md#fluent-response-assertions-with-testresponse) |
| `AssertableInertia` | Fluent assertions over an Inertia page object - see [HTTP Tests](http-tests.md#testing-inertia-responses) |

You won't reach for everything in one test. A typical action test uses
the first three; a DI-heavy test adds `TestContainer`; an HTTP test
swaps `TestDatabase` for the `handle_request` pipeline; a payments
test installs the encryption keyring.

## The default workhorse

Every real test in the framework looks like this:

```rust
use suprnova::testing::TestDatabase;
use crate::migrations::Migrator;

#[tokio::test]
async fn create_user_persists_it() {
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();

    let alice = User::create(attrs! {
        name: "Alice",
        email: "alice@example.com",
    })
    .await
    .unwrap();

    assert!(alice.id > 0);

    let row = users::Entity::find_by_id(alice.id)
        .one(db.conn())
        .await
        .unwrap();
    assert!(row.is_some());
}
```

`TestDatabase::fresh::<M>()` opens a fresh `sqlite::memory:` connection,
runs your migrator end-to-end, and registers the connection in the test
container. Any code that calls `DB::connection()` or
`App::resolve::<DbConnection>()` afterwards resolves to it - including
the `#[suprnova::model]` query builder and any service you resolved
out of the container. When the `TestDatabase` drops, the registration
goes with it.

The `test_database!()` macro is one-liner sugar for the
`crate::migrations::Migrator` case:

```rust
use suprnova::test_database;

#[tokio::test]
async fn shortcut() {
    let db = test_database!();         // == TestDatabase::fresh::<crate::migrations::Migrator>()
    // ...
}
```

For tests that want precise column-shape control (cast round-trips,
query-builder SQL surface), use `TestDatabase::sqlite_memory()` -
same container wiring, no migrator. The DDL is yours. See
[Database Tests](database-testing.md) for the full catalogue plus the
`execute_unprepared` / `fetch_one` / `fetch_all` helpers.

## `#[suprnova_test]` - when you want the sugar

`#[suprnova_test]` is an attribute macro that wraps `#[tokio::test]`,
calls `App::init()` + `App::boot_services()` so `#[injectable]` types
resolve, and binds a fresh `TestDatabase`. It's optional sugar over
the explicit form above, useful when a test resolves
container-registered services:

```rust
use suprnova::suprnova_test;
use suprnova::{App, testing::TestDatabase};

#[suprnova_test]
async fn create_user_via_action(db: TestDatabase) {
    let action = App::resolve::<CreateUserAction>().unwrap();
    let user = action.execute("test@example.com").await.unwrap();

    assert_eq!(user.email, "test@example.com");
    assert!(user.id > 0);
}
```

If the function takes a `TestDatabase` parameter (by name), the macro
binds the fresh database to that name. If it doesn't, the database is
still constructed and registered (so `DB::connection()` works) - it
just isn't bound to a local.

Override the migrator with the `migrator = …` key:

```rust
#[suprnova_test(migrator = my_crate::tests::IsolatedMigrator)]
async fn create_user_with_isolated_schema(db: TestDatabase) {
    // ...
}
```

Unknown keys are a compile error (typo `migrtor = …` won't silently
keep the default migrator).

## `describe!` and `test!` - when grouping helps

For test files where the same action has many cases, the Jest-shaped
`describe!` + `test!` pair gives you nested grouping and named failure
output:

```rust
use suprnova::{App, describe, test, expect, testing::TestDatabase};
use crate::migrations::Migrator;

describe!("ListTodosAction", {
    test!("returns empty list when no todos exist", async fn(db: TestDatabase) {
        let todos = App::resolve::<ListTodosAction>().unwrap().execute().await.unwrap();
        expect!(todos).to_be_empty();
    });

    test!("returns all todos", async fn(db: TestDatabase) {
        Todo::create(attrs! { title: "Buy bread" }).await.unwrap();
        Todo::create(attrs! { title: "Walk dog" }).await.unwrap();

        let todos = App::resolve::<ListTodosAction>().unwrap().execute().await.unwrap();
        expect!(todos).to_have_length(2);
    });

    describe!("with pagination", {
        test!("returns first page", async fn(db: TestDatabase) {
            // nested groups compose
        });
    });
});
```

`test!` accepts three shapes:

```rust
// Async test with TestDatabase parameter
test!("creates a user", async fn(db: TestDatabase) { … });

// Async test without database
test!("calculates the right sum", async fn() { … });

// Sync test
test!("adds numbers", fn() { … });
```

The named-test wrapper threads the test name through the `expect!`
machinery so a failure surfaces:

```text
Test: "returns all todos"
  at src/actions/todo_action.rs:25

  expect!(actual).to_equal(expected)

  Expected: 2
  Received: 0
```

Without `describe!`/`test!` you get the standard `panic!` output. With
them, the location and human-readable test name lead the message.

## `expect!` - the matcher catalog

`expect!(value)` returns an `Expect<T>` wrapper. The matchers are typed
to `T` - calling `to_be_some()` on a `String` is a compile error, not
a runtime panic.

```rust
use suprnova::expect;

// Equality (T: Debug + PartialEq)
expect!(actual).to_equal(expected);
expect!(actual).to_not_equal(unexpected);

// Boolean
expect!(condition).to_be_true();
expect!(condition).to_be_false();

// Option<T>
expect!(option).to_be_some();
expect!(option).to_be_none();
expect!(option).to_contain_value(5);     // Some(5) check

// Result<T, E>
expect!(result).to_be_ok();
expect!(result).to_be_err();

// String / &str
expect!(s).to_contain("substring");
expect!(s).to_start_with("prefix");
expect!(s).to_end_with("suffix");
expect!(s).to_have_length(10);
expect!(s).to_be_empty();

// Vec<T>
expect!(v).to_have_length(3);
expect!(v).to_contain(&item);
expect!(v).to_be_empty();

// Ordering (T: Debug + PartialOrd)
expect!(10).to_be_greater_than(5);
expect!(5).to_be_less_than(10);
expect!(10).to_be_greater_than_or_equal(10);
expect!(5).to_be_less_than_or_equal(5);
```

You can use `expect!` outside `test!` - the file/line in the failure
message comes from `concat!(file!(), ":", line!())`. The named-test
header is the only thing the macro doesn't add on its own.

## `TestContainer` - DI fakes that don't bleed

The container chapter covers the [three-layer lookup](container.md) in
detail. For tests, the two entry points are `TestContainer::fake()`
(thread-local) and `TestContainer::scope(…).await` (task-local).

### Thread-local, the common case

`TestContainer::fake()` returns a guard. Until the guard drops,
`TestContainer::singleton` / `bind` / `factory` writes land on the
thread-local override layer and shadow the global container:

```rust
use std::sync::Arc;
use suprnova::App;
use suprnova::testing::TestContainer;

#[tokio::test]
async fn order_dispatches_email() {
    let _guard = TestContainer::fake();

    let fake = Arc::new(FakeEmailGateway::new());
    let probe = Arc::clone(&fake);
    TestContainer::bind::<dyn EmailGateway>(fake);

    place_order(123).await.unwrap();

    assert_eq!(probe.sent_count(), 1);
}
```

The guard clears the container it installed, wherever it drops. A
harness can share it with spawned tasks inside an `Arc`, so the last
reference may drop on another thread: that thread's own container is
left alone, and this one's is gone at its next lookup. Only
`TestContainer::fake()` makes a guard.

`TestDatabase::fresh` / `sqlite_memory` install their own
`TestContainer::fake` guard internally - you don't stack them unless
you're testing the registry itself.

### Task-local, for `multi_thread` runtimes

The thread-local layer is set on whichever OS thread called `fake()`.
A `multi_thread` tokio runtime can migrate your future to another
worker thread across an `.await`, and the override silently disappears.
`TestContainer::scope` solves that by binding the override to the
future instead:

```rust
use suprnova::testing::TestContainer;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cross_worker_safe() {
    TestContainer::scope(async {
        TestContainer::bind::<dyn HttpClient>(Arc::new(FakeHttpClient::new()));
        do_async_work_that_may_hop_workers().await;
    })
    .await;
}
```

`tokio::spawn`'d sub-tasks do not inherit tokio task-locals; use
`TestContainer::spawn` instead - it captures the current scope's
container and re-installs it inside the spawned future:

```rust
TestContainer::scope(async {
    TestContainer::bind::<dyn HttpClient>(Arc::new(FakeHttpClient::new()));
    let h = TestContainer::spawn(async {
        App::make::<dyn HttpClient>().unwrap()  // sees the fake
    });
    let _client = h.await.unwrap();
})
.await;
```

### Why named connections survive parallel tests

The thread-local container is per-test, but Suprnova also has a
process-global `ConnectionRegistry` keyed by name (`__read_replica__`,
custom connection labels) that survives a thread-local reset. A naive
`Drop` impl would clear that registry every time *any*
`TestContainerGuard` went away - wiping another concurrent test's
named connection halfway through it running.

So the guards are refcounted process-wide: `fake()` counts up, each
guard's drop counts down, and only the last guard to drop clears the
named registry. Two parallel tests using
`__read_replica__` are safe: whichever guard drops last owns the
clear.

You don't call this from a test - it runs from `TestContainerGuard`'s
`Drop`. You only need to know it's there if you're debugging a
"named connection vanished mid-test" symptom, which usually means a
sibling test forgot to wait for its own guard to drop first.

## Console tests

`suprnova::console::test` runs a console command through the dispatcher the
`console` binary uses and returns a `ConsoleRun` with what the command
printed, what it asked, and how it ended. A command prints with
`console::line` and asks with `console::ask` and `console::confirm`, so the
test can read the output and prepare the answers:

```rust
use suprnova::console;

#[tokio::test]
async fn greet_says_hello() {
    let run = console::test(["greet", "--name", "Alice"]).run().await;

    run.assert_successful();
    run.assert_output_contains("Hello, Alice!");
}
```

See [Console](console.md#testing-a-command) for the questions, the assert
methods, and the rules a command follows to be testable.

## Encryption key test helpers

Tests that exercise encrypted casts (`casts = { secret = AsEncrypted }`
on a `#[model(...)]`), signed payloads, or the keyring's previous-key
fallback need an `APP_KEY` installed in-process. The framework ships
two test-only helpers under the `testing` feature:

```rust
use suprnova::testing::install_test_encryption_key;

#[tokio::test]
async fn cast_roundtrip() {
    install_test_encryption_key();   // idempotent; deterministic 32-zero-byte key
    let db = TestDatabase::sqlite_memory().await.unwrap();
    // … encrypt + read back …
}
```

`install_test_encryption_key` is idempotent - the underlying `Crypt`
facade is `OnceLock`-backed, so the second call is a no-op. Most cast
test binaries call it from every test that touches an encrypted cast;
the first wins, the rest are free.

For rotation tests (writes under the old key, reads under the new
key), use the keyring variant:

```rust
use suprnova::crypto::EncryptionKey;
use suprnova::testing::install_test_encryption_keyring;

let new = EncryptionKey::from_base64("...").unwrap();
let old = EncryptionKey::from_base64("...").unwrap();
let installed = install_test_encryption_keyring(new, vec![old]);
assert!(installed, "first install wins");
```

The keyring helper returns `true` only if the call actually installed
the ring (the `OnceLock` was empty). To build a value that was encrypted
under an old key, use `suprnova::crypto::testing::encrypt_string_under`
rather than installing twice. It takes the key, the `CryptPurpose`, and the
plaintext, and returns the wire string. `encrypt_string_for_under` takes a
context between the purpose and the plaintext, the way
`Crypt::encrypt_string_for` does.

```rust
use suprnova::crypto::testing::encrypt_string_under;
use suprnova::testing::install_test_encryption_keyring;
use suprnova::{Crypt, CryptPurpose, EncryptionKey};

let new = EncryptionKey::generate();
let old = EncryptionKey::generate();
install_test_encryption_keyring(new, vec![old.clone()]);

// A value written when `old` was the current key.
let stored = encrypt_string_under(&old, CryptPurpose::Cast, "a value").unwrap();

let (plain, origin) =
    Crypt::decrypt_string_with_origin(CryptPurpose::Cast, &stored).unwrap();
assert_eq!(plain, "a value");
assert!(origin.needs_reencryption());
```

Both `install_test_encryption_key` and the keyring helper live in
`suprnova::testing`. The `suprnova::crypto::testing` functions are compiled
with the `testing` feature, so an application that ships without the feature
has none of them. They bypass the production `APP_KEY` validation path.

## Moving the clock

A test of an expiry should not sleep. The framework reads the time through
`suprnova::clock::now()`: signed URL deadlines, session idle timeouts, due
scheduled tasks, prune cutoffs, queue availability, the timestamps a
`#[model]` writes and the window of the Redis rate limiter. Under the
`testing` feature, `TestClock` moves what `clock::now()` returns, so a test
jumps to the moment it wants.

`tokio::time::pause` does not do this. It moves the timers of Tokio and
nothing that reads the wall clock.

```rust
use chrono::{DateTime, Duration, Utc};
use suprnova::http::Request;
use suprnova::routing::SignatureVerdict;
use suprnova::routing::url::{signature_verdict, signed_url};
use suprnova::testing::{install_test_encryption_key, TestClock};

#[test]
fn a_signed_url_expires() {
    install_test_encryption_key();
    let start = DateTime::from_timestamp(1_900_000_000, 0).unwrap();
    let clock = TestClock::travel_to(start);

    let deadline = (suprnova::clock::now() + Duration::minutes(10)).timestamp();
    let url = signed_url("/download?file=report", Some(deadline)).unwrap();
    let verdict = || signature_verdict(&Request::for_test("GET", &url)).unwrap();

    assert_eq!(verdict(), SignatureVerdict::Valid);

    clock.advance(Duration::minutes(9));
    assert_eq!(verdict(), SignatureVerdict::Valid);

    clock.advance(Duration::minutes(2));
    assert_eq!(verdict(), SignatureVerdict::Expired);
}
```

### The guard

| Call | Effect |
|---|---|
| `TestClock::freeze()` | Stops the clock of the current thread at the time of the call |
| `TestClock::travel_to(at)` | Sets the clock of the current thread to `at` (a `DateTime<Utc>`) and stops it there |
| `guard.advance(by)` | Moves the clock forward by a `chrono::Duration`, or back for a negative one |
| `guard.set(at)` | Sets the clock to `at` |
| `guard.now()` | The time the clock shows |

Both constructors return a `TestClockGuard`. Keep it in a binding, because
dropping it at once gives the system clock back.

A clock under a guard stands still until you move it. Two reads in a row
return the same time, so "exactly at the deadline" is a state your test can
hold.

When the guard drops, the thread gets its previous clock back. That also
happens when the test panics. Guards nest: an inner guard hands back the time
of the outer one when it drops. A guard is not `Send`, because it belongs to
one thread.

### Threads and tasks

A guard holds for the current thread. A `#[tokio::test]` of the default
`current_thread` flavour runs the test, and any task it spawns, on that
thread, so a guard works across `.await` there.

For a `multi_thread` runtime, or for code that spawns onto other threads,
use `TestClock::scope`. It sets the clock for one future, across awaits and
worker-thread hops. The closure gets a `TestClockHandle` with `advance`,
`set` and `now`:

```rust
use chrono::{DateTime, Duration};
use suprnova::testing::TestClock;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoped_clock() {
    let at = DateTime::from_timestamp(1_900_000_000, 0).unwrap();

    TestClock::scope(at, |clock| async move {
        assert_eq!(suprnova::clock::now(), at);

        clock.advance(Duration::hours(1));
        assert_eq!(suprnova::clock::now(), at + Duration::hours(1));

        // A task started with `tokio::spawn` does not inherit the scope.
        // Run it through the handle to give it the same clock.
        let mover = clock.clone();
        let seen = tokio::spawn(async move {
            mover.run(async { suprnova::clock::now() }).await
        })
        .await
        .unwrap();
        assert_eq!(seen, at + Duration::hours(1));
    })
    .await;
}
```

Outside the scope, the system clock answers again.

### Parallel tests

No call moves the clock of the whole process. A guard affects its own
thread and a scope affects its own future, so tests that run side by side
do not see each other's time.

### Your own code

Code of your own that reads the time to make a decision, such as "is this
invite older than seven days", should read `suprnova::clock::now()` when
its tests need to move that time. A direct `chrono::Utc::now()` follows the
system clock.

The health endpoint reports the time of the machine and ignores a test
clock.

### The `testing` feature

`TestClock`, `TestClockGuard` and `TestClockHandle` exist when the `testing`
feature is on. It is in the default feature set, so an application that
depends on `suprnova` with default features has them in its tests. See
[The `testing` feature and production builds](#the-testing-feature-and-production-builds)
for a build that leaves the feature off. Without the feature,
`suprnova::clock::now()` is `Utc::now()` and nothing else.

## The `testing` feature and production builds

`suprnova` exposes its test helpers (`Storage::fake()`, `TestContainer`,
`TestDatabase`, crypto rotation hooks like `_test_install_key`) behind a
Cargo feature named `testing`. The feature is in the default set, so
consuming test suites get them for free:

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v3.1.0" }

[dev-dependencies]
# `testing` is on transitively via the dependency above - nothing extra.
```

The hooks are `#[doc(hidden)]` and prefixed with `_test_`, so they
aren't reachable from idiomatic application code even when the feature
is on. The load-bearing safeguard is `Server::from_config`: it
validates `APP_KEY` on **every** boot, not only when the keyring is
uninitialized. A pre-installed test key cannot bypass that check -
boot fails fast if `APP_KEY` is missing or malformed regardless of
whether anything in-process pre-installed a key.

If you prefer the helpers not to be linked into your production
artifact at all (defence in depth), depend on `suprnova` with default
features off and enable only what you ship:

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v3.1.0", default-features = false, features = ["..."] }

[dev-dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v3.1.0", features = ["testing", "..."] }
```

This is a tightening, not a fix - boot validation closes the actual
exploit regardless of which posture you pick.

### Why Suprnova diverges

Laravel's PHP test harness gets parallel-test isolation almost for free
because the runtime is single-threaded per request and tests fork a
new process per file. The Suprnova test binary is one process running
many `#[tokio::test]`s on one or more worker threads concurrently. A
single global container would mean one test's fake bleeds into the
next test's lookup the instant they overlap on a worker thread.

That's why `TestContainer` has both flavours - thread-local for the
common `current_thread` case, task-local for `multi_thread`. The
refcounted clear of the process-global
named-connection registry exists for the same reason: shared state that
can't be made per-test must at least know not to wipe itself while
another test is still leaning on it.

The matcher catalogue (`expect!`) is typed because Rust lets it be.
Jest's `expect(x).toBeSome()` only knows at runtime whether `x` is an
`Option`; Suprnova's `Expect<T>` knows at compile time, so a wrong
matcher is a build error, not a flaky test.

## Where each piece lives

| Piece | Source |
|---|---|
| `#[suprnova_test]` attribute macro | `suprnova-macros/src/suprnova_test.rs` |
| `describe!` / `test!` proc-macros | `suprnova-macros/src/describe.rs`, `test_macro.rs` |
| `expect!` macro + `Expect<T>` matchers | `framework/src/lib.rs` (macro), `framework/src/testing/expect.rs` (impls) |
| `TestDatabase::fresh` / `sqlite_memory` / helpers | `framework/src/database/testing.rs` |
| `test_database!` macro | `framework/src/database/testing.rs` |
| `TestContainer` + `TestContainerGuard` | `framework/src/container/testing.rs` |
| `install_test_encryption_key[ring]` | `framework/src/testing/mod.rs` |
| `clock::now`, `TestClock`, `TestClockGuard`, `TestClockHandle` | `framework/src/clock.rs` (re-exported from `suprnova::testing`) |
| `encrypt_string_under`, `encrypt_string_for_under` | `framework/src/crypto/testing.rs` |
| `console::test`, `ConsoleRun` | `framework/src/console/testing.rs` |
| Per-surface fakes (Mail, Notify, Queue, Bus, Events, Storage, HTTP) | per-domain `testing` submodules - see [Mocking](mocking.md) |
| `TestResponse` | `framework/src/testing/response.rs` |
| `AssertableInertia`, `ReloadRequest` | `framework/src/testing/inertia.rs` |

## Running tests

The standard cargo invocations apply:

```bash
# Whole workspace
cargo test --workspace

# One crate
cargo test -p suprnova

# One test by name (substring match)
cargo test create_user_persists_it

# With println! and dbg! output
cargo test -- --nocapture
```

Suprnova doesn't ship its own test runner; the framework integrates
with cargo's. Database tests run in parallel by default - the
thread-local container and per-test in-memory SQLite are designed for
exactly that.

## Next

- [HTTP Tests](http-tests.md) - driving the full request pipeline
  through `handle_request`
- [Database Tests](database-testing.md) - `TestDatabase`, factories
  in tests, seeders in tests, parallel-safe DB testing
- [Mocking and Fakes](mocking.md) - the seven external-surface fakes
  and the patterns they share
- [Service Container](container.md) - the three-layer lookup that
  `TestContainer` overrides
- [Error Model](error-model.md) - `FrameworkError` shapes you'll be
  asserting on
