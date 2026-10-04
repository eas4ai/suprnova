//! A signed integer column compared with a `u64` above `i64::MAX` (PAR-044
//! follow-ups).
//!
//! No signed integer column holds such a value on any database, so a
//! comparison with one has the same answer as on a `u64` column of
//! Postgres or SQLite: `=`, `>`, `>=` and `IN` match no row, and `!=`,
//! `<>`, `<`, `<=` and `NOT IN` match every row whose column is not NULL.
//! MySQL compares the value natively; Postgres and SQLite get the answer
//! without the value, which they cannot bind.

use sea_orm::{DatabaseConnection, DbBackend};
use sea_orm_migration::prelude::*;
use serial_test::serial;
use suprnova::eloquent::Builder;
use suprnova::schema::Schema;
use suprnova::testing::TestContainer;
use suprnova::{DbConnection, FrameworkError, Model, attrs, model};

use super::cases::drop_tables;
use super::mysql::connect_mysql;
use super::postgres::connect_postgres;
use super::sqlite::connect_sqlite;
use super::unsigned_reads::BEYOND;

/// Signed integers of three widths: the key, a nullable `BIGINT` and an
/// `INTEGER`.
#[model(table = "sc_entries", fillable = ["score", "tier", "label"])]
pub struct ScEntry {
    pub id: i64,
    pub score: Option<i64>,
    pub tier: i32,
    pub label: String,
}

async fn create_entries(conn: &DatabaseConnection) {
    let manager = SchemaManager::new(conn);
    drop_tables(conn, &["sc_entries"]).await;
    Schema::create(&manager, "sc_entries", |t| {
        t.id();
        t.big_integer("score").nullable();
        t.integer("tier");
        t.string("label");
    })
    .await
    .expect("create sc_entries");
}

/// The labels of the entries `query` matches, sorted.
async fn labels(query: Builder<ScEntry>) -> Vec<String> {
    let mut labels: Vec<String> = query
        .get()
        .await
        .expect("the query runs")
        .iter()
        .map(|entry| entry.label.clone())
        .collect();
    labels.sort();
    labels
}

/// Every comparison of a signed column with a value above `i64::MAX` gives
/// the same answer on SQLite, Postgres and MySQL, on a nullable `BIGINT`, an
/// `INTEGER` and the key. On Postgres and SQLite a mass update to such a
/// value is refused as a database error and changes nothing. It fails while
/// the value binds as text, which Postgres refuses to compare with an
/// integer column.
pub async fn signed_columns_compare_with_a_u64_beyond_them(conn: &DatabaseConnection) {
    create_entries(conn).await;
    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));
    for (label, score, tier) in [("a", Some(1i64), 1i32), ("b", Some(5), 2), ("c", None, 3)] {
        ScEntry::create(attrs! { label: label, score: score, tier: tier })
            .await
            .expect("create an entry");
    }
    let none: Vec<&str> = Vec::new();
    let held = vec!["a", "b"];
    let all = vec!["a", "b", "c"];

    for beyond in BEYOND {
        let cases: Vec<(&str, Builder<ScEntry>, Vec<&str>)> = vec![
            (
                "filter",
                ScEntry::query().filter("score", beyond),
                none.clone(),
            ),
            (
                "=",
                ScEntry::query().filter_op("score", "=", beyond),
                none.clone(),
            ),
            (
                "!=",
                ScEntry::query().filter_op("score", "!=", beyond),
                held.clone(),
            ),
            (
                "<>",
                ScEntry::query().filter_op("score", "<>", beyond),
                held.clone(),
            ),
            (
                "<",
                ScEntry::query().filter_op("score", "<", beyond),
                held.clone(),
            ),
            (
                "<=",
                ScEntry::query().filter_op("score", "<=", beyond),
                held.clone(),
            ),
            (
                ">",
                ScEntry::query().filter_op("score", ">", beyond),
                none.clone(),
            ),
            (
                ">=",
                ScEntry::query().filter_op("score", ">=", beyond),
                none.clone(),
            ),
            (
                "IN",
                ScEntry::query().where_in("score", vec![beyond]),
                none.clone(),
            ),
            (
                "IN with a held value",
                ScEntry::query().where_in("score", vec![beyond, 1]),
                vec!["a"],
            ),
            (
                "NOT IN",
                ScEntry::query().where_not_in("score", vec![beyond]),
                held.clone(),
            ),
            (
                "BETWEEN up to it",
                ScEntry::query().where_between("score", 2..=beyond),
                vec!["b"],
            ),
            (
                "NOT =",
                ScEntry::query().filter_not("score", beyond),
                held.clone(),
            ),
            (
                "NOT !=",
                ScEntry::query().where_none(["score"], "!=", beyond),
                none.clone(),
            ),
            (
                "an INTEGER column",
                ScEntry::query().filter("tier", beyond),
                none.clone(),
            ),
            (
                "below it on an INTEGER column",
                ScEntry::query().filter_op("tier", "<", beyond),
                all.clone(),
            ),
            (
                "the key",
                ScEntry::query().filter("id", beyond),
                none.clone(),
            ),
            (
                "below it on the key",
                ScEntry::query().filter_op("id", "<", beyond),
                all.clone(),
            ),
        ];
        for (name, query, expected) in cases {
            assert_eq!(labels(query).await, expected, "{name} {beyond}");
        }
    }

    if conn.get_database_backend() != DbBackend::MySql {
        let error = ScEntry::query()
            .update_all(attrs! { score: u64::MAX })
            .await
            .expect_err("no signed column holds u64::MAX");
        assert!(
            matches!(error, FrameworkError::Database(ref message) if message.contains("score")),
            "a database error naming the column: {error:?}"
        );
        assert_eq!(
            labels(ScEntry::query().filter("score", 5)).await,
            vec!["b"],
            "nothing was written"
        );
    }

    drop_tables(conn, &["sc_entries"]).await;
}

#[tokio::test]
async fn sqlite_signed_columns_compare_with_a_u64_beyond_them() {
    signed_columns_compare_with_a_u64_beyond_them(&connect_sqlite().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_signed_columns_compare_with_a_u64_beyond_them() {
    signed_columns_compare_with_a_u64_beyond_them(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_signed_columns_compare_with_a_u64_beyond_them() {
    signed_columns_compare_with_a_u64_beyond_them(&connect_mysql().await).await;
}
