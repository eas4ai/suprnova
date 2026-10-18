//! Reads a table's columns from the database's own catalog.
//!
//! The framework shares table names with Laravel and with the packages a
//! Laravel database may hold. Several of its stores and migrations have to
//! tell those layouts apart before they touch a table: the failed-jobs
//! store refuses to start a worker over a table it cannot write, a
//! migration leaves a table Laravel created alone, and an upgrade
//! recognizes a table an earlier framework release created. Each engine
//! keeps this in a different catalog, so the queries live here once.

use sea_orm::{ConnectionTrait, DbBackend, DbErr, Statement};

/// One column as the catalog reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CatalogColumn {
    /// Column name, lowercased.
    pub(crate) name: String,
    /// The type, lowercased: `DATA_TYPE` on MySQL (`longtext`, `bigint`),
    /// `data_type` on Postgres (`bigint`, `uuid`, `character varying`),
    /// and the declared type on SQLite (`varchar`, `integer`, `datetime`).
    pub(crate) data_type: String,
    /// The full type, lowercased: `COLUMN_TYPE` on MySQL, which shows
    /// `unsigned` and lengths; the same as [`Self::data_type`] elsewhere.
    pub(crate) column_type: String,
    /// Whether the column accepts `NULL`.
    pub(crate) nullable: bool,
    /// The declared length of a character column, when the catalog has one.
    pub(crate) max_length: Option<i64>,
}

impl CatalogColumn {
    /// Whether the column holds whole numbers.
    pub(crate) fn is_integer(&self) -> bool {
        self.data_type.contains("int")
    }
}

/// A table name split into the schema it names, if any, and the table.
///
/// The framework's configured table names (`QUEUE_DB_TABLE`,
/// `QUEUE_FAILED_DB_TABLE`) may be schema-qualified, as `public.jobs`, and
/// the catalog has to look the table up in that schema: Postgres's schema,
/// MySQL's database, an attached SQLite database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TableName<'a> {
    /// The schema, when the name gives one.
    pub(crate) schema: Option<&'a str>,
    /// The table within it.
    pub(crate) table: &'a str,
}

/// Split `table` at its one `.`, after checking that each segment is a
/// plain identifier (letters, digits and underscores, not starting with a
/// digit): SQLite's `PRAGMA table_info` takes the name inline.
///
/// # Errors
///
/// Returns [`DbErr`] naming `table` when a segment is not a plain
/// identifier or there is more than one `.`.
pub(crate) fn table_name(table: &str) -> Result<TableName<'_>, DbErr> {
    let plain = |segment: &str| {
        segment
            .bytes()
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic() || first == b'_')
            && segment
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    };
    let name = match table.split_once('.') {
        Some((schema, name)) => TableName {
            schema: Some(schema),
            table: name,
        },
        None => TableName {
            schema: None,
            table,
        },
    };
    if name.schema.is_some_and(|schema| !plain(schema)) || !plain(name.table) {
        return Err(DbErr::Custom(format!(
            "{table:?} is not a plain table identifier, optionally qualified by its schema"
        )));
    }
    Ok(name)
}

/// The `PRAGMA` prefix SQLite takes for the schema of `name`: `"main".` or
/// nothing.
fn sqlite_schema(name: TableName<'_>) -> String {
    name.schema
        .map(|schema| format!("\"{schema}\"."))
        .unwrap_or_default()
}

/// `name` quoted as Postgres's `to_regclass` takes it, so the lookup finds
/// the table the stores' quoted statements name: in its schema when the
/// name gives one, otherwise the one an unqualified name resolves to on
/// this connection (a temporary table shadows `public`, and the search path
/// decides among the rest).
fn regclass(name: TableName<'_>) -> String {
    match name.schema {
        Some(schema) => format!("\"{schema}\".\"{}\"", name.table),
        None => format!("\"{}\"", name.table),
    }
}

/// Whether `table` exists.
///
/// # Errors
///
/// As [`table_columns`].
pub(crate) async fn table_exists<C>(conn: &C, table: &str) -> Result<bool, DbErr>
where
    C: ConnectionTrait + ?Sized,
{
    Ok(!table_columns(conn, table).await?.is_empty())
}

/// The names of `table`'s indexes, unique ones included, as the catalog
/// stores them. Empty when the table does not exist.
///
/// # Errors
///
/// Returns [`DbErr`] when the catalog query fails, or when `table` is not
/// a plain identifier, optionally qualified by its schema.
pub(crate) async fn index_names<C>(conn: &C, table: &str) -> Result<Vec<String>, DbErr>
where
    C: ConnectionTrait + ?Sized,
{
    let name = table_name(table)?;
    let backend = conn.get_database_backend();
    let rows = match backend {
        DbBackend::Sqlite => {
            conn.query_all_raw(Statement::from_string(
                backend,
                format!(
                    "PRAGMA {}index_list(\"{}\")",
                    sqlite_schema(name),
                    name.table
                ),
            ))
            .await?
        }
        DbBackend::Postgres => {
            conn.query_all_raw(Statement::from_sql_and_values(
                backend,
                "SELECT i.relname::text AS name FROM pg_catalog.pg_index x \
                 JOIN pg_catalog.pg_class i ON i.oid = x.indexrelid \
                 WHERE x.indrelid = to_regclass($1)",
                [regclass(name).into()],
            ))
            .await?
        }
        DbBackend::MySql => {
            let (filter, values): (&str, Vec<sea_orm::Value>) = match name.schema {
                Some(schema) => ("TABLE_SCHEMA = ?", vec![schema.into(), name.table.into()]),
                None => ("TABLE_SCHEMA = DATABASE()", vec![name.table.into()]),
            };
            conn.query_all_raw(Statement::from_sql_and_values(
                backend,
                format!(
                    "SELECT DISTINCT INDEX_NAME AS name FROM information_schema.STATISTICS \
                     WHERE {filter} AND TABLE_NAME = ?"
                ),
                values,
            ))
            .await?
        }
        other => {
            return Err(DbErr::Custom(format!(
                "the catalog of a {other:?} database cannot be read"
            )));
        }
    };
    rows.iter()
        .map(|row| row.try_get::<String>("", "name"))
        .collect()
}

