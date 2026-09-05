//! RenderCache Tier 0, Task 11 - the database-authoritative generation
//! ledger and its migration.
//!
//! Generation truth lives in the application database, advances inside the
//! owning transaction, disappears on rollback, and is logically
//! append-only. A digest the ledger has never seen reads back as
//! generation 0, not absent: see `current_reports_zero_for_a_never_advanced_digest`
//! and `current_zero_fills_every_requested_digest_even_when_some_are_found`
//! for why that distinction matters (`GenerationSet::get_digest` vs `None`
//! is exactly what a decoded cache entry's freshness recheck compares).

use sea_orm_migration::{MigrationTrait, MigratorTrait};
use suprnova::render_cache::ledger::{SqlGenerationLedger, advance_in_current_transaction};
use suprnova::render_cache::{DependencyIdentity, RenderCache};
use suprnova::testing::TestDatabase;
use suprnova::{DB, FrameworkError};
use suprnova_live::render_cache::generation::GenerationLedger;

mod render_cache_middleware_support;
use render_cache_middleware_support::{
    Harness, boot_with_render_cache_on_live_server_for_test, counting_route, dispatch_get,
};

struct RenderCacheTestMigrator;

#[async_trait::async_trait]
impl MigratorTrait for RenderCacheTestMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(suprnova::render_cache::migration::Migration)]
    }
}

async fn boot() -> TestDatabase {
    suprnova::render_cache::mark_installed();
    TestDatabase::fresh::<RenderCacheTestMigrator>()
        .await
        .expect("render cache migration should apply cleanly to a fresh SQLite database")
}

#[tokio::test]
async fn current_reports_zero_for_a_never_advanced_digest() {
    let _db = boot().await;
    let ledger = SqlGenerationLedger::new();
    let posts = DependencyIdentity::table("posts");

    let set = ledger.current(&[posts.digest()]).await.expect("current");

    // The exact assertion R36 exists to pin: an unobserved digest is
    // `Some(0)`, never `None`. A `current` that only inserts rows the query
    // actually found would leave this `None`, and `CoherenceCheck::compare`
    // would then read `Some(0)` (observed) against `None` (reread), judge
    // them unequal, and report every fresh dependency as moved forever.
    assert_eq!(set.get_digest(&posts.digest()), Some(0));
}

#[tokio::test]
async fn current_zero_fills_every_requested_digest_even_when_some_are_found() {
    let _db = boot().await;
    let ledger = SqlGenerationLedger::new();
    let posts = DependencyIdentity::table("posts");
    let comments = DependencyIdentity::table("comments");

    DB::transaction(|_tx| {
        Box::pin(async move {
            advance_in_current_transaction(&[DependencyIdentity::table("posts")]).await
        })
    })
    .await
    .expect("commit");

    // `posts` has a row (generation 1); `comments` has never been touched
    // and has none. Both digests must come back present in the same
    // `GenerationSet` - this is the scenario where "only insert what the
    // query returned" silently drops the untouched half.
    let set = ledger
        .current(&[posts.digest(), comments.digest()])
        .await
        .expect("current");
    assert_eq!(set.get_digest(&posts.digest()), Some(1));
    assert_eq!(set.get_digest(&comments.digest()), Some(0));
}

