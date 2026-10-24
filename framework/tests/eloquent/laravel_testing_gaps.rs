//! PAR-181: full-text search on the model builder. `where_full_text` and
//! `or_where_full_text` (and their `filter_` names) render
//! `MATCH (..) AGAINST (..)` on MySQL and MariaDB and `@@` on Postgres, and
//! on SQLite they are an error that names SQLite and the call.
//!
//! The `postgres_` and `mysql_` tests need a disposable database:
//!
//! ```text
//! PG_TEST_URL=postgres://... MYSQL_TEST_URL=mysql://... \
//!   cargo nextest run -p suprnova --test eloquent --run-ignored only \
//!   -E 'test(/laravel_testing_gaps::(postgres|mysql)_/)' --test-threads 1
//! ```

use sea_orm::{DbBackend, Value as SeaValue};
use sea_orm_migration::SchemaManager;
use serial_test::serial;
use suprnova::database::{DatabaseConfig, DbConnection};
use suprnova::schema::Schema;
use suprnova::testing::TestContainer;
use suprnova::{Builder, DB, FullTextMode, FullTextOptions, Model, attrs, model};

use crate::query_fixture::Fixture;

#[model(table = "ft_posts", timestamps = false, fillable = ["title", "body"])]
pub struct FtPost {
    pub id: u64,
    pub title: String,
    pub body: String,
}

fn posts() -> Builder<FtPost> {
    FtPost::query()
}

fn rendered(query: Builder<FtPost>, backend: DbBackend) -> (String, Vec<SeaValue>) {
    query
        .try_to_sql_with_bindings_for(backend)
        .unwrap_or_else(|error| panic!("{backend:?} renders: {error}"))
}

#[test]
fn where_full_text_renders_match_against_on_mysql() {
    let (sql, values) = rendered(
        posts().where_full_text(["title", "body"], "laravel"),
        DbBackend::MySql,
    );
    assert_eq!(
        sql,
        "SELECT * FROM ft_posts WHERE MATCH (`title`, `body`) AGAINST (? IN NATURAL LANGUAGE MODE)"
    );
    assert_eq!(values, vec![SeaValue::from("laravel")]);

    let (sql, _) = rendered(
        posts().where_full_text_with(
            ["body"],
            "+laravel -php",
            FullTextOptions::new().mode(FullTextMode::Boolean),
        ),
        DbBackend::MySql,
    );
    assert_eq!(
        sql,
        "SELECT * FROM ft_posts WHERE MATCH (`body`) AGAINST (? IN BOOLEAN MODE)"
    );

    let (sql, _) = rendered(
        posts().filter_full_text_with(["body"], "laravel", FullTextOptions::new().expanded()),
        DbBackend::MySql,
    );
    assert_eq!(
        sql,
        "SELECT * FROM ft_posts WHERE MATCH (`body`) AGAINST (? IN NATURAL LANGUAGE MODE WITH QUERY EXPANSION)"
    );
}

#[test]
fn where_full_text_renders_to_tsvector_at_at_on_postgres() {
    let (sql, values) = rendered(
        posts().where_full_text(["title", "body"], "laravel"),
        DbBackend::Postgres,
    );
    assert_eq!(
        sql,
        concat!(
            "SELECT * FROM ft_posts WHERE ",
            r#"(to_tsvector('english', "title") || to_tsvector('english', "body")) "#,
            "@@ plainto_tsquery('english', $1)",
        )
    );
    assert_eq!(values, vec![SeaValue::from("laravel")]);

    let (sql, _) = rendered(
        posts().where_full_text_with(
            ["body"],
            "\"exact phrase\" -php",
            FullTextOptions::new()
                .mode(FullTextMode::Websearch)
                .language("german"),
        ),
        DbBackend::Postgres,
    );
    assert_eq!(
        sql,
        r#"SELECT * FROM ft_posts WHERE (to_tsvector('german', "body")) @@ websearch_to_tsquery('german', $1)"#
    );
}

