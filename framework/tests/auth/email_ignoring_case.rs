//! AGENT-008, item 3 of the kits: a users table refuses a second account
//! whose address differs only in case
//! (`suprnova::auth::migrations::UniqueEmailIgnoringCase`).
//!
//! Sign-in, password reset and agent auth's email match all compare
//! `LOWER(email)`, so two such accounts would be one address reaching
//! either. Each engine runs the migration as a migrator does, inside a
//! transaction, twice. The PostgreSQL, MySQL and MariaDB tests each work in
//! a table of their own and drop it after.

use std::time::Duration;

use sea_orm::{
    ConnectionTrait, Database, DatabaseConnection, DbBackend, DbErr, Statement, TransactionTrait,
};
use sea_orm_migration::prelude::*;
use suprnova::auth::migrations::UniqueEmailIgnoringCase;

/// Run the migration's `up` for `table` inside a transaction.
async fn up(db: &DatabaseConnection, table: &str) -> Result<(), DbErr> {
    let transaction = db.begin().await?;
    UniqueEmailIgnoringCase::on(table, "email")
        .up(&SchemaManager::new(&transaction))
        .await?;
    transaction.commit().await
}

async fn down(db: &DatabaseConnection, table: &str) -> Result<(), DbErr> {
    UniqueEmailIgnoringCase::on(table, "email")
        .down(&SchemaManager::new(db))
        .await
}

/// Create `table` as the kits' users migration does, with the address
/// unique as written; `collation` sets the column's on MySQL.
async fn create_users(db: &DatabaseConnection, table: &str, unique: bool, collation: &str) {
    let backend = db.get_database_backend();
    let unique = if unique { " UNIQUE" } else { "" };
    let sql = match backend {
        DbBackend::Sqlite => format!(
            "CREATE TABLE \"{table}\" (id INTEGER PRIMARY KEY AUTOINCREMENT, \
             email VARCHAR(255) NOT NULL{unique})"
        ),
        DbBackend::Postgres => format!(
            "CREATE TABLE \"{table}\" (id BIGSERIAL PRIMARY KEY, \
             email VARCHAR(255) NOT NULL{unique})"
        ),
        DbBackend::MySql => format!(
            "CREATE TABLE `{table}` (id BIGINT AUTO_INCREMENT PRIMARY KEY, \
             email VARCHAR(255) {collation} NOT NULL{unique})"
        ),
        _ => panic!("an engine the framework supports"),
    };
    db.execute_unprepared(&sql).await.expect("the users table");
}

async fn insert(db: &DatabaseConnection, table: &str, email: &str) -> Result<(), DbErr> {
    let backend = db.get_database_backend();
    let sql = match backend {
        DbBackend::MySql => format!("INSERT INTO `{table}` (email) VALUES (?)"),
        DbBackend::Postgres => format!("INSERT INTO \"{table}\" (email) VALUES ($1)"),
        _ => format!("INSERT INTO \"{table}\" (email) VALUES (?)"),
    };
    db.execute_raw(Statement::from_sql_and_values(backend, sql, [email.into()]))
        .await
        .map(|_| ())
}

async fn drop_users(db: &DatabaseConnection, table: &str) {
    let sql = match db.get_database_backend() {
        DbBackend::MySql => format!("DROP TABLE IF EXISTS `{table}`"),
        _ => format!("DROP TABLE IF EXISTS \"{table}\""),
    };
    db.execute_unprepared(&sql).await.expect("drop the table");
}

