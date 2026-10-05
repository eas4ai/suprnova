//! Integration tests for `LengthAwarePaginator`, `CursorPaginator`,
//! `Pagination::length_aware`/`cursor`, and the Inertia bridge.
//!
//! The cursor tests stand up a real in-memory SQLite database via the
//! `TestContainer` thread-local override so `Pagination::cursor` -
//! which uses `DB::connection()` internally - sees a connection.

use sea_orm::{
    ActiveModelTrait, ConnectionTrait, Database, DbBackend, EntityTrait, Schema, Set, Statement,
};
use serde::Serialize;
use serde_json::json;
use suprnova::testing::TestContainer;
use suprnova::{
    CursorPaginator, DbConnection, EncryptionKey, IntoInertiaScroll, LengthAwarePaginator,
    Pagination,
};

/// Cursor pagination encrypts every emitted payload via the framework's
/// `Crypt` facade (codex review finding #1 - no plaintext base64
/// fallback). Tests that exercise `Pagination::cursor` need a key
/// installed, but `Crypt` is a process-global `OnceLock`, so we install
/// it exactly once for the binary.
fn ensure_crypt() {
    static INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INIT.get_or_init(|| {
        suprnova::Crypt::init(EncryptionKey::generate());
    });
}

// Toy in-memory SQLite entity used by the integration tests.
mod toy {
    use sea_orm::DeriveEntityModel;
    use sea_orm::entity::prelude::*;
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize, Deserialize)]
    #[sea_orm(table_name = "items")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i32,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

// --- unit-level tests over the paginator types ---

#[test]
fn has_more_pages() {
    let p = LengthAwarePaginator::new(vec![1, 2, 3], 25, 10, 2);
    assert_eq!(p.last_page, 3);
    assert!(p.has_more_pages());
}

#[test]
fn last_page_no_more() {
    let p = LengthAwarePaginator::new(vec![1, 2, 3, 4, 5], 25, 10, 3);
    assert!(!p.has_more_pages());
}

#[test]
fn total_zero_yields_empty_data() {
    let p: LengthAwarePaginator<i32> = LengthAwarePaginator::new(vec![], 0, 10, 1);
    assert_eq!(p.last_page, 0);
    assert!(p.data.is_empty());
}

// --- SeaORM integration via in-memory SQLite ---

async fn make_db_with_n_rows(n: i32) -> sea_orm::DatabaseConnection {
    let conn = Database::connect("sqlite::memory:").await.unwrap();
    let schema = Schema::new(DbBackend::Sqlite);
    let stmt = schema.create_table_from_entity(toy::Entity);
    conn.execute(&stmt).await.unwrap();

    for i in 1..=n {
        let m = toy::ActiveModel {
            id: Set(i),
            name: Set(format!("item-{:02}", i)),
        };
        m.insert(&conn).await.unwrap();
    }
    conn.execute_raw(Statement::from_string(
        DbBackend::Sqlite,
        "SELECT 1".to_string(),
    ))
    .await
    .unwrap();
    conn
}

/// Mount a SeaORM connection on the thread-local test container so
/// `DB::connection()` resolves to it inside `Pagination::cursor`.
fn install_db(conn: sea_orm::DatabaseConnection) {
    let db = DbConnection::from_raw(conn);
    TestContainer::singleton(db);
}

#[tokio::test]
async fn seaorm_length_aware_page_2_returns_10_rows() {
    let _guard = TestContainer::fake();
    let conn = make_db_with_n_rows(25).await;
    install_db(conn.clone());

    let p = Pagination::length_aware::<toy::Entity>(toy::Entity::find(), 10, 2)
        .await
        .unwrap();
    assert_eq!(p.total, 25);
    assert_eq!(p.per_page, 10);
    assert_eq!(p.current_page, 2);
    assert_eq!(p.last_page, 3);
    assert_eq!(p.data.len(), 10);
    assert!(p.has_more_pages());
}

