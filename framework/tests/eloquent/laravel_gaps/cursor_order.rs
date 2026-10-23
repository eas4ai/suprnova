//! `cursor_paginate` keeps the query's orderings, as Laravel's
//! `cursorPaginate` honours `orderBy`: a descending order walks downward,
//! the cursor carries every ordered column and the next page applies them,
//! and a query with no order walks the primary key ascending. An ordering
//! on the model's own column carries the same whether it is bare or
//! qualified with the model's table; an ordering on a joined table's column
//! is refused, as the row holds none of its values. A joined query quotes
//! the columns it orders by, as it quotes its other references to the
//! model's table.

use crate::key_ring::rotation_keys;
use suprnova::context::Context;
use suprnova::sea_orm::DatabaseBackend;
use suprnova::testing::TestDatabase;
use suprnova::{Builder, Cursor, CursorPaginator, DB, Model, attrs, model};

/// A row with a creation time that two rows share, so an order on it alone
/// needs the primary key to break the tie.
#[model(table = "gap_cursor_rows", timestamps = false)]
pub struct GapCursorRow {
    /// The test assigns the key.
    pub id: i64,
    /// A label gives the row content.
    pub title: String,
    /// A creation time; rows 2 and 4 share one.
    pub created_at: i64,
}

/// Five rows: by `created_at`, 1 (10), 3 (20), 2 and 4 (30), 5 (40).
async fn fixture() -> TestDatabase {
    Context::test_clear_query();
    rotation_keys();
    let db = TestDatabase::sqlite_memory().await.expect("database");
    db.execute_unprepared(
        "CREATE TABLE gap_cursor_rows (id INTEGER PRIMARY KEY, title TEXT NOT NULL, created_at INTEGER NOT NULL)",
    )
    .await
    .expect("schema");
    for (id, created_at) in [(1, 10), (2, 30), (3, 20), (4, 30), (5, 40)] {
        GapCursorRow::create(attrs! { id: id, title: format!("row {id}"), created_at: created_at })
            .await
            .expect("row");
    }
    db
}

/// The keys of `page`, in the order the page holds them.
fn ids(page: &CursorPaginator<GapCursorRow>) -> Vec<i64> {
    page.data.iter().map(|row| row.id).collect()
}

/// A page as a walk reads it: its rows' keys and its next and previous
/// cursors.
type Page = (Vec<i64>, Option<String>, Option<String>);

/// Every page `fetch` gives, walking the next cursors to the end, then the
/// page before the second one.
async fn walk_pages(fetch: impl AsyncFn() -> Page) -> (Vec<Vec<i64>>, Vec<i64>) {
    Context::test_clear_query();
    let mut pages = Vec::new();
    let mut second_prev = None;
    loop {
        let (keys, next, prev) = fetch().await;
        pages.push(keys);
        if pages.len() == 2 {
            second_prev = prev;
        }
        match next {
            Some(next) => Context::test_set_query("cursor", next),
            None => break,
        }
    }
    Context::test_set_query(
        "cursor",
        second_prev.expect("the second page has a previous one"),
    );
    let (back, _, _) = fetch().await;
    Context::test_clear_query();
    (pages, back)
}

/// Every page `query` gives two rows at a time; see [`walk_pages`].
async fn walk(query: impl Fn() -> Builder<GapCursorRow>) -> (Vec<Vec<i64>>, Vec<i64>) {
    walk_pages(async || {
        let page = query().cursor_paginate(2).await.expect("a page");
        (ids(&page), page.next_cursor, page.prev_cursor)
    })
    .await
}

/// A row owned by a row of another table. Both tables carry `id` and
/// `title`, and the owners' keys (10 and 20) differ from the rows' (1 and
/// 2), so a cursor that took an owner's column for the row's would show.
#[model(table = "gap_cursor_owned", timestamps = false)]
pub struct GapCursorOwned {
    /// The test assigns the key.
    pub id: i64,
    /// A label; the owners table has a `title` too.
    pub title: String,
    /// The owner's key.
    pub owner_id: i64,
}