#[test]
fn the_filter_names_render_what_the_where_names_render() {
    for backend in [DbBackend::MySql, DbBackend::Postgres] {
        assert_eq!(
            rendered(posts().filter_full_text(["body"], "laravel"), backend),
            rendered(posts().where_full_text(["body"], "laravel"), backend),
        );
        assert_eq!(
            rendered(
                posts()
                    .filter("id", 1)
                    .or_filter_full_text(["body"], "laravel"),
                backend
            ),
            rendered(
                posts()
                    .filter("id", 1)
                    .or_where_full_text(["body"], "laravel"),
                backend
            ),
        );
        let options = || FullTextOptions::new().mode(FullTextMode::Websearch);
        assert_eq!(
            rendered(
                posts()
                    .filter("id", 1)
                    .or_filter_full_text_with(["body"], "laravel", options()),
                backend
            ),
            rendered(
                posts()
                    .filter("id", 1)
                    .or_where_full_text_with(["body"], "laravel", options()),
                backend
            ),
        );
    }
}

#[test]
fn or_where_full_text_is_joined_with_or() {
    let (sql, values) = rendered(
        posts()
            .filter("title", "Cooking")
            .or_where_full_text(["body"], "laravel"),
        DbBackend::Postgres,
    );
    assert_eq!(
        sql,
        concat!(
            "SELECT * FROM ft_posts WHERE title = $1 ",
            r#"OR (to_tsvector('english', "body")) @@ plainto_tsquery('english', $2)"#,
        )
    );
    assert_eq!(
        values,
        vec![SeaValue::from("Cooking"), SeaValue::from("laravel")]
    );

    let (sql, _) = rendered(
        posts()
            .filter("title", "Cooking")
            .or_where_full_text(["title", "body"], "laravel"),
        DbBackend::MySql,
    );
    assert_eq!(
        sql,
        "SELECT * FROM ft_posts WHERE title = ? OR MATCH (`title`, `body`) AGAINST (? IN NATURAL LANGUAGE MODE)"
    );
}

#[test]
fn where_full_text_on_sqlite_is_an_error_naming_sqlite_and_the_call() {
    for (call, query) in [
        (
            "where_full_text",
            posts().where_full_text(["body"], "laravel"),
        ),
        (
            "or_where_full_text",
            posts()
                .filter("title", "Cooking")
                .or_where_full_text(["body"], "laravel"),
        ),
    ] {
        let error = query
            .try_to_sql_with_bindings_for(DbBackend::Sqlite)
            .expect_err("SQLite has no full-text search");
        let text = error.to_string();
        assert!(text.contains("SQLite"), "{call}: {text}");
        assert!(text.contains(call), "{call}: {text}");
    }
}

#[tokio::test]
async fn a_sqlite_terminal_refuses_where_full_text() {
    let fx = Fixture::sqlite().await;
    fx.exec(
        "CREATE TABLE ft_posts (id INTEGER PRIMARY KEY, title TEXT NOT NULL, body TEXT NOT NULL)",
    )
    .await;

    let error = posts()
        .where_full_text(["title", "body"], "laravel")
        .get()
        .await
        .expect_err("SQLite has no full-text search");
    let text = error.to_string();
    assert!(text.contains("SQLite"), "{text}");
    assert!(text.contains("where_full_text"), "{text}");

    let error = posts()
        .filter("title", "Cooking")
        .or_where_full_text(["body"], "laravel")
        .count()
        .await
        .expect_err("SQLite has no full-text search");
    assert!(error.to_string().contains("SQLite"), "{error}");
}

#[test]
fn bad_columns_and_languages_are_refused_before_any_sql() {
    let no_columns: [&str; 0] = [];
    let error = posts()
        .where_full_text(no_columns, "laravel")
        .try_to_sql_with_bindings_for(DbBackend::Postgres)
        .expect_err("a search over no column");
    assert!(error.to_string().contains("at least one column"), "{error}");

    let error = posts()
        .where_full_text_with(
            ["body"],
            "laravel",
            FullTextOptions::new().language("pg_catalog.english'; --"),
        )
        .try_to_sql_with_bindings_for(DbBackend::Postgres)
        .expect_err("an injected language");
    assert!(error.to_string().contains("language"), "{error}");
}

