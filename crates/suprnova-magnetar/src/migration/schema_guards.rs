//! Backend-neutral catalog guards for migration DDL.
//!
//! These helpers intentionally inspect the catalog before emitting DDL instead
//! of using backend-specific `IF NOT EXISTS` clauses.

use sea_orm::sea_query::{Alias, Index, IntoTableRef, TableRef};
use sea_orm::{ConnectionTrait, DbBackend, Statement};

use crate::{Error, Result};

use super::database_error;

/// The count of guarded DDL statements submitted by one helper call.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GuardReport {
    /// Number of statements submitted after catalog inspection.
    pub statements: usize,
}

/// Returns whether a table exists in the selected schema.
pub async fn has_table<C: ConnectionTrait + ?Sized>(
    database: &C,
    schema: Option<&str>,
    table: &str,
) -> Result<bool> {
    let backend = database.get_database_backend();
    let (sql, values) = match backend {
        DbBackend::Sqlite => (
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ? LIMIT 1",
            vec![table.to_owned().into()],
        ),
        DbBackend::Postgres => (
            "SELECT 1 FROM information_schema.tables WHERE table_schema = COALESCE($1, current_schema()) AND table_name = $2 AND table_type = 'BASE TABLE' LIMIT 1",
            vec![schema.map(str::to_owned).into(), table.to_owned().into()],
        ),
        DbBackend::MySql => (
            "SELECT 1 FROM information_schema.tables WHERE table_schema = COALESCE(?, DATABASE()) AND table_name = ? AND table_type = 'BASE TABLE' LIMIT 1",
            vec![schema.map(str::to_owned).into(), table.to_owned().into()],
        ),
        _ => return Err(super::unsupported_backend_error(backend)),
    };
    Ok(database.query_one_raw(Statement::from_sql_and_values(backend, sql, values))
        .await
        .map_err(|error| database_error("checking migration table", error))?
        .is_some())
}

/// Returns whether a column exists in the selected table.
pub async fn has_column<C: ConnectionTrait + ?Sized>(
    database: &C,
    schema: Option<&str>,
    table: &str,
    column: &str,
) -> Result<bool> {
    let backend = database.get_database_backend();
    let (sql, values) = match backend {
        DbBackend::Sqlite => (
            "SELECT 1 FROM pragma_table_info(?) WHERE name = ? LIMIT 1",
            vec![table.to_owned().into(), column.to_owned().into()],
        ),
        DbBackend::Postgres => (
            "SELECT 1 FROM information_schema.columns WHERE table_schema = COALESCE($1, current_schema()) AND table_name = $2 AND column_name = $3 LIMIT 1",
            vec![
                schema.map(str::to_owned).into(),
                table.to_owned().into(),
                column.to_owned().into(),
            ],
        ),
        DbBackend::MySql => (
            "SELECT 1 FROM information_schema.columns WHERE table_schema = COALESCE(?, DATABASE()) AND table_name = ? AND column_name = ? LIMIT 1",
            vec![
                schema.map(str::to_owned).into(),
                table.to_owned().into(),
                column.to_owned().into(),
            ],
        ),
        _ => return Err(super::unsupported_backend_error(backend)),
    };
    Ok(database.query_one_raw(Statement::from_sql_and_values(backend, sql, values))
        .await
        .map_err(|error| database_error("checking migration column", error))?
        .is_some())
}

/// Returns whether all requested columns exist in the selected table.
pub async fn has_columns<C: ConnectionTrait + ?Sized>(
    database: &C,
    schema: Option<&str>,
    table: &str,
    columns: &[&str],
) -> Result<bool> {
    for column in columns {
        if !has_column(database, schema, table, column).await? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Returns whether an index exists in the selected schema and table.
pub async fn has_index<C: ConnectionTrait + ?Sized>(
    database: &C,
    schema: Option<&str>,
    table: &str,
    index: &str,
) -> Result<bool> {
    let backend = database.get_database_backend();
    let (sql, values) = match backend {
        DbBackend::Sqlite => (
            "SELECT 1 FROM sqlite_master WHERE type = 'index' AND tbl_name = ? AND name = ? LIMIT 1",
            vec![table.to_owned().into(), index.to_owned().into()],
        ),
        DbBackend::Postgres => (
            "SELECT 1 FROM pg_indexes WHERE schemaname = COALESCE($1, current_schema()) AND tablename = $2 AND indexname = $3 LIMIT 1",
            vec![
                schema.map(str::to_owned).into(),
                table.to_owned().into(),
                index.to_owned().into(),
            ],
        ),
        DbBackend::MySql => (
            "SELECT 1 FROM information_schema.statistics WHERE table_schema = COALESCE(?, DATABASE()) AND table_name = ? AND index_name = ? LIMIT 1",
            vec![
                schema.map(str::to_owned).into(),
                table.to_owned().into(),
                index.to_owned().into(),
            ],
        ),
        _ => return Err(super::unsupported_backend_error(backend)),
    };
    Ok(database.query_one_raw(Statement::from_sql_and_values(backend, sql, values))
        .await
        .map_err(|error| database_error("checking migration index", error))?
        .is_some())
}

/// Creates a lookup index only when it is absent and every column is present.
pub async fn create_index_if_missing<C: ConnectionTrait + ?Sized>(
    database: &C,
    schema: Option<&str>,
    table: &str,
    index: &str,
    columns: &[&str],
) -> Result<GuardReport> {
    validate_identifier(table)?;
    validate_identifier(index)?;
    if columns.is_empty() {
        return Err(Error::InvalidInput {
            field: "index columns".to_owned(),
            message: "an index needs at least one column".to_owned(),
        });
    }
    for column in columns {
        validate_identifier(column)?;
    }
    if has_index(database, schema, table, index).await?
        || !has_columns(database, schema, table, columns).await?
    {
        return Ok(GuardReport::default());
    }

    let backend = database.get_database_backend();
    let table_ref = table_ref(backend, schema, table)?;
    let mut create = Index::create().name(index).table(table_ref).to_owned();
    for column in columns {
        create.col(Alias::new(*column));
    }
    database.execute(&create)
        .await
        .map_err(|error| database_error("creating guarded migration index", error))?;
    Ok(GuardReport { statements: 1 })
}

fn table_ref(backend: DbBackend, schema: Option<&str>, table: &str) -> Result<TableRef> {
    match (backend, schema) {
        (DbBackend::Postgres, Some(schema)) => {
            validate_identifier(schema)?;
            Ok((Alias::new(schema), Alias::new(table)).into_table_ref())
        }
        (DbBackend::MySql, Some(_)) => Err(Error::InvalidInput {
            field: "mysql schema-qualified index".to_owned(),
            message: "MySQL guarded index emission requires an unqualified application binding"
                .to_owned(),
        }),
        (DbBackend::Sqlite | DbBackend::Postgres | DbBackend::MySql, None) => {
            Ok(Alias::new(table).into_table_ref())
        }
        (DbBackend::Sqlite, Some(_)) => Err(Error::InvalidInput {
            field: "sqlite schema-qualified index".to_owned(),
            message: "SQLite guarded index emission requires an unqualified table".to_owned(),
        }),
        _ => Err(super::unsupported_backend_error(backend)),
    }
}

fn validate_identifier(value: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(Error::InvalidInput {
            field: "migration identifier".to_owned(),
            message: format!("unsupported identifier {value:?}"),
        });
    }
    Ok(())
}
