//! PAR-006: query helpers on the `DB::table` builder.
//!
//! `where_any` / `where_all` / `where_none` and their `or_` forms, the
//! `or_where_in` / `or_where_not_in` family with a list or a subquery,
//! `where_in` / `where_not_in` with a subquery, `or_where_raw`,
//! `or_where_null` / `or_where_not_null`, and `reorder`. Each query runs
//! against real rows and is compared with the same query in raw SQL.
//!
//! The SQLite tests run by default; the `postgres_` and `mysql_` tests run
//! every scenario against a disposable server on one connection with
//! temporary tables:
//!
//! ```text
//! PG_TEST_URL=postgres://... MYSQL_TEST_URL=mysql://... \
//!   cargo nextest run -p suprnova --test database -E 'test(/^query_helpers::/)' --run-ignored all
//! ```

use sea_orm::{DatabaseBackend, Value as SeaValue};
use suprnova::{DB, DbTableBuilder, Direction, DynamicRow};

use crate::query_fixture::{Fixture, json_rows};

/// `pq_items` carries three integer columns for the grouped helpers, a
/// nullable `label`, and two text columns for a search across columns.
/// `pq_slots` is the subquery source: each slot names an item and a day.
async fn seed(fx: &Fixture) {
    for sql in [
        "CREATE TEMPORARY TABLE pq_items (id INTEGER PRIMARY KEY, a INTEGER NOT NULL, \
         b INTEGER NOT NULL, c INTEGER NOT NULL, label VARCHAR(20) NULL, \
         code VARCHAR(20) NOT NULL, description VARCHAR(100) NOT NULL)",
        "CREATE TEMPORARY TABLE pq_slots (id INTEGER PRIMARY KEY, item_id INTEGER NOT NULL, \
         day VARCHAR(10) NOT NULL)",
        "INSERT INTO pq_items (id, a, b, c, label, code, description) VALUES \
         (1, 1, 2, 0, 'x', 'ab-1', 'alpha'), \
         (2, 1, 0, 2, NULL, 'cd-2', 'beta'), \
         (3, 1, 0, 0, 'y', 'ef-3', 'gamma ab'), \
         (4, 0, 2, 2, NULL, 'gh-4', 'delta'), \
         (5, 0, 0, 0, 'z', 'ij-5', 'epsilon')",
        "INSERT INTO pq_slots (id, item_id, day) VALUES (1, 2, 'mon'), (2, 4, 'mon'), \
         (3, 5, 'tue')",
    ] {
        fx.exec(sql).await;
    }
}

/// Run `query` ordered by id and the raw `WHERE` clause `raw_where`, check
/// they return the same rows, and return the ids.
async fn ids_matching(query: DbTableBuilder, raw_where: &str) -> Vec<i64> {
    let built = json_rows(
        query
            .order_by_asc("id")
            .get()
            .await
            .unwrap_or_else(|e| panic!("builder query for `{raw_where}` failed: {e}"))
            .into_vec(),
    );
    let sql = format!("SELECT * FROM pq_items WHERE {raw_where} ORDER BY id ASC");
    let raw = json_rows(
        DB::select(&sql, Vec::<SeaValue>::new())
            .await
            .unwrap_or_else(|e| panic!("raw query failed: {sql}: {e}")),
    );
    assert_eq!(built, raw, "the builder and `{raw_where}` disagree");
    built
        .iter()
        .map(|row| row["id"].as_i64().expect("integer id"))
        .collect()
}

fn items() -> DbTableBuilder {
    DB::table("pq_items")
}

fn slots_on(day: &str) -> DbTableBuilder {
    DB::table("pq_slots").select(["item_id"]).filter("day", day)
}

// ---------- Scenarios ---------------------------------------------------------

/// The falsifier: `where_any` keeps its `OR` inside parentheses, so
/// `a = 1 AND (b = 2 OR c = 2)` never returns row 4, whose `a` is 0.
async fn grouped_helpers_keep_their_or_inside_parentheses() {
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
            items().where_any(["b", "c"], "=", 2).filter("a", 1),
            "(b = 2 OR c = 2) AND a = 1"
        )
        .await,
        vec![1, 2],
        "the grouping holds whichever side the plain filter is on"
    );
    assert_eq!(
        ids_matching(
            items().filter("a", 0).where_all(["b", "c"], "=", 2),
            "a = 0 AND (b = 2 AND c = 2)"
        )
        .await,
        vec![4]
    );
    assert_eq!(
        ids_matching(
            items().filter("a", 1).where_none(["b", "c"], "=", 2),
            "a = 1 AND NOT (b = 2 OR c = 2)"
        )
        .await,
        vec![3]
    );
    assert_eq!(
        ids_matching(
            items().where_any(["code", "description"], "like", "%ab%"),
            "(code LIKE '%ab%' OR description LIKE '%ab%')"
        )
        .await,
        vec![1, 3],
        "one search term across several columns"
    );
}