// ---- Against Postgres and MySQL -------------------------------------------------

/// One connection to the server `env` names, installed as the default
/// connection, with `ft_posts` created by the schema builder and its
/// full-text index over the title and body.
async fn live(env: &str) -> (suprnova::testing::TestContainerGuard, DbConnection) {
    let url = std::env::var(env).expect("explicit disposable database URL required");
    let guard = TestContainer::fake();
    let config = DatabaseConfig::builder()
        .url(url)
        .max_connections(1)
        .min_connections(1)
        .logging(false)
        .build();
    let connection = DbConnection::connect(&config)
        .await
        .expect("connect test database");
    TestContainer::singleton(connection.clone());

    let manager = SchemaManager::new(connection.inner());
    Schema::drop_if_exists(&manager, "ft_posts")
        .await
        .expect("a clean start");
    Schema::create(&manager, "ft_posts", |t| {
        t.id();
        t.string("title");
        t.text("body");
        t.full_text(&["title", "body"]);
    })
    .await
    .expect("create the table and its full-text index");
    for (title, body) in [
        ("Release notes", "Suprnova follows Laravel closely"),
        ("Gardening", "Tomatoes need sun and water"),
        ("Cooking", "Bread needs time and patience"),
    ] {
        DB::table("ft_posts")
            .insert(attrs! { title: title, body: body })
            .await
            .expect("insert");
    }
    (guard, connection)
}

async fn titles(query: Builder<FtPost>) -> Vec<String> {
    query
        .order_by_asc("title")
        .get()
        .await
        .unwrap_or_else(|error| panic!("full-text query: {error}"))
        .iter()
        .map(|post| post.title.clone())
        .collect()
}

async fn finish(connection: DbConnection) {
    Schema::drop_if_exists(&SchemaManager::new(connection.inner()), "ft_posts")
        .await
        .expect("cleanup");
}

/// A word in a row's body finds the row through the model, and
/// `or_where_full_text` keeps the rows either side matches.
async fn model_full_text_search() {
    assert_eq!(
        titles(posts().where_full_text(["title", "body"], "laravel")).await,
        vec!["Release notes"]
    );
    assert_eq!(
        titles(
            posts()
                .filter("title", "Cooking")
                .or_where_full_text(["title", "body"], "laravel")
        )
        .await,
        vec!["Cooking", "Release notes"],
        "OR keeps the rows either side matches; AND would keep none"
    );
    assert!(
        titles(
            posts()
                .filter("title", "Cooking")
                .where_full_text(["title", "body"], "laravel")
        )
        .await
        .is_empty()
    );
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable PostgreSQL at PG_TEST_URL"]
async fn postgres_model_where_full_text_finds_a_row_by_a_word_in_its_body() {
    let (_guard, connection) = live("PG_TEST_URL").await;
    model_full_text_search().await;
    assert_eq!(
        titles(posts().where_full_text_with(
            ["title", "body"],
            "needs -tomatoes",
            FullTextOptions::new().mode(FullTextMode::Websearch),
        ))
        .await,
        vec!["Cooking"],
        "websearch_to_tsquery reads the exclusion"
    );
    finish(connection).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_model_where_full_text_finds_a_row_by_a_word_in_its_body() {
    let (_guard, connection) = live("MYSQL_TEST_URL").await;
    model_full_text_search().await;
    assert_eq!(
        titles(posts().where_full_text_with(
            ["title", "body"],
            "+needs -tomatoes",
            FullTextOptions::new().mode(FullTextMode::Boolean),
        ))
        .await,
        vec!["Cooking"],
        "boolean mode reads the exclusion"
    );
    finish(connection).await;
}
