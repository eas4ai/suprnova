//! PAR-181: full-text search on the schema builder and on `DB::table`.
//!
//! `Blueprint::full_text` creates a `FULLTEXT` index on MySQL and MariaDB
//! and a `GIN` index over `to_tsvector` on Postgres, `drop_full_text` drops
//! it, and `where_full_text` and `or_where_full_text` render
//! `MATCH (..) AGAINST (..)` on MySQL and MariaDB and `@@` on Postgres. On
//! SQLite each call is an error that names SQLite and the call.
//!
//! The `postgres_` and `mysql_` tests need a disposable database:
//!
//! ```text
//! PG_TEST_URL=postgres://... MYSQL_TEST_URL=mysql://... \
//!   cargo nextest run -p suprnova --test schema --run-ignored only \
//!   -E 'test(/laravel_testing_gaps::(postgres|mysql)_/)' --test-threads 1
//! ```
//!
//! Explicit execution without the URL fails immediately; it never reports a
//! silent pass.

use sea_orm::{
    ConnectionTrait, DatabaseConnection, DbBackend, DbErr, Statement, TransactionTrait,
    Value as SeaValue,
};
use sea_orm_migration::{MigrationTrait, SchemaManager};
use serial_test::serial;
use suprnova::database::DbTableBuilder;
use suprnova::schema::Schema;
use suprnova::testing::TestContainer;
use suprnova::{Blueprint, DB, DbConnection, FullTextMode, FullTextOptions, attrs};

use super::mysql::connect_mysql;
use super::postgres::connect_postgres;
use super::sqlite::connect_sqlite;

const ARTICLES: &str = "ft_articles";
const INDEX: &str = "ft_articles_title_body_fulltext";

fn migration_error(result: Result<(), DbErr>) -> String {
    match result {
        Err(DbErr::Migration(text)) => text,
        other => panic!("expected DbErr::Migration, got {other:?}"),
    }
}

fn articles_with(define: impl FnOnce(&mut Blueprint)) -> impl FnOnce(&mut Blueprint) {
    move |t: &mut Blueprint| {
        t.id();
        t.string("title");
        t.text("body");
        define(t);
    }
}

/// The index statement `Schema::create` runs after the table for `backend`.
/// MySQL reads the configured character set while it plans the table, so
/// the environment lock is held.
fn index_sql(backend: DbBackend, define: impl FnOnce(&mut Blueprint)) -> String {
    let _env = crate::env_lock::lock_env();
    let steps = Blueprint::create_sql(ARTICLES, backend, articles_with(define)).expect("plan");
    assert_eq!(steps.len(), 2, "the table, then the index: {steps:?}");
    steps[1].clone()
}

// ---- Blueprint::full_text ----------------------------------------------------

#[test]
fn full_text_creates_a_fulltext_index_on_mysql() {
    let sql = index_sql(DbBackend::MySql, |t| {
        t.full_text(&["title", "body"]);
    });
    assert_eq!(
        sql,
        "CREATE FULLTEXT INDEX `ft_articles_title_body_fulltext` ON `ft_articles` (`title`, `body`)"
    );
}

#[test]
fn full_text_creates_a_gin_index_over_to_tsvector_on_postgres() {
    let sql = index_sql(DbBackend::Postgres, |t| {
        t.full_text(&["title", "body"]);
    });
    assert_eq!(
        sql,
        concat!(
            r#"CREATE INDEX "ft_articles_title_body_fulltext" ON "ft_articles" USING gin "#,
            r#"((to_tsvector('english', "title") || to_tsvector('english', "body")))"#,
        )
    );
}

#[test]
fn the_language_names_the_postgres_configuration_and_mysql_ignores_it() {
    let sql = index_sql(DbBackend::Postgres, |t| {
        t.full_text(&["body"]).language("french");
    });
    assert_eq!(
        sql,
        r#"CREATE INDEX "ft_articles_body_fulltext" ON "ft_articles" USING gin ((to_tsvector('french', "body")))"#
    );

    let sql = index_sql(DbBackend::MySql, |t| {
        t.full_text(&["body"]).language("french");
    });
    assert_eq!(
        sql,
        "CREATE FULLTEXT INDEX `ft_articles_body_fulltext` ON `ft_articles` (`body`)"
    );
}

