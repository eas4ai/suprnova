//! A `u64` above `i64::MAX` compared with, or written to, a column whose
//! type the query builder does not know (PAR-044 follow-ups).
//!
//! `DB::table`, a raw fragment and a joined table's column carry no column
//! type, so the value binds as the exact number the engine compares it as:
//! `numeric` on Postgres, its digits on SQLite (which applies the column's
//! affinity, as for a literal), and an unsigned integer on MySQL. Each count
//! must equal the engine's own count for the stored rows, whatever the
//! column's type. A write never stores a rounded value: an integer column
//! refuses it, while numeric and text columns store it exactly.

use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, Value};
use serial_test::serial;
use suprnova::testing::TestContainer;
use suprnova::{DB, DatabaseUserProvider, DbConnection, Model, UserProvider, attrs};

use super::cases::{drop_tables, run};
use super::mysql::connect_mysql;
use super::postgres::connect_postgres;
use super::sqlite::connect_sqlite;
use super::unsigned_keys::{TABLES, UkOrder, create_tables};
use super::unsigned_reads::{BEYOND, seed_orders};

/// A table of every kind of column a large number can meet: an exact
/// decimal, a double, text and a signed integer.
async fn create_amounts(conn: &DatabaseConnection) {
    let backend = conn.get_database_backend();
    let (key, numeric, double) = match backend {
        DbBackend::Postgres => ("BIGSERIAL PRIMARY KEY", "NUMERIC(20,0)", "DOUBLE PRECISION"),
        DbBackend::MySql => (
            "BIGINT AUTO_INCREMENT PRIMARY KEY",
            "DECIMAL(20,0)",
            "DOUBLE",
        ),
        _ => ("INTEGER PRIMARY KEY AUTOINCREMENT", "NUMERIC", "REAL"),
    };
    drop_tables(conn, &["ux_amounts"]).await;
    run(
        conn,
        &format!(
            "CREATE TABLE ux_amounts (id {key}, big {numeric}, ratio {double}, \
             code TEXT, small BIGINT, label TEXT)"
        ),
    )
    .await
    .expect("create ux_amounts");
}

/// The engine's own count for `condition` over `ux_amounts`.
async fn truth(conn: &DatabaseConnection, condition: &str) -> u64 {
    let backend = conn.get_database_backend();
    let row = conn
        .query_one_raw(Statement::from_string(
            backend,
            format!("SELECT COUNT(*) AS n FROM ux_amounts WHERE {condition}"),
        ))
        .await
        .expect("the engine's count")
        .expect("a row");
    let n: i64 = row.try_get("", "n").expect("the count");
    n as u64
}

/// `column` of the row labelled `label`, read through text.
async fn stored(conn: &DatabaseConnection, column: &str, label: &str) -> Option<String> {
    let text = if conn.get_database_backend() == DbBackend::MySql {
        format!("CAST({column} AS CHAR)")
    } else {
        format!("CAST({column} AS TEXT)")
    };
    conn.query_one_raw(Statement::from_string(
        conn.get_database_backend(),
        format!("SELECT {text} AS v FROM ux_amounts WHERE label = '{label}'"),
    ))
    .await
    .expect("read the stored value")
    .and_then(|row| row.try_get::<Option<String>>("", "v").ok().flatten())
}

