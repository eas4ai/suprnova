//! `DB::table` writes of a `u64` above `i64::MAX` (PAR-044 follow-ups).
//!
//! An attribute map carries a number as JSON, and a JSON `u64` above
//! `i64::MAX` used to bind as text: Postgres refused the statement and
//! SQLite stored a rounded real. MySQL's unsigned columns hold the value
//! exactly; Postgres and SQLite have no integer column that holds it, so
//! the write is refused there before anything is sent, as a model's is.

use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use serial_test::serial;
use suprnova::testing::TestContainer;
use suprnova::{DB, DbConnection, attrs};

use super::cases::{count, drop_tables};
use super::mysql::connect_mysql;
use super::postgres::connect_postgres;
use super::sqlite::connect_sqlite;
use super::unsigned_keys::{TABLES, create_tables};

/// `quantity` of the order labelled `label`, as the database stores it,
/// read through text so no decoder stands between the test and the value.
async fn stored_quantity(conn: &DatabaseConnection, label: &str) -> Option<String> {
    let backend = conn.get_database_backend();
    let text = if backend == DbBackend::MySql {
        "CAST(quantity AS CHAR)"
    } else {
        "CAST(quantity AS TEXT)"
    };
    conn.query_one_raw(Statement::from_string(
        backend,
        format!("SELECT {text} AS v FROM uk_orders WHERE label = '{label}'"),
    ))
    .await
    .expect("read the stored quantity")
    .expect("the row")
    .try_get("", "v")
    .expect("the stored quantity as text")
}

/// On Postgres and SQLite `insert` and `update` of a `u64` above
/// `i64::MAX` fail with a database error naming the column, nothing is
/// sent, and nothing is stored. On MySQL both store the exact value. It
/// fails while the value binds as text, which SQLite stores as a rounded
/// real and Postgres refuses only after the statement is sent.
pub async fn table_writes_beyond_a_signed_column(conn: &DatabaseConnection) {
    create_tables(conn).await;
    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));
    DB::table("uk_orders")
        .insert(attrs! { label: "kept", quantity: 1 })
        .await
        .expect("insert a small quantity");
    let rows = count(conn, "uk_orders").await;
    let top = u64::MAX;
    let below = u64::MAX - 1;

    if conn.get_database_backend() == DbBackend::MySql {
        DB::table("uk_orders")
            .insert(attrs! { label: "top", quantity: top })
            .await
            .expect("MySQL's unsigned column holds u64::MAX");
        assert_eq!(
            stored_quantity(conn, "top").await,
            Some(top.to_string()),
            "insert stores the exact value"
        );
        DB::table("uk_orders")
            .filter("label", "kept")
            .update(attrs! { quantity: below })
            .await
            .expect("update to a value above i64::MAX");
        assert_eq!(
            stored_quantity(conn, "kept").await,
            Some(below.to_string()),
            "update stores the exact value"
        );
        drop_tables(conn, TABLES).await;
        return;
    }

    DB::enable_query_log().expect("enable the query log");
    let insert = DB::table("uk_orders")
        .insert(attrs! { label: "top", quantity: top })
        .await
        .expect_err("no signed column holds u64::MAX");
    let update = DB::table("uk_orders")
        .filter("label", "kept")
        .update(attrs! { quantity: below })
        .await
        .expect_err("no signed column holds u64::MAX - 1");
    assert!(
        DB::get_query_log().expect("query log").is_empty(),
        "nothing reached the database"
    );
    DB::disable_query_log().expect("disable the query log");
    for (operation, error, value) in [("insert", insert, top), ("update", update, below)] {
        let message = error.to_string();
        assert!(
            message.contains("uk_orders.quantity") && message.contains(&value.to_string()),
            "{operation}: the error names the column and the value: {message}"
        );
        assert!(
            matches!(error, suprnova::FrameworkError::Database(_)),
            "{operation}: a database error, which a client sees as a generic 500: {error:?}"
        );
    }
    assert_eq!(count(conn, "uk_orders").await, rows, "nothing was inserted");
    assert_eq!(
        stored_quantity(conn, "kept").await.as_deref(),
        Some("1"),
        "nothing was updated"
    );

    drop_tables(conn, TABLES).await;
}

#[tokio::test]
#[serial]
async fn sqlite_table_writes_beyond_a_signed_column() {
    table_writes_beyond_a_signed_column(&connect_sqlite().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_table_writes_beyond_a_signed_column() {
    table_writes_beyond_a_signed_column(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_table_writes_beyond_a_signed_column() {
    table_writes_beyond_a_signed_column(&connect_mysql().await).await;
}
