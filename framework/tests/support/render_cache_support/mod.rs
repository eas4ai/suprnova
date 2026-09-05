//! Shared boot for the Task 12 write-path tests in `render_cache_orm.rs`:
//! an in-memory SQLite database with the RenderCache migration applied,
//! plus the throwaway models those tests need to exercise the write side
//! end to end:
//!
//! - [`Post`] - a plain, auto-increment-keyed model with a `views` counter
//!   column (`increment`/`increment_each`) and a `tags` many-to-many
//!   relation (`attach`/`detach`/`sync`). `create` / `save` /
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
//! - [`Tag`] / [`PostTagPivot`] - the many-to-many side of `Post::tags`.
//! - [`Author`] / [`Book`] - a `#[model(touches = ["author"])]` pair: a
//!   `Book` write must bust `Author`'s cached representation too.

use chrono::{DateTime, Utc};
// A wildcard `sea_orm_migration::prelude::*` import here would bring
// `sea_query::ExprTrait` into this module's scope, which collides with
// `Ord::max` on `i64` inside the macro-generated code for a `#[model]`
// struct that also declares a `relations = { ... }` block - the macro
// expands in place, so its own generated `.max()` calls become ambiguous
// between the two traits. Named imports only, matching
// `render_cache_ledger.rs`.
use sea_orm_migration::{MigrationTrait, MigratorTrait};
use suprnova::model;
use suprnova::testing::TestDatabase;

#[model(
    table = "posts",
    timestamps = false,
    fillable = ["title", "views"],
    relations = {
        tags: BelongsToMany<Tag, PostTagPivot> {
            with_timestamps,
        },
    },
)]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub views: i64,
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

#[model(table = "tags", timestamps = false, fillable = ["name"])]
pub struct Tag {
    pub id: i64,
    pub name: String,
}

#[model(table = "post_tags", primary_key = "id")]
pub struct PostTagPivot {
    pub id: i64,
    pub post_id: i64,
    pub tag_id: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[model(table = "authors", fillable = ["name"])]
pub struct Author {
    pub id: i64,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[model(
    table = "books",
    timestamps = false,
    fillable = ["author_id", "title"],
    touches = ["author"],
    relations = {
        author: BelongsTo<Author> { fk = "author_id" },
    },
)]
pub struct Book {
    pub id: i64,
    pub author_id: i64,
    pub title: String,
}

struct RenderCacheOrmMigrator;

#[async_trait::async_trait]
impl MigratorTrait for RenderCacheOrmMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(suprnova::render_cache::migration::Migration)]
    }
}

/// Boots a fresh in-memory SQLite database with the RenderCache migration
/// and every table this file's models need, and marks a RenderCache
/// runtime installed for this process so the write side's gate
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
            title TEXT NOT NULL, \
            views INTEGER NOT NULL DEFAULT 0\
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
    db.execute_unprepared(
        "CREATE TABLE tags (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
    )
    .await
    .expect("create tags table");
    db.execute_unprepared(
        "CREATE TABLE post_tags (\
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            post_id INTEGER NOT NULL, \
            tag_id INTEGER NOT NULL, \
            created_at TEXT NOT NULL, \
            updated_at TEXT NOT NULL\
         )",
    )
    .await
    .expect("create post_tags table");
    db.execute_unprepared(
        "CREATE TABLE authors (\
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            name TEXT NOT NULL, \
            created_at TEXT NOT NULL, \
            updated_at TEXT NOT NULL\
         )",
    )
    .await
    .expect("create authors table");
    db.execute_unprepared(
        "CREATE TABLE books (\
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            author_id INTEGER NOT NULL, \
            title TEXT NOT NULL\
         )",
    )
    .await
    .expect("create books table");
    Box::leak(Box::new(db));
}
