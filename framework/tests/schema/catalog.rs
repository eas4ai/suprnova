//! Reads a table's columns from the database's own catalog, so a test
//! checks what the database stored and not what the builder meant to send.

use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};

/// One column as the catalog reports it.
pub struct CatalogColumn {
    /// Column name.
    pub name: String,
    /// The type family. On Postgres and MySQL it is `data_type`. On SQLite
    /// it is the column affinity SQLite derives from the declared type, since
    /// the declared text (`datetime_text`, `real(10, 2)`) is sea-query's
    /// spelling and the affinity is what SQLite acts on.
    pub family: String,
    /// Whether the column accepts `NULL`.
    pub nullable: bool,
    /// The default expression as text, lower-cased.
    pub default: Option<String>,
    /// The declared length of a `char` or `varchar` column, or the length
    /// the catalog reports for any other column on MySQL.
    pub length: Option<i64>,
    /// The precision of a `decimal` column: `numeric_precision` on Postgres
    /// and MySQL, the first number of the declared type on SQLite. For other
    /// numeric columns Postgres and MySQL report the width of the type.
    pub precision: Option<i64>,
    /// The scale of a `decimal` column, from `numeric_scale` or the second
    /// number of the SQLite declared type.
    pub scale: Option<i64>,
    /// The full declared type: `column_type` on MySQL (which shows
    /// `unsigned`) without an integer's display width, `data_type` on
    /// Postgres, the declared text on SQLite.
    pub declared: String,
}

/// MariaDB, and MySQL before 8.0.19, write an integer column's display
/// width into `column_type` (`bigint(20) unsigned`); MySQL 8.4 leaves it
/// out (`bigint unsigned`). The width changes nothing the column stores, so
/// it is dropped and both engines report the same type.
fn without_display_width(declared: &str) -> String {
    let lower = declared.to_lowercase();
    for integer in ["tinyint", "smallint", "mediumint", "bigint", "int"] {
        let Some(rest) = lower.strip_prefix(integer) else {
            continue;
        };
        if let Some((width, tail)) = rest.strip_prefix('(').and_then(|r| r.split_once(')'))
            && !width.is_empty()
            && width.bytes().all(|b| b.is_ascii_digit())
        {
            return format!("{integer}{tail}");
        }
        break;
    }
    declared.to_owned()
}

/// SQLite's rules for the affinity of a declared type (section 3.1 of the
/// datatype documentation), applied in the documented order.
fn sqlite_affinity(declared: &str) -> &'static str {
    let declared = declared.to_lowercase();
    if declared.contains("int") {
        "integer"
    } else if declared.contains("char") || declared.contains("clob") || declared.contains("text") {
        "text"
    } else if declared.is_empty() || declared.contains("blob") {
        "blob"
    } else if declared.contains("real") || declared.contains("floa") || declared.contains("doub") {
        "real"
    } else {
        "numeric"
    }
}

/// The length inside `char(n)` or `varchar(n)`, `None` for any other type.
fn sqlite_declared_length(declared: &str) -> Option<i64> {
    let declared = declared.to_lowercase();
    let inner = declared
        .strip_prefix("varchar(")
        .or_else(|| declared.strip_prefix("char("))?;
    inner.strip_suffix(')')?.trim().parse().ok()
}

/// The two numbers inside `name(p, s)`, `None` for any other declared type.
fn sqlite_declared_precision(declared: &str) -> Option<(i64, i64)> {
    let inner = declared.split_once('(')?.1.strip_suffix(')')?;
    let (precision, scale) = inner.split_once(',')?;
    Some((precision.trim().parse().ok()?, scale.trim().parse().ok()?))
}

