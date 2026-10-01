//! PAR-006: query helpers on the model query builder.
//!
//! The same helpers as the `DB::table` builder, under both of the model
//! builder's names (`where_*` and `filter_*`), checked against raw SQL over
//! real rows. The model's soft-delete filter still applies, so a grouped
//! helper must not let an `or` escape past it either.
//!
//! The SQLite tests run by default; the `postgres_` and `mysql_` tests run
//! every scenario against a disposable server:
//!
//! ```text
//! PG_TEST_URL=postgres://... MYSQL_TEST_URL=mysql://... \
//!   cargo nextest run -p suprnova --test eloquent -E 'test(/^query_helpers_model::/)' --run-ignored all
//! ```

use sea_orm::{DatabaseBackend, Value as SeaValue};
use serde_json::json;
use suprnova::{Builder, DB, DbTableBuilder, Direction, Model, model};

use crate::query_fixture::{Fixture, json_rows};

#[model(table = "qh_items", soft_deletes)]
pub struct QhItem {
    pub id: i64,
    pub a: i64,
    pub b: i64,
    pub c: i64,
    pub label: Option<String>,
    pub code: String,
    pub description: String,
    pub deleted_at: Option<String>,
}

/// Rows 1 to 5 mirror the `DB::table` helper tests. Row 6 is trashed and
/// matches every grouped condition, so a helper that let an `or` escape
/// past the soft-delete filter would return it.
async fn seed(fx: &Fixture) {
    for sql in [
        "CREATE TEMPORARY TABLE qh_items (id BIGINT PRIMARY KEY, a BIGINT NOT NULL, \
         b BIGINT NOT NULL, c BIGINT NOT NULL, label VARCHAR(20) NULL, \
         code VARCHAR(20) NOT NULL, description VARCHAR(100) NOT NULL, \
         deleted_at VARCHAR(30) NULL)",
        "CREATE TEMPORARY TABLE qh_slots (id BIGINT PRIMARY KEY, item_id BIGINT NOT NULL, \
         day VARCHAR(10) NOT NULL)",
        "INSERT INTO qh_items (id, a, b, c, label, code, description, deleted_at) VALUES \
         (1, 1, 2, 0, 'x', 'ab-1', 'alpha', NULL), \
         (2, 1, 0, 2, NULL, 'cd-2', 'beta', NULL), \
         (3, 1, 0, 0, 'y', 'ef-3', 'gamma ab', NULL), \
         (4, 0, 2, 2, NULL, 'gh-4', 'delta', NULL), \
         (5, 0, 0, 0, 'z', 'ij-5', 'epsilon', NULL), \
         (6, 1, 2, 2, NULL, 'ab-6', 'trashed', '2026-01-01 00:00:00')",
        "INSERT INTO qh_slots (id, item_id, day) VALUES (1, 2, 'mon'), (2, 4, 'mon'), \
         (3, 5, 'tue'), (4, 6, 'mon')",
    ] {
        fx.exec(sql).await;
    }
}

/// Run `query` ordered by id, check it matches the raw `WHERE` clause
/// `raw_where` over the rows that are not trashed, and return the ids.
async fn ids_matching(query: Builder<QhItem>, raw_where: &str) -> Vec<i64> {
    let ids: Vec<i64> = query
        .order_by_asc("id")
        .get()
        .await
        .unwrap_or_else(|e| panic!("model query for `{raw_where}` failed: {e}"))
        .iter()
        .map(|item| item.id)
        .collect();
    let sql =
        format!("SELECT id FROM qh_items WHERE deleted_at IS NULL AND ({raw_where}) ORDER BY id ASC");
    let raw: Vec<i64> = json_rows(
        DB::select(&sql, Vec::<SeaValue>::new())
            .await
            .unwrap_or_else(|e| panic!("raw query failed: {sql}: {e}")),
    )
    .iter()
    .map(|row| row["id"].as_i64().expect("integer id"))
    .collect();
    assert_eq!(ids, raw, "the model builder and `{raw_where}` disagree");
    ids
}