#[tokio::test]
async fn generations_advance_only_when_the_owning_transaction_commits() {
    let _db = boot().await;
    let ledger = SqlGenerationLedger::new();
    let posts = DependencyIdentity::table("posts");

    assert_eq!(
        ledger
            .current(&[posts.digest()])
            .await
            .expect("current")
            .get(&posts),
        Some(0)
    );

    DB::transaction(|_tx| {
        Box::pin(async move {
            advance_in_current_transaction(&[DependencyIdentity::table("posts")]).await
        })
    })
    .await
    .expect("commit");

    assert_eq!(
        ledger
            .current(&[posts.digest()])
            .await
            .expect("current")
            .get(&posts),
        Some(1)
    );

    let rolled_back: Result<(), FrameworkError> = DB::transaction(|_tx| {
        Box::pin(async move {
            advance_in_current_transaction(&[DependencyIdentity::table("posts")]).await?;
            Err(FrameworkError::internal("abort"))
        })
    })
    .await;
    assert!(rolled_back.is_err());

    assert_eq!(
        ledger
            .current(&[posts.digest()])
            .await
            .expect("current")
            .get(&posts),
        Some(1),
        "rollback advances nothing"
    );

    // `DB::scalar` decodes the aggregate by ordinal position rather than
    // through `DynamicRow`'s static column-type detection: SQLite reports
    // no declared type for a bare `COUNT(*)` expression column, so the
    // `DB::select` + `DynamicRow::get_int("n")` path silently finds nothing
    // to decode and returns an empty row - a real trap for this exact
    // query shape, not a fact about the ledger under test.
    let log_rows: i64 = DB::scalar(
        "SELECT COUNT(*) FROM suprnova_render_generation_log",
        vec![],
    )
    .await
    .expect("log count");
    assert_eq!(log_rows, 1, "the log is append-only per committed change");

    // A count alone would still pass if the row logged the wrong identity,
    // generation, or epoch. Assert what actually landed.
    let log_row = DB::select(
        "SELECT identity, generation, epoch FROM suprnova_render_generation_log WHERE identity = ?",
        vec![hex::encode(posts.digest()).into()],
    )
    .await
    .expect("log row")
    .into_iter()
    .next()
    .expect("one log row for posts");
    assert_eq!(
        log_row.get_string("identity").expect("identity"),
        hex::encode(posts.digest())
    );
    assert_eq!(log_row.get_int("generation").expect("generation"), 1);
    assert_eq!(log_row.get_int("epoch").expect("epoch"), 1);
}

#[tokio::test]
async fn advancing_two_identities_in_one_transaction_logs_two_rows() {
    let _db = boot().await;

    DB::transaction(|_tx| {
        Box::pin(async move {
            advance_in_current_transaction(&[
                DependencyIdentity::table("posts"),
                DependencyIdentity::table("comments"),
            ])
            .await
        })
    })
    .await
    .expect("commit");

    let log_rows: i64 = DB::scalar(
        "SELECT COUNT(*) FROM suprnova_render_generation_log",
        vec![],
    )
    .await
    .expect("log count");
    assert_eq!(log_rows, 2);

    let comments_log_row = DB::select(
        "SELECT identity, generation, epoch FROM suprnova_render_generation_log WHERE identity = ?",
        vec![hex::encode(DependencyIdentity::table("comments").digest()).into()],
    )
    .await
    .expect("comments log row")
    .into_iter()
    .next()
    .expect("one log row for comments");
    assert_eq!(
        comments_log_row.get_string("identity").expect("identity"),
        hex::encode(DependencyIdentity::table("comments").digest())
    );
    assert_eq!(
        comments_log_row.get_int("generation").expect("generation"),
        1
    );
    assert_eq!(comments_log_row.get_int("epoch").expect("epoch"), 1);

    let ledger = SqlGenerationLedger::new();
    let posts = DependencyIdentity::table("posts");
    let comments = DependencyIdentity::table("comments");
    let set = ledger
        .current(&[posts.digest(), comments.digest()])
        .await
        .expect("current");
    assert_eq!(set.get(&posts), Some(1));
    assert_eq!(set.get(&comments), Some(1));
}

#[tokio::test]
async fn advancing_the_same_identity_twice_advances_generation_and_logs_twice() {
    let _db = boot().await;
    let posts = DependencyIdentity::table("posts");

    for _ in 0..2 {
        DB::transaction(|_tx| {
            Box::pin(async move {
                advance_in_current_transaction(&[DependencyIdentity::table("posts")]).await
            })
        })
        .await
        .expect("commit");
    }

    let ledger = SqlGenerationLedger::new();
    let set = ledger.current(&[posts.digest()]).await.expect("current");
    assert_eq!(set.get(&posts), Some(2));

    let log_rows: i64 = DB::scalar(
        "SELECT COUNT(*) FROM suprnova_render_generation_log WHERE identity = ?",
        vec![hex::encode(posts.digest()).into()],
    )
    .await
    .expect("log count");
    assert_eq!(log_rows, 2);
}

