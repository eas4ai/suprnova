//! Model query contracts for missing rows, expressions and ordering.

use crate::query_fixture::Fixture;
use crate::query_helpers_model::QhItem;
use chrono::{DateTime, Utc};
use sea_orm::DbBackend;
use serde_json::json;
use suprnova::{Builder, DB, Model, UpdateAttrs, model};

/// A renamed creation timestamp proves ordering uses declared model metadata.
#[model(
    table = "gap_times",
    created_at = "created_on",
    updated_at = "modified_on"
)]
pub struct GapTime {
    /// Identify rows so chronological ordering can be asserted.
    pub id: i64,
    /// Exercise the renamed creation timestamp declaration.
    pub created_on: DateTime<Utc>,
    /// Supply the paired timestamp that enables timestamp management.
    pub modified_on: DateTime<Utc>,
}

/// A parent whose eager children rank by the seeded order in a window.
#[model(table = "gap_random_bins", relations = {
    items: HasMany<QhItem> { fk = "b" },
})]
pub struct GapRandomBin {
    /// Each bin owns the items whose `b` names it.
    pub id: i64,
}

async fn fixture() -> Fixture {
    let fx = Fixture::sqlite().await;
    fx.exec("CREATE TABLE qh_items (id INTEGER PRIMARY KEY, a INTEGER, b INTEGER, c INTEGER, label TEXT, code TEXT, description TEXT, deleted_at TEXT)").await;
    fx.exec("INSERT INTO qh_items VALUES (1, 10, 1, 1, 'a', 'one', '', NULL), (2, 20, 1, 1, 'a', 'two', '', NULL), (3, 30, 2, 1, 'b', 'three', '', NULL)").await;
    fx
}

#[tokio::test]
async fn first_or_fail_names_the_model_or_preserves_the_custom_message() {
    let _fx = fixture().await;
    let query = || QhItem::query().filter("id", 99);
    let error = query().first_or_fail().await.unwrap_err();
    assert_eq!(error.status_code(), 404);
    assert!(error.to_string().contains("QhItem"));
    let error = query()
        .first_or_fail_with("No matching item")
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), 404);
    assert!(error.to_string().contains("No matching item"));
    assert_eq!(
        QhItem::query()
            .first_or_fail_with("unused")
            .await
            .unwrap()
            .id,
        1
    );
    assert_eq!(
        QhItem::query()
            .where_raw("invalid SQL", vec![])
            .first_or_fail()
            .await
            .unwrap_err()
            .status_code(),
        500
    );
}