/// Whether `table` has an index named `index`.
async fn has_index(db: &DatabaseConnection, table: &str, index: &str) -> bool {
    let backend = db.get_database_backend();
    let statement = match backend {
        DbBackend::Sqlite => Statement::from_sql_and_values(
            backend,
            "SELECT 1 FROM sqlite_master WHERE type = 'index' AND tbl_name = ? AND name = ?",
            [table.into(), index.into()],
        ),
        DbBackend::Postgres => Statement::from_sql_and_values(
            backend,
            "SELECT 1 FROM pg_indexes WHERE schemaname = current_schema() \
             AND tablename = $1 AND indexname = $2",
            [table.into(), index.into()],
        ),
        DbBackend::MySql => Statement::from_sql_and_values(
            backend,
            "SELECT 1 FROM information_schema.STATISTICS WHERE TABLE_SCHEMA = DATABASE() \
             AND TABLE_NAME = ? AND INDEX_NAME = ? LIMIT 1",
            [table.into(), index.into()],
        ),
        _ => panic!("an engine the framework supports"),
    };
    db.query_one_raw(statement)
        .await
        .expect("the index lookup")
        .is_some()
}

/// The engine's index definitions on `table`, sorted, so two runs compare
/// whole.
async fn indexes_of(db: &DatabaseConnection, table: &str) -> Vec<String> {
    let backend = db.get_database_backend();
    let statement = match backend {
        DbBackend::Sqlite => Statement::from_sql_and_values(
            backend,
            "SELECT name || ' ' || COALESCE(sql, '') FROM sqlite_master \
             WHERE type = 'index' AND tbl_name = ?",
            [table.into()],
        ),
        DbBackend::Postgres => Statement::from_sql_and_values(
            backend,
            "SELECT indexdef FROM pg_indexes WHERE schemaname = current_schema() \
             AND tablename = $1",
            [table.into()],
        ),
        DbBackend::MySql => Statement::from_sql_and_values(
            backend,
            "SELECT CONCAT(INDEX_NAME, ' ', COALESCE(COLUMN_NAME, ''), ' ', \
             COALESCE(EXPRESSION, ''), ' ', NON_UNIQUE) FROM information_schema.STATISTICS \
             WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ?",
            [table.into()],
        ),
        _ => panic!("an engine the framework supports"),
    };
    let mut indexes: Vec<String> = db
        .query_all_raw(statement)
        .await
        .expect("the indexes")
        .into_iter()
        .map(|row| row.try_get_by_index::<String>(0).expect("a line"))
        .collect();
    indexes.sort();
    indexes
}

/// The migration, twice, on a users table of `db`: the second run changes
/// nothing, and the table then refuses a second address differing only in
/// case.
async fn refuses_case_duplicates(db: &DatabaseConnection, table: &str, collation: &str) {
    create_users(db, table, true, collation).await;
    up(db, table).await.expect("the first run");
    let first = indexes_of(db, table).await;
    up(db, table).await.expect("the second run");
    assert_eq!(
        first,
        indexes_of(db, table).await,
        "the second run changed nothing"
    );
    assert!(has_index(db, table, &format!("{table}_email_lower_unique")).await);
    insert(db, table, "alice@example.test")
        .await
        .expect("the first account");
    let duplicate = insert(db, table, "Alice@Example.TEST").await;
    assert!(duplicate.is_err(), "a case-only duplicate is refused");
    insert(db, table, "bob@example.test")
        .await
        .expect("another address");
    drop_users(db, table).await;
}

/// A table name no other run uses.
fn unique_table() -> String {
    format!("email_case_users_{}", uuid::Uuid::new_v4().simple())
}

/// A disposable database the variable `variable` names.
async fn engine(variable: &str) -> DatabaseConnection {
    let url = std::env::var(variable)
        .unwrap_or_else(|_| panic!("set {variable} to a disposable database"));
    let mut options = sea_orm::ConnectOptions::new(url);
    options
        .max_connections(2)
        .min_connections(0)
        .connect_timeout(Duration::from_secs(5))
        .acquire_timeout(Duration::from_secs(5));
    Database::connect(options)
        .await
        .expect("the test database must be reachable")
}

