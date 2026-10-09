//! Query shapes whose SQL differs by engine: unions with pages, counts and
//! orderings, random order, an offset with no limit, aggregates over a
//! query that already has a projection, JSON containment, and
//! `create_or_first` inside a transaction.
//!
//! The SQLite tests run by default; the `postgres_` and `mysql_` tests run
//! every scenario against a disposable server:
//!
//! ```text
//! PG_TEST_URL=postgres://... MYSQL_TEST_URL=mysql://... \
//!   cargo test -p suprnova --test eloquent query_shapes_engines:: -- --ignored --test-threads=1
//! ```

use sea_orm::DatabaseBackend;
use serde_json::json;
use suprnova::eloquent::FirstOrCreate;
use suprnova::{Builder, DB, FrameworkError, Model, attrs, model};

use crate::query_fixture::Fixture;

#[model(
    table = "qs_items",
    auto_increment = false,
    timestamps = false,
    fillable = ["id", "grp", "score", "email"]
)]
pub struct QsItem {
    pub id: i64,
    pub grp: String,
    pub score: i64,
    pub email: String,
}

/// JSON documents. The columns are read through `pluck` only, so their
/// native type never has to hydrate into a Rust field.
#[model(table = "qs_docs", timestamps = false)]
pub struct QsDoc {
    pub id: i64,
    pub meta: String,
    pub tags: String,
}

/// Six rows in three groups of two: `a` holds 1 and 2, `b` holds 3 and 4,
/// `c` holds 5 and 6. Scores run 10 to 60 with the id.
///
/// MySQL cannot name one `TEMPORARY` table twice in a statement, and a
/// union names its table once per arm, so there the table is an ordinary
/// one, dropped first in case a failed run left it and dropped again by
/// [`finish`].
async fn seed(fx: &Fixture) {
    let temporary = if fx.backend == DatabaseBackend::MySql {
        fx.exec("DROP TABLE IF EXISTS qs_items").await;
        ""
    } else {
        "TEMPORARY "
    };
    fx.exec(&format!(
        "CREATE {temporary}TABLE qs_items (id BIGINT PRIMARY KEY, grp VARCHAR(10) NOT NULL, \
         score BIGINT NOT NULL, email VARCHAR(50) NOT NULL UNIQUE)"
    ))
    .await;
    fx.exec(
        "INSERT INTO qs_items (id, grp, score, email) VALUES \
         (1, 'a', 10, 'e1'), (2, 'a', 20, 'e2'), (3, 'b', 30, 'e3'), \
         (4, 'b', 40, 'e4'), (5, 'c', 50, 'e5'), (6, 'c', 60, 'e6')",
    )
    .await;
}

/// Drop the ordinary table MySQL needed, then close the connection, which
/// drops the temporary ones.
async fn finish(fx: Fixture) {
    if fx.backend == DatabaseBackend::MySql {
        fx.exec("DROP TABLE IF EXISTS qs_items").await;
    }
    fx.close().await;
}

/// The JSON table, in each engine's own JSON types. On Postgres `meta` is
/// `jsonb` and `tags` is `json`, so containment is proven on both.
async fn seed_docs(fx: &Fixture) {
    let (meta, tags) = match fx.backend {
        DatabaseBackend::Postgres => ("JSONB", "JSON"),
        _ => ("JSON", "JSON"),
    };
    fx.exec(&format!(
        "CREATE TEMPORARY TABLE qs_docs (id BIGINT PRIMARY KEY, meta {meta} NOT NULL, \
         tags {tags} NOT NULL)"
    ))
    .await;
    fx.exec(
        "INSERT INTO qs_docs (id, meta, tags) VALUES \
         (1, '{\"active\": true, \"tier\": 1}', '[\"admin\", \"user\"]'), \
         (2, '{\"active\": false, \"tier\": 2}', '[\"user\"]'), \
         (3, '{\"active\": true}', '[]')",
    )
    .await;
}

fn items() -> Builder<QsItem> {
    QsItem::query()
}

fn ids(rows: &[QsItem]) -> Vec<i64> {
    rows.iter().map(|row| row.id).collect()
}

async fn sorted_ids(query: Builder<QsItem>) -> Vec<i64> {
    let mut ids: Vec<i64> = query
        .get()
        .await
        .unwrap_or_else(|e| panic!("query failed: {e}"))
        .iter()
        .map(|row| row.id)
        .collect();
    ids.sort_unstable();
    ids
}

// ---------- Scenarios ---------------------------------------------------------

