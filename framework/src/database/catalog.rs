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

/// The columns of `table`, in declaration order. Empty when the table does
/// not exist.
///
/// # Errors
///
/// Returns [`DbErr`] when the catalog query fails, or when `table` is not
/// a plain identifier (letters, digits and underscores): SQLite's
/// `PRAGMA table_info` takes the name inline.
pub(crate) async fn table_columns<C>(conn: &C, table: &str) -> Result<Vec<CatalogColumn>, DbErr>
where
    C: ConnectionTrait + ?Sized,
{
    if table.is_empty()
        || !table
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(DbErr::Custom(format!(
            "{table:?} is not a plain table identifier"
        )));
    }
    let backend = conn.get_database_backend();
    match backend {
        DbBackend::Sqlite => {
            let rows = conn
                .query_all_raw(Statement::from_string(
                    backend,
                    format!("PRAGMA table_info(\"{table}\")"),
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
            // The schema is the one an unqualified name resolves to on this
            // connection, as the stores' own statements resolve it: a
            // temporary table shadows `public`, and the search path decides
            // among the rest.
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
                    [table.into(), format!("\"{table}\"").into()],
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
            let rows = conn
                .query_all_raw(Statement::from_sql_and_values(
                    backend,
                    "SELECT COLUMN_NAME AS name, DATA_TYPE AS data_type, \
                     COLUMN_TYPE AS column_type, IS_NULLABLE AS nullable, \
                     CAST(CHARACTER_MAXIMUM_LENGTH AS SIGNED) AS max_length \
                     FROM information_schema.COLUMNS \
                     WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ? \
                     ORDER BY ORDINAL_POSITION",
                    [table.into()],
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