#[tokio::test]
async fn advance_outside_a_transaction_fails_and_advances_nothing() {
    let _db = boot().await;
    let posts = DependencyIdentity::table("posts");

    let result = advance_in_current_transaction(&[DependencyIdentity::table("posts")]).await;
    assert!(
        result.is_err(),
        "a generation advance outside the owning transaction must be refused, \
         not silently committed on its own"
    );

    let ledger = SqlGenerationLedger::new();
    let set = ledger.current(&[posts.digest()]).await.expect("current");
    assert_eq!(set.get(&posts), Some(0));
}

#[tokio::test]
async fn advance_with_no_identities_does_not_require_a_transaction() {
    let _db = boot().await;

    // `GenerationLedger::current` short-circuits on an empty request without
    // touching the database; `advance_in_current_transaction` must match
    // that, not demand an ambient transaction (or issue the epoch SELECT)
    // for a call that has nothing to advance.
    advance_in_current_transaction(&[])
        .await
        .expect("advancing nothing must not require an owning transaction");

    let log_rows: i64 = DB::scalar(
        "SELECT COUNT(*) FROM suprnova_render_generation_log",
        vec![],
    )
    .await
    .expect("log count");
    assert_eq!(log_rows, 0, "advancing nothing must not touch the log");
}

#[tokio::test]
async fn the_epoch_starts_at_one_and_advances_on_demand() {
    let _db = boot().await;
    let ledger = SqlGenerationLedger::new();

    assert_eq!(ledger.epoch().await.expect("epoch"), 1);
    ledger.advance_epoch().await.expect("advance");
    assert_eq!(ledger.epoch().await.expect("epoch"), 2);
}

#[tokio::test]
async fn migration_up_can_run_twice_without_erroring() {
    let db = boot().await;

    // Every `create_table` in the migration uses `if_not_exists()`, which
    // implies the migration is safe to re-run - but the epoch seed used to
    // be an unconditional `INSERT`, which would fail on the primary key the
    // second time. Prove the implication actually holds.
    let manager = sea_orm_migration::SchemaManager::new(db.conn());
    suprnova::render_cache::migration::Migration
        .up(&manager)
        .await
        .expect("running up() a second time must be a no-op, not a primary-key violation");

    let epoch: i64 = DB::scalar(
        "SELECT epoch FROM suprnova_render_epochs WHERE singleton = 1",
        vec![],
    )
    .await
    .expect("epoch");
    assert_eq!(
        epoch, 1,
        "the second run must not reset or duplicate the singleton row"
    );
    let epoch_rows: i64 = DB::scalar("SELECT COUNT(*) FROM suprnova_render_epochs", vec![])
        .await
        .expect("epoch row count");
    assert_eq!(epoch_rows, 1, "still exactly one singleton row");
}

// --- Live-DB tests (gated by #[ignore]) ---
//
// `render_cache_ledger` is a mixed file, same shape as `pagination.rs`:
// the tests above run against SQLite unconditionally; the four below
// exercise the same migration and ledger against real Postgres / MySQL,
// where the upsert dialect, the placeholder syntax, and the `CHECK`
// constraint on `suprnova_render_epochs` actually differ from SQLite's -
// plus one pair that only a real server can prove at all: that two
// concurrent transactions advancing the same identities in opposite
// orders do not deadlock.
//
// Skipped by default. To run them, point the URL env var at a DISPOSABLE
// database and pass `--ignored`:
//
//   PG_TEST_URL=postgres://postgres:pw@127.0.0.1:55998/suprnova_test \
//     cargo test -p suprnova --test render_cache_ledger -- --ignored live_postgres
//
//   MYSQL_TEST_URL=mysql://root:pw@127.0.0.1:55997/suprnova_test \
//     cargo test -p suprnova --test render_cache_ledger -- --ignored live_mysql

async fn try_connect_live(url: &str) -> Option<sea_orm::DatabaseConnection> {
    use sea_orm::ConnectOptions;
    use std::time::Duration;
    let mut opts = ConnectOptions::new(url.to_string());
    opts.connect_timeout(Duration::from_secs(2))
        .acquire_timeout(Duration::from_secs(2));
    sea_orm::Database::connect(opts).await.ok()
}