/// DATA-056 and DATA-005: a page of a union is a page of the whole union,
/// and its total counts the union's rows. A limit or an ordering added
/// after `union` belongs to the union; one added before it belongs to the
/// first query alone, as in Laravel.
async fn unions_page_count_and_order() {
    let a_or_b = || items().filter("grp", "a").union(items().filter("grp", "b"));

    let page = a_or_b()
        .paginate(3)
        .await
        .unwrap_or_else(|e| panic!("a page of a union: {e}"));
    assert_eq!(page.total, 4, "the total counts every row of the union");
    assert_eq!(page.data.len(), 3, "the page holds per_page rows");
    assert_eq!(page.last_page, 2);

    let ordered = a_or_b()
        .order_by_desc("id")
        .paginate(3)
        .await
        .unwrap_or_else(|e| panic!("an ordered page of a union: {e}"));
    assert_eq!(ids(&ordered.data), vec![4, 3, 2], "the union is ordered");
    assert_eq!(ordered.total, 4);

    let simple = a_or_b()
        .order_by_asc("id")
        .simple_paginate(3)
        .await
        .unwrap_or_else(|e| panic!("a simple page of a union: {e}"));
    assert_eq!(ids(&simple.data), vec![1, 2, 3]);
    assert!(simple.has_more, "a fourth row lies beyond the page");

    let first = items()
        .filter("grp", "b")
        .union(items().filter("grp", "a"))
        .order_by_asc("id")
        .first()
        .await
        .unwrap_or_else(|e| panic!("the first row of a union: {e}"))
        .expect("a row");
    assert_eq!(
        first.id, 1,
        "first() takes the first row of the whole union"
    );

    // Overlapping arms: UNION keeps one copy of row 2, UNION ALL keeps two.
    let overlap = || {
        items()
            .filter_op("score", ">=", 20)
            .filter_op("score", "<=", 30)
    };
    let distinct = items()
        .filter("grp", "a")
        .union(overlap())
        .paginate(10)
        .await
        .unwrap_or_else(|e| panic!("a page of an overlapping union: {e}"));
    assert_eq!(distinct.total, 3, "the total is the distinct union's rows");
    assert_eq!(distinct.data.len(), 3);
    let all = items()
        .filter("grp", "a")
        .union_all(overlap())
        .paginate(10)
        .await
        .unwrap_or_else(|e| panic!("a page of a UNION ALL: {e}"));
    assert_eq!(all.total, 4, "UNION ALL keeps the duplicate");
    assert_eq!(all.data.len(), 4);

    assert_eq!(
        a_or_b()
            .count()
            .await
            .unwrap_or_else(|e| panic!("count of a union: {e}")),
        4,
        "count() counts the union's rows"
    );

    // Set before `union`, the ordering and the limit keep the first query
    // to its highest id.
    let head_limited = items()
        .filter("grp", "a")
        .order_by_desc("id")
        .limit(1)
        .union(items().filter("grp", "b"))
        .order_by_asc("id");
    assert_eq!(sorted_ids(head_limited).await, vec![2, 3, 4]);
}

/// DATA-037: random order runs on every engine and returns every row.
async fn random_order_runs() {
    assert_eq!(
        sorted_ids(items().in_random_order()).await,
        vec![1, 2, 3, 4, 5, 6]
    );
    assert!(
        items()
            .in_random_order()
            .first()
            .await
            .unwrap_or_else(|e| panic!("first() in random order: {e}"))
            .is_some()
    );
}

/// DATA-054: an offset with no limit skips rows on every engine, on the
/// model builder and on `DB::table`.
async fn offset_without_limit_skips_rows() {
    let rows = items()
        .order_by_asc("id")
        .skip(4)
        .get()
        .await
        .unwrap_or_else(|e| panic!("skip without take: {e}"));
    assert_eq!(ids(&rows), vec![5, 6]);

    let rows = DB::table("qs_items")
        .order_by_asc("id")
        .offset(4)
        .get()
        .await
        .unwrap_or_else(|e| panic!("DB::table offset without limit: {e}"));
    let table_ids: Vec<i64> = rows
        .iter()
        .map(|row| row.get_int("id").expect("an integer id"))
        .collect();
    assert_eq!(table_ids, vec![5, 6]);
}