/// Rows 1 and 2, owned by owners 10 and 20.
async fn owned_fixture() -> TestDatabase {
    Context::test_clear_query();
    rotation_keys();
    let db = TestDatabase::sqlite_memory().await.expect("database");
    for sql in [
        "CREATE TABLE gap_cursor_owners (id INTEGER PRIMARY KEY, title TEXT NOT NULL)",
        "CREATE TABLE gap_cursor_owned (id INTEGER PRIMARY KEY, title TEXT NOT NULL, owner_id INTEGER NOT NULL)",
        "INSERT INTO gap_cursor_owners (id, title) VALUES (10, 'owner 10'), (20, 'owner 20')",
        "INSERT INTO gap_cursor_owned (id, title, owner_id) VALUES (1, 'row 1', 10), (2, 'row 2', 20)",
    ] {
        db.execute_unprepared(sql).await.expect("fixture");
    }
    db
}

/// The owned rows joined to their owners. Without a `select` the query
/// reads only the owned rows' columns.
fn owned() -> Builder<GapCursorOwned> {
    GapCursorOwned::query().join(
        "gap_cursor_owners",
        "gap_cursor_owners.id",
        "=",
        "gap_cursor_owned.owner_id",
    )
}

/// A model whose table name has capitals. Postgres folds an unquoted name
/// to lower case, so a joined query has to quote every reference it writes
/// to this table, its orderings included, or one names another table.
#[model(table = "GapCursorMixed", timestamps = false)]
pub struct GapCursorMixed {
    /// The test assigns the key.
    pub id: i64,
    /// A label to order by.
    pub title: String,
    /// The owner's key.
    pub owner_id: i64,
}

/// The mixed-case rows joined to their owners.
fn mixed() -> Builder<GapCursorMixed> {
    GapCursorMixed::query().join(
        "gap_cursor_owners",
        "gap_cursor_owners.id",
        "=",
        "GapCursorMixed.owner_id",
    )
}

/// Every page `query` gives one owned row at a time; see [`walk_pages`].
async fn walk_owned(query: impl Fn() -> Builder<GapCursorOwned>) -> (Vec<Vec<i64>>, Vec<i64>) {
    walk_pages(async || {
        let page = query().cursor_paginate(1).await.expect("a page");
        let keys = page.data.iter().map(|row| row.id).collect();
        (keys, page.next_cursor, page.prev_cursor)
    })
    .await
}

#[tokio::test]
async fn a_descending_key_order_walks_the_key_downward() {
    let _db = fixture().await;
    let (pages, back) = walk(|| GapCursorRow::query().order_by_desc("id")).await;
    assert_eq!(pages, [vec![5, 4], vec![3, 2], vec![1]]);
    assert_eq!(back, [5, 4]);
}

#[tokio::test]
async fn one_order_on_another_column_is_kept_with_the_key_breaking_ties() {
    let _db = fixture().await;
    let (pages, back) = walk(|| GapCursorRow::query().latest()).await;
    assert_eq!(pages, [vec![5, 2], vec![4, 3], vec![1]]);
    assert_eq!(back, [5, 2]);
    let (pages, back) = walk(|| GapCursorRow::query().order_by_asc("created_at")).await;
    assert_eq!(pages, [vec![1, 3], vec![2, 4], vec![5]]);
    assert_eq!(back, [1, 3]);

    let first = GapCursorRow::query()
        .latest()
        .cursor_paginate(2)
        .await
        .expect("the first page");
    let cursor = Cursor::decode(first.next_cursor.as_deref().expect("a next cursor"))
        .expect("the cursor decodes");
    assert_eq!(
        cursor.parameter("created_at"),
        Some(&suprnova::sea_orm::Value::BigInt(Some(30)))
    );
    assert_eq!(
        cursor.parameter("id"),
        Some(&suprnova::sea_orm::Value::BigInt(Some(2)))
    );
}

#[tokio::test]
async fn several_orders_are_encoded_in_the_cursor_and_applied_on_the_next_page() {
    let _db = fixture().await;
    let (pages, back) = walk(|| {
        GapCursorRow::query()
            .order_by_desc("created_at")
            .order_by_desc("id")
    })
    .await;
    assert_eq!(pages, [vec![5, 4], vec![2, 3], vec![1]]);
    assert_eq!(back, [5, 4]);
}

#[tokio::test]
async fn a_query_with_no_order_walks_the_key_ascending() {
    let _db = fixture().await;
    let (pages, back) = walk(GapCursorRow::query).await;
    assert_eq!(pages, [vec![1, 2], vec![3, 4], vec![5]]);
    assert_eq!(back, [1, 2]);
}

