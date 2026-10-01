//! PAR-001: joins on the `DB::table` builder.
//!
//! Every query shape here runs against real tables and is compared with
//! the same query written as raw SQL through `DB::select`, so a join that
//! renders, binds or quotes wrongly shows up as different rows rather than
//! as a string mismatch. The three shapes from issue #125 come first.
//!
//! The SQLite tests run by default. The `postgres_` and `mysql_` tests run
//! every scenario against a disposable server:
//!
//! ```text
//! PG_TEST_URL=postgres://... MYSQL_TEST_URL=mysql://... \
//!   cargo nextest run -p suprnova --test database -E 'test(/^joins::/)' --run-ignored all
//! ```
//!
//! The live tests use temporary tables on a single connection, so they
//! leave nothing behind. MySQL cannot open one temporary table twice in a
//! statement, so no query here names the same table twice.

use sea_orm::{DatabaseBackend, Value as SeaValue};
use serde_json::{Value, json};
use serial_test::serial;
use suprnova::DB;
use suprnova::render_cache::DependencyIdentity;
use suprnova::render_cache::collector::{Collector, begin_handler, current_report};

use crate::query_fixture::{Fixture, json_rows};

async fn seed(fx: &Fixture) {
    for sql in [
        "CREATE TEMPORARY TABLE pj_categories (id INTEGER PRIMARY KEY, name VARCHAR(100) NOT NULL)",
        "CREATE TEMPORARY TABLE pj_users (id INTEGER PRIMARY KEY, name VARCHAR(100) NOT NULL, \
         active INTEGER NOT NULL)",
        "CREATE TEMPORARY TABLE pj_posts (id INTEGER PRIMARY KEY, title VARCHAR(100) NOT NULL, \
         category_id INTEGER NULL, author_id INTEGER NULL, views INTEGER NOT NULL)",
        "CREATE TEMPORARY TABLE pj_statuses (id INTEGER PRIMARY KEY, name VARCHAR(100) NOT NULL)",
        "CREATE TEMPORARY TABLE pj_orders (id INTEGER PRIMARY KEY, status_id INTEGER NOT NULL, \
         total INTEGER NOT NULL)",
        "INSERT INTO pj_categories (id, name) VALUES (1, 'News'), (2, 'Rust'), (3, 'Empty')",
        "INSERT INTO pj_users (id, name, active) VALUES (1, 'Ada', 1), (2, 'Linus', 1), \
         (3, 'Grace', 0)",
        "INSERT INTO pj_posts (id, title, category_id, author_id, views) VALUES \
         (1, 'Alpha', 1, 2, 10), (2, 'Beta', 2, 2, 50), (3, 'Gamma', NULL, 1, 5), \
         (4, 'Delta', 2, NULL, 70)",
        "INSERT INTO pj_statuses (id, name) VALUES (1, 'open'), (2, 'paid'), (3, 'void')",
        "INSERT INTO pj_orders (id, status_id, total) VALUES (1, 1, 10), (2, 1, 20), (3, 2, 30)",
    ] {
        fx.exec(sql).await;
    }
}

async fn raw(sql: &str) -> Vec<Value> {
    json_rows(
        DB::select(sql, Vec::<SeaValue>::new())
            .await
            .unwrap_or_else(|e| panic!("raw comparison query failed: {sql}: {e}")),
    )
}

fn column(rows: &[Value], name: &str) -> Vec<Value> {
    rows.iter()
        .map(|row| row.get(name).cloned().unwrap_or(Value::Null))
        .collect()
}

// ---------- Scenarios (shared by every backend) ----------------------------