/// DATA-007: an aggregate replaces the query's projection and ignores its
/// ordering, and a query with HAVING is aggregated over its groups.
async fn aggregates_ignore_the_projection() {
    assert_eq!(
        items()
            .select(["id"])
            .count()
            .await
            .unwrap_or_else(|e| panic!("count after select: {e}")),
        6
    );
    assert_eq!(
        items()
            .select_raw("grp")
            .filter("grp", "a")
            .count()
            .await
            .unwrap_or_else(|e| panic!("count after select_raw: {e}")),
        2
    );
    assert_eq!(
        items()
            .select(["id"])
            .sum::<i64>("score")
            .await
            .unwrap_or_else(|e| panic!("sum after select: {e}")),
        210
    );
    assert_eq!(
        items()
            .select(["grp"])
            .max::<i64>("score")
            .await
            .unwrap_or_else(|e| panic!("max after select: {e}")),
        Some(60)
    );
    assert_eq!(
        items()
            .order_by_asc("grp")
            .count()
            .await
            .unwrap_or_else(|e| panic!("count of an ordered query: {e}")),
        6,
        "an ordering does not reach the aggregate"
    );
    assert_eq!(
        items()
            .select(["grp"])
            .group_by("grp")
            .having_op("grp", "<>", "c")
            .count()
            .await
            .unwrap_or_else(|e| panic!("count with HAVING: {e}")),
        2,
        "with HAVING the count is the number of groups that pass it"
    );
}

/// DATA-052: inside a transaction, `create_or_first` that loses to an
/// existing row returns that row, and the transaction stays usable.
async fn create_or_first_inside_a_transaction() {
    let (found, rows) = DB::transaction(|_tx| {
        Box::pin(async move {
            let found = QsItem::create_or_first(
                attrs! { email: "e1" },
                attrs! { id: 99, grp: "z", score: 0 },
            )
            .await?;
            let rows = QsItem::query().count().await?;
            Ok::<_, FrameworkError>((found, rows))
        })
    })
    .await
    .unwrap_or_else(|e| panic!("create_or_first inside a transaction: {e}"));
    assert_eq!(found.id, 1, "the existing row is returned");
    assert_eq!(rows, 6, "the transaction still answers after the conflict");
    assert_eq!(items().count().await.expect("count"), 6, "no row was added");
}

/// DATA-053: JSON containment binds the candidate as a JSON document.
async fn json_containment() {
    let pluck = |q: Builder<QsDoc>| async move {
        q.order_by_asc("id")
            .pluck::<i64>("id")
            .await
            .unwrap_or_else(|e| panic!("JSON containment: {e}"))
    };
    assert_eq!(
        pluck(QsDoc::query().filter_json_contains("meta", json!({ "active": true }))).await,
        vec![1, 3]
    );
    assert_eq!(
        pluck(QsDoc::query().filter_json_contains("meta", json!({ "tier": 1 }))).await,
        vec![1]
    );
    assert_eq!(
        pluck(QsDoc::query().filter_json_contains("tags", json!("admin"))).await,
        vec![1],
        "a string is a JSON string, not bare text"
    );
    assert_eq!(
        pluck(QsDoc::query().filter_json_contains("tags", json!(["user"]))).await,
        vec![1, 2]
    );
}

/// Aggregates whose offset skips the one row they return answer as Laravel
/// does: `count` and `sum` 0, `min` and `max` none, and `avg` the 0 this
/// builder's `avg` returns for an empty set. A grouped query over no rows
/// returns no row either. All of these used to fail with "aggregate query
/// returned no row".
async fn aggregates_when_no_row_comes_back() {
    fn fail<T>(e: FrameworkError) -> T {
        panic!("an aggregate with no row: {e}")
    }
    assert_eq!(items().skip(1).count().await.unwrap_or_else(fail), 0);
    assert_eq!(items().skip(10).count().await.unwrap_or_else(fail), 0);
    assert_eq!(
        items()
            .skip(10)
            .sum::<i64>("score")
            .await
            .unwrap_or_else(fail),
        0
    );
    assert_eq!(
        items()
            .skip(10)
            .avg::<f64>("score")
            .await
            .unwrap_or_else(fail),
        0.0
    );
    assert_eq!(
        items()
            .skip(10)
            .min::<i64>("score")
            .await
            .unwrap_or_else(fail),
        None
    );
    assert_eq!(
        items()
            .skip(10)
            .max::<i64>("score")
            .await
            .unwrap_or_else(fail),
        None
    );
    assert_eq!(
        items()
            .filter("grp", "none")
            .group_by("grp")
            .count()
            .await
            .unwrap_or_else(fail),
        0,
        "no group, no row"
    );
}

