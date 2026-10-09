//! Model query contracts for missing rows, expressions and ordering.

use crate::query_fixture::Fixture;
use crate::query_helpers_model::QhItem;
use chrono::{DateTime, Utc};
use sea_orm::DbBackend;
use serde_json::json;
use suprnova::{DB, Model, UpdateAttrs, model};

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