/// Issue #125, shape 1: chained left joins, a table alias, aliased columns.
async fn chained_left_joins_with_alias(_fx: &Fixture) {
    let built = DB::table("pj_posts")
        .left_join(
            "pj_categories",
            "pj_categories.id",
            "=",
            "pj_posts.category_id",
        )
        .left_join(
            "pj_users as authors",
            "authors.id",
            "=",
            "pj_posts.author_id",
        )
        .select([
            "pj_posts.title",
            "pj_categories.name as category_name",
            "authors.name as author_name",
        ])
        .order_by_asc("pj_posts.title")
        .get()
        .await
        .expect("chained left joins run")
        .into_vec();
    let expected = raw(
        "SELECT pj_posts.title, pj_categories.name AS category_name, authors.name AS author_name \
         FROM pj_posts \
         LEFT JOIN pj_categories ON pj_categories.id = pj_posts.category_id \
         LEFT JOIN pj_users AS authors ON authors.id = pj_posts.author_id \
         ORDER BY pj_posts.title ASC",
    )
    .await;

    let built = json_rows(built);
    assert_eq!(built, expected, "the builder and raw SQL disagree");
    assert_eq!(
        built,
        vec![
            json!({"title": "Alpha", "category_name": "News", "author_name": "Linus"}),
            json!({"title": "Beta", "category_name": "Rust", "author_name": "Linus"}),
            json!({"title": "Delta", "category_name": "Rust", "author_name": null}),
            json!({"title": "Gamma", "category_name": null, "author_name": "Ada"}),
        ]
    );
}

/// Issue #125, shape 2: a left join against a grouped subquery.
async fn left_join_against_a_grouped_subquery(fx: &Fixture) {
    let totals = DB::table("pj_orders")
        .select(["status_id"])
        .select_raw("COUNT(*) AS total")
        .group_by("status_id");
    let built = DB::table("pj_statuses")
        .left_join_sub(
            totals,
            "order_totals",
            "order_totals.status_id",
            "=",
            "pj_statuses.id",
        )
        .select(["pj_statuses.name"])
        .select_raw("COALESCE(order_totals.total, 0) AS total")
        .order_by_asc("pj_statuses.id")
        .get()
        .await
        .expect("left join against a subquery runs")
        .into_vec();
    let expected = raw(
        "SELECT pj_statuses.name, COALESCE(order_totals.total, 0) AS total \
         FROM pj_statuses \
         LEFT JOIN (SELECT status_id, COUNT(*) AS total FROM pj_orders GROUP BY status_id) \
         AS order_totals ON order_totals.status_id = pj_statuses.id \
         ORDER BY pj_statuses.id ASC",
    )
    .await;

    let built = json_rows(built);
    assert_eq!(built, expected, "the builder and raw SQL disagree");
    assert_eq!(
        column(&built, "name"),
        vec![json!("open"), json!("paid"), json!("void")]
    );
    // SQLite reports no column type for an aggregate, and the row
    // materialiser drops such a column for raw SQL and the builder alike
    // (see "Aggregate-column gotcha" in manual/queries.md). The totals are
    // checked through `count()` below, which reads them typed.
    if fx.backend != DatabaseBackend::Sqlite {
        assert_eq!(column(&built, "total"), vec![json!(2), json!(1), json!(0)]);
    }
    for (status, total) in [("open", 2), ("paid", 1)] {
        let matching = DB::table("pj_statuses")
            .left_join_sub(
                DB::table("pj_orders")
                    .select(["status_id"])
                    .select_raw("COUNT(*) AS total")
                    .group_by("status_id"),
                "order_totals",
                "order_totals.status_id",
                "=",
                "pj_statuses.id",
            )
            .filter("pj_statuses.name", status)
            .filter("order_totals.total", total)
            .count()
            .await
            .expect("count over the subquery join");
        assert_eq!(matching, 1, "{status} has {total} orders");
    }
    let without_orders = DB::table("pj_statuses")
        .left_join_sub(
            DB::table("pj_orders")
                .select(["status_id"])
                .group_by("status_id"),
            "order_totals",
            "order_totals.status_id",
            "=",
            "pj_statuses.id",
        )
        .where_null("order_totals.status_id")
        .count()
        .await
        .expect("count of statuses without orders");
    assert_eq!(without_orders, 1, "only void has no orders");
}