#[tokio::test]
async fn an_order_the_cursor_cannot_carry_is_refused_not_dropped() {
    let _db = fixture().await;
    let error = GapCursorRow::query()
        .order_by_raw("length(title)")
        .cursor_paginate(2)
        .await
        .expect_err("a raw order has no column to put in the cursor");
    assert_eq!(error.status_code(), 400, "{error}");

    // A cursor minted under one order does not page a query in another.
    let by_key = GapCursorRow::query()
        .order_by_desc("id")
        .cursor_paginate(2)
        .await
        .expect("the first page by key");
    Context::test_set_query("cursor", by_key.next_cursor.expect("a next cursor"));
    let error = GapCursorRow::query()
        .latest()
        .cursor_paginate(2)
        .await
        .expect_err("the cursor holds no created_at");
    assert_eq!(error.status_code(), 400, "{error}");
    Context::test_clear_query();
}

#[tokio::test]
async fn an_order_on_a_joined_tables_column_is_refused() {
    let _db = owned_fixture().await;
    let by_owner = || owned().order_by_asc("gap_cursor_owners.id");
    match by_owner().cursor_paginate(1).await {
        Err(error) => {
            assert_eq!(error.status_code(), 400, "{error}");
            assert!(
                error.to_string().contains("gap_cursor_owners.id"),
                "the error names the column: {error}"
            );
        }
        Ok(first) => {
            // Show what the accepted order did on the next page.
            let next_cursor = first.next_cursor.clone().unwrap_or_default();
            let boundary = Cursor::decode(&next_cursor)
                .map(|cursor| cursor.parameter("gap_cursor_owners.id").cloned());
            Context::test_set_query("cursor", next_cursor);
            let next = by_owner()
                .cursor_paginate(1)
                .await
                .map(|page| page.data.iter().map(|row| row.id).collect::<Vec<_>>());
            Context::test_clear_query();
            panic!(
                "the joined order was accepted: first page {:?}, cursor boundary {boundary:?}, next page {next:?}",
                first.data.iter().map(|row| row.id).collect::<Vec<_>>()
            );
        }
    }

    // A joined table under an alias is another table too.
    let error = GapCursorOwned::query()
        .join(
            "gap_cursor_owners as o",
            "o.id",
            "=",
            "gap_cursor_owned.owner_id",
        )
        .order_by_desc("o.title")
        .cursor_paginate(1)
        .await
        .expect_err("an aliased joined column is not the model's");
    assert_eq!(error.status_code(), 400, "{error}");
    assert!(error.to_string().contains("o.title"), "{error}");
}

#[tokio::test]
async fn an_order_on_the_models_own_qualified_column_is_carried_like_the_bare_one() {
    let _db = fixture().await;
    let bare = walk(|| GapCursorRow::query().order_by_desc("title")).await;
    let qualified = walk(|| GapCursorRow::query().order_by_desc("gap_cursor_rows.title")).await;
    assert_eq!(
        qualified,
        (vec![vec![5, 4], vec![3, 2], vec![1]], vec![5, 4])
    );
    assert_eq!(qualified, bare);

    // A qualified tie keeps the key as its tie-breaker, and the qualified
    // key is the key, so no second key is appended.
    let (pages, back) =
        walk(|| GapCursorRow::query().order_by_desc("gap_cursor_rows.created_at")).await;
    assert_eq!(pages, [vec![5, 2], vec![4, 3], vec![1]]);
    assert_eq!(back, [5, 2]);
    let first = GapCursorRow::query()
        .order_by_desc("gap_cursor_rows.id")
        .cursor_paginate(2)
        .await
        .expect("the first page by the qualified key");
    assert_eq!(ids(&first), [5, 4]);
    let cursor = Cursor::decode(first.next_cursor.as_deref().expect("a next cursor"))
        .expect("the cursor decodes");
    assert_eq!(
        cursor.parameter("gap_cursor_rows.id"),
        Some(&suprnova::sea_orm::Value::BigInt(Some(4)))
    );
    assert!(cursor.parameter("id").is_none(), "{cursor:?}");
}