/// Drops the three tables if they linger from a prior failed run (this
/// migration's `up` has no `IF NOT EXISTS` escape for the singleton insert),
/// then applies the migration fresh and mounts the connection on the
/// thread-local test container so `DB::*` resolves to it.
async fn reset_and_migrate(
    conn: sea_orm::DatabaseConnection,
) -> suprnova::testing::TestContainerGuard {
    use sea_orm::ConnectionTrait;
    for table in [
        "suprnova_render_epochs",
        "suprnova_render_generation_log",
        "suprnova_render_generations",
    ] {
        let _ = conn
            .execute_raw(sea_orm::Statement::from_string(
                conn.get_database_backend(),
                format!("DROP TABLE IF EXISTS {table}"),
            ))
            .await;
    }
    let manager = sea_orm_migration::SchemaManager::new(&conn);
    suprnova::render_cache::migration::Migration
        .up(&manager)
        .await
        .expect("migration applies to the live database");

    suprnova::render_cache::mark_installed();
    let guard = suprnova::testing::TestContainer::fake();
    suprnova::testing::TestContainer::singleton(suprnova::DbConnection::from_raw(conn));
    guard
}

#[tokio::test]
#[ignore = "requires live Postgres; run with --ignored live_postgres"]
async fn live_postgres_generation_ledger_advances_and_reads() {
    let url = std::env::var("PG_TEST_URL")
        .expect("set PG_TEST_URL to a disposable Postgres - this test drops and recreates tables");
    let conn = try_connect_live(&url)
        .await
        .expect("Postgres test DB not reachable - check PG_TEST_URL");
    let _guard = reset_and_migrate(conn).await;

    let ledger = SqlGenerationLedger::new();
    let posts = DependencyIdentity::table("posts");
    assert_eq!(
        ledger.current(&[posts.digest()]).await.unwrap().get(&posts),
        Some(0)
    );

    DB::transaction(|_tx| {
        Box::pin(async move {
            advance_in_current_transaction(&[DependencyIdentity::table("posts")]).await
        })
    })
    .await
    .expect("commit");
    assert_eq!(
        ledger.current(&[posts.digest()]).await.unwrap().get(&posts),
        Some(1)
    );
    assert_eq!(ledger.epoch().await.unwrap(), 1);
    ledger.advance_epoch().await.unwrap();
    assert_eq!(ledger.epoch().await.unwrap(), 2);
}

/// Runs two concurrent `advance_in_current_transaction` transactions over
/// the same two identities in opposite orders and asserts both commit.
///
/// Before the fix, `advance_in_current_transaction` locked rows in the
/// caller's slice order - the collector's first-seen order, which is
/// request-dependent. Two overlapping transactions requesting `[posts,
/// comments]` and `[comments, posts]` would each hold one row and block on
/// the other's, and the database's deadlock detector kills one of them.
/// `DB::transaction` does not retry, so that abort would propagate into
/// whatever business transaction the caller spliced this call into.
/// Sorting by digest before the loop makes every caller lock in the same
/// global order, so the circular wait this reproduces cannot form.
///
/// The `Barrier` synchronizes both tasks' first statement as closely as two
/// real network round trips allow, but on its own that is not reliable
/// proof of anything: against a local, unloaded server both transactions
/// can run to completion faster than the two tasks actually interleave, so
/// this would pass even with the caller-order bug still in place. Confirmed
/// by hand, reverting the sort:
///
/// - On MySQL/MariaDB, the barrier alone was enough - InnoDB's row locking
///   under `INSERT ... ON DUPLICATE KEY UPDATE` made the bad interleaving
///   land reliably (3/3 manual runs), and the test failed with SQLSTATE
///   40001 every time.
/// - On Postgres, the barrier alone was NOT enough - 5/5 manual runs
///   passed even with the bug present, because a local `ON CONFLICT`
///   upsert resolves faster than the two tasks reliably overlap. The
///   Postgres test below additionally installs a `BEFORE INSERT OR UPDATE`
///   trigger on `suprnova_render_generations` that sleeps briefly, widening
///   the window between a transaction's first and second lock acquisition
///   enough that the bad interleaving lands every time (3/3 manual runs
///   deadlocked with the bug present, 3/3 passed once the sort was
///   restored, under the identical widened timing).
async fn assert_concurrent_opposite_order_advances_do_not_deadlock() {
    let posts = DependencyIdentity::table("posts");
    let comments = DependencyIdentity::table("comments");
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));

    let task_a = {
        let barrier = std::sync::Arc::clone(&barrier);
        let posts = posts.clone();
        let comments = comments.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            DB::transaction(|_tx| {
                Box::pin(async move { advance_in_current_transaction(&[posts, comments]).await })
            })
            .await
        })
    };
    let task_b = {
        let barrier = std::sync::Arc::clone(&barrier);
        let posts = posts.clone();
        let comments = comments.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            DB::transaction(|_tx| {
                Box::pin(async move { advance_in_current_transaction(&[comments, posts]).await })
            })
            .await
        })
    };

    let (result_a, result_b) = tokio::join!(task_a, task_b);
    result_a
        .expect("task a must not panic")
        .expect("transaction a must not be aborted by a deadlock");
    result_b
        .expect("task b must not panic")
        .expect("transaction b must not be aborted by a deadlock");

    let ledger = SqlGenerationLedger::new();
    let set = ledger
        .current(&[posts.digest(), comments.digest()])
        .await
        .expect("current");
    assert_eq!(set.get(&posts), Some(2), "both transactions advanced posts");
    assert_eq!(
        set.get(&comments),
        Some(2),
        "both transactions advanced comments"
    );
}

