//! `DB::table` comparisons with a `u64` above `i64::MAX` (PAR-044
//! follow-ups).
//!
//! The model-less builder takes its values as SeaORM values, so a `u64`
//! arrives as an unsigned number. On Postgres and SQLite no integer column
//! holds one above `i64::MAX`, and a comparison with it has the meaning
//! the model builder gives it, which is MySQL's answer for the same rows.

use sea_orm::DatabaseConnection;
use serial_test::serial;
use suprnova::database::DbTableBuilder;
use suprnova::testing::TestContainer;
use suprnova::{DB, DbConnection, Model};

use super::cases::drop_tables;
use super::mysql::connect_mysql;
use super::postgres::connect_postgres;
use super::sqlite::connect_sqlite;
use super::unsigned_keys::{TABLES, UkOrder, create_tables};
use super::unsigned_reads::{BEYOND, seed_orders};

/// `DB::table` gives a comparison with a value above `i64::MAX` the same
/// meaning as the model builder, on every database, through `filter`,
/// `filter_op`, `where_in`, `where_not_in`, the grouped `where_any` and
/// `where_none`, a join's `ON` condition, and as a subquery of the model
/// builder. It fails while Postgres and SQLite refuse the parameter
/// instead.
pub async fn table_reads_beyond_a_signed_column_answer_truthfully(conn: &DatabaseConnection) {
    create_tables(conn).await;
    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));
    seed_orders().await;

    for beyond in BEYOND {
        let orders = || DB::table("uk_orders");
        let cases: Vec<(&str, DbTableBuilder, u64)> = vec![
            ("filter", orders().filter("quantity", beyond), 0),
            ("=", orders().filter_op("quantity", "=", beyond), 0),
            ("!=", orders().filter_op("quantity", "!=", beyond), 2),
            ("<>", orders().filter_op("quantity", "<>", beyond), 2),
            ("<", orders().filter_op("quantity", "<", beyond), 2),
            ("<=", orders().filter_op("quantity", "<=", beyond), 2),
            (">", orders().filter_op("quantity", ">", beyond), 0),
            (">=", orders().filter_op("quantity", ">=", beyond), 0),
            ("IN", orders().where_in("quantity", vec![beyond]), 0),
            (
                "IN with a held value",
                orders().where_in("quantity", vec![beyond, 1]),
                1,
            ),
            ("NOT IN", orders().where_not_in("quantity", vec![beyond]), 2),
            (
                "NOT IN with a held value",
                orders().where_not_in("quantity", vec![beyond, 1]),
                1,
            ),
            (
                "where_any",
                orders().where_any(["quantity", "uk_customer_id"], "=", beyond),
                0,
            ),
            (
                "where_none =",
                orders().where_none(["quantity"], "=", beyond),
                2,
            ),
            (
                "where_none !=",
                orders().where_none(["quantity"], "!=", beyond),
                0,
            ),
            ("on the key", orders().filter("id", beyond), 0),
            (
                "a join's ON",
                DB::table("uk_customers").join_with("uk_orders", |join| {
                    join.on("uk_orders.uk_customer_id", "=", "uk_customers.id")
                        .filter("uk_orders.quantity", beyond)
                }),
                0,
            ),
            (
                "below it in a join's ON",
                DB::table("uk_customers").join_with("uk_orders", |join| {
                    join.on("uk_orders.uk_customer_id", "=", "uk_customers.id")
                        .filter_op("uk_orders.quantity", "<", beyond)
                }),
                2,
            ),
        ];
        for (name, query, expected) in cases {
            assert_eq!(
                query.count().await.expect("the query runs"),
                expected,
                "{name} {beyond}"
            );
        }
        assert!(
            UkOrder::query()
                .where_in(
                    "id",
                    DB::table("uk_orders")
                        .select(["id"])
                        .filter("quantity", beyond),
                )
                .get()
                .await
                .expect("a model query over a subquery")
                .is_empty(),
            "IN a subquery {beyond}"
        );
    }

    drop_tables(conn, TABLES).await;
}

#[tokio::test]
async fn sqlite_table_reads_beyond_a_signed_column_answer_truthfully() {
    table_reads_beyond_a_signed_column_answer_truthfully(&connect_sqlite().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_table_reads_beyond_a_signed_column_answer_truthfully() {
    table_reads_beyond_a_signed_column_answer_truthfully(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_table_reads_beyond_a_signed_column_answer_truthfully() {
    table_reads_beyond_a_signed_column_answer_truthfully(&connect_mysql().await).await;
}