#[tokio::test]
async fn facade_length_aware_rejects_zero_per_page() {
    // A live DB is installed so that, were the guard removed, the call
    // would succeed (LIMIT 0 → empty page) and `expect_err` would panic -
    // this is a real regression signal, not a no-DB false pass. The 400
    // assertion distinguishes the param guard from a 500 DB error.
    let _guard = TestContainer::fake();
    let conn = make_db_with_n_rows(5).await;
    install_db(conn);

    let err = Pagination::length_aware::<toy::Entity>(toy::Entity::find(), 0, 1)
        .await
        .expect_err("per_page == 0 must be rejected, not silently paginated");
    assert_eq!(
        err.status_code(),
        400,
        "zero per_page must be a 400 param error, matching Builder::paginate"
    );
}

#[tokio::test]
async fn facade_cursor_rejects_zero_per_page() {
    ensure_crypt();
    let _guard = TestContainer::fake();
    let conn = make_db_with_n_rows(5).await;
    install_db(conn);

    let err = Pagination::cursor::<toy::Entity, toy::Column>(
        toy::Entity::find(),
        None,
        0,
        toy::Column::Id,
    )
    .await
    .expect_err("per_page == 0 must be rejected, not silently paginated");
    assert_eq!(
        err.status_code(),
        400,
        "zero per_page must be a 400 param error, matching Builder::cursor_paginate"
    );
}

#[tokio::test]
async fn pagination_cursor_walks_forward_until_exhausted() {
    ensure_crypt();
    let _guard = TestContainer::fake();
    let conn = make_db_with_n_rows(25).await;
    install_db(conn);

    let per_page: u64 = 10;
    let mut visited: Vec<i32> = Vec::new();
    let mut cursor: Option<String> = None;

    for _ in 0..10 {
        let page = Pagination::cursor::<toy::Entity, toy::Column>(
            toy::Entity::find(),
            cursor.as_deref(),
            per_page,
            toy::Column::Id,
        )
        .await
        .unwrap();

        for r in &page.data {
            visited.push(r.id);
        }
        cursor = page.next_cursor.clone();
        if cursor.is_none() {
            break;
        }
    }

    assert_eq!(visited.len(), 25);
    assert_eq!(visited.first(), Some(&1));
    assert_eq!(visited.last(), Some(&25));
}

#[tokio::test]
async fn pagination_cursor_emits_prev_cursor_on_page_2() {
    ensure_crypt();
    let _guard = TestContainer::fake();
    let conn = make_db_with_n_rows(25).await;
    install_db(conn);

    // Page 1 - first page, prev_cursor must be None.
    let page1 = Pagination::cursor::<toy::Entity, toy::Column>(
        toy::Entity::find(),
        None,
        10,
        toy::Column::Id,
    )
    .await
    .unwrap();
    assert!(
        page1.prev_cursor.is_none(),
        "first page must have no prev_cursor"
    );
    let next1 = page1
        .next_cursor
        .clone()
        .expect("page 1 should have a next cursor");
    let page1_ids: Vec<i32> = page1.data.iter().map(|r| r.id).collect();
    assert_eq!(page1_ids, (1..=10).collect::<Vec<i32>>());

    // Page 2 - using page 1's next_cursor.
    let page2 = Pagination::cursor::<toy::Entity, toy::Column>(
        toy::Entity::find(),
        Some(&next1),
        10,
        toy::Column::Id,
    )
    .await
    .unwrap();
    let page2_ids: Vec<i32> = page2.data.iter().map(|r| r.id).collect();
    assert_eq!(page2_ids, (11..=20).collect::<Vec<i32>>());
    let prev2 = page2
        .prev_cursor
        .clone()
        .expect("page 2 must emit a prev_cursor");

    // Following page 2's prev_cursor takes us back to page 1's rows.
    let back = Pagination::cursor::<toy::Entity, toy::Column>(
        toy::Entity::find(),
        Some(&prev2),
        10,
        toy::Column::Id,
    )
    .await
    .unwrap();
    let back_ids: Vec<i32> = back.data.iter().map(|r| r.id).collect();
    assert_eq!(
        back_ids,
        (1..=10).collect::<Vec<i32>>(),
        "prev_cursor from page 2 must return to page 1's contents"
    );
    // back has 10 rows and there are no more rows before id=1 → no prev.
    assert!(
        back.prev_cursor.is_none(),
        "walked back to the first page; prev_cursor should be None"
    );
    // We came from page 2, so we always have a way forward.
    assert!(back.next_cursor.is_some());
}