#[tokio::test]
async fn a_joined_query_pages_by_the_models_own_columns_and_key() {
    let _db = owned_fixture().await;
    let (pages, back) = walk_owned(owned).await;
    assert_eq!(pages, [vec![1], vec![2]]);
    assert_eq!(back, [1]);

    let by_title = || owned().order_by_desc("gap_cursor_owned.title");
    let (pages, back) = walk_owned(by_title).await;
    assert_eq!(pages, [vec![2], vec![1]]);
    assert_eq!(back, [2]);

    // The cursor holds the row's own values, never its owner's.
    let first = by_title().cursor_paginate(1).await.expect("the first page");
    let cursor = Cursor::decode(first.next_cursor.as_deref().expect("a next cursor"))
        .expect("the cursor decodes");
    assert_eq!(
        cursor.parameter("gap_cursor_owned.title"),
        Some(&suprnova::sea_orm::Value::String(Some("row 2".to_owned())))
    );
    assert_eq!(
        cursor.parameter("gap_cursor_owned.id"),
        Some(&suprnova::sea_orm::Value::BigInt(Some(2)))
    );
}

#[tokio::test]
async fn a_joined_query_quotes_the_columns_it_orders_by() {
    let db = owned_fixture().await;
    for sql in [
        r#"CREATE TABLE "GapCursorMixed" (id INTEGER PRIMARY KEY, title TEXT NOT NULL, owner_id INTEGER NOT NULL)"#,
        r#"INSERT INTO "GapCursorMixed" (id, title, owner_id) VALUES (1, 'row 1', 10), (2, 'row 2', 20)"#,
    ] {
        db.execute_unprepared(sql).await.expect("fixture");
    }

    // An ordering the query names, on each engine, on the query and on the
    // first query of a union, which orders its own rows.
    for (backend, title, owner) in [
        (
            DatabaseBackend::Sqlite,
            r#""GapCursorMixed"."title""#,
            r#""gap_cursor_owners"."title""#,
        ),
        (
            DatabaseBackend::Postgres,
            r#""GapCursorMixed"."title""#,
            r#""gap_cursor_owners"."title""#,
        ),
        (
            DatabaseBackend::MySql,
            "`GapCursorMixed`.`title`",
            "`gap_cursor_owners`.`title`",
        ),
    ] {
        let ordered = || {
            mixed()
                .order_by_desc("GapCursorMixed.title")
                .order_by_asc("gap_cursor_owners.title")
        };
        let expected = format!("ORDER BY {title} DESC, {owner} ASC");
        let (sql, _) = ordered()
            .try_to_sql_with_bindings_for(backend)
            .expect("the ordered query renders");
        assert!(sql.contains(&expected), "{backend:?}: {sql}");
        let (sql, _) = ordered()
            .limit(1)
            .union(GapCursorMixed::query())
            .try_to_sql_with_bindings_for(backend)
            .expect("the union renders");
        assert!(sql.contains(&expected), "{backend:?} union: {sql}");
        let (sql, _) = mixed()
            .in_order_of("GapCursorMixed.id", [2, 1])
            .try_to_sql_with_bindings_for(backend)
            .expect("the sequence order renders");
        let id = title.replace("title", "id");
        assert!(sql.contains(&format!("WHEN {id} = ")), "{backend:?}: {sql}");
    }

    // The key `cursor_paginate` adds, on the first page and the next, where
    // the cursor's boundary names it too.
    DB::enable_query_log().expect("the query log turns on");
    let first = mixed().cursor_paginate(1).await.expect("the first page");
    assert_eq!(first.data.iter().map(|row| row.id).collect::<Vec<_>>(), [1]);
    Context::test_set_query("cursor", first.next_cursor.expect("a next cursor"));
    let second = mixed().cursor_paginate(1).await.expect("the next page");
    Context::test_clear_query();
    assert_eq!(
        second.data.iter().map(|row| row.id).collect::<Vec<_>>(),
        [2]
    );
    let logged: Vec<String> = DB::get_query_log()
        .expect("the query log reads")
        .into_iter()
        .map(|query| query.sql)
        .filter(|sql| sql.contains("GapCursorMixed"))
        .collect();
    assert_eq!(logged.len(), 2, "{logged:#?}");
    for sql in &logged {
        assert!(
            sql.contains(r#"ORDER BY "GapCursorMixed"."id" ASC"#),
            "{sql}"
        );
    }
    assert!(
        logged[1].contains(r#""GapCursorMixed"."id" > "#),
        "{}",
        logged[1]
    );
}