async fn or_forms_of_the_grouped_helpers() {
    assert_eq!(
        ids_matching(
            items().filter("id", 5).or_where_any(["b", "c"], "=", 2),
            "id = 5 OR (b = 2 OR c = 2)"
        )
        .await,
        vec![1, 2, 4, 5]
    );
    assert_eq!(
        ids_matching(
            items().filter("id", 5).or_where_all(["b", "c"], "=", 2),
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
            items()
                .filter("label", "z")
                .or_where_any(["b", "c"], "=", 2)
                .filter("a", 1),
            "(label = 'z' OR (b = 2 OR c = 2)) AND a = 1"
        )
        .await,
        vec![1, 2],
        "an or_ helper folds into the condition before it and no further"
    );
}

async fn or_where_in_with_a_list() {
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
            items().filter("id", 1).or_where_not_in("id", [1i64, 2, 3, 4]),
            "id = 1 OR id NOT IN (1, 2, 3, 4)"
        )
        .await,
        vec![1, 5]
    );
    assert_eq!(
        ids_matching(items().where_in("id", [3i64, 5]), "id IN (3, 5)").await,
        vec![3, 5]
    );
    assert_eq!(
        ids_matching(items().where_in("id", Vec::<i64>::new()), "1 = 0").await,
        Vec::<i64>::new(),
        "an empty list matches nothing"
    );
    assert_eq!(
        ids_matching(items().where_not_in("id", Vec::<i64>::new()), "1 = 1").await,
        vec![1, 2, 3, 4, 5],
        "an empty exclusion list excludes nothing"
    );
}

/// The falsifier: a subquery used by `where_in` keeps its own bound value,
/// in its own position between the outer query's values.
async fn where_in_with_a_subquery() {
    assert_eq!(
        ids_matching(
            items()
                .filter("c", 2)
                .where_in("id", slots_on("mon"))
                .filter_op("b", ">=", 0),
            "c = 2 AND id IN (SELECT item_id FROM pq_slots WHERE day = 'mon') AND b >= 0"
        )
        .await,
        vec![2, 4]
    );
    assert_eq!(
        ids_matching(
            items().where_not_in("id", slots_on("mon")),
            "id NOT IN (SELECT item_id FROM pq_slots WHERE day = 'mon')"
        )
        .await,
        vec![1, 3, 5]
    );
    assert_eq!(
        ids_matching(
            items().filter("id", 1).or_where_in("id", slots_on("tue")),
            "id = 1 OR id IN (SELECT item_id FROM pq_slots WHERE day = 'tue')"
        )
        .await,
        vec![1, 5]
    );
    assert_eq!(
        ids_matching(
            items()
                .filter("id", 2)
                .or_where_not_in("id", DB::table("pq_slots").select(["item_id"])),
            "id = 2 OR id NOT IN (SELECT item_id FROM pq_slots)"
        )
        .await,
        vec![1, 2, 3]
    );
}

async fn raw_and_null_or_forms() {
    assert_eq!(
        ids_matching(
            items().filter("id", 1).or_where_raw("b + c = ?", vec![4.into()]),
            "id = 1 OR b + c = 4"
        )
        .await,
        vec![1, 4]
    );
    assert_eq!(
        ids_matching(
            items()
                .where_raw("a + b = ?", vec![2.into()])
                .or_where_raw("c = ? AND a = ?", vec![2.into(), 0.into()])
                .filter_op("id", "<", 5),
            "(a + b = 2 OR (c = 2 AND a = 0)) AND id < 5"
        )
        .await,
        vec![4],
        "raw fragments take their values in statement order"
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
            items().filter("id", 2).or_where_not_null("label"),
            "id = 2 OR label IS NOT NULL"
        )
        .await,
        vec![1, 2, 3, 5]
    );
    assert_eq!(
        ids_matching(items().where_null("label"), "label IS NULL").await,
        vec![2, 4]
    );
    assert_eq!(
        ids_matching(items().where_not_null("label"), "label IS NOT NULL").await,
        vec![1, 3, 5]
    );
}