/// Issue #125, shape 3: a correlated `EXISTS`, and its `NOT EXISTS` twin.
async fn correlated_exists(_fx: &Fixture) {
    let has_posts = DB::table("pj_users")
        .where_exists(
            DB::table("pj_posts")
                .select_raw("1")
                .where_column("pj_posts.author_id", "pj_users.id"),
        )
        .order_by_asc("pj_users.id")
        .get()
        .await
        .expect("where_exists runs")
        .into_vec();
    let expected = raw("SELECT * FROM pj_users \
         WHERE EXISTS (SELECT 1 FROM pj_posts WHERE pj_posts.author_id = pj_users.id) \
         ORDER BY pj_users.id ASC")
    .await;
    let has_posts = json_rows(has_posts);
    assert_eq!(has_posts, expected, "the builder and raw SQL disagree");
    assert_eq!(
        column(&has_posts, "name"),
        vec![json!("Ada"), json!("Linus")]
    );

    let without_posts = DB::table("pj_users")
        .where_not_exists(
            DB::table("pj_posts")
                .select_raw("1")
                .where_column("pj_posts.author_id", "pj_users.id"),
        )
        .order_by_asc("pj_users.id")
        .get()
        .await
        .expect("where_not_exists runs")
        .into_vec();
    let expected = raw("SELECT * FROM pj_users \
         WHERE NOT EXISTS (SELECT 1 FROM pj_posts WHERE pj_posts.author_id = pj_users.id) \
         ORDER BY pj_users.id ASC")
    .await;
    let without_posts = json_rows(without_posts);
    assert_eq!(without_posts, expected, "the builder and raw SQL disagree");
    assert_eq!(column(&without_posts, "name"), vec![json!("Grace")]);
}

/// The closure form combines `on`, `or_on`, `where` and `or_where`. An
/// `or_*` condition folds into the condition before it, so it widens that
/// one condition and never the whole `ON` clause.
async fn join_closure_combines_on_and_where(_fx: &Fixture) {
    let built = DB::table("pj_posts")
        .join_with("pj_users", |join| {
            join.on("pj_users.id", "=", "pj_posts.author_id")
                .db_where_op("pj_posts.views", ">", 40)
                .or_where("pj_users.name", "Ada")
        })
        .select(["pj_posts.title", "pj_users.name"])
        .order_by_asc("pj_posts.id")
        .get()
        .await
        .expect("closure join runs")
        .into_vec();
    let expected = raw("SELECT pj_posts.title, pj_users.name FROM pj_posts \
         INNER JOIN pj_users ON pj_users.id = pj_posts.author_id \
         AND (pj_posts.views > 40 OR pj_users.name = 'Ada') \
         ORDER BY pj_posts.id ASC")
    .await;
    let built = json_rows(built);
    assert_eq!(built, expected, "the builder and raw SQL disagree");
    assert_eq!(column(&built, "title"), vec![json!("Beta"), json!("Gamma")]);

    let either_key = DB::table("pj_posts")
        .join_with("pj_categories", |join| {
            join.on("pj_categories.id", "=", "pj_posts.category_id")
                .or_on("pj_categories.id", "=", "pj_posts.author_id")
        })
        .select(["pj_posts.title", "pj_categories.name"])
        .order_by_asc("pj_posts.id")
        .order_by_asc("pj_categories.id")
        .get()
        .await
        .expect("or_on join runs")
        .into_vec();
    let expected = raw("SELECT pj_posts.title, pj_categories.name FROM pj_posts \
         INNER JOIN pj_categories ON (pj_categories.id = pj_posts.category_id \
         OR pj_categories.id = pj_posts.author_id) \
         ORDER BY pj_posts.id ASC, pj_categories.id ASC")
    .await;
    let either_key = json_rows(either_key);
    assert_eq!(either_key, expected, "the builder and raw SQL disagree");
    assert_eq!(
        column(&either_key, "title"),
        vec![
            json!("Alpha"),
            json!("Alpha"),
            json!("Beta"),
            json!("Gamma"),
            json!("Delta")
        ]
    );
}