#[tokio::test]
#[ignore = "requires live Postgres; run with --ignored live_postgres"]
async fn live_postgres_concurrent_advances_in_opposite_order_do_not_deadlock() {
    let url = std::env::var("PG_TEST_URL")
        .expect("set PG_TEST_URL to a disposable Postgres - this test drops and recreates tables");
    let conn = try_connect_live(&url)
        .await
        .expect("Postgres test DB not reachable - check PG_TEST_URL");
    let _guard = reset_and_migrate(conn).await;
    widen_postgres_lock_window().await;
    assert_concurrent_opposite_order_advances_do_not_deadlock().await;
}

/// Widens the window between a transaction's first and second lock
/// acquisition on Postgres, so the barrier-synchronized concurrency test
/// above reliably overlaps instead of racing to completion on a fast local
/// server - see that test's doc for the manual runs that showed the barrier
/// alone is not sufficient here. Self-cleaning: the trigger lives on
/// `suprnova_render_generations`, which the next live test's
/// `reset_and_migrate` drops and recreates from scratch.
async fn widen_postgres_lock_window() {
    use sea_orm::ConnectionTrait;
    let conn = suprnova::DB::connection().expect("primary connection registered");
    conn.inner()
        .execute_unprepared(
            "CREATE OR REPLACE FUNCTION suprnova_render_test_delay() RETURNS trigger AS $$
             BEGIN
                 PERFORM pg_sleep(0.2);
                 RETURN NEW;
             END;
             $$ LANGUAGE plpgsql",
        )
        .await
        .expect("install delay function");
    conn.inner()
        .execute_unprepared(
            "CREATE TRIGGER suprnova_render_test_delay_trigger
             BEFORE INSERT OR UPDATE ON suprnova_render_generations
             FOR EACH ROW EXECUTE FUNCTION suprnova_render_test_delay()",
        )
        .await
        .expect("install delay trigger");
}

#[tokio::test]
#[ignore = "requires live MySQL; run with --ignored live_mysql"]
async fn live_mysql_concurrent_advances_in_opposite_order_do_not_deadlock() {
    let url = std::env::var("MYSQL_TEST_URL")
        .expect("set MYSQL_TEST_URL to a disposable MySQL - this test drops and recreates tables");
    let conn = try_connect_live(&url)
        .await
        .expect("MySQL test DB not reachable - check MYSQL_TEST_URL");
    let _guard = reset_and_migrate(conn).await;
    assert_concurrent_opposite_order_advances_do_not_deadlock().await;
}

