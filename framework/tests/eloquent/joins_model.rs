//! PAR-001: joins and `where_exists` on the model query builder.
//!
//! A model query that joins another table selects the model's own columns
//! by default, so the joined table's `id` never lands in the model. Both
//! tables here soft-delete, so the soft-delete filter must name its table
//! or the database rejects `deleted_at` as ambiguous.
//!
//! The SQLite tests run by default; the `postgres_` and `mysql_` tests run
//! every scenario against a disposable server:
//!
//! ```text
//! PG_TEST_URL=postgres://... MYSQL_TEST_URL=mysql://... \
//!   cargo nextest run -p suprnova --test eloquent -E 'test(/^joins_model::/)' --run-ignored all
//! ```

use sea_orm::{DatabaseBackend, Value as SeaValue};
use suprnova::render_cache::DependencyIdentity;
use suprnova::render_cache::collector::{Collector, begin_handler, current_report};
use suprnova::{DB, Model, attrs, model};

use crate::query_fixture::{Fixture, json_rows};

#[model(table = "jm_users", soft_deletes)]
pub struct JmUser {
    pub id: i64,
    pub name: String,
    pub deleted_at: Option<String>,
}

#[model(table = "jm_posts", soft_deletes)]
pub struct JmPost {
    pub id: i64,
    pub author_id: i64,
    pub title: String,
    pub views: i64,
    pub deleted_at: Option<String>,
}

/// A model whose table name has capitals. Postgres folds an unquoted name
/// to lower case, so a joined query has to quote this name the same way
/// everywhere it writes it, or one reference names a different table.
#[model(table = "JmAuthors", soft_deletes, relations = {
    posts: HasMany<JmPost> { fk = "author_id" },
})]
pub struct JmAuthor {
    pub id: i64,
    pub name: String,
    pub deleted_at: Option<String>,
}

/// User ids are 1 to 3 and post ids start at 101, so a post that carries
/// a user's id is easy to spot. Grace and post 105 are trashed.
async fn seed(fx: &Fixture) {
    for sql in [
        "CREATE TEMPORARY TABLE jm_users (id BIGINT PRIMARY KEY, name VARCHAR(100) NOT NULL, \
         deleted_at VARCHAR(30) NULL)",
        "CREATE TEMPORARY TABLE jm_posts (id BIGINT PRIMARY KEY, author_id BIGINT NOT NULL, \
         title VARCHAR(100) NOT NULL, views BIGINT NOT NULL, deleted_at VARCHAR(30) NULL)",
        "INSERT INTO jm_users (id, name, deleted_at) VALUES (1, 'Ada', NULL), \
         (2, 'Linus', NULL), (3, 'Grace', '2026-01-01 00:00:00')",
        "INSERT INTO jm_posts (id, author_id, title, views, deleted_at) VALUES \
         (101, 1, 'Alpha', 10, NULL), (102, 2, 'Beta', 50, NULL), (103, 1, 'Gamma', 5, NULL), \
         (104, 3, 'Delta', 70, NULL), (105, 2, 'Old', 1, '2026-01-01 00:00:00')",
    ] {
        fx.exec(sql).await;
    }
}

async fn raw_ids(sql: &str) -> Vec<i64> {
    json_rows(
        DB::select(sql, Vec::<SeaValue>::new())
            .await
            .unwrap_or_else(|e| panic!("raw comparison query failed: {sql}: {e}")),
    )
    .iter()
    .map(|row| row["id"].as_i64().expect("integer id"))
    .collect()
}

// ---------- Scenarios ---------------------------------------------------------

