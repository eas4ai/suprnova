//! MySQL and MariaDB checks for the time columns of a group of
//! framework-owned tables.
//!
//! MySQL's `TIMESTAMP` refuses any time after 2038-01-19 03:14:07 UTC. Every
//! framework table group proves the same two things on both engines: a table
//! its migrations create fresh holds a time after 2038, and a table an older
//! version created with `TIMESTAMP` columns is moved to `DATETIME` by those
//! migrations with every stored time kept in UTC, its nullability and its
//! `DEFAULT CURRENT_TIMESTAMP` kept, even when the migrations run over a
//! session in another time zone.
//!
//! Both checks run against `MYSQL_TEST_URL`, a disposable database: they drop
//! and recreate the group's tables.

use std::future::Future;
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, Statement, Value};

/// The local time every seeded time column holds in the `+05:00` session,
/// and the UTC time that is.
const SHIFTED_LOCAL: &str = "2030-01-02 08:04:05";
const SHIFTED_UTC: &str = "2030-01-02 03:04:05";

/// A connection to `MYSQL_TEST_URL` with one pooled connection, so the
/// session settings a check makes apply to every statement it sends. `zone`
/// replaces the driver's default UTC session time zone.
async fn connect(zone: Option<&str>) -> DatabaseConnection {
    let mut url = std::env::var("MYSQL_TEST_URL")
        .expect("set MYSQL_TEST_URL to a disposable MariaDB/MySQL database");
    if let Some(zone) = zone {
        let separator = if url.contains('?') { '&' } else { '?' };
        url = format!("{url}{separator}timezone={}", zone.replace('+', "%2B"));
    }
    let mut options = ConnectOptions::new(url);
    options
        .max_connections(1)
        .min_connections(0)
        .connect_timeout(Duration::from_secs(5))
        .acquire_timeout(Duration::from_secs(30));
    Database::connect(options)
        .await
        .expect("MariaDB/MySQL test database must be reachable")
}

async fn execute(db: &DatabaseConnection, sql: &str) {
    db.execute_unprepared(sql)
        .await
        .unwrap_or_else(|error| panic!("{sql}: {error}"));
}

/// One column of a group table, as the catalogue reports it.
#[derive(Debug)]
struct Column {
    name: String,
    kind: String,
    nullable: bool,
    default: Option<String>,
    auto_increment: bool,
}

impl Column {
    fn is_time(&self) -> bool {
        self.kind == "datetime" || self.kind == "timestamp"
    }

    fn defaults_to_now(&self) -> bool {
        self.default.as_deref().is_some_and(|default| {
            default
                .to_ascii_lowercase()
                .starts_with("current_timestamp")
        })
    }
}

async fn columns(db: &DatabaseConnection, table: &str) -> Vec<Column> {
    db.query_all_raw(Statement::from_sql_and_values(
        db.get_database_backend(),
        "SELECT COLUMN_NAME AS name, DATA_TYPE AS kind, IS_NULLABLE AS nullable, \
         COLUMN_DEFAULT AS column_default, EXTRA AS extra \
         FROM information_schema.COLUMNS \
         WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ? ORDER BY ORDINAL_POSITION",
        [table.into()],
    ))
    .await
    .unwrap_or_else(|error| panic!("read the columns of {table}: {error}"))
    .iter()
    .map(|row| {
        let text = |column: &str| -> String {
            row.try_get::<String>("", column)
                .unwrap_or_else(|error| panic!("{table}.{column}: {error}"))
        };
        let default: Option<String> = row
            .try_get("", "column_default")
            .unwrap_or_else(|error| panic!("{table} column default: {error}"));
        Column {
            name: text("name"),
            kind: text("kind").to_ascii_lowercase(),
            nullable: text("nullable").eq_ignore_ascii_case("yes"),
            // MariaDB reports the default of a nullable column as `NULL`.
            default: default.filter(|default| !default.eq_ignore_ascii_case("null")),
            auto_increment: text("extra")
                .to_ascii_lowercase()
                .contains("auto_increment"),
        }
    })
    .collect()
}