/// Inner, right and cross joins, and an inner join against a subquery.
async fn join_kinds(_fx: &Fixture) {
    let inner = DB::table("pj_posts")
        .join("pj_users as u", "u.id", "=", "pj_posts.author_id")
        .select(["pj_posts.title", "u.name as author"])
        .order_by_asc("pj_posts.id")
        .get()
        .await
        .expect("inner join runs")
        .into_vec();
    let expected = raw("SELECT pj_posts.title, u.name AS author FROM pj_posts \
         INNER JOIN pj_users AS u ON u.id = pj_posts.author_id ORDER BY pj_posts.id ASC")
    .await;
    let inner = json_rows(inner);
    assert_eq!(inner, expected);
    assert_eq!(
        column(&inner, "title"),
        vec![json!("Alpha"), json!("Beta"), json!("Gamma")]
    );

    let right = DB::table("pj_posts")
        .right_join(
            "pj_categories",
            "pj_categories.id",
            "=",
            "pj_posts.category_id",
        )
        .select(["pj_categories.name", "pj_posts.title"])
        .order_by_asc("pj_categories.id")
        .order_by_asc("pj_posts.id")
        .get()
        .await
        .expect("right join runs")
        .into_vec();
    let expected = raw("SELECT pj_categories.name, pj_posts.title FROM pj_posts \
         RIGHT JOIN pj_categories ON pj_categories.id = pj_posts.category_id \
         ORDER BY pj_categories.id ASC, pj_posts.id ASC")
    .await;
    let right = json_rows(right);
    assert_eq!(right, expected);
    assert_eq!(
        right.last(),
        Some(&json!({"name": "Empty", "title": null})),
        "the category without posts survives a right join"
    );

    let cross = DB::table("pj_statuses")
        .cross_join("pj_categories")
        .select([
            "pj_statuses.name as status",
            "pj_categories.name as category",
        ])
        .order_by_asc("pj_statuses.id")
        .order_by_asc("pj_categories.id")
        .get()
        .await
        .expect("cross join runs")
        .into_vec();
    let expected = raw(
        "SELECT pj_statuses.name AS status, pj_categories.name AS category \
         FROM pj_statuses CROSS JOIN pj_categories \
         ORDER BY pj_statuses.id ASC, pj_categories.id ASC",
    )
    .await;
    let cross = json_rows(cross);
    assert_eq!(cross, expected);
    assert_eq!(cross.len(), 9);

    let busy_authors = DB::table("pj_users")
        .join_sub(
            DB::table("pj_posts")
                .select(["author_id"])
                .select_raw("COUNT(*) AS post_count")
                .group_by("author_id"),
            "counts",
            "counts.author_id",
            "=",
            "pj_users.id",
        )
        .filter_op("counts.post_count", ">=", 2)
        .select(["pj_users.name"])
        .order_by_asc("pj_users.id")
        .get()
        .await
        .expect("join_sub runs")
        .into_vec();
    let expected = raw("SELECT pj_users.name FROM pj_users \
         INNER JOIN (SELECT author_id, COUNT(*) AS post_count FROM pj_posts GROUP BY author_id) \
         AS counts ON counts.author_id = pj_users.id WHERE counts.post_count >= 2 \
         ORDER BY pj_users.id ASC")
    .await;
    let busy_authors = json_rows(busy_authors);
    assert_eq!(busy_authors, expected);
    assert_eq!(busy_authors, vec![json!({"name": "Linus"})]);
}

