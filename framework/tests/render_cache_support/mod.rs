//! Shared boot for the Task 12 write-path tests in `render_cache_orm.rs`:
//! an in-memory SQLite database with the RenderCache migration applied,
//! plus three throwaway models covering the shapes those tests need to
//! exercise the write side end to end:
//!
//! - [`Post`] - a plain, auto-increment-keyed model. `create` / `save` /
//!   `update_all` / the model-less table builder / a raw statement.
//! - [`Trashable`] - a `soft_deletes` model, whose `delete` / `restore` /
//!   `force_delete` are macro-generated inherent overrides that bypass
//!   the `Model` trait defaults entirely (ruling R48), and whose
//!   `save_with_tx` exercises the explicit-transaction advance path
//!   (ruling R47).
//! - [`Widget`] - a `String`-keyed model. Its primary key's JSON encoding
//!   keeps its quotes (`"widget-1"`, not `widget-1`), which is exactly
//!   the encoding ruling R45 exists to keep the write side agreeing
//!   with. An auto-increment integer key never exercises this: `42` and
//!   `"42"` only diverge once the key is not already bare digits.

use chrono::{DateTime, Utc};
use sea_orm_migration::prelude::*;
use suprnova::model;
use suprnova::testing::TestDatabase;

#[model(table = "posts", timestamps = false, fillable = ["title"])]
pub struct Post {
    pub id: i64,
    pub title: String,
}

#[model(
    table = "trashables",
    timestamps = false,
    soft_deletes,
    fillable = ["title"]
)]
pub struct Trashable {
    pub id: i64,
    pub title: String,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[model(
    table = "widgets",
    key_type = "String",
    timestamps = false,
    fillable = ["id", "name"]
)]
pub struct Widget {
    pub id: String,
    pub name: String,
}

struct RenderCacheOrmMigrator;

#[async_trait::async_trait]
impl MigratorTrait for RenderCacheOrmMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(suprnova::render_cache::migration::Migration)]
    }
}

/// Boots a fresh in-memory SQLite database with the RenderCache migration
/// and the `posts` / `trashables` / `widgets` tables, and marks a
/// RenderCache runtime installed for this process so the write side's gate
/// (`render_cache::is_installed`) opens.
///
/// Every test in `render_cache_orm.rs` calls this without binding the
/// result (`boot().await;`), matching the shape every test in this suite
/// needs: the returned `TestDatabase` (and the `TestContainerGuard`
/// registration inside it) must outlive that statement for `DB::connection()`
/// to find it for the rest of the test. Leaking it is deliberate, not an
/// oversight: each `#[tokio::test]` function builds and tears down its own
/// tokio runtime, so nothing registered here is shared with, or needs
/// cleaning up before, any other test. Tests use the default (single
/// worker thread) flavour deliberately - `TestContainer::fake()` writes a
/// thread-local, and a multi-thread runtime can migrate a future between
/// worker threads between polls, which would make that registration
/// invisible to whichever thread resumes the test.
pub async fn boot() {
    suprnova::render_cache::mark_installed();
    let db = TestDatabase::fresh::<RenderCacheOrmMigrator>()
        .await
        .expect("render cache migration should apply cleanly to a fresh SQLite database");
    db.execute_unprepared(
        "CREATE TABLE posts (\
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            title TEXT NOT NULL\
         )",
    )
    .await
    .expect("create posts table");
    db.execute_unprepared(
        "CREATE TABLE trashables (\
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            title TEXT NOT NULL, \
            deleted_at TEXT\
         )",
    )
    .await
    .expect("create trashables table");
    db.execute_unprepared(
        "CREATE TABLE widgets (\
            id TEXT PRIMARY KEY, \
            name TEXT NOT NULL\
         )",
    )
    .await
    .expect("create widgets table");
    Box::leak(Box::new(db));
}