#[test]
fn a_language_that_is_not_a_name_is_refused_before_any_sql() {
    for backend in [DbBackend::Postgres, DbBackend::MySql] {
        let _env = crate::env_lock::lock_env();
        let error = Blueprint::create_sql(
            ARTICLES,
            backend,
            articles_with(|t| {
                t.full_text(&["body"])
                    .language("english'); DROP TABLE users; --");
            }),
        )
        .expect_err("an injected language is refused");
        let text = error.to_string();
        assert!(text.contains("language"), "{backend:?}: {text}");
        assert!(text.contains("ft_articles"), "{backend:?}: {text}");
    }
}

#[test]
fn full_text_over_a_column_the_table_does_not_declare_is_refused() {
    let error = Blueprint::create_sql(
        ARTICLES,
        DbBackend::Postgres,
        articles_with(|t| {
            t.full_text(&["title", "summary"]);
        }),
    )
    .expect_err("an index over an undeclared column");
    let text = error.to_string();
    assert!(text.contains("summary"), "{text}");
    assert!(
        text.contains(INDEX.replace("body", "summary").as_str()),
        "{text}"
    );
}

#[test]
fn full_text_without_columns_or_twice_over_the_same_columns_is_refused() {
    let error = Blueprint::create_sql(
        ARTICLES,
        DbBackend::Postgres,
        articles_with(|t| {
            t.full_text(&[]);
        }),
    )
    .expect_err("an index without columns");
    assert!(
        error.to_string().contains("without named columns"),
        "{error}"
    );

    let error = Blueprint::create_sql(
        ARTICLES,
        DbBackend::Postgres,
        articles_with(|t| {
            t.full_text(&["title", "body"]);
            t.full_text(&["title", "body"]);
        }),
    )
    .expect_err("two indexes with one name");
    assert!(error.to_string().contains(INDEX), "{error}");
}

#[test]
fn full_text_on_sqlite_is_an_error_naming_sqlite_and_the_call() {
    let error = Blueprint::create_sql(
        ARTICLES,
        DbBackend::Sqlite,
        articles_with(|t| {
            t.full_text(&["title", "body"]);
        }),
    )
    .expect_err("SQLite has no full-text index");
    let text = error.to_string();
    assert!(text.contains("SQLite"), "{text}");
    assert!(text.contains("full_text"), "{text}");
}

#[tokio::test]
async fn schema_create_on_sqlite_refuses_full_text_before_any_statement() {
    let conn = connect_sqlite().await;
    let manager = SchemaManager::new(&conn);
    let text = migration_error(
        Schema::create(
            &manager,
            ARTICLES,
            articles_with(|t| {
                t.full_text(&["title", "body"]);
            }),
        )
        .await,
    );
    assert!(text.contains("SQLite"), "{text}");
    assert!(text.contains("full_text"), "{text}");
    assert!(
        !Schema::has_table(&manager, ARTICLES).await.expect("probe"),
        "the refusal comes before the table statement"
    );
}

#[tokio::test]
async fn schema_table_on_sqlite_refuses_full_text_and_drop_full_text() {
    let conn = connect_sqlite().await;
    let manager = SchemaManager::new(&conn);
    Schema::create(&manager, ARTICLES, articles_with(|_| {}))
        .await
        .expect("create");

    let text = migration_error(
        Schema::table(&manager, ARTICLES, |t| {
            t.drop_full_text(&["title", "body"]);
        })
        .await,
    );
    assert!(text.contains("SQLite"), "{text}");
    assert!(text.contains("drop_full_text"), "{text}");

    let text = migration_error(
        Schema::table(&manager, ARTICLES, |t| {
            t.full_text(&["title", "body"]);
        })
        .await,
    );
    assert!(text.contains("SQLite"), "{text}");
    assert!(text.contains("full_text"), "{text}");
}