#[tokio::test]
async fn pagination_cursor_last_page_no_next() {
    ensure_crypt();
    let _guard = TestContainer::fake();
    let conn = make_db_with_n_rows(25).await;
    install_db(conn);

    let mut cursor: Option<String> = None;
    let mut last_page_rows: Vec<i32> = Vec::new();
    for _ in 0..10 {
        let p = Pagination::cursor::<toy::Entity, toy::Column>(
            toy::Entity::find(),
            cursor.as_deref(),
            10,
            toy::Column::Id,
        )
        .await
        .unwrap();
        last_page_rows = p.data.iter().map(|r| r.id).collect();
        if p.next_cursor.is_none() {
            // Last page reached. With 25 rows, this is page 3 (rows 21..=25).
            assert_eq!(last_page_rows, (21..=25).collect::<Vec<i32>>());
            return;
        }
        cursor = p.next_cursor;
    }
    panic!("walked too many pages; last page: {last_page_rows:?}");
}

/// DATA-019: an ordering already on the query does not reorder the keyset
/// walk. Ordered by id descending, the first page used to be 5 and 4, the
/// next `id > 4` gave 5 again, and 1 to 3 were never shown.
#[tokio::test]
async fn pagination_cursor_replaces_an_existing_order_and_walks_every_row_once() {
    use sea_orm::QueryOrder;
    ensure_crypt();
    let _guard = TestContainer::fake();
    install_db(make_db_with_n_rows(5).await);

    let mut seen: Vec<i32> = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..10 {
        let page = Pagination::cursor::<toy::Entity, toy::Column>(
            toy::Entity::find().order_by_desc(toy::Column::Id),
            cursor.as_deref(),
            2,
            toy::Column::Id,
        )
        .await
        .unwrap();
        seen.extend(page.data.iter().map(|r| r.id));
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert_eq!(seen, vec![1, 2, 3, 4, 5]);
}

/// DATA-019: an offset on the query positions the first page only. Kept
/// on every page, it skipped two rows after each cursor.
#[tokio::test]
async fn pagination_cursor_applies_an_offset_to_the_first_page_only() {
    use sea_orm::QuerySelect;
    ensure_crypt();
    let _guard = TestContainer::fake();
    install_db(make_db_with_n_rows(8).await);

    let mut seen: Vec<i32> = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..10 {
        let page = Pagination::cursor::<toy::Entity, toy::Column>(
            toy::Entity::find().offset(2),
            cursor.as_deref(),
            2,
            toy::Column::Id,
        )
        .await
        .unwrap();
        seen.extend(page.data.iter().map(|r| r.id));
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert_eq!(seen, vec![3, 4, 5, 6, 7, 8]);
}

// The typed counts get a table of their own, so the live tests below never
// drop the `items` table another live test is reading.
mod counted {
    use sea_orm::DeriveEntityModel;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "typed_counts")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i32,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// Six rows in `typed_counts`, in a table made fresh for the run.
async fn seed_counted(conn: &sea_orm::DatabaseConnection) {
    let backend = conn.get_database_backend();
    conn.execute_raw(Statement::from_string(
        backend,
        "DROP TABLE IF EXISTS typed_counts".to_string(),
    ))
    .await
    .unwrap();
    conn.execute(&Schema::new(backend).create_table_from_entity(counted::Entity))
        .await
        .unwrap();
    for id in 1..=6 {
        counted::ActiveModel { id: Set(id) }
            .insert(conn)
            .await
            .unwrap();
    }
}

/// A typed count and a length-aware total answer as Laravel's do when the
/// query carries a limit or an offset.
///
/// Laravel's `count()` keeps the limit and offset on the aggregate
/// statement, `select count(*) as aggregate from t limit 1 offset 1`. They
/// bound the one row the aggregate returns, not the rows it counts: a limit
/// of one or more keeps the whole count, an offset of one or more skips it,
/// and the count is then 0. Its `exists()` asks about the bounded rows, so
/// the second row exists. Its `getCountForPagination()` drops the limit and
/// the offset, so a page's total is every matching row. Both typed paths
/// used to count the bounded subset instead: 2 for `limit(2)`, 1 for
/// `limit(1).offset(1)`, and a total of 2 under `limit(2)`. A bare offset
/// failed outright on SQLite and MySQL.
async fn typed_counts_follow_laravel(conn: sea_orm::DatabaseConnection) {
    use suprnova::database::QueryBuilder;
    seed_counted(&conn).await;
    let _guard = TestContainer::fake();
    install_db(conn.clone());
    let typed = QueryBuilder::<counted::Entity>::new;
    let count = |query: QueryBuilder<counted::Entity>, case: &'static str| async move {
        query
            .count()
            .await
            .unwrap_or_else(|e| panic!("count {case}: {e}"))
    };

    assert_eq!(count(typed(), "unbounded").await, 6);
    assert_eq!(
        count(typed().limit(2), "limit(2)").await,
        6,
        "a limit keeps the aggregate's row"
    );
    assert_eq!(
        count(typed().limit(1).offset(1), "limit(1).offset(1)").await,
        0,
        "an offset skips the aggregate's row"
    );
    assert_eq!(
        count(typed().offset(10), "offset(10)").await,
        0,
        "a bare offset skips it on every engine"
    );
    assert_eq!(count(typed().offset(0), "offset(0)").await, 6);
    assert_eq!(count(typed().limit(0), "limit(0)").await, 0);

    let exists = |query: QueryBuilder<counted::Entity>, case: &'static str| async move {
        query
            .exists()
            .await
            .unwrap_or_else(|e| panic!("exists {case}: {e}"))
    };
    assert!(
        exists(typed().limit(1).offset(1), "limit(1).offset(1)").await,
        "the second row exists"
    );
    assert!(
        exists(typed().offset(5), "offset(5)").await,
        "the sixth row exists"
    );
    assert!(
        !exists(typed().offset(6), "offset(6)").await,
        "no row lies past the sixth"
    );
    assert!(!exists(typed().limit(0), "limit(0)").await);

    use sea_orm::QuerySelect;
    let page = Pagination::length_aware::<counted::Entity>(
        counted::Entity::find().limit(2).offset(1),
        4,
        2,
    )
    .await
    .unwrap_or_else(|e| panic!("a page of a bounded query: {e}"));
    assert_eq!(
        page.total, 6,
        "the total ignores the query's limit and offset"
    );
    assert_eq!(page.last_page, 2);
    assert_eq!(
        page.data.iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![5, 6],
        "the page replaces the query's limit and offset"
    );
}