/// Every time column of `tables`: (table, column, nullable, defaults to now).
async fn time_columns(
    db: &DatabaseConnection,
    tables: &[&str],
) -> Vec<(String, String, bool, bool)> {
    let mut found = Vec::new();
    for table in tables {
        for column in columns(db, table).await {
            if column.is_time() {
                found.push((
                    (*table).to_owned(),
                    column.name.clone(),
                    column.nullable,
                    column.defaults_to_now(),
                ));
            }
        }
    }
    found
}

async fn drop_tables(db: &DatabaseConnection, tables: &[&str]) {
    execute(db, "SET FOREIGN_KEY_CHECKS = 0").await;
    for table in tables {
        execute(db, &format!("DROP TABLE IF EXISTS `{table}`")).await;
    }
    execute(db, "SET FOREIGN_KEY_CHECKS = 1").await;
}

/// Inserts one row into `table`, with `time` in every time column and a
/// placeholder in every other column that needs a value, with foreign keys
/// unchecked so the tables can be seeded in any order.
async fn seed(db: &DatabaseConnection, table: &str, time: Value) {
    let mut names = Vec::new();
    let mut values = Vec::new();
    for column in columns(db, table).await {
        let value = if column.is_time() {
            time.clone()
        } else if column.auto_increment || column.nullable || column.default.is_some() {
            continue;
        } else {
            match column.kind.as_str() {
                "char" | "varchar" | "text" | "tinytext" | "mediumtext" | "longtext" | "binary"
                | "varbinary" | "blob" | "tinyblob" | "mediumblob" | "longblob" => Value::from("x"),
                "json" => Value::from("{}"),
                "tinyint" | "smallint" | "mediumint" | "int" | "bigint" | "decimal" | "double"
                | "float" => Value::from(1_i64),
                other => panic!("no placeholder for {table}.{} of type {other}", column.name),
            }
        };
        names.push(format!("`{}`", column.name));
        values.push(value);
    }
    let placeholders = vec!["?"; values.len()].join(", ");
    let sql = format!(
        "INSERT INTO `{table}` ({}) VALUES ({placeholders})",
        names.join(", ")
    );
    execute(db, "SET FOREIGN_KEY_CHECKS = 0").await;
    db.execute_raw(Statement::from_sql_and_values(
        db.get_database_backend(),
        &sql,
        values,
    ))
    .await
    .unwrap_or_else(|error| panic!("seed {table}: {error}"));
    execute(db, "SET FOREIGN_KEY_CHECKS = 1").await;
}

/// The stored value of every time column of `tables`, as text.
async fn stored_times(
    db: &DatabaseConnection,
    tables: &[&str],
) -> Vec<(String, String, Option<String>)> {
    let mut stored = Vec::new();
    for (table, column, _, _) in time_columns(db, tables).await {
        let value: Option<String> = db
            .query_one_raw(Statement::from_string(
                db.get_database_backend(),
                format!("SELECT CAST(`{column}` AS CHAR) AS value FROM `{table}`"),
            ))
            .await
            .unwrap_or_else(|error| panic!("read {table}.{column}: {error}"))
            .unwrap_or_else(|| panic!("{table} has its seeded row"))
            .try_get("", "value")
            .unwrap_or_else(|error| panic!("{table}.{column} as text: {error}"));
        stored.push((table, column, value));
    }
    stored
}