#[test]
fn drop_full_text_belongs_to_schema_table() {
    let error = Blueprint::create_sql(
        ARTICLES,
        DbBackend::Postgres,
        articles_with(|t| {
            t.drop_full_text(&["title", "body"]);
        }),
    )
    .expect_err("Schema::create cannot drop an index");
    let text = error.to_string();
    assert!(text.contains("drop_full_text"), "{text}");
    assert!(text.contains("Schema::table"), "{text}");
}

// ---- DB::table where_full_text -----------------------------------------------

fn rendered(query: DbTableBuilder, backend: DbBackend) -> (String, Vec<SeaValue>) {
    query
        .to_sql_for(backend)
        .unwrap_or_else(|error| panic!("{backend:?} renders: {error}"))
}

#[test]
fn where_full_text_renders_match_against_on_mysql() {
    let (sql, values) = rendered(
        DB::table("articles").where_full_text(["title", "body"], "laravel"),
        DbBackend::MySql,
    );
    assert_eq!(
        sql,
        "SELECT * FROM `articles` WHERE MATCH (`title`, `body`) AGAINST (? IN NATURAL LANGUAGE MODE)"
    );
    assert_eq!(values, vec![SeaValue::from("laravel")]);
}

#[test]
fn the_mode_picks_the_mysql_modifier() {
    let modifier = |options: FullTextOptions| {
        rendered(
            DB::table("articles").where_full_text_with(["body"], "laravel", options),
            DbBackend::MySql,
        )
        .0
    };
    assert_eq!(
        modifier(FullTextOptions::new().mode(FullTextMode::Boolean)),
        "SELECT * FROM `articles` WHERE MATCH (`body`) AGAINST (? IN BOOLEAN MODE)"
    );
    assert_eq!(
        modifier(FullTextOptions::new().expanded()),
        "SELECT * FROM `articles` WHERE MATCH (`body`) AGAINST (? IN NATURAL LANGUAGE MODE WITH QUERY EXPANSION)"
    );
    assert_eq!(
        modifier(
            FullTextOptions::new()
                .mode(FullTextMode::Boolean)
                .expanded()
        ),
        "SELECT * FROM `articles` WHERE MATCH (`body`) AGAINST (? IN BOOLEAN MODE)",
        "MySQL has no query expansion in boolean mode, so Laravel drops it"
    );
    assert_eq!(
        modifier(FullTextOptions::new().mode(FullTextMode::Websearch)),
        "SELECT * FROM `articles` WHERE MATCH (`body`) AGAINST (? IN NATURAL LANGUAGE MODE)",
        "MySQL has no websearch mode and reads the text as natural language, as Laravel does"
    );
}

#[test]
fn where_full_text_renders_to_tsvector_at_at_on_postgres() {
    let (sql, values) = rendered(
        DB::table("articles").where_full_text(["title", "body"], "laravel"),
        DbBackend::Postgres,
    );
    assert_eq!(
        sql,
        concat!(
            r#"SELECT * FROM "articles" WHERE "#,
            r#"(to_tsvector('english', "title") || to_tsvector('english', "body")) "#,
            r#"@@ plainto_tsquery('english', $1)"#,
        )
    );
    assert_eq!(values, vec![SeaValue::from("laravel")]);
}

#[test]
fn the_mode_and_language_pick_the_postgres_function_and_configuration() {
    let (sql, _) = rendered(
        DB::table("articles").where_full_text_with(
            ["body"],
            "laravel -php",
            FullTextOptions::new()
                .mode(FullTextMode::Websearch)
                .language("french"),
        ),
        DbBackend::Postgres,
    );
    assert_eq!(
        sql,
        r#"SELECT * FROM "articles" WHERE (to_tsvector('french', "body")) @@ websearch_to_tsquery('french', $1)"#
    );

    let (sql, _) = rendered(
        DB::table("articles").where_full_text_with(
            ["body"],
            "laravel",
            FullTextOptions::new()
                .mode(FullTextMode::Boolean)
                .expanded(),
        ),
        DbBackend::Postgres,
    );
    assert_eq!(
        sql,
        r#"SELECT * FROM "articles" WHERE (to_tsvector('english', "body")) @@ plainto_tsquery('english', $1)"#,
        "Postgres has no boolean mode or query expansion and reads the text as natural language, as Laravel does"
    );
}