/// Comparisons with a value above `i64::MAX` on a numeric, a double and an
/// integer column through `DB::table`, a raw fragment and a model's own
/// column give the engine's own count. On SQLite an INTEGER column can hold
/// a REAL above `i64::MAX`, and the count includes it. It fails while the
/// framework settles the comparison from the value alone, as if every
/// column were a signed integer one.
pub async fn comparisons_match_the_engine(conn: &DatabaseConnection) {
    create_amounts(conn).await;
    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));
    let backend = conn.get_database_backend();
    run(
        conn,
        "INSERT INTO ux_amounts (big, ratio, code, small, label) VALUES \
         (18446744073709551615, 1e19, '18446744073709551615', 1, 'top'), \
         (5, 5, '5', 5, 'small')",
    )
    .await
    .expect("insert the rows");
    if backend == DbBackend::Sqlite {
        // An INTEGER column holds a REAL above i64::MAX, as raw SQL, an
        // older write or a text binding leaves one.
        run(
            conn,
            "INSERT INTO ux_amounts (small, label) VALUES (1.8446744073709552e19, 'real')",
        )
        .await
        .expect("insert a REAL into an INTEGER column");
    }
    let amounts = || DB::table("ux_amounts");
    let cases: Vec<(&str, u64, Result<u64, suprnova::FrameworkError>)> = vec![
        (
            "big = 18446744073709551615",
            truth(conn, "big = 18446744073709551615").await,
            amounts().filter("big", u64::MAX).count().await,
        ),
        (
            "big IN (18446744073709551615)",
            truth(conn, "big IN (18446744073709551615)").await,
            amounts().where_in("big", vec![u64::MAX]).count().await,
        ),
        (
            "big < 18446744073709551615",
            truth(conn, "big < 18446744073709551615").await,
            amounts().filter_op("big", "<", u64::MAX).count().await,
        ),
        (
            "ratio > 9223372036854775808",
            truth(conn, "ratio > 9223372036854775808").await,
            amounts().filter_op("ratio", ">", BEYOND[0]).count().await,
        ),
        (
            "small > 9223372036854775808",
            truth(conn, "small > 9223372036854775808").await,
            amounts().filter_op("small", ">", BEYOND[0]).count().await,
        ),
        (
            "small = 18446744073709551615",
            truth(conn, "small = 18446744073709551615").await,
            amounts().filter("small", u64::MAX).count().await,
        ),
        (
            "where_raw big = ?",
            truth(conn, "big = 18446744073709551615").await,
            amounts()
                .where_raw("big = ?", vec![Value::from(u64::MAX)])
                .count()
                .await,
        ),
    ];
    for (condition, expected, framework) in cases {
        assert_eq!(
            framework.unwrap_or_else(|error| panic!("{condition}: {error}")),
            expected,
            "{condition}: the engine's own count"
        );
    }

    drop_tables(conn, &["ux_amounts"]).await;
}

/// A model column of a joined table, which the model's binder does not
/// know, compares with a value above `i64::MAX` as the engine does. It
/// fails while the value binds as text, which Postgres refuses to compare
/// with an integer column.
pub async fn a_joined_column_compares_as_the_engine_does(conn: &DatabaseConnection) {
    create_tables(conn).await;
    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));
    seed_orders().await;
    let joined = || {
        UkOrder::query().join(
            "uk_customers",
            "uk_customers.id",
            "=",
            "uk_orders.uk_customer_id",
        )
    };
    for beyond in BEYOND {
        assert_eq!(
            joined()
                .filter("uk_customers.id", beyond)
                .count()
                .await
                .expect("= on a joined column"),
            0,
            "= {beyond}"
        );
        assert_eq!(
            joined()
                .filter_op("uk_customers.id", "<", beyond)
                .count()
                .await
                .expect("< on a joined column"),
            3,
            "< {beyond}"
        );
        assert_eq!(
            UkOrder::query()
                .where_raw("quantity < ?", vec![beyond.into()])
                .count()
                .await
                .expect("a raw fragment"),
            2,
            "where_raw {beyond}"
        );
    }
    drop_tables(conn, TABLES).await;
}

/// `DB::table` writes of `u64::MAX`: a numeric and a text column store it
/// exactly, a double stores the nearest double as it does any number, and
/// an integer column refuses it with nothing stored. SQLite stores a large
/// number in a NUMERIC column as a rounded REAL, so there that write is
/// refused too. It fails while every such write is refused, the text one
/// included.
pub async fn table_writes_follow_the_column(conn: &DatabaseConnection) {
    create_amounts(conn).await;
    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));
    let backend = conn.get_database_backend();
    let digits = u64::MAX.to_string();

    DB::table("ux_amounts")
        .insert(attrs! { code: u64::MAX, label: "text" })
        .await
        .expect("a text column takes the digits");
    assert_eq!(
        stored(conn, "code", "text").await.as_deref(),
        Some(digits.as_str())
    );
    DB::table("ux_amounts")
        .filter("label", "text")
        .update(attrs! { code: u64::MAX - 1 })
        .await
        .expect("an update of a text column");
    assert_eq!(
        stored(conn, "code", "text").await,
        Some((u64::MAX - 1).to_string())
    );

    DB::table("ux_amounts")
        .insert(attrs! { ratio: u64::MAX, label: "double" })
        .await
        .expect("a double column takes the nearest double");

    let numeric = DB::table("ux_amounts")
        .insert(attrs! { big: u64::MAX, label: "numeric" })
        .await;
    if backend == DbBackend::Sqlite {
        assert!(numeric.is_err(), "SQLite would store a rounded REAL");
        assert_eq!(stored(conn, "big", "numeric").await, None, "nothing stored");
    } else {
        numeric.expect("an exact decimal column takes the value");
        assert_eq!(stored(conn, "big", "numeric").await, Some(digits.clone()));
    }

    assert!(
        DB::table("ux_amounts")
            .insert(attrs! { small: u64::MAX, label: "integer" })
            .await
            .is_err(),
        "no signed integer column holds u64::MAX"
    );
    assert_eq!(
        truth(conn, "label = 'integer'").await,
        0,
        "nothing was inserted"
    );

    drop_tables(conn, &["ux_amounts"]).await;
}