#[tokio::test]
async fn agent_008_users_email_is_unique_ignoring_case_on_sqlite() {
    let db = Database::connect("sqlite::memory:").await.expect("SQLite");
    // SQLite's own `UNIQUE` compares case: without the migration both
    // addresses would be stored.
    create_users(&db, "plain_users", true, "").await;
    insert(&db, "plain_users", "alice@example.test")
        .await
        .expect("first");
    insert(&db, "plain_users", "ALICE@example.test")
        .await
        .expect("SQLite's unique column alone takes it");
    refuses_case_duplicates(&db, "users", "").await;
    // `down` drops the index, and the duplicate goes in again.
    create_users(&db, "users", true, "").await;
    up(&db, "users").await.expect("up");
    down(&db, "users").await.expect("down");
    assert!(!has_index(&db, "users", "users_email_lower_unique").await);
    insert(&db, "users", "carol@example.test")
        .await
        .expect("first");
    insert(&db, "users", "CAROL@example.test")
        .await
        .expect("without the index, case differs");
    down(&db, "users")
        .await
        .expect("down twice changes nothing");
}

#[tokio::test]
async fn agent_008_email_index_migration_refuses_existing_case_duplicates() {
    let db = Database::connect("sqlite::memory:").await.expect("SQLite");
    create_users(&db, "users", false, "").await;
    for email in [
        "zelda@mailbox.test",
        "ZELDA@mailbox.test",
        "quincy@mailbox.test",
        "Quincy@Mailbox.test",
        "quincy@MAILBOX.test",
        "wolfgang@mailbox.test",
    ] {
        insert(&db, "users", email).await.expect("an account");
    }
    let refused = up(&db, "users")
        .await
        .expect_err("duplicates stop the migration");
    let message = refused.to_string();
    assert!(message.contains("2 address(es)"), "{message}");
    assert!(message.contains("Merge"), "{message}");
    for email in ["zelda", "quincy", "wolfgang", "mailbox"] {
        assert!(
            !message.to_lowercase().contains(email),
            "{email} in {message}"
        );
    }
    assert!(!has_index(&db, "users", "users_email_lower_unique").await);
}

#[tokio::test]
async fn agent_008_email_index_migration_refuses_an_identifier_it_cannot_quote() {
    let db = Database::connect("sqlite::memory:").await.expect("SQLite");
    create_users(&db, "users", true, "").await;
    let refused = up(&db, "users\"; DROP TABLE users; --")
        .await
        .expect_err("a name the SQL cannot carry");
    assert!(refused.to_string().contains("identifier"), "{refused}");
    // The table is still there.
    insert(&db, "users", "gina@example.test")
        .await
        .expect("the table");
}

#[tokio::test]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn agent_008_users_email_is_unique_ignoring_case_on_postgres() {
    let db = engine("PG_TEST_URL").await;
    refuses_case_duplicates(&db, &unique_table(), "").await;
}

#[tokio::test]
#[ignore = "requires disposable MySQL 8.0.13 or later at MYSQL_TEST_URL"]
async fn agent_008_users_email_is_unique_ignoring_case_on_mysql() {
    let db = engine("MYSQL_TEST_URL").await;
    // A binary collation makes the column's own unique index compare case,
    // so the refusal below is the functional index's.
    refuses_case_duplicates(&db, &unique_table(), "COLLATE utf8mb4_bin").await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB at MARIADB_TEST_URL"]
async fn agent_008_users_email_on_mariadb_relies_on_its_ci_collation() {
    let db = engine("MARIADB_TEST_URL").await;
    let table = unique_table();
    create_users(&db, &table, true, "").await;
    // MariaDB has no functional index: the migration adds none, twice.
    up(&db, &table).await.expect("the first run");
    up(&db, &table).await.expect("the second run");
    assert!(!has_index(&db, &table, &format!("{table}_email_lower_unique")).await);
    // The column's default `_ci` collation makes its unique index compare
    // without case.
    insert(&db, &table, "alice@example.test")
        .await
        .expect("first");
    let duplicate = insert(&db, &table, "ALICE@example.test").await;
    assert!(duplicate.is_err(), "the _ci unique index refuses it");
    down(&db, &table).await.expect("down changes nothing");
    drop_users(&db, &table).await;
}