#[tokio::test]
#[ignore = "requires live MySQL; run with --ignored live_mysql"]
async fn live_mysql_generation_ledger_advances_and_reads() {
    let url = std::env::var("MYSQL_TEST_URL")
        .expect("set MYSQL_TEST_URL to a disposable MySQL - this test drops and recreates tables");
    let conn = try_connect_live(&url)
        .await
        .expect("MySQL test DB not reachable - check MYSQL_TEST_URL");
    let _guard = reset_and_migrate(conn).await;

    let ledger = SqlGenerationLedger::new();
    let posts = DependencyIdentity::table("posts");
    assert_eq!(
        ledger.current(&[posts.digest()]).await.unwrap().get(&posts),
        Some(0)
    );

    DB::transaction(|_tx| {
        Box::pin(async move {
            advance_in_current_transaction(&[DependencyIdentity::table("posts")]).await
        })
    })
    .await
    .expect("commit");
    assert_eq!(
        ledger.current(&[posts.digest()]).await.unwrap().get(&posts),
        Some(1)
    );
    assert_eq!(ledger.epoch().await.unwrap(), 1);
    ledger.advance_epoch().await.unwrap();
    assert_eq!(ledger.epoch().await.unwrap(), 2);
}

/// fix1 item 1: on Postgres, a failed statement poisons the enclosing
/// transaction, and `COMMIT` on a poisoned transaction returns the
/// ROLLBACK tag without raising - so `tx.commit()` reports `Ok` even
/// though the whole transaction actually rolled back. Before the fix, a
/// missing `suprnova_render_epochs` table was swallowed unconditionally,
/// which - inside a caller's own `DB::transaction`, against a database
/// that never ran the RenderCache migration - meant the row write above
/// would appear to succeed while silently being discarded.
///
/// SQLite and MySQL do not abort the enclosing transaction on a statement
/// error, so this failure mode is invisible to both: only a real Postgres
/// server proves it. This test deliberately does NOT run the migration -
/// it drops the three `suprnova_render_*` tables (and its own probe
/// table) if they linger, so this database has no RenderCache schema at
/// all, then marks a RenderCache runtime installed so the write side's
/// gate opens and the probe is actually attempted.
#[tokio::test]
#[ignore = "requires live Postgres; run with --ignored live_postgres"]
async fn live_postgres_a_write_inside_a_transaction_on_an_unmigrated_database_fails_loudly() {
    use sea_orm::ConnectionTrait;

    let url = std::env::var("PG_TEST_URL")
        .expect("set PG_TEST_URL to a disposable Postgres - this test drops tables");
    let conn = try_connect_live(&url)
        .await
        .expect("Postgres test DB not reachable - check PG_TEST_URL");

    for table in [
        "suprnova_render_epochs",
        "suprnova_render_generation_log",
        "suprnova_render_generations",
        "live_pg_unmigrated_probe",
    ] {
        let _ = conn
            .execute_raw(sea_orm::Statement::from_string(
                conn.get_database_backend(),
                format!("DROP TABLE IF EXISTS {table}"),
            ))
            .await;
    }
    conn.execute_raw(sea_orm::Statement::from_string(
        conn.get_database_backend(),
        "CREATE TABLE live_pg_unmigrated_probe (id INT PRIMARY KEY)".to_owned(),
    ))
    .await
    .expect("create probe table");

    let guard = suprnova::testing::TestContainer::fake();
    suprnova::testing::TestContainer::singleton(suprnova::DbConnection::from_raw(conn));
    suprnova::render_cache::mark_installed();

    // A real row write, through the exact production path (`DB::statement`,
    // which every non-`SELECT` raw statement routes render-cache
    // advancement through) - not a direct call into the ledger. If the
    // missing-table failure were swallowed here, this closure would return
    // `Ok`, `DB::transaction` would issue `COMMIT`, and Postgres would
    // silently roll back everything while reporting success.
    let result: Result<(), FrameworkError> = DB::transaction(|_tx| {
        Box::pin(async move {
            DB::statement(
                "INSERT INTO live_pg_unmigrated_probe (id) VALUES (1)",
                vec![],
            )
            .await?;
            Ok(())
        })
    })
    .await;

    assert!(
        result.is_err(),
        "a missing-table failure inside the caller's transaction must propagate, not be \
         swallowed - on Postgres, swallowing it lets a later commit() on the poisoned \
         transaction report success while silently discarding the row write"
    );

    let rows: i64 = DB::scalar("SELECT COUNT(*) FROM live_pg_unmigrated_probe", vec![])
        .await
        .expect("count");
    assert_eq!(
        rows, 0,
        "the whole transaction must have actually rolled back - the row write must not have \
         silently committed"
    );

    drop(guard);
}