/// Values in a subquery join, a join condition, an `EXISTS` subquery and
/// the outer `WHERE` each bind in the position their placeholder takes in
/// the statement. A swap on SQLite or MySQL compares a number with a
/// string; on Postgres it reads the wrong `$N`. Either returns other rows.
///
/// Each of the four values decides the result on its own: with any one
/// of them read from another slot, the answer is not `["paid"]`.
async fn bound_values_keep_their_positions(_fx: &Fixture) {
    let large_orders = DB::table("pj_orders")
        .select(["status_id"])
        .select_raw("COUNT(*) AS large")
        .filter_op("total", ">=", 15)
        .group_by("status_id");
    let built = DB::table("pj_statuses")
        .left_join_sub_with(large_orders, "big", |join| {
            join.on("big.status_id", "=", "pj_statuses.id")
                .filter_op("big.large", ">=", 1)
        })
        .where_exists(
            DB::table("pj_categories")
                .select_raw("1")
                .filter("pj_categories.name", "Rust"),
        )
        .filter_op("pj_statuses.name", "<>", "open")
        .where_not_null("big.status_id")
        .select(["pj_statuses.name"])
        .order_by_asc("pj_statuses.id")
        .get()
        .await
        .expect("a statement binding in four places runs")
        .into_vec();
    let expected = raw("SELECT pj_statuses.name FROM pj_statuses \
         LEFT JOIN (SELECT status_id, COUNT(*) AS large FROM pj_orders WHERE total >= 15 \
         GROUP BY status_id) AS big ON big.status_id = pj_statuses.id AND big.large >= 1 \
         WHERE EXISTS (SELECT 1 FROM pj_categories WHERE pj_categories.name = 'Rust') \
         AND pj_statuses.name <> 'open' AND big.status_id IS NOT NULL \
         ORDER BY pj_statuses.id ASC")
    .await;
    let built = json_rows(built);
    assert_eq!(built, expected, "the builder and raw SQL disagree");
    assert_eq!(built, vec![json!({"name": "paid"})]);
}

/// The falsifier: a value given to a join's `where`, to a joined
/// subquery or to an `EXISTS` subquery never appears in the SQL text. It
/// travels as a bound parameter, in statement order, and every table,
/// alias and column in the join is quoted for the backend.
async fn join_values_never_reach_the_sql_text(fx: &Fixture) {
    DB::enable_query_log().expect("enable the query log");
    DB::flush_query_log().expect("flush the query log");
    let rows = DB::table("pj_posts")
        .join_with("pj_users", |join| {
            join.on("pj_users.id", "=", "pj_posts.author_id")
                .filter("pj_users.name", "Linus")
        })
        .left_join_sub(
            DB::table("pj_categories").filter_op("name", "<>", "Empty"),
            "cats",
            "cats.id",
            "=",
            "pj_posts.category_id",
        )
        .where_exists(
            DB::table("pj_statuses")
                .select_raw("1")
                .filter("pj_statuses.name", "paid"),
        )
        .select(["pj_posts.title", "cats.name as category"])
        .order_by_asc("pj_posts.id")
        .get()
        .await
        .expect("the statement runs")
        .into_vec();
    let log = DB::get_query_log().expect("read the query log");
    DB::disable_query_log().expect("disable the query log");

    assert_eq!(
        json_rows(rows),
        vec![
            json!({"title": "Alpha", "category": "News"}),
            json!({"title": "Beta", "category": "Rust"}),
        ]
    );
    // Other tests may run on other threads and log their own joins; only
    // this statement joins a subquery under the alias `cats`.
    let entry = log
        .iter()
        .find(|q| q.sql.contains("pj_posts") && q.sql.contains("cats"))
        .expect("the join statement is in the query log");
    for value in ["Linus", "Empty", "paid"] {
        assert!(
            !entry.sql.contains(value),
            "{value} reached the SQL text: {}",
            entry.sql
        );
    }
    assert_eq!(entry.bindings.len(), 3, "bindings: {:?}", entry.bindings);
    for (binding, value) in entry.bindings.iter().zip(["Linus", "Empty", "paid"]) {
        assert!(
            binding.contains(value),
            "bindings out of statement order: {:?}",
            entry.bindings
        );
    }
    let q = fx.quote();
    for identifier in ["pj_users", "pj_categories", "cats", "name", "author_id"] {
        assert!(
            entry.sql.contains(&format!("{q}{identifier}{q}")),
            "{identifier} is not quoted for the backend: {}",
            entry.sql
        );
    }
    if fx.backend == DatabaseBackend::Postgres {
        let first = entry.sql.find("$1").expect("$1 is in the statement");
        let second = entry.sql.find("$2").expect("$2 is in the statement");
        let third = entry.sql.find("$3").expect("$3 is in the statement");
        assert!(
            first < second && second < third,
            "placeholders out of order: {}",
            entry.sql
        );
    }
}