#[tokio::test]
async fn sqlite_typed_counts_follow_laravel() {
    typed_counts_follow_laravel(Database::connect("sqlite::memory:").await.unwrap()).await;
}

/// DATA-020: the Inertia scroll metadata names the query parameter the
/// paginator reads, so infinite scroll asks for `posts_page=2`, not
/// `page=2`.
#[test]
fn inertia_scroll_metadata_uses_the_paginators_page_name() {
    let (meta, _) = LengthAwarePaginator::new(vec![1, 2], 10, 2, 1)
        .with_page_name("posts_page")
        .into_inertia_scroll();
    assert_eq!(meta.page_name, "posts_page");

    let (meta, _) = LengthAwarePaginator::new(vec![1, 2], 10, 2, 1).into_inertia_scroll();
    assert_eq!(meta.page_name, "page", "the default name stays `page`");
}

/// DATA-020: the cursor paginator's metadata names its cursor parameter.
#[test]
fn inertia_scroll_metadata_uses_the_paginators_cursor_name() {
    let (meta, _) = CursorPaginator::new(vec![1, 2], 2, Some("next".to_string()), None)
        .with_cursor_name("after")
        .into_inertia_scroll();
    assert_eq!(meta.page_name, "after");

    let (meta, _) =
        CursorPaginator::new(vec![1, 2], 2, Some("next".to_string()), None).into_inertia_scroll();
    assert_eq!(meta.page_name, "cursor", "the default name stays `cursor`");
}