fn items() -> Builder<QhItem> {
    QhItem::query()
}

fn slots_on(day: &str) -> DbTableBuilder {
    DB::table("qh_slots").select(["item_id"]).filter("day", day)
}

// ---------- Scenarios ---------------------------------------------------------

/// The falsifier: `where("a", 1).where_any(["b", "c"], "=", 2)` never
/// returns a row whose `a` is not 1, nor the trashed row.
async fn grouped_helpers_stay_grouped() {
    assert_eq!(
        ids_matching(
            items().filter("a", 1).where_any(["b", "c"], "=", 2),
            "a = 1 AND (b = 2 OR c = 2)"
        )
        .await,
        vec![1, 2]
    );
    assert_eq!(
        ids_matching(
            items().filter("a", 1).filter_any(["b", "c"], "=", 2),
            "a = 1 AND (b = 2 OR c = 2)"
        )
        .await,
        vec![1, 2],
        "filter_any is the same method"
    );
    assert_eq!(
        ids_matching(
            items().where_all(["b", "c"], "=", 2),
            "b = 2 AND c = 2"
        )
        .await,
        vec![4]
    );
    assert_eq!(
        ids_matching(
            items().filter("a", 1).filter_none(["b", "c"], "=", 2),
            "a = 1 AND NOT (b = 2 OR c = 2)"
        )
        .await,
        vec![3]
    );
    assert_eq!(
        ids_matching(
            items().where_any(["code", "description"], "like", "%ab%"),
            "code LIKE '%ab%' OR description LIKE '%ab%'"
        )
        .await,
        vec![1, 3]
    );
}

async fn or_forms() {
    assert_eq!(
        ids_matching(
            items().filter("id", 5).or_where_any(["b", "c"], "=", 2),
            "id = 5 OR b = 2 OR c = 2"
        )
        .await,
        vec![1, 2, 4, 5]
    );
    assert_eq!(
        ids_matching(
            items().filter("id", 5).or_filter_all(["b", "c"], "=", 2),
            "id = 5 OR (b = 2 AND c = 2)"
        )
        .await,
        vec![4, 5]
    );
    assert_eq!(
        ids_matching(
            items().filter("id", 1).or_where_none(["b", "c"], "=", 2),
            "id = 1 OR NOT (b = 2 OR c = 2)"
        )
        .await,
        vec![1, 3, 5]
    );
    assert_eq!(
        ids_matching(
            items().filter("id", 1).or_where_all(["b", "c"], "=", 2),
            "id = 1 OR (b = 2 AND c = 2)"
        )
        .await,
        vec![1, 4]
    );
    assert_eq!(
        ids_matching(
            items().filter("id", 1).or_filter_any(["b", "c"], "=", 2),
            "id = 1 OR b = 2 OR c = 2"
        )
        .await,
        vec![1, 2, 4]
    );
    assert_eq!(
        ids_matching(
            items().filter("id", 2).or_filter_none(["b", "c"], "=", 2),
            "id = 2 OR NOT (b = 2 OR c = 2)"
        )
        .await,
        vec![2, 3, 5]
    );
    assert_eq!(
        ids_matching(
            items().filter("a", 0).or_where_in("id", [1i64, 2]),
            "a = 0 OR id IN (1, 2)"
        )
        .await,
        vec![1, 2, 4, 5]
    );
    assert_eq!(
        ids_matching(
            items().filter("id", 1).or_filter_not_in("id", [1i64, 2, 3, 4]),
            "id = 1 OR id NOT IN (1, 2, 3, 4)"
        )
        .await,
        vec![1, 5]
    );
    assert_eq!(
        ids_matching(
            items().filter("id", 1).or_where_raw("b + c = ?", vec![json!(4)]),
            "id = 1 OR b + c = 4"
        )
        .await,
        vec![1, 4]
    );
    assert_eq!(
        ids_matching(
            items().filter("id", 1).or_filter_raw("a = ?", vec![json!(0)]),
            "id = 1 OR a = 0"
        )
        .await,
        vec![1, 4, 5]
    );
    assert_eq!(
        ids_matching(
            items().filter("id", 1).or_where_null("label"),
            "id = 1 OR label IS NULL"
        )
        .await,
        vec![1, 2, 4]
    );
    assert_eq!(
        ids_matching(
            items().filter("id", 2).or_filter_not_null("label"),
            "id = 2 OR label IS NOT NULL"
        )
        .await,
        vec![1, 2, 3, 5]
    );
    assert_eq!(
        ids_matching(
            items().filter("id", 3).or_filter_null("label"),
            "id = 3 OR label IS NULL"
        )
        .await,
        vec![2, 3, 4]
    );
    assert_eq!(
        ids_matching(
            items().filter("id", 4).or_where_not_null("label"),
            "id = 4 OR label IS NOT NULL"
        )
        .await,
        vec![1, 3, 4, 5]
    );
}