async fn run_every_scenario(fx: &Fixture) {
    chained_left_joins_with_alias(fx).await;
    left_join_against_a_grouped_subquery(fx).await;
    correlated_exists(fx).await;
    join_closure_combines_on_and_where(fx).await;
    join_kinds(fx).await;
    bound_values_keep_their_positions(fx).await;
    join_values_never_reach_the_sql_text(fx).await;
}

// ---------- SQLite ---------------------------------------------------------

async fn seeded_sqlite() -> Fixture {
    let fx = Fixture::sqlite().await;
    seed(&fx).await;
    fx
}

#[tokio::test]
async fn chained_left_joins_with_an_alias_match_raw_sql() {
    let fx = seeded_sqlite().await;
    chained_left_joins_with_alias(&fx).await;
}

#[tokio::test]
async fn a_left_join_against_a_grouped_subquery_matches_raw_sql() {
    let fx = seeded_sqlite().await;
    left_join_against_a_grouped_subquery(&fx).await;
}

#[tokio::test]
async fn a_correlated_exists_matches_raw_sql() {
    let fx = seeded_sqlite().await;
    correlated_exists(&fx).await;
}

#[tokio::test]
async fn a_join_closure_combines_on_or_on_where_and_or_where() {
    let fx = seeded_sqlite().await;
    join_closure_combines_on_and_where(&fx).await;
}

#[tokio::test]
async fn inner_right_cross_and_subquery_joins_match_raw_sql() {
    let fx = seeded_sqlite().await;
    join_kinds(&fx).await;
}

#[tokio::test]
async fn subquery_join_and_exists_values_bind_in_statement_order() {
    let fx = seeded_sqlite().await;
    bound_values_keep_their_positions(&fx).await;
}

#[tokio::test]
#[serial]
async fn a_join_value_never_appears_in_the_sql_text() {
    let fx = seeded_sqlite().await;
    join_values_never_reach_the_sql_text(&fx).await;
}

#[tokio::test]
async fn a_join_observes_every_table_it_reads() {
    let _fx = seeded_sqlite().await;
    let report = Collector::scope(async {
        begin_handler();
        DB::table("pj_posts")
            .join(
                "pj_users as authors",
                "authors.id",
                "=",
                "pj_posts.author_id",
            )
            .left_join_sub(
                DB::table("pj_orders").select(["status_id"]),
                "o",
                "o.status_id",
                "=",
                "pj_posts.id",
            )
            .where_exists(DB::table("pj_statuses").select_raw("1"))
            .where_in(
                "pj_posts.category_id",
                DB::table("pj_categories").select(["id"]),
            )
            .get()
            .await
            .expect("the statement runs");
        current_report().expect("a collector is active")
    })
    .await;
    for table in [
        "pj_posts",
        "pj_users",
        "pj_orders",
        "pj_statuses",
        "pj_categories",
    ] {
        assert!(
            report.observed.contains(&DependencyIdentity::table(table)),
            "{table} was read but not observed: {:?}",
            report.observed
        );
    }
}