#[test]
fn or_where_full_text_is_joined_with_or() {
    let (sql, values) = rendered(
        DB::table("articles")
            .filter("status", "draft")
            .or_where_full_text(["articles.body"], "laravel")
            .where_full_text_with(
                ["title"],
                "rust",
                FullTextOptions::new().mode(FullTextMode::Boolean),
            ),
        DbBackend::MySql,
    );
    assert_eq!(
        sql,
        concat!(
            "SELECT * FROM `articles` WHERE `status` = ? ",
            "OR MATCH (`articles`.`body`) AGAINST (? IN NATURAL LANGUAGE MODE) ",
            "AND MATCH (`title`) AGAINST (? IN BOOLEAN MODE)",
        )
    );
    assert_eq!(
        values,
        vec![
            SeaValue::from("draft"),
            SeaValue::from("laravel"),
            SeaValue::from("rust"),
        ]
    );

    let (sql, _) = rendered(
        DB::table("articles")
            .filter("status", "draft")
            .or_where_full_text_with(
                ["body"],
                "laravel",
                FullTextOptions::new().language("simple"),
            ),
        DbBackend::Postgres,
    );
    assert_eq!(
        sql,
        concat!(
            r#"SELECT * FROM "articles" WHERE "status" = $1 "#,
            r#"OR (to_tsvector('simple', "body")) @@ plainto_tsquery('simple', $2)"#,
        )
    );
}

#[test]
fn where_full_text_on_sqlite_is_an_error_naming_sqlite_and_the_call() {
    for (call, query) in [
        (
            "where_full_text",
            DB::table("articles").where_full_text(["body"], "laravel"),
        ),
        (
            "or_where_full_text",
            DB::table("articles")
                .filter("status", "draft")
                .or_where_full_text(["body"], "laravel"),
        ),
    ] {
        let error = query
            .to_sql_for(DbBackend::Sqlite)
            .expect_err("SQLite has no full-text search");
        let text = error.to_string();
        assert!(text.contains("SQLite"), "{call}: {text}");
        assert!(text.contains(call), "{call}: {text}");
    }
}

#[tokio::test]
async fn a_sqlite_terminal_refuses_where_full_text_before_any_io() {
    let conn = connect_sqlite().await;
    Schema::create(&SchemaManager::new(&conn), ARTICLES, articles_with(|_| {}))
        .await
        .expect("create");
    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));

    let error = DB::table(ARTICLES)
        .where_full_text(["title", "body"], "laravel")
        .get()
        .await
        .expect_err("SQLite has no full-text search");
    let text = error.to_string();
    assert!(text.contains("SQLite"), "{text}");
    assert!(text.contains("where_full_text"), "{text}");

    let error = DB::table(ARTICLES)
        .filter("title", "x")
        .or_where_full_text(["body"], "laravel")
        .count()
        .await
        .expect_err("SQLite has no full-text search");
    assert!(error.to_string().contains("SQLite"), "{error}");
}

#[test]
fn bad_columns_and_languages_are_refused_before_any_sql() {
    let no_columns: [&str; 0] = [];
    let error = DB::table("articles")
        .where_full_text(no_columns, "laravel")
        .to_sql_for(DbBackend::MySql)
        .expect_err("a search over no column");
    assert!(error.to_string().contains("at least one column"), "{error}");

    let error = DB::table("articles")
        .where_full_text(["body; DROP TABLE users"], "laravel")
        .to_sql_for(DbBackend::MySql)
        .expect_err("an injected column");
    assert!(
        error.to_string().contains("body; DROP TABLE users"),
        "{error}"
    );

    let error = DB::table("articles")
        .where_full_text_with(
            ["body"],
            "laravel",
            FullTextOptions::new().language("english'), ('x"),
        )
        .to_sql_for(DbBackend::Postgres)
        .expect_err("an injected language");
    assert!(error.to_string().contains("language"), "{error}");
}

