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
use suprnova::render_cache::DependencyIdentity;
use suprnova::render_cache::ledger::{SqlGenerationLedger, advance_in_current_transaction};
use suprnova::testing::TestDatabase;
use suprnova::{DB, FrameworkError};
use suprnova_live::render_cache::generation::GenerationLedger;

struct RenderCacheTestMigrator;

#[async_trait::async_trait]
impl MigratorTrait for RenderCacheTestMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(suprnova::render_cache::migration::Migration)]
    }
}

async fn boot() -> TestDatabase {
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
async fn the_epoch_starts_at_one_and_advances_on_demand() {
    let _db = boot().await;
    let ledger = SqlGenerationLedger::new();

    assert_eq!(ledger.epoch().await.expect("epoch"), 1);
    ledger.advance_epoch().await.expect("advance");
    assert_eq!(ledger.epoch().await.expect("epoch"), 2);
}

// --- Live-DB tests (gated by #[ignore]) ---
//
// `render_cache_ledger` is a mixed file, same shape as `pagination.rs`:
// the tests above run against SQLite unconditionally; these two exercise
// the same migration and ledger against real Postgres / MySQL, where the
// upsert dialect, the placeholder syntax, and the `CHECK` constraint on
// `suprnova_render_epochs` actually differ from SQLite's.
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