#[tokio::test]
async fn invalid_join_identifiers_fail_before_any_sql_runs() {
    let _fx = seeded_sqlite().await;
    let bad_table = DB::table("pj_posts")
        .join(
            "pj_users; DROP TABLE pj_posts",
            "pj_users.id",
            "=",
            "pj_posts.author_id",
        )
        .get()
        .await;
    assert!(bad_table.is_err(), "an injected table name must be refused");

    let bad_alias = DB::table("pj_posts")
        .left_join("pj_users as a b", "a.id", "=", "pj_posts.author_id")
        .get()
        .await;
    assert!(bad_alias.is_err(), "a malformed alias must be refused");

    let bad_column = DB::table("pj_posts")
        .join("pj_users", "pj_users.id) OR (1", "=", "pj_posts.author_id")
        .get()
        .await;
    assert!(bad_column.is_err(), "an injected column must be refused");

    let bad_operator = DB::table("pj_posts")
        .join(
            "pj_users",
            "pj_users.id",
            "= 1 OR 1 =",
            "pj_posts.author_id",
        )
        .get()
        .await;
    assert!(
        bad_operator.is_err(),
        "an operator outside the allowlist must be refused"
    );

    let bad_sub_alias = DB::table("pj_posts")
        .join_sub(
            DB::table("pj_users"),
            "u; --",
            "u.id",
            "=",
            "pj_posts.author_id",
        )
        .get()
        .await;
    assert!(
        bad_sub_alias.is_err(),
        "a malformed subquery alias must be refused"
    );

    let bad_inner = DB::table("pj_users")
        .where_exists(DB::table("pj_posts").where_column("pj_posts.author_id", "1=1; --"))
        .get()
        .await;
    assert!(
        bad_inner.is_err(),
        "the subquery is validated like the outer query"
    );

    let posts = DB::table("pj_posts").count().await.expect("count");
    assert_eq!(posts, 4, "nothing ran");
}

#[tokio::test]
async fn a_join_without_an_on_condition_is_refused() {
    let _fx = seeded_sqlite().await;
    let result = DB::table("pj_posts")
        .join_with("pj_users", |join| join)
        .get()
        .await;
    let err = result.expect_err("an inner join needs at least one condition");
    assert!(
        err.to_string().contains("pj_users"),
        "the error names the join: {err}"
    );
}

#[tokio::test]
async fn update_and_delete_refuse_a_join() {
    let _fx = seeded_sqlite().await;
    let updated = DB::table("pj_posts")
        .join("pj_users", "pj_users.id", "=", "pj_posts.author_id")
        .filter("pj_users.name", "Ada")
        .update(suprnova::attrs! { title: "changed" })
        .await;
    assert!(updated.is_err(), "an UPDATE would ignore the join");

    let deleted = DB::table("pj_posts")
        .join("pj_users", "pj_users.id", "=", "pj_posts.author_id")
        .filter("pj_users.name", "Ada")
        .delete()
        .await;
    assert!(deleted.is_err(), "a DELETE would ignore the join");

    let untouched = DB::table("pj_posts")
        .filter("title", "changed")
        .count()
        .await
        .expect("count");
    assert_eq!(untouched, 0);
    assert_eq!(DB::table("pj_posts").count().await.expect("count"), 4);
}

#[tokio::test]
async fn count_on_a_grouped_query_counts_the_groups() {
    let _fx = seeded_sqlite().await;
    let groups = DB::table("pj_orders")
        .select(["status_id"])
        .group_by("status_id")
        .count()
        .await
        .expect("grouped count");
    assert_eq!(groups, 2, "two statuses have orders");

    let joined = DB::table("pj_posts")
        .join("pj_users", "pj_users.id", "=", "pj_posts.author_id")
        .count()
        .await
        .expect("joined count");
    assert_eq!(joined, 3, "three posts have an author");
}

// ---------- Live engines ----------------------------------------------------

#[tokio::test]
#[serial]
#[ignore = "requires disposable PostgreSQL at PG_TEST_URL"]
async fn postgres_every_join_shape_matches_raw_sql() {
    let fx = Fixture::live("PG_TEST_URL", DatabaseBackend::Postgres).await;
    seed(&fx).await;
    run_every_scenario(&fx).await;
    fx.close().await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_every_join_shape_matches_raw_sql() {
    let fx = Fixture::live("MYSQL_TEST_URL", DatabaseBackend::MySql).await;
    seed(&fx).await;
    run_every_scenario(&fx).await;
    fx.close().await;
}