/// `DatabaseUserProvider` over a text identifier column finds a user whose
/// identifier is a 20-digit number, on every database, and over an integer
/// key answers no such user for an id above `i64::MAX` on Postgres and
/// SQLite. It fails while a 20-digit id binds as a number against a text
/// column.
pub async fn a_text_identifier_finds_a_twenty_digit_id(conn: &DatabaseConnection) {
    create_amounts(conn).await;
    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));
    run(
        conn,
        "INSERT INTO ux_amounts (code, small, label) VALUES \
         ('18446744073709551615', 1, 'top'), ('18446744073709551614', 2, 'below')",
    )
    .await
    .expect("insert the users");

    let by_code = DatabaseUserProvider::new("ux_amounts").identifier_column("code");
    for (id, label) in [
        (u64::MAX.to_string(), "top"),
        ((u64::MAX - 1).to_string(), "below"),
    ] {
        let user = by_code
            .retrieve_by_id(&id)
            .await
            .expect("a lookup by a text identifier")
            .unwrap_or_else(|| panic!("the user {id}"));
        assert_eq!(user.get_auth_identifier(), id, "{label}");
    }
    let by_id = DatabaseUserProvider::new("ux_amounts");
    if conn.get_database_backend() != DbBackend::MySql {
        for beyond in BEYOND {
            assert!(
                by_id
                    .retrieve_by_id(&beyond.to_string())
                    .await
                    .unwrap_or_else(|error| panic!("a lookup by an integer key {beyond}: {error}"))
                    .is_none(),
                "no signed key holds an id above i64::MAX"
            );
        }
    }

    drop_tables(conn, &["ux_amounts"]).await;
}

#[tokio::test]
async fn sqlite_comparisons_match_the_engine() {
    comparisons_match_the_engine(&connect_sqlite().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_comparisons_match_the_engine() {
    comparisons_match_the_engine(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_comparisons_match_the_engine() {
    comparisons_match_the_engine(&connect_mysql().await).await;
}

#[tokio::test]
async fn sqlite_a_joined_column_compares_as_the_engine_does() {
    a_joined_column_compares_as_the_engine_does(&connect_sqlite().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_a_joined_column_compares_as_the_engine_does() {
    a_joined_column_compares_as_the_engine_does(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_a_joined_column_compares_as_the_engine_does() {
    a_joined_column_compares_as_the_engine_does(&connect_mysql().await).await;
}

#[tokio::test]
async fn sqlite_table_writes_follow_the_column() {
    table_writes_follow_the_column(&connect_sqlite().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_table_writes_follow_the_column() {
    table_writes_follow_the_column(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_table_writes_follow_the_column() {
    table_writes_follow_the_column(&connect_mysql().await).await;
}

#[tokio::test]
async fn sqlite_a_text_identifier_finds_a_twenty_digit_id() {
    a_text_identifier_finds_a_twenty_digit_id(&connect_sqlite().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_a_text_identifier_finds_a_twenty_digit_id() {
    a_text_identifier_finds_a_twenty_digit_id(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_a_text_identifier_finds_a_twenty_digit_id() {
    a_text_identifier_finds_a_twenty_digit_id(&connect_mysql().await).await;
}
