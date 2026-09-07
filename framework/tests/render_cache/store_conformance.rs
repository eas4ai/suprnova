//! Every framework-side `RenderStore` provider answers one conformance
//! suite.
//!
//! The scenarios are the engine's
//! `suprnova_live_test_support::render_store_conformance`, written against
//! the trait alone, so the file-backed L1, the database-backed L1 on each
//! dialect, and the Redis-backed L1 are held to the same words as the
//! in-process store `crates/suprnova-live/tests/render_cache_store_conformance.rs`
//! runs them over. What a provider's own submodule proves - the dialect's
//! upsert, the sweep, the tally across a reopen, the Lua scripts - stays
//! there; this file proves only that each of them is a `RenderStore`.
//!
//! SQLite and the file store run unconditionally. PostgreSQL, MySQL, and
//! Redis run through `#[ignore]`d tests that `scripts/check-postgres.sh`,
//! `scripts/check-mysql.sh`, and `scripts/check-redis.sh` select by name.
//!
//! Every store here is booted empty, and every one is built with
//! [`MAX_BYTES`], which is what the suite's oversized scenario publishes one
//! byte past.

use suprnova::render_cache::file_store::FileRenderStore;
use suprnova::render_cache::providers::{RedisRenderStore, SqlRenderStore};
use suprnova_live::render_cache::entry::EntryLimits;
use suprnova_live_test_support::render_store_conformance;

use crate::render_cache_tiers_support;
use render_cache_tiers_support::{boot, boot_redis, keys, reset_and_migrate, try_connect_live};

/// The byte bound every store under conformance is built with.
const MAX_BYTES: usize = 1024 * 1024;

/// The same bound as the `u64` the framework's stores are configured with.
const MAX_STORE_BYTES: u64 = 1024 * 1024;

/// Runs the suite over one store with the fixture key ring and the default
/// decoding bounds.
async fn conforms(store: &dyn suprnova_live::render_cache::store::RenderStore) {
    render_store_conformance::run_all(store, &keys(), &EntryLimits::default(), MAX_BYTES).await;
}

#[tokio::test]
async fn the_file_store_conforms() {
    let directory = tempfile::tempdir().expect("a temporary directory for the file L1");
    let store = FileRenderStore::open(directory.path(), MAX_STORE_BYTES).expect("open the file L1");

    conforms(&store).await;
}

#[tokio::test]
async fn the_sql_store_conforms() {
    let _db = boot().await;
    let store = SqlRenderStore::new(MAX_STORE_BYTES);

    conforms(&store).await;
}

#[tokio::test]
#[ignore = "requires live Postgres; run with --ignored live_postgres"]
async fn live_postgres_render_store_conforms() {
    let url = std::env::var("PG_TEST_URL")
        .expect("set PG_TEST_URL to a disposable Postgres - this test drops and recreates tables");
    let conn = try_connect_live(&url)
        .await
        .expect("Postgres test DB not reachable - check PG_TEST_URL");
    let _guard = reset_and_migrate(conn).await;
    let store = SqlRenderStore::new(MAX_STORE_BYTES);

    conforms(&store).await;
}

#[tokio::test]
#[ignore = "requires live MySQL; run with --ignored live_mysql"]
async fn live_mysql_render_store_conforms() {
    let url = std::env::var("MYSQL_TEST_URL")
        .expect("set MYSQL_TEST_URL to a disposable MySQL - this test drops and recreates tables");
    let conn = try_connect_live(&url)
        .await
        .expect("MySQL test DB not reachable - check MYSQL_TEST_URL");
    let _guard = reset_and_migrate(conn).await;
    let store = SqlRenderStore::new(MAX_STORE_BYTES);

    conforms(&store).await;
}

#[tokio::test]
#[ignore = "requires live Redis; run with --ignored live_redis"]
async fn live_redis_render_store_conforms() {
    // The prefix is this test's alone, so the suite's "starts empty" and
    // "ends empty" assertions are about its own keys and nothing else on
    // the instance; the guard removes them however the test ends.
    let (config, _conn, _cleanup) = boot_redis().await;
    let store = RedisRenderStore::connect(&config, MAX_STORE_BYTES)
        .await
        .expect("connect the Redis L1 store");

    conforms(&store).await;
}