/// Final review, F1 / ruling R117: the render transaction is a snapshot on
/// every backend. A write that commits while a cached render is running
/// (after the handler's own read, before the observation window closes) must
/// never leave that render's candidate published as current. With the
/// snapshot, the window-close read still sees the pre-write generations, the
/// fresh reread outside the transaction sees the write, and the candidate is
/// discarded as moved; the next request renders again and no entry exists
/// under the route's key.
///
/// SQLite hides this by construction (a WAL read transaction is a snapshot
/// from its first read), which is why `render_cache_middleware.rs`'s own
/// version of this scenario proves nothing about PostgreSQL, whose default
/// isolation is `READ COMMITTED`: there the window-close read saw the
/// committed write, the stored generations matched the fresh reread, and a
/// body built from the pre-write data was published as current. The render
/// transaction now asks for `REPEATABLE READ` on PostgreSQL (and MySQL,
/// where InnoDB already defaults to it).
///
/// Proven by revert on PostgreSQL: with `render_isolation_level` returning
/// `None` for every backend, the "renders again" assertion below fails with
/// `left: 1, right: 2` and `inspect` finds the stale entry published.
async fn assert_a_write_committed_during_a_cached_render_is_never_published_as_current(
    harness: &Harness,
) {
    counting_route::write_during_next_render(harness);
    dispatch_get(harness, "/cached/1", &[]).await;
    assert_eq!(
        counting_route::renders(),
        1,
        "the raced render still runs; only publication is declined"
    );
    let key = RenderCache::key_for_route_for_test("/cached/{id}", &[("id", "1")], None);
    assert!(
        RenderCache::inspect(&key).await.expect("inspect").is_none(),
        "a candidate whose observed table moved during its render is never published as \
         current"
    );

    dispatch_get(harness, "/cached/1", &[]).await;
    assert_eq!(
        counting_route::renders(),
        2,
        "nothing was published under the current generations, so the next request renders \
         again"
    );

    // Positive control: the un-raced render publishes, so a build that
    // cannot publish at all does not pass this test vacuously.
    dispatch_get(harness, "/cached/1", &[]).await;
    assert_eq!(
        counting_route::renders(),
        2,
        "control: the second render published and this dispatch is a hit"
    );
    assert!(
        RenderCache::inspect(&key).await.expect("inspect").is_some(),
        "control: the un-raced render's entry exists"
    );
}

#[tokio::test]
#[ignore = "requires live Postgres; run with --ignored live_postgres"]
async fn live_postgres_a_write_committed_during_a_cached_render_is_never_published_as_current() {
    let url = std::env::var("PG_TEST_URL")
        .expect("set PG_TEST_URL to a disposable Postgres - this test drops and recreates tables");
    let conn = try_connect_live(&url)
        .await
        .expect("Postgres test DB not reachable - check PG_TEST_URL");
    let harness =
        boot_with_render_cache_on_live_server_for_test(suprnova::DbConnection::from_raw(conn))
            .await;
    assert_a_write_committed_during_a_cached_render_is_never_published_as_current(&harness).await;
}

#[tokio::test]
#[ignore = "requires live MySQL; run with --ignored live_mysql"]
async fn live_mysql_a_write_committed_during_a_cached_render_is_never_published_as_current() {
    let url = std::env::var("MYSQL_TEST_URL")
        .expect("set MYSQL_TEST_URL to a disposable MySQL - this test drops and recreates tables");
    let conn = try_connect_live(&url)
        .await
        .expect("MySQL test DB not reachable - check MYSQL_TEST_URL");
    let harness =
        boot_with_render_cache_on_live_server_for_test(suprnova::DbConnection::from_raw(conn))
            .await;
    assert_a_write_committed_during_a_cached_render_is_never_published_as_current(&harness).await;
}
