//! `cursor_paginate` keeps the query's orderings, as Laravel's
//! `cursorPaginate` honours `orderBy`: a descending order walks downward,
//! the cursor carries every ordered column and the next page applies them,
//! and a query with no order walks the primary key ascending.

use crate::key_ring::rotation_keys;
use suprnova::context::Context;
use suprnova::testing::TestDatabase;
use suprnova::{Builder, Cursor, CursorPaginator, Model, attrs, model};

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

/// Every page `query` gives two rows at a time, walking the next cursors
/// to the end, then the page before the second one.
async fn walk(query: impl Fn() -> Builder<GapCursorRow>) -> (Vec<Vec<i64>>, Vec<i64>) {
    Context::test_clear_query();
    let mut pages = Vec::new();
    let mut second_prev = None;
    loop {
        let page = query().cursor_paginate(2).await.expect("a page");
        pages.push(ids(&page));
        if pages.len() == 2 {
            second_prev = page.prev_cursor.clone();
        }
        match page.next_cursor {
            Some(next) => Context::test_set_query("cursor", next),
            None => break,
        }
    }
    Context::test_set_query(
        "cursor",
        second_prev.expect("the second page has a previous one"),
    );
    let back = query().cursor_paginate(2).await.expect("the page before");
    Context::test_clear_query();
    (pages, ids(&back))
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