#[tokio::test]
async fn chronological_ordering_uses_the_renamed_creation_timestamp() {
    let fx = Fixture::sqlite().await;
    fx.exec("CREATE TABLE gap_times (id INTEGER PRIMARY KEY, created_on TEXT, modified_on TEXT)")
        .await;
    fx.exec("INSERT INTO gap_times VALUES (1, '2026-10-03 00:00:00', '2026-10-01 00:00:00'), (2, '2026-10-01 00:00:00', '2026-10-03 00:00:00'), (3, '2026-10-02 00:00:00', '2026-10-02 00:00:00')").await;
    assert_eq!(
        GapTime::query().oldest().first().await.unwrap().unwrap().id,
        2
    );
    assert_eq!(
        GapTime::query().latest().first().await.unwrap().unwrap().id,
        1
    );
    assert_eq!(GapTime::oldest().first().await.unwrap().unwrap().id, 2);
    assert_eq!(GapTime::latest().first().await.unwrap().unwrap().id, 1);
    assert_eq!(
        GapTime::query()
            .oldest_by("modified_on")
            .first()
            .await
            .unwrap()
            .unwrap()
            .id,
        1
    );
    assert!(
        GapTime::query()
            .oldest_by("created_on; --")
            .get()
            .await
            .is_err()
    );
    assert!(
        GapTime::query()
            .filter("id", 99)
            .latest()
            .first()
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn model_expressions_update_group_filter_and_keep_raw_bindings() {
    let _fx = fixture().await;
    let mut attrs = UpdateAttrs::new();
    attrs
        .insert("a", DB::raw("a + 5"))
        .insert("code", "changed");
    assert_eq!(
        QhItem::query().filter("id", 1).update(attrs).await.unwrap(),
        1
    );
    assert_eq!(
        QhItem::query()
            .filter("id", 1)
            .first()
            .await
            .unwrap()
            .unwrap()
            .a,
        15
    );
    let query = QhItem::query()
        .select_raw_with_bindings("b, SUM(a) + ? AS a", vec![2.into()])
        .where_raw("a > ?", vec![json!(0)])
        .group_by(DB::raw("b + 0"))
        .having_op(DB::raw("SUM(a)"), ">", 32)
        .order_by_raw_with_bindings("SUM(a) + ? DESC", vec![1.into()]);
    let (sql, values) = query.to_sql_with_bindings_for(DbBackend::Postgres);
    assert!(sql.contains("SUM(a) + $1"), "{sql}");
    assert!(sql.contains("a > $2"), "{sql}");
    assert!(sql.contains("GROUP BY b + 0 HAVING SUM(a) > $3"), "{sql}");
    assert!(sql.contains("ORDER BY SUM(a) + $4 DESC"), "{sql}");
    assert_eq!(values.len(), 4);
    let (sql, values) = query.to_sql_with_bindings_for(DbBackend::Sqlite);
    assert_eq!(
        DB::select(&sql, values).await.unwrap()[0]
            .get_int("a")
            .unwrap(),
        37
    );
    assert_eq!(
        QhItem::query().max::<i64>(DB::raw("a + 1")).await.unwrap(),
        Some(31)
    );
    assert!(
        QhItem::query()
            .update([("a; --", DB::raw("0"))])
            .await
            .is_err()
    );
    assert!(
        QhItem::query()
            .select_raw_with_bindings("? + ?", vec![1.into()])
            .get()
            .await
            .is_err()
    );
    assert!(
        QhItem::query()
            .order_by_raw_with_bindings("? + ?", vec![1.into()])
            .get()
            .await
            .is_err()
    );
}

#[tokio::test]
async fn model_ranges_accept_raw_expressions_and_scalar_subqueries() {
    let _fx = fixture().await;
    let rows = QhItem::query()
        .where_between(DB::raw("a + 1"), 11..=21)
        .order_by_asc("id")
        .get()
        .await
        .unwrap();
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![1, 2]);
    let query = DB::table("qh_items").select(["a"]).filter("id", 2);
    let rows = QhItem::query()
        .where_between(query, 20..=20)
        .get()
        .await
        .unwrap();
    assert_eq!(rows.len(), 3);
    let rows = QhItem::query()
        .where_not_between(DB::raw("a + 1"), 11..=21)
        .get()
        .await
        .unwrap();
    assert_eq!(rows[0].id, 3);
    assert_eq!(
        QhItem::query()
            .max::<i64>(DB::table("qh_items").select(["a"]).filter("id", 2))
            .await
            .unwrap(),
        Some(20)
    );
}

#[tokio::test]
async fn seeded_random_order_emits_engine_sql_and_sqlite_accepts_every_seed() {
    let _fx = fixture().await;
    for backend in [DbBackend::MySql, DbBackend::Postgres] {
        let first = QhItem::query()
            .in_random_order_seeded(42)
            .to_sql_with_bindings_for(backend);
        assert_eq!(
            first,
            QhItem::query()
                .in_random_order_seeded(42)
                .to_sql_with_bindings_for(backend)
        );
        if backend == DbBackend::MySql {
            assert!(first.0.contains("ORDER BY RAND(42)"));
        } else {
            assert!(first.0.contains("SELECT setseed("));
            assert!(first.0.contains("ORDER BY CASE WHEN"));
            assert!(first.0.contains("THEN RANDOM() ELSE RANDOM() END"));
            assert!(!first.0.contains("CROSS JOIN"));
        }
    }
    for seed in [0, 42, u64::MAX] {
        let mut ids: Vec<_> = QhItem::query()
            .in_random_order_seeded(seed)
            .get()
            .await
            .unwrap()
            .iter()
            .map(|r| r.id)
            .collect();
        ids.sort_unstable();
        assert_eq!(ids, vec![1, 2, 3]);
    }
    assert!(
        QhItem::query()
            .filter("id", 99)
            .in_random_order_seeded(0)
            .get()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn random_order_without_a_seed_emits_plain_engine_sql_and_returns_each_row() {
    let _fx = fixture().await;
    for (backend, expected) in [
        (DbBackend::MySql, "ORDER BY RAND()"),
        (DbBackend::Postgres, "ORDER BY RANDOM()"),
        (DbBackend::Sqlite, "ORDER BY RANDOM()"),
    ] {
        let (sql, values) = QhItem::query()
            .in_random_order()
            .to_sql_with_bindings_for(backend);
        assert!(sql.contains(expected), "{backend:?}: {sql}");
        assert!(!sql.contains("setseed"), "{backend:?}: {sql}");
        assert!(values.is_empty());
    }
    let mut ids: Vec<_> = QhItem::query()
        .in_random_order()
        .get()
        .await
        .unwrap()
        .iter()
        .map(|r| r.id)
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![1, 2, 3]);
}

#[tokio::test]
async fn seeded_random_order_repeats_for_the_same_seed_on_sqlite() {
    let fx = Fixture::sqlite().await;
    fx.exec("CREATE TABLE qh_items (id INTEGER PRIMARY KEY, a INTEGER, b INTEGER, c INTEGER, label TEXT, code TEXT, description TEXT, deleted_at TEXT)").await;
    let rows: Vec<String> = (1..=12)
        .map(|id| format!("({id}, {id}, 1, 1, 'x', 'code{id}', '', NULL)"))
        .collect();
    fx.exec(&format!("INSERT INTO qh_items VALUES {}", rows.join(", ")))
        .await;
    let seeded_ids = || async {
        QhItem::query()
            .in_random_order_seeded(42)
            .get()
            .await
            .unwrap()
            .iter()
            .map(|r| r.id)
            .collect::<Vec<_>>()
    };
    let first = seeded_ids().await;
    assert_eq!(first, seeded_ids().await);
    let mut sorted = first;
    sorted.sort_unstable();
    assert_eq!(sorted, (1..=12).collect::<Vec<_>>());
    let mut other = QhItem::query()
        .in_random_order_seeded(7)
        .get()
        .await
        .unwrap()
        .iter()
        .map(|r| r.id)
        .collect::<Vec<_>>();
    other.sort_unstable();
    assert_eq!(other, (1..=12).collect::<Vec<_>>());
}

#[tokio::test]
async fn seeded_random_order_over_a_join_on_sqlite_repeats_each_row_once() {
    let fx = Fixture::sqlite().await;
    fx.exec("CREATE TABLE qh_items (id INTEGER PRIMARY KEY, a INTEGER, b INTEGER, c INTEGER, label TEXT, code TEXT, description TEXT, deleted_at TEXT)").await;
    let rows: Vec<String> = (1..=12)
        .map(|id| format!("({id}, {id}, 1, 1, 'x', 'code{id}', '', NULL)"))
        .collect();
    fx.exec(&format!("INSERT INTO qh_items VALUES {}", rows.join(", ")))
        .await;
    fx.exec("CREATE TABLE qh_links (a_id INTEGER, note TEXT)")
        .await;
    let links: Vec<String> = (1..=12).map(|id| format!("({id}, 'n{id}')")).collect();
    fx.exec(&format!("INSERT INTO qh_links VALUES {}", links.join(", ")))
        .await;
    let joined_ids = || async {
        QhItem::query()
            .join("qh_links", "qh_items.id", "=", "qh_links.a_id")
            .in_random_order_seeded(42)
            .get()
            .await
            .unwrap()
            .iter()
            .map(|r| r.id)
            .collect::<Vec<_>>()
    };
    let first = joined_ids().await;
    assert_eq!(first, joined_ids().await);
    let mut sorted = first;
    sorted.sort_unstable();
    assert_eq!(sorted, (1..=12).collect::<Vec<_>>());
}

/// Rows a seeded SQLite order must keep apart: ids 1 and 4294967297 agree
/// mod 2^32, and adding a seed to an id near `i64::MAX` overflows SQLite's
/// integer range. Column `a` is indexed and runs against the id order, so a
/// filter on it reads the rows in another order than a full scan does.
/// Bin 1 holds the two small ids and bin 2 the two large ones.
async fn wide_key_fixture() -> Fixture {
    let fx = Fixture::sqlite().await;
    fx.exec("CREATE TABLE qh_items (id INTEGER PRIMARY KEY, a INTEGER, b INTEGER, c INTEGER, label TEXT, code TEXT, description TEXT, deleted_at TEXT)").await;
    fx.exec("CREATE INDEX qh_items_a ON qh_items (a)").await;
    fx.exec(&format!(
        "INSERT INTO qh_items VALUES (1, 4, 1, 1, 'x', 'one', '', NULL), \
         (4294967297, 3, 1, 1, 'x', 'two', '', NULL), \
         ({}, 2, 2, 2, 'x', 'three', '', NULL), ({}, 1, 2, 2, 'x', 'four', '', NULL)",
        i64::MAX - 1,
        i64::MAX
    ))
    .await;
    fx.exec("CREATE TABLE gap_random_bins (id INTEGER PRIMARY KEY)")
        .await;
    fx.exec("INSERT INTO gap_random_bins VALUES (1), (2)").await;
    fx
}

fn wide_ids() -> Vec<i64> {
    vec![1, 4_294_967_297, i64::MAX - 1, i64::MAX]
}

async fn item_ids(query: Builder<QhItem>) -> Vec<i64> {
    query.get().await.unwrap().iter().map(|r| r.id).collect()
}

#[tokio::test]
async fn seeded_random_order_on_sqlite_is_the_same_whichever_plan_reads_the_rows() {
    let _fx = wide_key_fixture().await;
    for seed in [1, 42, u64::MAX] {
        let scanned = item_ids(QhItem::query().in_random_order_seeded(seed)).await;
        let indexed = item_ids(
            QhItem::query()
                .filter_op("a", ">", 0)
                .in_random_order_seeded(seed),
        )
        .await;
        assert_eq!(scanned, indexed, "seed {seed}");
        assert_eq!(
            scanned,
            item_ids(QhItem::query().in_random_order_seeded(seed)).await
        );
        let mut sorted = scanned;
        sorted.sort_unstable();
        assert_eq!(sorted, wide_ids(), "seed {seed}");
    }
    // Seed 42 gives ids 1 and 4294967297 one expression value; the key
    // orders the pair.
    let order = item_ids(
        QhItem::query()
            .filter_op("a", ">", 0)
            .in_random_order_seeded(42),
    )
    .await;
    let at = |id| order.iter().position(|&x| x == id).unwrap();
    assert!(at(1) < at(4_294_967_297), "{order:?}");
}

#[tokio::test]
async fn seeded_random_order_orders_a_union_and_its_pages_on_sqlite() {
    let _fx = wide_key_fixture().await;
    let plain = item_ids(QhItem::query().in_random_order_seeded(42)).await;
    let union = || {
        QhItem::query()
            .filter("b", 1)
            .union(QhItem::query().filter("b", 2))
            .in_random_order_seeded(42)
    };
    let first = item_ids(union()).await;
    assert_eq!(first, item_ids(union()).await);
    assert_eq!(first, plain, "a union orders by the same key as its rows");
    let page = union().paginate(3).await.unwrap();
    assert_eq!(page.total, 4);
    assert_eq!(
        page.data.iter().map(|r| r.id).collect::<Vec<_>>(),
        plain[..3]
    );
    let simple = union().simple_paginate(2).await.unwrap();
    assert_eq!(
        simple.data.iter().map(|r| r.id).collect::<Vec<_>>(),
        plain[..2]
    );
    assert!(simple.has_more);
    assert_eq!(item_ids(union().skip(2).take(2)).await, plain[2..]);
    // Set before `union`, the order and limit keep the first query to the
    // first of its own rows in the seeded order.
    let head = QhItem::query()
        .filter("b", 1)
        .in_random_order_seeded(42)
        .take(1)
        .union(QhItem::query().filter("b", 2));
    let mut ids = item_ids(head).await;
    ids.sort_unstable();
    assert_eq!(ids, vec![1, i64::MAX - 1, i64::MAX]);
    assert!(
        QhItem::query()
            .filter("b", 9)
            .union(QhItem::query().filter("b", 8))
            .in_random_order_seeded(42)
            .get()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn seeded_random_order_ranks_eager_children_per_parent_on_sqlite() {
    let _fx = wide_key_fixture().await;
    let plain = item_ids(QhItem::query().in_random_order_seeded(42)).await;
    let first_of = |bin: i64| -> i64 {
        let members: &[i64] = if bin == 1 {
            &[1, 4_294_967_297]
        } else {
            &[i64::MAX - 1, i64::MAX]
        };
        *plain.iter().find(|id| members.contains(id)).unwrap()
    };
    let loaded = |bins: Vec<GapRandomBin>| -> Vec<(i64, Vec<i64>)> {
        bins.iter()
            .map(|bin| {
                (
                    bin.id,
                    bin.items_loaded().iter().map(|r| r.id).collect::<Vec<_>>(),
                )
            })
            .collect()
    };
    let expected = vec![(1, vec![first_of(1)]), (2, vec![first_of(2)])];
    let bins = GapRandomBin::query()
        .order_by_asc("id")
        .with_where(("items", |q: Builder<QhItem>| {
            q.in_random_order_seeded(42).take(1)
        }))
        .get()
        .await
        .unwrap()
        .into_vec();
    assert_eq!(loaded(bins), expected);
    let bins = GapRandomBin::query()
        .order_by_asc("id")
        .with_where(("items", |q: Builder<QhItem>| {
            q.filter_op("a", ">", 2)
                .union(QhItem::query().filter_op("a", "<=", 2))
                .in_random_order_seeded(42)
                .take(1)
        }))
        .get()
        .await
        .unwrap()
        .into_vec();
    assert_eq!(loaded(bins), expected, "a window over a union");
}