// ---- Against Postgres and MySQL -------------------------------------------------

/// The migration the engine tests run: articles with a full-text index over
/// their title and body.
struct CreateArticles;

impl sea_orm_migration::MigrationName for CreateArticles {
    fn name(&self) -> &str {
        "m20261010_000001_create_ft_articles"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateArticles {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        Schema::create(
            manager,
            ARTICLES,
            articles_with(|t| {
                t.full_text(&["title", "body"]);
            }),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        Schema::drop_if_exists(manager, ARTICLES).await
    }
}

/// The titles of the rows `query` returns, in title order.
async fn titles(query: DbTableBuilder) -> Vec<String> {
    query
        .order_by_asc("title")
        .get()
        .await
        .unwrap_or_else(|error| panic!("full-text query: {error}"))
        .into_vec()
        .iter()
        .map(|row| row.get_string("title").expect("title"))
        .collect()
}

/// Whether `table` has an index called `name`, and, on Postgres, its
/// definition; on MySQL, its index type.
async fn index_shape(conn: &DatabaseConnection, name: &str) -> Option<String> {
    let backend = conn.get_database_backend();
    let sql = match backend {
        DbBackend::Postgres => format!(
            "SELECT indexdef FROM pg_indexes WHERE tablename = '{ARTICLES}' AND indexname = '{name}'"
        ),
        _ => format!(
            "SELECT DISTINCT INDEX_TYPE FROM information_schema.STATISTICS \
             WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = '{ARTICLES}' AND INDEX_NAME = '{name}'"
        ),
    };
    conn.query_one_raw(Statement::from_string(backend, sql))
        .await
        .expect("catalog query")
        .map(|row| row.try_get_by_index::<String>(0).expect("a string"))
}

async fn seed_articles() {
    for (title, body) in [
        ("Release notes", "Suprnova follows Laravel closely"),
        ("Gardening", "Tomatoes need sun and water"),
        ("Cooking", "Bread needs time and patience"),
    ] {
        DB::table(ARTICLES)
            .insert(attrs! { title: title, body: body })
            .await
            .expect("insert");
    }
}

/// The migration creates the index, a word in a row's body finds the row,
/// `or_where_full_text` widens the match with `OR`, the mode reaches the
/// engine, and `drop_full_text` drops the index.
async fn full_text_round_trip(conn: &DatabaseConnection) {
    let manager = SchemaManager::new(conn);
    Schema::drop_if_exists(&manager, ARTICLES)
        .await
        .expect("a clean start");
    CreateArticles
        .up(&manager)
        .await
        .expect("the migration creates the full-text index");

    let shape = index_shape(conn, INDEX).await.expect("the index exists");
    match conn.get_database_backend() {
        DbBackend::Postgres => {
            assert!(shape.contains("USING gin"), "{shape}");
            assert!(
                shape.contains("to_tsvector('english'::regconfig, (title)::text)"),
                "{shape}"
            );
            assert!(
                shape.contains("to_tsvector('english'::regconfig, body)"),
                "{shape}"
            );
        }
        _ => assert_eq!(shape, "FULLTEXT"),
    }

    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));
    seed_articles().await;

    assert_eq!(
        titles(DB::table(ARTICLES).where_full_text(["title", "body"], "laravel")).await,
        vec!["Release notes"],
        "a word in the body finds its row and no other"
    );
    assert_eq!(
        titles(
            DB::table(ARTICLES)
                .filter("title", "Cooking")
                .or_where_full_text(["title", "body"], "laravel")
        )
        .await,
        vec!["Cooking", "Release notes"],
        "OR keeps the rows either side matches; AND would keep none"
    );
    assert!(
        titles(
            DB::table(ARTICLES)
                .filter("title", "Cooking")
                .where_full_text(["title", "body"], "laravel")
        )
        .await
        .is_empty(),
        "where_full_text joins with AND"
    );