/// A cursor bounds the whole union, on every page and in both directions.
/// It used to filter the first query only, so the page after 1 and 2 of
/// `b UNION a` was 1 and 2 again: the arm holding them was never bounded.
#[cfg(feature = "testing")]
async fn cursor_pages_walk_the_whole_union() {
    use suprnova::context::Context;

    crate::key_ring::rotation_keys();
    Context::test_clear_query();
    let b_or_a = || items().filter("grp", "b").union(items().filter("grp", "a"));
    let page = |name: &'static str| async move {
        b_or_a()
            .cursor_paginate(2)
            .await
            .unwrap_or_else(|e| panic!("{name} of a union: {e}"))
    };

    let first = page("the first page").await;
    assert_eq!(ids(&first.data), vec![1, 2]);
    Context::test_set_query("cursor", first.next_cursor.clone().expect("a next cursor"));
    let second = page("the second page").await;
    assert_eq!(ids(&second.data), vec![3, 4], "the cursor bounds every arm");
    assert!(second.next_cursor.is_none(), "nothing lies after 4");
    Context::test_set_query(
        "cursor",
        second.prev_cursor.clone().expect("a previous cursor"),
    );
    let back = page("the page before").await;
    assert_eq!(ids(&back.data), vec![1, 2]);
    Context::test_clear_query();
}

/// The keyset walks `chunk_by_id` and `lazy_by_id` bound the whole union
/// with their cursor, so every row comes once and the walk ends. The
/// cursor used to filter the first query only: after 1 and 2, the batch of
/// `b UNION a` was 1 and 2 again, forever. A walk stops here after a few
/// batches, so the old behavior fails the test instead of hanging it.
async fn keyset_walks_cover_the_whole_union() {
    use std::sync::{Arc, Mutex};

    const MOST_BATCHES: usize = 4;
    let b_or_a = || items().filter("grp", "b").union(items().filter("grp", "a"));

    async fn chunked(query: Builder<QsItem>) -> Vec<Vec<i64>> {
        let batches = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&batches);
        let walked = query
            .chunk_by_id(2, move |batch| {
                let sink = Arc::clone(&sink);
                async move {
                    let mut batches = sink.lock().unwrap_or_else(|e| e.into_inner());
                    batches.push(batch.iter().map(|row| row.id).collect::<Vec<_>>());
                    if batches.len() > MOST_BATCHES {
                        return Err(FrameworkError::internal("the walk repeats its batches"));
                    }
                    Ok(())
                }
            })
            .await;
        let batches = batches.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Err(e) = walked {
            panic!("chunk_by_id over a union: {e}; batches {batches:?}");
        }
        batches
    }

    async fn streamed(query: Builder<QsItem>) -> Vec<i64> {
        let mut stream = query.lazy_by_id(2);
        let mut seen = Vec::new();
        while let Some(row) = stream.next().await {
            let row = row.unwrap_or_else(|e| panic!("lazy_by_id over a union: {e}"));
            seen.push(row.id);
            assert!(
                seen.len() <= MOST_BATCHES * 2,
                "the stream repeats its rows: {seen:?}"
            );
        }
        seen
    }

    assert_eq!(chunked(b_or_a()).await, vec![vec![1, 2], vec![3, 4]]);
    assert_eq!(streamed(b_or_a()).await, vec![1, 2, 3, 4]);
    // The union's own offset and limit still bound the walk.
    assert_eq!(
        chunked(b_or_a().skip(1).limit(3)).await,
        vec![vec![2, 3], vec![4]]
    );
    assert_eq!(streamed(b_or_a().skip(1).limit(3)).await, vec![2, 3, 4]);
}

// ---------- SQLite ------------------------------------------------------------

async fn seeded_sqlite() -> Fixture {
    let fx = Fixture::sqlite().await;
    seed(&fx).await;
    fx
}

#[tokio::test]
async fn sqlite_unions_page_count_and_order() {
    let _fx = seeded_sqlite().await;
    unions_page_count_and_order().await;
}

#[tokio::test]
async fn sqlite_random_order_runs() {
    let _fx = seeded_sqlite().await;
    random_order_runs().await;
}

#[tokio::test]
async fn sqlite_offset_without_limit_skips_rows() {
    let _fx = seeded_sqlite().await;
    offset_without_limit_skips_rows().await;
}

#[tokio::test]
async fn sqlite_aggregates_ignore_the_projection() {
    let _fx = seeded_sqlite().await;
    aggregates_ignore_the_projection().await;
}

#[tokio::test]
async fn sqlite_create_or_first_inside_a_transaction() {
    let _fx = seeded_sqlite().await;
    create_or_first_inside_a_transaction().await;
}

#[tokio::test]
async fn sqlite_aggregates_when_no_row_comes_back() {
    let _fx = seeded_sqlite().await;
    aggregates_when_no_row_comes_back().await;
}

#[cfg(feature = "testing")]
#[tokio::test]
async fn sqlite_cursor_pages_walk_the_whole_union() {
    let _fx = seeded_sqlite().await;
    cursor_pages_walk_the_whole_union().await;
}