/// The columns of `table`, in declaration order. Empty when the table does
/// not exist. `table` may name its schema, as `public.jobs`.
///
/// # Errors
///
/// Returns [`DbErr`] when the catalog query fails, or when `table` is not
/// a plain identifier (letters, digits and underscores), optionally
/// qualified by its schema: SQLite's `PRAGMA table_info` takes the name
/// inline.
pub(crate) async fn table_columns<C>(conn: &C, table: &str) -> Result<Vec<CatalogColumn>, DbErr>
where
    C: ConnectionTrait + ?Sized,
{
    let name = table_name(table)?;
    let backend = conn.get_database_backend();
    match backend {
        DbBackend::Sqlite => {
            let rows = conn
                .query_all_raw(Statement::from_string(
                    backend,
                    format!(
                        "PRAGMA {}table_info(\"{}\")",
                        sqlite_schema(name),
                        name.table
                    ),
                ))
                .await?;
            let mut columns = Vec::with_capacity(rows.len());
            for row in rows {
                let name: String = row.try_get("", "name")?;
                let declared: String = row.try_get("", "type")?;
                let not_null: i64 = row.try_get("", "notnull")?;
                let pk: i64 = row.try_get("", "pk")?;
                let declared = declared.to_ascii_lowercase();
                let base = declared
                    .split('(')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                let max_length = declared
                    .split_once('(')
                    .and_then(|(_, rest)| rest.split(')').next())
                    .and_then(|inner| inner.split(',').next())
                    .and_then(|n| n.trim().parse().ok());
                columns.push(CatalogColumn {
                    name: name.to_ascii_lowercase(),
                    data_type: base,
                    column_type: declared,
                    // An `INTEGER PRIMARY KEY` is the rowid and never NULL,
                    // whatever `notnull` says.
                    nullable: not_null == 0 && pk == 0,
                    max_length,
                });
            }
            Ok(columns)
        }
        DbBackend::Postgres => {
            // The schema is the one the name gives, or the one an
            // unqualified name resolves to on this connection, as the
            // stores' own statements resolve it (see `regclass`).
            let rows = conn
                .query_all_raw(Statement::from_sql_and_values(
                    backend,
                    "SELECT column_name::text AS name, data_type::text AS data_type, \
                     is_nullable::text AS nullable, \
                     character_maximum_length::bigint AS max_length \
                     FROM information_schema.columns \
                     WHERE table_name = $1 AND table_schema = ( \
                         SELECT n.nspname::text FROM pg_catalog.pg_class c \
                         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                         WHERE c.oid = to_regclass($2)) \
                     ORDER BY ordinal_position",
                    [name.table.into(), regclass(name).into()],
                ))
                .await?;
            rows.into_iter()
                .map(|row| {
                    let data_type: String = row.try_get("", "data_type")?;
                    let nullable: String = row.try_get("", "nullable")?;
                    let name: String = row.try_get("", "name")?;
                    let data_type = data_type.to_ascii_lowercase();
                    Ok(CatalogColumn {
                        name: name.to_ascii_lowercase(),
                        column_type: data_type.clone(),
                        data_type,
                        nullable: nullable.eq_ignore_ascii_case("yes"),
                        max_length: row.try_get("", "max_length")?,
                    })
                })
                .collect()
        }
        DbBackend::MySql => {
            let (filter, values): (&str, Vec<sea_orm::Value>) = match name.schema {
                Some(schema) => ("TABLE_SCHEMA = ?", vec![schema.into(), name.table.into()]),
                None => ("TABLE_SCHEMA = DATABASE()", vec![name.table.into()]),
            };
            let rows = conn
                .query_all_raw(Statement::from_sql_and_values(
                    backend,
                    format!(
                        "SELECT COLUMN_NAME AS name, DATA_TYPE AS data_type, \
                         COLUMN_TYPE AS column_type, IS_NULLABLE AS nullable, \
                         CAST(CHARACTER_MAXIMUM_LENGTH AS SIGNED) AS max_length \
                         FROM information_schema.COLUMNS \
                         WHERE {filter} AND TABLE_NAME = ? \
                         ORDER BY ORDINAL_POSITION"
                    ),
                    values,
                ))
                .await?;
            rows.into_iter()
                .map(|row| {
                    let name: String = row.try_get("", "name")?;
                    let data_type: String = row.try_get("", "data_type")?;
                    let column_type: String = row.try_get("", "column_type")?;
                    let nullable: String = row.try_get("", "nullable")?;
                    Ok(CatalogColumn {
                        name: name.to_ascii_lowercase(),
                        data_type: data_type.to_ascii_lowercase(),
                        column_type: column_type.to_ascii_lowercase(),
                        nullable: nullable.eq_ignore_ascii_case("yes"),
                        max_length: row.try_get("", "max_length")?,
                    })
                })
                .collect()
        }
        other => Err(DbErr::Custom(format!(
            "the catalog of a {other:?} database cannot be read"
        ))),
    }
}

/// The column of `columns` named `name`.
pub(crate) fn column<'a>(columns: &'a [CatalogColumn], name: &str) -> Option<&'a CatalogColumn> {
    columns.iter().find(|column| column.name == name)
}