// --- Live-DB tests (gated by #[ignore]) ---
//
// These exercise `Pagination::cursor` against real Postgres / MySQL,
// validating that the typed `sea_orm::Value` boundary binds correctly
// for native int4 / bigint / uuid columns on each dialect.
//
// They're skipped by default. To run them, point the URL env var at a
// DISPOSABLE database and pass `--ignored`. The variable is required -
// `populate_n` drops and recreates its table, so there is deliberately no
// localhost default to fall back onto.
//
//   PG_TEST_URL=postgres://postgres:pw@127.0.0.1:55998/suprnova_test \
//     cargo test -p suprnova --test pagination -- --ignored postgres
//
//   MYSQL_TEST_URL=mysql://root:pw@127.0.0.1:55997/suprnova_test \
//     cargo test -p suprnova --test pagination -- --ignored mysql
//
// The toy entity's `id` is `i32` (Int) on every dialect - so the
// cursor wire format roundtrips `Value::Int(Some(42))` through
// Postgres `int4`, MySQL `INT`, etc. without dialect-specific casts.

async fn try_connect_live(url: &str) -> Option<sea_orm::DatabaseConnection> {
    use sea_orm::ConnectOptions;
    use std::time::Duration;
    let mut opts = ConnectOptions::new(url.to_string());
    opts.connect_timeout(Duration::from_secs(2))
        .acquire_timeout(Duration::from_secs(2));
    sea_orm::Database::connect(opts).await.ok()
}

async fn populate_n(conn: &sea_orm::DatabaseConnection, n: i32) {
    // Drop the table if it lingers from a prior failed run; SeaORM's
    // `create_table_from_entity` doesn't issue IF NOT EXISTS.
    let _ = conn
        .execute_raw(Statement::from_string(
            conn.get_database_backend(),
            "DROP TABLE IF EXISTS items".to_string(),
        ))
        .await;
    let schema = Schema::new(conn.get_database_backend());
    let stmt = schema.create_table_from_entity(toy::Entity);
    conn.execute(&stmt).await.unwrap();
    for i in 1..=n {
        toy::ActiveModel {
            id: Set(i),
            name: Set(format!("item-{:02}", i)),
        }
        .insert(conn)
        .await
        .unwrap();
    }
}

