//! Laravel 13.27 `whereBinary` - byte-exact comparison, and the backend
//! split it forces.
//!
//! MySQL and MariaDB implement it as the `binary` operator modifier
//! (`col = binary ?`). Postgres and SQLite have no equivalent, so both
//! builders refuse when the statement renders rather than degrading to a
//! plain `=`, which would compare under the column's collation and match
//! rows the caller asked to exclude. Laravel throws in the base grammar;
//! Suprnova returns `Err`, per the house rule that public-surface code
//! does not panic.

use sea_orm::DbBackend;
use suprnova::testing::TestDatabase;
use suprnova::{DB, Model, attrs, model};

#[model(table = "bin_users", timestamps = false, relations = {
    posts: HasMany<BinPost>,
})]
pub struct BinUser {
    pub id: i64,
    pub name: String,
    pub email: String,
}

#[model(table = "bin_posts", timestamps = false)]
pub struct BinPost {
    pub id: i64,
    pub bin_user_id: i64,
    pub title: String,
}

async fn migrate(db: &TestDatabase) {
    db.execute_unprepared(
        "CREATE TABLE bin_users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            email TEXT NOT NULL
        )",
    )
    .await
    .expect("create table");
    db.execute_unprepared(
        "CREATE TABLE bin_posts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            bin_user_id INTEGER NOT NULL,
            title TEXT NOT NULL
        )",
    )
    .await
    .expect("create table");
}

// ---- MySQL / MariaDB rendering ----------------------------------------

#[test]
fn where_binary_renders_the_mysql_operator_modifier() {
    let (sql, vals) = BinUser::query()
        .where_binary("name", "Alice")
        .try_to_sql_with_bindings_for(DbBackend::MySql)
        .expect("MySQL supports binary comparison");
    assert!(
        sql.contains("WHERE name = binary ?"),
        "expected the `= binary` modifier; got: {sql}"
    );
    assert_eq!(vals.len(), 1, "the value stays bound; got: {vals:?}");
}

#[test]
fn where_not_binary_renders_the_negated_modifier() {
    let (sql, _vals) = BinUser::query()
        .where_not_binary("name", "Alice")
        .try_to_sql_with_bindings_for(DbBackend::MySql)
        .expect("MySQL supports binary comparison");
    assert!(
        sql.contains("WHERE name != binary ?"),
        "expected the negated modifier; got: {sql}"
    );
}

#[test]
fn or_where_binary_folds_into_the_preceding_clause() {
    let (sql, vals) = BinUser::query()
        .filter("email", "a@x.com")
        .or_where_binary("name", "Alice")
        .or_where_not_binary("name", "Bob")
        .try_to_sql_with_bindings_for(DbBackend::MySql)
        .expect("MySQL supports binary comparison");
    assert!(
        sql.contains("(email = ? OR name = binary ? OR name != binary ?)"),
        "consecutive or_* calls stay in one flat group; got: {sql}"
    );
    assert_eq!(vals.len(), 3, "got: {vals:?}");
}

#[test]
fn filter_binary_aliases_match_the_laravel_spelling() {
    let laravel = BinUser::query()
        .filter("email", "a@x.com")
        .or_where_binary("name", "Alice")
        .try_to_sql_with_bindings_for(DbBackend::MySql)
        .expect("MySQL supports binary comparison")
        .0;
    let rust_shape = BinUser::query()
        .filter("email", "a@x.com")
        .or_filter_binary("name", "Alice")
        .try_to_sql_with_bindings_for(DbBackend::MySql)
        .expect("MySQL supports binary comparison")
        .0;
    assert_eq!(laravel, rust_shape);
}

// ---- Postgres / SQLite refusal ----------------------------------------

#[test]
fn where_binary_refuses_on_postgres() {
    let err = BinUser::query()
        .where_binary("name", "Alice")
        .try_to_sql_with_bindings_for(DbBackend::Postgres)
        .expect_err("Postgres has no binary comparison operator");
    assert!(
        format!("{err}").contains("where_binary is not supported"),
        "got: {err}"
    );
}

#[test]
fn where_binary_refuses_on_sqlite() {
    let err = BinUser::query()
        .where_binary("name", "Alice")
        .try_to_sql_with_bindings_for(DbBackend::Sqlite)
        .expect_err("SQLite has no binary comparison operator");
    assert!(
        format!("{err}").contains("where_binary is not supported"),
        "got: {err}"
    );
}

#[tokio::test]
async fn eloquent_terminal_surfaces_the_refusal_instead_of_running_the_query() {
    let db = TestDatabase::sqlite_memory().await.expect("sqlite");
    migrate(&db).await;
    BinUser::create(attrs! { name: "Alice", email: "a@x.com" })
        .await
        .expect("insert the seed row");

    let err = BinUser::query()
        .where_binary("name", "alice")
        .get()
        .await
        .expect_err("the terminal must refuse, not fall back to a collation match");
    assert!(
        format!("{err}").contains("where_binary is not supported"),
        "got: {err}"
    );
}

#[tokio::test]
async fn db_table_terminal_surfaces_the_refusal() {
    let db = TestDatabase::sqlite_memory().await.expect("sqlite");
    migrate(&db).await;

    let err = DB::table("bin_users")
        .where_binary("name", "alice")
        .get()
        .await
        .expect_err("the model-less builder refuses on SQLite too");
    assert!(
        format!("{err}").contains("where_binary is not supported"),
        "got: {err}"
    );
}

// ---- Identifier trust boundary ----------------------------------------

#[tokio::test]
async fn injection_in_the_binary_column_is_rejected_at_terminal() {
    let db = TestDatabase::sqlite_memory().await.expect("sqlite");
    migrate(&db).await;

    let err = BinUser::query()
        .where_binary("name) OR (1=1", "Alice")
        .get()
        .await
        .expect_err("attacker-controlled binary-compare column must be rejected");
    assert!(
        format!("{err}").contains("SQL identifier"),
        "identifier validation must bite before the backend gate; got: {err}"
    );
}

// ---- Correlated subquery ----------------------------------------------

#[test]
fn binary_term_renders_inside_a_where_has_subquery() {
    // `render_subquery_term` is a second exhaustive match over
    // `WhereTerm` and it qualifies bare columns with the target table.
    // This pins that it learned the Binary arm, qualifier and all.
    let (sql, vals) = BinUser::query()
        .where_has::<BinPost, _>("posts", |q| q.where_binary("title", "Rust"))
        .try_to_sql_with_bindings_for(DbBackend::MySql)
        .expect("MySQL supports binary comparison");
    assert!(
        sql.contains("bin_posts.title = binary ?"),
        "the subquery renderer must qualify the column and keep the modifier; got: {sql}"
    );
    assert_eq!(vals.len(), 1, "got: {vals:?}");
}

#[test]
fn binary_term_inside_a_subquery_also_refuses_on_postgres() {
    let err = BinUser::query()
        .where_has::<BinPost, _>("posts", |q| q.where_binary("title", "Rust"))
        .try_to_sql_with_bindings_for(DbBackend::Postgres)
        .expect_err("the refusal must reach into correlated subqueries too");
    assert!(
        format!("{err}").contains("where_binary is not supported"),
        "got: {err}"
    );
}