/// The falsifier: a model query joining another table hydrates the
/// model's own `id`, never the joined table's.
async fn a_join_hydrates_the_models_own_columns() {
    let posts = JmPost::query()
        .join("jm_users", "jm_users.id", "=", "jm_posts.author_id")
        .filter("jm_users.name", "Ada")
        .order_by_asc("jm_posts.id")
        .get()
        .await
        .expect("a joined model query runs");
    let ids: Vec<i64> = posts.iter().map(|post| post.id).collect();
    assert_eq!(ids, vec![101, 103], "the posts keep their own ids");
    assert!(posts.iter().all(|post| post.author_id == 1));
    assert_eq!(
        ids,
        raw_ids(
            "SELECT jm_posts.id FROM jm_posts INNER JOIN jm_users ON jm_users.id = jm_posts.author_id \
             WHERE jm_posts.deleted_at IS NULL AND jm_users.name = 'Ada' ORDER BY jm_posts.id ASC"
        )
        .await
    );

    let left = JmUser::query()
        .left_join("jm_posts as p", "p.author_id", "=", "jm_users.id")
        .filter_op("p.views", ">", 20)
        .get()
        .await
        .expect("a left join with an alias runs");
    let left_ids: Vec<i64> = left.iter().map(|user| user.id).collect();
    assert_eq!(left_ids, vec![2], "Linus, not post 102");
}

/// An explicit select overrides the default projection - including one
/// that asks for the joined table's `id`.
async fn an_explicit_select_overrides_the_default() {
    let star = JmPost::query()
        .join("jm_users", "jm_users.id", "=", "jm_posts.author_id")
        .select(["jm_posts.*"])
        .filter("jm_users.name", "Linus")
        .get()
        .await
        .expect("select of the model table's star runs");
    assert_eq!(star.iter().map(|p| p.id).collect::<Vec<_>>(), vec![102]);

    let theirs = JmPost::query()
        .join("jm_users", "jm_users.id", "=", "jm_posts.author_id")
        .select([
            "jm_users.id",
            "jm_posts.author_id",
            "jm_posts.title",
            "jm_posts.views",
            "jm_posts.deleted_at",
        ])
        .filter("jm_users.name", "Linus")
        .get()
        .await
        .expect("an explicit select runs");
    assert_eq!(
        theirs.iter().map(|p| p.id).collect::<Vec<_>>(),
        vec![2],
        "the caller asked for the user's id, so the post carries it"
    );
}

/// Issue #125 shape 3 on a model: a correlated `EXISTS`, and the
/// `NOT EXISTS` twin. The model's soft-delete filter still applies.
async fn correlated_exists_on_a_model() {
    let popular = DB::table("jm_posts")
        .select_raw("1")
        .where_column("jm_posts.author_id", "jm_users.id")
        .filter_op("jm_posts.views", ">", 20);
    let with = JmUser::query()
        .where_exists(popular.clone())
        .order_by_asc("id")
        .get()
        .await
        .expect("where_exists runs");
    assert_eq!(with.iter().map(|u| u.id).collect::<Vec<_>>(), vec![2]);
    assert_eq!(
        vec![2],
        raw_ids(
            "SELECT id FROM jm_users WHERE deleted_at IS NULL AND EXISTS \
             (SELECT 1 FROM jm_posts WHERE jm_posts.author_id = jm_users.id \
             AND jm_posts.views > 20) ORDER BY id ASC"
        )
        .await
    );

    let without = JmUser::query()
        .where_not_exists(popular)
        .order_by_asc("id")
        .get()
        .await
        .expect("where_not_exists runs");
    assert_eq!(without.iter().map(|u| u.id).collect::<Vec<_>>(), vec![1]);
}

/// A model joined to a grouped subquery, with values in the subquery and
/// the outer query, plus `count` over a join.
async fn subquery_joins_and_count_on_a_model() {
    let live_post_counts = DB::table("jm_posts")
        .select(["author_id"])
        .select_raw("COUNT(*) AS post_count")
        .where_null("deleted_at")
        .group_by("author_id");
    let prolific = JmUser::query()
        .join_sub(live_post_counts, "pc", "pc.author_id", "=", "jm_users.id")
        .filter_op("pc.post_count", ">=", 2)
        .get()
        .await
        .expect("join_sub runs");
    assert_eq!(
        prolific
            .iter()
            .map(|u| (u.id, u.name.as_str()))
            .collect::<Vec<_>>(),
        vec![(1, "Ada")]
    );

    let joined_rows = JmUser::query()
        .left_join("jm_posts", "jm_posts.author_id", "=", "jm_users.id")
        .count()
        .await
        .expect("count over a join");
    // Ada has two posts, Linus two (the join does not apply the posts'
    // soft-delete filter, as in Laravel), Grace is trashed.
    assert_eq!(joined_rows, 4);

    let with_closure = JmPost::query()
        .join_with("jm_users", |join| {
            join.on("jm_users.id", "=", "jm_posts.author_id")
                .db_where_op("jm_posts.views", ">", 8)
                .or_where("jm_users.name", "Ada")
        })
        .order_by_asc("jm_posts.id")
        .get()
        .await
        .expect("closure join runs");
    assert_eq!(
        with_closure.iter().map(|p| p.id).collect::<Vec<_>>(),
        vec![101, 102, 103, 104]
    );
}