#[tokio::test]
#[ignore = "requires live Postgres; run with --ignored postgres"]
async fn live_postgres_cursor_walks_with_typed_int_boundary() {
    // No default. `populate_n` below issues `DROP TABLE IF EXISTS items`
    // and recreates it, so a default of `localhost:5432` would silently
    // point a destructive test at whatever Postgres the developer happens
    // to be running - which is, on most machines, a real one. Requiring
    // the variable makes the target an explicit choice.
    let url = std::env::var("PG_TEST_URL")
        .expect("set PG_TEST_URL to a disposable Postgres - this test DROPs and recreates tables");
    let conn = try_connect_live(&url)
        .await
        .expect("Postgres test DB not reachable - check PG_TEST_URL");
    populate_n(&conn, 25).await;

    // `Pagination::cursor` encrypts its cursor, so a key must exist. The
    // in-memory tests above already call this; these two did not, and the
    // omission was invisible because nothing ran them until CI-01 wired
    // them into the gate.
    ensure_crypt();

    let _guard = TestContainer::fake();
    install_db(conn);

    let mut visited: Vec<i32> = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..10 {
        let p = Pagination::cursor::<toy::Entity, toy::Column>(
            toy::Entity::find(),
            cursor.as_deref(),
            10,
            toy::Column::Id,
        )
        .await
        .unwrap();
        for r in &p.data {
            visited.push(r.id);
        }
        cursor = p.next_cursor.clone();
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(visited.len(), 25);
    assert_eq!(visited.first(), Some(&1));
    assert_eq!(visited.last(), Some(&25));
}

#[tokio::test]
#[ignore = "requires live MySQL; run with --ignored mysql"]
async fn live_mysql_cursor_walks_with_typed_int_boundary() {
    // Required, not defaulted, for the same reason as the Postgres case
    // above: `populate_n` is destructive.
    let url = std::env::var("MYSQL_TEST_URL")
        .expect("set MYSQL_TEST_URL to a disposable MySQL - this test DROPs and recreates tables");
    let conn = try_connect_live(&url)
        .await
        .expect("MySQL test DB not reachable - check MYSQL_TEST_URL");
    populate_n(&conn, 25).await;

    // `Pagination::cursor` encrypts its cursor, so a key must exist. The
    // in-memory tests above already call this; these two did not, and the
    // omission was invisible because nothing ran them until CI-01 wired
    // them into the gate.
    ensure_crypt();

    let _guard = TestContainer::fake();
    install_db(conn);

    let mut visited: Vec<i32> = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..10 {
        let p = Pagination::cursor::<toy::Entity, toy::Column>(
            toy::Entity::find(),
            cursor.as_deref(),
            10,
            toy::Column::Id,
        )
        .await
        .unwrap();
        for r in &p.data {
            visited.push(r.id);
        }
        cursor = p.next_cursor.clone();
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(visited.len(), 25);
    assert_eq!(visited.first(), Some(&1));
    assert_eq!(visited.last(), Some(&25));
}

#[tokio::test]
#[ignore = "requires live Postgres; run with --ignored postgres"]
async fn live_postgres_typed_counts_follow_laravel() {
    // Required, not defaulted: `seed_counted` drops and recreates its table.
    let url = std::env::var("PG_TEST_URL")
        .expect("set PG_TEST_URL to a disposable Postgres - this test DROPs and recreates tables");
    let conn = try_connect_live(&url)
        .await
        .expect("Postgres test DB not reachable - check PG_TEST_URL");
    typed_counts_follow_laravel(conn).await;
}

#[tokio::test]
#[ignore = "requires live MySQL; run with --ignored mysql"]
async fn live_mysql_typed_counts_follow_laravel() {
    let url = std::env::var("MYSQL_TEST_URL")
        .expect("set MYSQL_TEST_URL to a disposable MySQL - this test DROPs and recreates tables");
    let conn = try_connect_live(&url)
        .await
        .expect("MySQL test DB not reachable - check MYSQL_TEST_URL");
    typed_counts_follow_laravel(conn).await;
}

// --- IntoInertiaScroll wiring ---

#[test]
fn length_aware_into_inertia_scroll() {
    let p = LengthAwarePaginator::new(vec!["a", "b", "c"], 25, 10, 2);
    let (meta, data) = p.into_inertia_scroll();
    assert_eq!(meta.page_name, "page");
    assert_eq!(meta.current_page, Some(json!(2_i64)));
    assert_eq!(meta.previous_page, Some(json!(1_i64)));
    assert_eq!(meta.next_page, Some(json!(3_i64)));
    assert_eq!(data, vec!["a", "b", "c"]);
}

#[test]
fn cursor_into_inertia_scroll() {
    let p: CursorPaginator<String> = CursorPaginator::new(
        vec!["row-1".to_string(), "row-2".to_string()],
        10,
        Some("opaque-next".to_string()),
        Some("opaque-prev".to_string()),
    );
    let (meta, data) = p.into_inertia_scroll();
    assert_eq!(meta.page_name, "cursor");
    assert_eq!(meta.next_page, Some(json!("opaque-next")));
    assert_eq!(meta.previous_page, Some(json!("opaque-prev")));
    assert_eq!(data.len(), 2);
}

#[test]
fn inertia_paginate_facade_produces_inertia_response() {
    #[derive(Serialize)]
    struct Row {
        id: i32,
    }
    let p = LengthAwarePaginator::new(vec![Row { id: 1 }, Row { id: 2 }], 2, 10, 1);
    // Just exercise the facade - we don't try to serialize the full
    // Inertia response here (that path runs through a scroll-flagged
    // `Prop` which needs an InertiaContext / request).
    let _resp = suprnova::Inertia::paginate("Users/Index", "users", p);
}