/// The tables `migrate` creates fresh have `DATETIME` time columns, and a
/// time after 2038 written into each of them reads back unchanged.
pub async fn assert_fresh_tables_hold_times_after_2038<F, Fut>(tables: &[&str], migrate: F)
where
    F: Fn(DatabaseConnection) -> Fut,
    Fut: Future<Output = ()>,
{
    let db = connect(None).await;
    drop_tables(&db, tables).await;
    migrate(db.clone()).await;

    let columns = time_columns(&db, tables).await;
    assert!(!columns.is_empty(), "{tables:?} have time columns");
    for (table, column, _, _) in &columns {
        let kind = columns_kind(&db, table, column).await;
        assert_eq!(kind, "datetime", "{table}.{column} is DATETIME");
    }
    let after_2038 = Utc
        .with_ymd_and_hms(2040, 6, 1, 12, 0, 0)
        .single()
        .expect("a valid time");
    for table in tables {
        seed(&db, table, Value::from(after_2038)).await;
    }
    for (table, column, _, _) in &columns {
        let read: DateTime<Utc> = db
            .query_one_raw(Statement::from_string(
                db.get_database_backend(),
                format!("SELECT `{column}` AS value FROM `{table}`"),
            ))
            .await
            .unwrap_or_else(|error| panic!("read {table}.{column}: {error}"))
            .unwrap_or_else(|| panic!("{table} has its row"))
            .try_get("", "value")
            .unwrap_or_else(|error| panic!("decode {table}.{column}: {error}"));
        assert_eq!(read, after_2038, "{table}.{column} holds a time after 2038");
    }
    drop_tables(&db, tables).await;
}

async fn columns_kind(db: &DatabaseConnection, table: &str, column: &str) -> String {
    columns(db, table)
        .await
        .into_iter()
        .find(|candidate| candidate.name == column)
        .unwrap_or_else(|| panic!("{table}.{column} exists"))
        .kind
}

/// A group table an older migration created with `TIMESTAMP` time columns,
/// holding rows written through a `+05:00` session, is moved to `DATETIME` by
/// `migrate`, run twice over that session: every stored time stays the same
/// UTC time, and each column keeps its nullability and its
/// `DEFAULT CURRENT_TIMESTAMP`.
pub async fn assert_upgrade_keeps_utc_times<F, Fut>(tables: &[&str], migrate: F)
where
    F: Fn(DatabaseConnection) -> Fut,
    Fut: Future<Output = ()>,
{
    let db = connect(None).await;
    drop_tables(&db, tables).await;
    migrate(db.clone()).await;
    // The shape an older migration created: the same tables with `TIMESTAMP`
    // time columns.
    let shape = time_columns(&db, tables).await;
    assert!(!shape.is_empty(), "{tables:?} have time columns");
    for (table, column, nullable, now) in &shape {
        let null = if *nullable { "NULL" } else { "NOT NULL" };
        let default = if *now {
            " DEFAULT CURRENT_TIMESTAMP"
        } else {
            ""
        };
        execute(
            &db,
            &format!("ALTER TABLE `{table}` MODIFY `{column}` TIMESTAMP {null}{default}"),
        )
        .await;
        assert_eq!(columns_kind(&db, table, column).await, "timestamp");
    }

    let shifted = connect(Some("+05:00")).await;
    for table in tables {
        seed(&shifted, table, Value::from(SHIFTED_LOCAL)).await;
    }
    let before = stored_times(&db, tables).await;
    for (table, column, value) in &before {
        assert_eq!(
            value.as_deref(),
            Some(SHIFTED_UTC),
            "{table}.{column} stored the +05:00 local time as UTC"
        );
    }

    migrate(shifted.clone()).await;
    migrate(shifted).await;

    let after: Vec<(String, String, bool, bool)> = time_columns(&db, tables).await;
    assert_eq!(after, shape, "nullability and defaults are kept");
    for (table, column, _, _) in &after {
        assert_eq!(
            columns_kind(&db, table, column).await,
            "datetime",
            "{table}.{column} is DATETIME"
        );
    }
    assert_eq!(
        stored_times(&db, tables).await,
        before,
        "every stored time is the same UTC time"
    );
    drop_tables(&db, tables).await;
}