/// The falsifier: a subquery used by `where_in` keeps its own bound value.
async fn subqueries_in_the_where_in_family() {
    assert_eq!(
        ids_matching(
            items()
                .filter("c", 2)
                .where_in("id", slots_on("mon"))
                .filter_op("b", ">=", 0),
            "c = 2 AND id IN (SELECT item_id FROM qh_slots WHERE day = 'mon') AND b >= 0"
        )
        .await,
        vec![2, 4]
    );
    assert_eq!(
        ids_matching(
            items().filter_in("id", slots_on("tue")),
            "id IN (SELECT item_id FROM qh_slots WHERE day = 'tue')"
        )
        .await,
        vec![5]
    );
    assert_eq!(
        ids_matching(
            items().where_not_in("id", slots_on("mon")),
            "id NOT IN (SELECT item_id FROM qh_slots WHERE day = 'mon')"
        )
        .await,
        vec![1, 3, 5]
    );
    assert_eq!(
        ids_matching(
            items().filter("id", 1).or_where_in("id", slots_on("tue")),
            "id = 1 OR id IN (SELECT item_id FROM qh_slots WHERE day = 'tue')"
        )
        .await,
        vec![1, 5]
    );
    assert_eq!(
        ids_matching(
            items()
                .filter("id", 2)
                .or_where_not_in("id", DB::table("qh_slots").select(["item_id"])),
            "id = 2 OR id NOT IN (SELECT item_id FROM qh_slots)"
        )
        .await,
        vec![1, 2, 3]
    );

    let (sql, bindings) = items()
        .filter("c", 2)
        .where_in("id", slots_on("mon"))
        .filter_op("b", ">=", 0)
        .try_to_sql_with_bindings_for(DatabaseBackend::Postgres)
        .expect("renders for Postgres");
    assert!(!sql.contains("mon"), "the subquery value is bound: {sql}");
    assert_eq!(bindings.len(), 3, "{sql}");
    assert!(
        matches!(&bindings[1], SeaValue::String(Some(day)) if day.as_str() == "mon"),
        "the subquery's value sits between the outer ones: {bindings:?}"
    );
    let first = sql.find("$1").expect("$1");
    let second = sql.find("$2").expect("$2");
    let third = sql.find("$3").expect("$3");
    assert!(first < second && second < third, "{sql}");
}

async fn reorder_drops_orderings() {
    let ids = |items: &[QhItem]| -> Vec<i64> { items.iter().map(|item| item.id).collect() };

    let replaced = items()
        .order_by_desc("a")
        .order_by_desc("id")
        .reorder_by("id", Direction::Asc)
        .get()
        .await
        .expect("reorder_by runs");
    assert_eq!(ids(&replaced), vec![1, 2, 3, 4, 5]);

    let dropped = items()
        .latest_by("id")
        .reorder()
        .order_by_asc("b")
        .order_by_asc("id")
        .get()
        .await
        .expect("reorder then order_by runs");
    assert_eq!(ids(&dropped), vec![2, 3, 5, 1, 4]);

    let (sql, _) = items()
        .order_by_desc("id")
        .reorder()
        .try_to_sql_with_bindings_for(DatabaseBackend::Sqlite)
        .expect("renders");
    assert!(!sql.contains("ORDER BY"), "reorder() leaves no ordering: {sql}");
}