async fn reorder_drops_earlier_orderings() {
    let ids = |rows: Vec<DynamicRow>| -> Vec<i64> {
        rows.iter()
            .map(|row| row.get_int("id").expect("integer id"))
            .collect()
    };

    let replaced = items()
        .order_by_desc("a")
        .order_by_desc("id")
        .reorder_by("id", Direction::Asc)
        .get()
        .await
        .expect("reorder_by runs")
        .into_vec();
    assert_eq!(ids(replaced), vec![1, 2, 3, 4, 5]);

    let dropped_then_added = items()
        .order_by_desc("id")
        .reorder()
        .order_by_asc("b")
        .order_by_asc("id")
        .get()
        .await
        .expect("reorder then order_by runs")
        .into_vec();
    assert_eq!(
        ids(dropped_then_added),
        vec![2, 3, 5, 1, 4],
        "the descending id ordering is gone"
    );

    let descending = items()
        .order_by_asc("id")
        .reorder_by("id", Direction::Desc)
        .get()
        .await
        .expect("reorder_by desc runs")
        .into_vec();
    assert_eq!(ids(descending), vec![5, 4, 3, 2, 1]);

    // The issue #128 shape: drop an inherited ordering before using the
    // query as a subquery.
    let base = DB::table("pq_slots")
        .select(["item_id"])
        .order_by_desc("id");
    assert_eq!(
        ids_matching(
            items().where_in("id", base.reorder()),
            "id IN (SELECT item_id FROM pq_slots)"
        )
        .await,
        vec![2, 4, 5]
    );
}

async fn run_every_scenario() {
    grouped_helpers_keep_their_or_inside_parentheses().await;
    or_forms_of_the_grouped_helpers().await;
    or_where_in_with_a_list().await;
    where_in_with_a_subquery().await;
    raw_and_null_or_forms().await;
    reorder_drops_earlier_orderings().await;
}

// ---------- SQLite ---------------------------------------------------------

async fn seeded_sqlite() -> Fixture {
    let fx = Fixture::sqlite().await;
    seed(&fx).await;
    fx
}

#[tokio::test]
async fn where_any_all_and_none_keep_their_or_inside_parentheses() {
    let _fx = seeded_sqlite().await;
    grouped_helpers_keep_their_or_inside_parentheses().await;
}

#[tokio::test]
async fn or_where_any_all_and_none_fold_into_the_previous_condition() {
    let _fx = seeded_sqlite().await;
    or_forms_of_the_grouped_helpers().await;
}

#[tokio::test]
async fn or_where_in_and_or_where_not_in_take_a_list() {
    let _fx = seeded_sqlite().await;
    or_where_in_with_a_list().await;
}

#[tokio::test]
async fn where_in_family_takes_a_subquery_with_its_own_values() {
    let _fx = seeded_sqlite().await;
    where_in_with_a_subquery().await;
}

#[tokio::test]
async fn or_where_raw_null_and_not_null_match_raw_sql() {
    let _fx = seeded_sqlite().await;
    raw_and_null_or_forms().await;
}

#[tokio::test]
async fn reorder_drops_orderings_and_reorder_by_sets_a_new_one() {
    let _fx = seeded_sqlite().await;
    reorder_drops_earlier_orderings().await;
}

#[tokio::test]
async fn an_empty_column_list_adds_no_condition() {
    let _fx = seeded_sqlite().await;
    assert_eq!(
        ids_matching(
            items()
                .filter("a", 1)
                .where_any(Vec::<&str>::new(), "=", 2)
                .or_where_none(Vec::<&str>::new(), "=", 2),
            "a = 1"
        )
        .await,
        vec![1, 2, 3],
        "Laravel adds no clause for an empty column list"
    );
}

#[tokio::test]
async fn invalid_input_to_a_helper_fails_before_any_sql_runs() {
    let _fx = seeded_sqlite().await;
    let bad_operator = items().where_any(["b", "c"], "= 2 OR 1 =", 2).get().await;
    assert!(bad_operator.is_err(), "an operator outside the allowlist");

    let bad_column = items()
        .where_all(["b", "c; DROP TABLE pq_items"], "=", 2)
        .get()
        .await;
    assert!(bad_column.is_err(), "an injected column name");

    let bad_subquery = items()
        .where_in("id", DB::table("pq_slots").select(["item_id; --"]))
        .get()
        .await;
    assert!(bad_subquery.is_err(), "the subquery is validated too");

    let bad_raw = items()
        .or_where_raw("b = ? AND c = ?", vec![1.into()])
        .get()
        .await;
    assert!(bad_raw.is_err(), "placeholders and bindings must agree");

    assert_eq!(items().count().await.expect("count"), 5, "nothing ran");
}

// ---------- Live engines ----------------------------------------------------

#[tokio::test]
#[ignore = "requires disposable PostgreSQL at PG_TEST_URL"]
async fn postgres_every_query_helper_matches_raw_sql() {
    let fx = Fixture::live("PG_TEST_URL", DatabaseBackend::Postgres).await;
    seed(&fx).await;
    run_every_scenario().await;
    fx.close().await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_every_query_helper_matches_raw_sql() {
    let fx = Fixture::live("MYSQL_TEST_URL", DatabaseBackend::MySql).await;
    seed(&fx).await;
    run_every_scenario().await;
    fx.close().await;
}