    let (mode, text) = match conn.get_database_backend() {
        DbBackend::Postgres => (FullTextMode::Websearch, "needs -tomatoes"),
        _ => (FullTextMode::Boolean, "+needs -tomatoes"),
    };
    assert_eq!(
        titles(DB::table(ARTICLES).where_full_text_with(
            ["title", "body"],
            text,
            FullTextOptions::new().mode(mode),
        ))
        .await,
        vec!["Cooking"],
        "the engine reads the exclusion in {mode:?} mode"
    );

    Schema::table(&manager, ARTICLES, |t| {
        t.drop_full_text(&["title", "body"]);
    })
    .await
    .expect("drop_full_text drops the index");
    assert_eq!(index_shape(conn, INDEX).await, None, "the index is gone");

    CreateArticles.down(&manager).await.expect("down");
}

/// `Schema::table` adds a full-text index, in another language, to a table
/// that already holds rows, and a search in that language finds them.
///
/// The table never had a full-text index before: MariaDB 12.3 leaves the
/// rows already in a table out of a `FULLTEXT` index added after an earlier
/// one, whichever statement adds it.
async fn full_text_added_to_a_table_with_rows(conn: &DatabaseConnection) {
    let manager = SchemaManager::new(conn);
    Schema::drop_if_exists(&manager, ARTICLES)
        .await
        .expect("a clean start");
    Schema::create(&manager, ARTICLES, articles_with(|_| {}))
        .await
        .expect("create");
    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));
    seed_articles().await;

    Schema::table(&manager, ARTICLES, |t| {
        t.full_text(&["body"]).language("simple");
    })
    .await
    .expect("Schema::table adds a full-text index to an existing table");
    let shape = index_shape(conn, "ft_articles_body_fulltext")
        .await
        .expect("the added index exists");
    match conn.get_database_backend() {
        DbBackend::Postgres => {
            assert!(
                shape.contains("to_tsvector('simple'::regconfig, body)"),
                "{shape}"
            );
        }
        _ => assert_eq!(shape, "FULLTEXT"),
    }
    assert_eq!(
        titles(DB::table(ARTICLES).where_full_text_with(
            ["body"],
            "patience",
            FullTextOptions::new().language("simple"),
        ))
        .await,
        vec!["Cooking"]
    );

    Schema::drop(&manager, ARTICLES).await.expect("drop");
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_full_text_index_finds_a_row_by_a_word_in_its_body() {
    full_text_round_trip(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_schema_table_adds_a_full_text_index_to_a_table_with_rows() {
    full_text_added_to_a_table_with_rows(&connect_postgres().await).await;
}

/// The query's `to_tsvector` expression is the index's, so Postgres can
/// answer it from the index: with sequential scans off, the plan names it.
#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_where_full_text_is_answered_from_the_gin_index() {
    let conn = connect_postgres().await;
    let manager = SchemaManager::new(&conn);
    Schema::drop_if_exists(&manager, ARTICLES)
        .await
        .expect("a clean start");
    CreateArticles.up(&manager).await.expect("migrate");

    let (sql, values) = DB::table(ARTICLES)
        .where_full_text(["title", "body"], "laravel")
        .to_sql_for(DbBackend::Postgres)
        .expect("render");
    let txn = conn.begin().await.expect("begin");
    txn.execute_unprepared("SET LOCAL enable_seqscan = off")
        .await
        .expect("prefer the index");
    let plan = txn
        .query_all_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            format!("EXPLAIN {sql}"),
            values,
        ))
        .await
        .expect("explain")
        .iter()
        .map(|row| row.try_get_by_index::<String>(0).expect("plan line"))
        .collect::<Vec<_>>()
        .join("\n");
    txn.rollback().await.expect("rollback");
    CreateArticles.down(&manager).await.expect("down");
    assert!(plan.contains(INDEX), "{plan}");
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_full_text_index_finds_a_row_by_a_word_in_its_body() {
    full_text_round_trip(&connect_mysql().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_schema_table_adds_a_full_text_index_to_a_table_with_rows() {
    full_text_added_to_a_table_with_rows(&connect_mysql().await).await;
}