async fn sqlite_columns(conn: &DatabaseConnection, table: &str) -> Vec<CatalogColumn> {
    let rows = conn
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            format!("PRAGMA table_info('{table}')"),
        ))
        .await
        .expect("PRAGMA table_info");
    rows.iter()
        .map(|row| {
            let declared: String = row.try_get("", "type").expect("type");
            let not_null: i64 = row.try_get("", "notnull").expect("notnull");
            let default: Option<String> = row.try_get("", "dflt_value").expect("dflt_value");
            let precision = sqlite_declared_precision(&declared);
            CatalogColumn {
                name: row.try_get("", "name").expect("name"),
                family: sqlite_affinity(&declared).to_owned(),
                nullable: not_null == 0,
                default: default.map(|d| d.to_lowercase()),
                length: sqlite_declared_length(&declared),
                precision: precision.map(|(p, _)| p),
                scale: precision.map(|(_, s)| s),
                declared,
            }
        })
        .collect()
}

async fn information_schema_columns(
    conn: &DatabaseConnection,
    backend: DbBackend,
    sql: String,
) -> Vec<CatalogColumn> {
    let rows = conn
        .query_all_raw(Statement::from_string(backend, sql))
        .await
        .expect("information_schema.columns");
    rows.iter()
        .map(|row| {
            let nullable: String = row.try_get("", "c_nullable").expect("c_nullable");
            let default: Option<String> = row.try_get("", "c_default").expect("c_default");
            CatalogColumn {
                name: row.try_get("", "c_name").expect("c_name"),
                family: row.try_get("", "c_type").expect("c_type"),
                nullable: nullable.eq_ignore_ascii_case("yes"),
                default: default.map(|d| d.to_lowercase()),
                length: row.try_get("", "c_length").expect("c_length"),
                precision: row.try_get("", "c_precision").expect("c_precision"),
                scale: row.try_get("", "c_scale").expect("c_scale"),
                declared: row.try_get("", "c_declared").expect("c_declared"),
            }
        })
        .collect()
}

/// Returns the columns of `table` in declaration order.
pub async fn columns(conn: &DatabaseConnection, table: &str) -> Vec<CatalogColumn> {
    match conn.get_database_backend() {
        DbBackend::Sqlite => sqlite_columns(conn, table).await,
        DbBackend::Postgres => {
            let sql = format!(
                "SELECT column_name::text AS c_name, data_type::text AS c_type, \
                 is_nullable::text AS c_nullable, column_default::text AS c_default, \
                 character_maximum_length::bigint AS c_length, \
                 numeric_precision::bigint AS c_precision, \
                 numeric_scale::bigint AS c_scale, \
                 data_type::text AS c_declared \
                 FROM information_schema.columns \
                 WHERE table_schema = current_schema() AND table_name = '{table}' \
                 ORDER BY ordinal_position"
            );
            information_schema_columns(conn, DbBackend::Postgres, sql).await
        }
        DbBackend::MySql => {
            let sql = format!(
                "SELECT CAST(column_name AS CHAR) AS c_name, \
                 CAST(data_type AS CHAR) AS c_type, \
                 CAST(is_nullable AS CHAR) AS c_nullable, \
                 CAST(column_default AS CHAR) AS c_default, \
                 CAST(character_maximum_length AS SIGNED) AS c_length, \
                 CAST(numeric_precision AS SIGNED) AS c_precision, \
                 CAST(numeric_scale AS SIGNED) AS c_scale, \
                 CAST(column_type AS CHAR) AS c_declared \
                 FROM information_schema.columns \
                 WHERE table_schema = DATABASE() AND table_name = '{table}' \
                 ORDER BY ordinal_position"
            );
            let mut columns = information_schema_columns(conn, DbBackend::MySql, sql).await;
            for column in &mut columns {
                column.declared = without_display_width(&column.declared);
            }
            columns
        }
        other => panic!("the schema tests have no catalog query for {other:?}"),
    }
}

#[test]
fn the_display_width_of_an_integer_is_dropped() {
    for (declared, expected) in [
        ("bigint(20) unsigned", "bigint unsigned"),
        ("bigint(20)", "bigint"),
        ("int(11)", "int"),
        ("tinyint(1)", "tinyint"),
        ("bigint unsigned", "bigint unsigned"),
        ("varchar(255)", "varchar(255)"),
        ("decimal(10,2)", "decimal(10,2)"),
        ("enum('a','b')", "enum('a','b')"),
    ] {
        assert_eq!(without_display_width(declared), expected, "{declared}");
    }
}