async fn run_every_scenario() {
    grouped_helpers_stay_grouped().await;
    or_forms().await;
    subqueries_in_the_where_in_family().await;
    reorder_drops_orderings().await;
}

// ---------- SQLite ---------------------------------------------------------

async fn seeded_sqlite() -> Fixture {
    let fx = Fixture::sqlite().await;
    seed(&fx).await;
    fx
}

#[tokio::test]
async fn model_where_any_all_and_none_stay_inside_their_parentheses() {
    let _fx = seeded_sqlite().await;
    grouped_helpers_stay_grouped().await;
}

#[tokio::test]
async fn model_or_helpers_match_raw_sql() {
    let _fx = seeded_sqlite().await;
    or_forms().await;
}

#[tokio::test]
async fn model_where_in_family_takes_a_subquery_with_its_own_values() {
    let _fx = seeded_sqlite().await;
    subqueries_in_the_where_in_family().await;
    // Two subqueries over one table, which MySQL cannot do with the live
    // tests' temporary tables, so this one runs on SQLite only.
    assert_eq!(
        ids_matching(
            items()
                .filter_not_in("id", slots_on("mon"))
                .or_filter_in("id", slots_on("tue")),
            "id NOT IN (SELECT item_id FROM qh_slots WHERE day = 'mon') \
             OR id IN (SELECT item_id FROM qh_slots WHERE day = 'tue')"
        )
        .await,
        vec![1, 3, 5]
    );
}

#[tokio::test]
async fn model_reorder_drops_orderings_and_reorder_by_sets_one() {
    let _fx = seeded_sqlite().await;
    reorder_drops_orderings().await;
}

#[tokio::test]
async fn model_helpers_refuse_invalid_input_before_any_sql_runs() {
    let _fx = seeded_sqlite().await;
    assert!(
        items().where_any(["b", "c"], "= 2 OR 1 =", 2).get().await.is_err(),
        "an operator outside the allowlist"
    );
    assert!(
        items().where_none(["b; --"], "=", 2).get().await.is_err(),
        "an injected column"
    );
    assert!(
        items()
            .where_in("id", DB::table("qh_slots").select(["item_id) OR (1"]))
            .get()
            .await
            .is_err(),
        "the subquery is validated too"
    );
    assert!(
        items()
            .or_where_raw("a = ? AND b = ?", vec![json!(1)])
            .get()
            .await
            .is_err(),
        "placeholders and bindings must agree"
    );
}

#[tokio::test]
async fn model_empty_column_list_adds_no_condition() {
    let _fx = seeded_sqlite().await;
    assert_eq!(
        ids_matching(
            items()
                .filter("a", 1)
                .where_any(Vec::<&str>::new(), "=", 2)
                .or_where_all(Vec::<&str>::new(), "=", 2),
            "a = 1"
        )
        .await,
        vec![1, 2, 3]
    );
}

// ---------- Live engines ----------------------------------------------------

#[tokio::test]
#[ignore = "requires disposable PostgreSQL at PG_TEST_URL"]
async fn postgres_model_query_helpers_match_raw_sql() {
    let fx = Fixture::live("PG_TEST_URL", DatabaseBackend::Postgres).await;
    seed(&fx).await;
    run_every_scenario().await;
    fx.close().await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_model_query_helpers_match_raw_sql() {
    let fx = Fixture::live("MYSQL_TEST_URL", DatabaseBackend::MySql).await;
    seed(&fx).await;
    run_every_scenario().await;
    fx.close().await;
}