#[tokio::test]
async fn sqlite_keyset_walks_cover_the_whole_union() {
    let _fx = seeded_sqlite().await;
    keyset_walks_cover_the_whole_union().await;
}

// ---------- Live engines ----------------------------------------------------

/// One connection to the server named by `env`, seeded with both tables.
async fn live(env: &str, backend: DatabaseBackend) -> Fixture {
    let fx = Fixture::live(env, backend).await;
    seed(&fx).await;
    seed_docs(&fx).await;
    fx
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL at PG_TEST_URL"]
async fn postgres_unions_page_count_and_order() {
    let fx = live("PG_TEST_URL", DatabaseBackend::Postgres).await;
    unions_page_count_and_order().await;
    finish(fx).await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL at PG_TEST_URL"]
async fn postgres_random_order_runs() {
    let fx = live("PG_TEST_URL", DatabaseBackend::Postgres).await;
    random_order_runs().await;
    finish(fx).await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL at PG_TEST_URL"]
async fn postgres_offset_without_limit_skips_rows() {
    let fx = live("PG_TEST_URL", DatabaseBackend::Postgres).await;
    offset_without_limit_skips_rows().await;
    finish(fx).await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL at PG_TEST_URL"]
async fn postgres_aggregates_ignore_the_projection() {
    let fx = live("PG_TEST_URL", DatabaseBackend::Postgres).await;
    aggregates_ignore_the_projection().await;
    finish(fx).await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL at PG_TEST_URL"]
async fn postgres_create_or_first_inside_a_transaction() {
    let fx = live("PG_TEST_URL", DatabaseBackend::Postgres).await;
    create_or_first_inside_a_transaction().await;
    finish(fx).await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL at PG_TEST_URL"]
async fn postgres_json_containment() {
    let fx = live("PG_TEST_URL", DatabaseBackend::Postgres).await;
    json_containment().await;
    finish(fx).await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_unions_page_count_and_order() {
    let fx = live("MYSQL_TEST_URL", DatabaseBackend::MySql).await;
    unions_page_count_and_order().await;
    finish(fx).await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_random_order_runs() {
    let fx = live("MYSQL_TEST_URL", DatabaseBackend::MySql).await;
    random_order_runs().await;
    finish(fx).await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_offset_without_limit_skips_rows() {
    let fx = live("MYSQL_TEST_URL", DatabaseBackend::MySql).await;
    offset_without_limit_skips_rows().await;
    finish(fx).await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_aggregates_ignore_the_projection() {
    let fx = live("MYSQL_TEST_URL", DatabaseBackend::MySql).await;
    aggregates_ignore_the_projection().await;
    finish(fx).await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_create_or_first_inside_a_transaction() {
    let fx = live("MYSQL_TEST_URL", DatabaseBackend::MySql).await;
    create_or_first_inside_a_transaction().await;
    finish(fx).await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_json_containment() {
    let fx = live("MYSQL_TEST_URL", DatabaseBackend::MySql).await;
    json_containment().await;
    finish(fx).await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL at PG_TEST_URL"]
async fn postgres_aggregates_when_no_row_comes_back() {
    let fx = live("PG_TEST_URL", DatabaseBackend::Postgres).await;
    aggregates_when_no_row_comes_back().await;
    finish(fx).await;
}

#[cfg(feature = "testing")]
#[tokio::test]
#[ignore = "requires disposable PostgreSQL at PG_TEST_URL"]
async fn postgres_cursor_pages_walk_the_whole_union() {
    let fx = live("PG_TEST_URL", DatabaseBackend::Postgres).await;
    cursor_pages_walk_the_whole_union().await;
    finish(fx).await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL at PG_TEST_URL"]
async fn postgres_keyset_walks_cover_the_whole_union() {
    let fx = live("PG_TEST_URL", DatabaseBackend::Postgres).await;
    keyset_walks_cover_the_whole_union().await;
    finish(fx).await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_aggregates_when_no_row_comes_back() {
    let fx = live("MYSQL_TEST_URL", DatabaseBackend::MySql).await;
    aggregates_when_no_row_comes_back().await;
    finish(fx).await;
}

#[cfg(feature = "testing")]
#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_cursor_pages_walk_the_whole_union() {
    let fx = live("MYSQL_TEST_URL", DatabaseBackend::MySql).await;
    cursor_pages_walk_the_whole_union().await;
    finish(fx).await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_keyset_walks_cover_the_whole_union() {
    let fx = live("MYSQL_TEST_URL", DatabaseBackend::MySql).await;
    keyset_walks_cover_the_whole_union().await;
    finish(fx).await;
}