/// The falsifier again, on the SQL the model builder renders: a join
/// value is a bound parameter, the join's identifiers are quoted, and the
/// values bind in statement order.
async fn join_values_are_bound(fx: &Fixture) {
    let query = JmPost::query()
        .join_with("jm_users", |join| {
            join.on("jm_users.id", "=", "jm_posts.author_id")
                .filter("jm_users.name", "Linus")
        })
        .left_join_sub(
            DB::table("jm_users").filter_op("name", "<>", "Marker-Sub"),
            "others",
            "others.id",
            "=",
            "jm_posts.author_id",
        )
        .where_exists(DB::table("jm_users").filter("name", "Marker-Exists"))
        .filter("jm_posts.title", "Marker-Outer");
    let (sql, bindings) = query
        .try_to_sql_with_bindings_for(fx.backend)
        .expect("the query renders");
    for value in ["Linus", "Marker-Sub", "Marker-Exists", "Marker-Outer"] {
        assert!(!sql.contains(value), "{value} reached the SQL text: {sql}");
    }
    let strings: Vec<String> = bindings
        .iter()
        .filter_map(|value| match value {
            SeaValue::String(Some(text)) => Some(text.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(
        strings,
        vec!["Linus", "Marker-Sub", "Marker-Exists", "Marker-Outer"],
        "bindings follow the statement order: {sql}"
    );
    let q = fx.quote();
    for identifier in ["jm_users", "others", "author_id"] {
        assert!(
            sql.contains(&format!("{q}{identifier}{q}")),
            "{identifier} is not quoted: {sql}"
        );
    }
    assert!(
        sql.contains(&format!("{q}jm_posts{q}.*")),
        "a joined model query selects its own table: {sql}"
    );

    let (postgres, _) = query
        .try_to_sql_with_bindings_for(DatabaseBackend::Postgres)
        .expect("the query renders for Postgres");
    let positions: Vec<usize> = (1..=4)
        .map(|n| {
            postgres
                .find(&format!("${n}"))
                .unwrap_or_else(|| panic!("${n} missing: {postgres}"))
        })
        .collect();
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "Postgres placeholders out of order: {postgres}"
    );
}

async fn run_every_scenario(fx: &Fixture) {
    a_join_hydrates_the_models_own_columns().await;
    an_explicit_select_overrides_the_default().await;
    correlated_exists_on_a_model().await;
    subquery_joins_and_count_on_a_model().await;
    join_values_are_bound(fx).await;
}

// ---------- SQLite ---------------------------------------------------------

async fn seeded_sqlite() -> Fixture {
    let fx = Fixture::sqlite().await;
    seed(&fx).await;
    fx
}

#[tokio::test]
async fn a_joined_model_query_never_hydrates_the_joined_tables_id() {
    let _fx = seeded_sqlite().await;
    a_join_hydrates_the_models_own_columns().await;
}

#[tokio::test]
async fn an_explicit_select_overrides_the_joined_default() {
    let _fx = seeded_sqlite().await;
    an_explicit_select_overrides_the_default().await;
}

#[tokio::test]
async fn where_exists_and_where_not_exists_correlate_on_a_model() {
    let _fx = seeded_sqlite().await;
    correlated_exists_on_a_model().await;
    // `filter_exists` / `filter_not_exists` are the Rust-shape names of the
    // same two methods. Linus has a trashed post, so only Ada qualifies.
    let both = JmUser::query()
        .filter_exists(
            DB::table("jm_posts")
                .select_raw("1")
                .where_column("jm_posts.author_id", "jm_users.id"),
        )
        .filter_not_exists(
            DB::table("jm_posts")
                .select_raw("1")
                .where_column("jm_posts.author_id", "jm_users.id")
                .where_not_null("jm_posts.deleted_at"),
        )
        .get()
        .await
        .expect("filter_exists and filter_not_exists run");
    assert_eq!(both.iter().map(|u| u.id).collect::<Vec<_>>(), vec![1]);
}

#[tokio::test]
async fn subquery_joins_closures_and_count_work_on_a_model() {
    let _fx = seeded_sqlite().await;
    subquery_joins_and_count_on_a_model().await;
}

#[tokio::test]
async fn a_model_join_value_is_bound_never_written_into_the_sql() {
    let fx = seeded_sqlite().await;
    join_values_are_bound(&fx).await;
}

/// Both tables soft-delete, so `only_trashed`'s `deleted_at IS NOT NULL`
/// must name the model's table or the database rejects the column as
/// ambiguous - whichever side of the join the call is written on.
#[tokio::test]
async fn only_trashed_names_the_models_table_when_the_query_joins() {
    let _fx = seeded_sqlite().await;
    let trashed_after_join = JmPost::query()
        .join("jm_users", "jm_users.id", "=", "jm_posts.author_id")
        .only_trashed()
        .get()
        .await
        .expect("only_trashed after a join runs");
    assert_eq!(
        trashed_after_join.iter().map(|p| p.id).collect::<Vec<_>>(),
        vec![105]
    );

    let trashed_before_join = JmPost::query()
        .only_trashed()
        .join("jm_users", "jm_users.id", "=", "jm_posts.author_id")
        .get()
        .await
        .expect("only_trashed before a join runs");
    assert_eq!(
        trashed_before_join.iter().map(|p| p.id).collect::<Vec<_>>(),
        vec![105]
    );

    let everything = JmPost::query()
        .join("jm_users", "jm_users.id", "=", "jm_posts.author_id")
        .with_trashed()
        .order_by_asc("jm_posts.id")
        .get()
        .await
        .expect("with_trashed with a join runs");
    assert_eq!(
        everything.iter().map(|p| p.id).collect::<Vec<_>>(),
        vec![101, 102, 103, 104, 105]
    );
}

/// The joined query quotes the model's table in its default select, so it
/// has to quote it in the FROM, in the soft-delete filter, in a `where_has`
/// correlation and in the key `model_keys` selects too.
#[tokio::test]
async fn a_joined_model_query_quotes_every_reference_to_its_mixed_case_table() {
    // Every shape is checked before the test fails, so a failure names all
    // the statements that write the table unquoted.
    let mut unquoted: Vec<String> = Vec::new();
    let mut assert_all_quoted = |shape: &str, sql: &str| {
        if !sql.contains(r#"FROM "JmAuthors""#)
            || sql.matches("JmAuthors").count() != sql.matches(r#""JmAuthors""#).count()
        {
            unquoted.push(format!("{shape}: {sql}"));
        }
    };

    let joined = || JmAuthor::query().join("jm_posts", "jm_posts.author_id", "=", "JmAuthors.id");
    for backend in [DatabaseBackend::Sqlite, DatabaseBackend::Postgres] {
        for (shape, query) in [
            ("default", joined()),
            ("only_trashed", joined().only_trashed()),
            (
                "where_has",
                joined().where_has::<JmPost, _>("posts", |posts| posts),
            ),
        ] {
            let (sql, _) = query
                .try_to_sql_with_bindings_for(backend)
                .expect("the query renders");
            assert_all_quoted(&format!("{shape} on {backend:?}"), &sql);
        }
    }

    // `model_keys` renders and runs in one call, so read its SQL off the
    // query log. SQLite ignores case, so the query runs either way.
    let fx = seeded_sqlite().await;
    fx.exec(
        r#"CREATE TEMPORARY TABLE "JmAuthors" (id BIGINT PRIMARY KEY, name VARCHAR(100) NOT NULL, deleted_at VARCHAR(30) NULL)"#,
    )
    .await;
    DB::enable_query_log().expect("the query log turns on");
    joined().model_keys().await.expect("model_keys runs");
    let logged: Vec<String> = DB::get_query_log()
        .expect("the query log reads")
        .into_iter()
        .map(|query| query.sql)
        .filter(|sql| sql.contains("JmAuthors"))
        .collect();
    assert!(!logged.is_empty(), "model_keys ran no logged query");
    for sql in &logged {
        assert_all_quoted("model_keys", sql);
    }

    assert!(
        unquoted.is_empty(),
        "a reference to the table is not quoted:\n{}",
        unquoted.join("\n")
    );
}

#[tokio::test]
async fn mass_writes_refuse_a_join() {
    let _fx = seeded_sqlite().await;
    let joined = || {
        JmPost::query()
            .join("jm_users", "jm_users.id", "=", "jm_posts.author_id")
            .filter("jm_users.name", "Ada")
    };
    assert!(
        joined()
            .update_all(attrs! { title: "changed" })
            .await
            .is_err(),
        "update_all would ignore the join"
    );
    assert!(
        joined().delete_all().await.is_err(),
        "delete_all would ignore the join"
    );
    assert!(
        joined().force_delete_all().await.is_err(),
        "force_delete_all would ignore the join"
    );
    assert!(
        joined().increment_each([("views", 1)]).await.is_err(),
        "increment_each would ignore the join"
    );
    let untouched = JmPost::query()
        .order_by_asc("id")
        .get()
        .await
        .expect("read back");
    assert_eq!(untouched.len(), 4, "nothing was deleted");
    assert!(untouched.iter().all(|p| p.title != "changed"));
    assert_eq!(untouched[0].views, 10, "nothing was incremented");
}

#[tokio::test]
async fn a_model_join_observes_the_joined_tables() {
    let _fx = seeded_sqlite().await;
    let report = Collector::scope(async {
        begin_handler();
        JmPost::query()
            .join("jm_users", "jm_users.id", "=", "jm_posts.author_id")
            .get()
            .await
            .expect("a joined model query runs");
        JmUser::query()
            .where_exists(DB::table("jm_posts").select_raw("1"))
            .count()
            .await
            .expect("an exists count runs");
        current_report().expect("a collector is active")
    })
    .await;
    for table in ["jm_posts", "jm_users"] {
        assert!(
            report.observed.contains(&DependencyIdentity::table(table)),
            "{table} was read but not observed: {:?}",
            report.observed
        );
    }
}

#[tokio::test]
async fn an_invalid_join_on_a_model_fails_before_any_sql_runs() {
    let _fx = seeded_sqlite().await;
    let bad_table = JmPost::query()
        .join("jm_users u", "u.id", "=", "jm_posts.author_id")
        .get()
        .await;
    assert!(bad_table.is_err(), "a table alias needs `as`");
    let bad_operator = JmPost::query()
        .join("jm_users", "jm_users.id", "<=>", "jm_posts.author_id")
        .get()
        .await;
    assert!(bad_operator.is_err(), "an operator outside the allowlist");
    let empty_on = JmPost::query()
        .join_with("jm_users", |join| join)
        .get()
        .await;
    assert!(empty_on.is_err(), "an inner join needs a condition");
}

// ---------- Live engines ----------------------------------------------------

#[tokio::test]
#[ignore = "requires disposable PostgreSQL at PG_TEST_URL"]
async fn postgres_model_joins_match_raw_sql() {
    let fx = Fixture::live("PG_TEST_URL", DatabaseBackend::Postgres).await;
    seed(&fx).await;
    run_every_scenario(&fx).await;
    fx.close().await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_model_joins_match_raw_sql() {
    let fx = Fixture::live("MYSQL_TEST_URL", DatabaseBackend::MySql).await;
    seed(&fx).await;
    run_every_scenario(&fx).await;
    fx.close().await;
}
