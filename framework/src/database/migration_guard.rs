//! Schema guards for the framework's own migrations.
//!
//! An app registers a framework migration in its `Migrator` and may run
//! it over tables that already exist: created by an earlier, differently
//! named migration, by a scaffolded copy of the same schema, or by hand.
//! Each framework `up` therefore creates its tables with `if_not_exists`
//! and its indexes through [`create_index_if_missing`], and an upgrade
//! moves an older table's MySQL `TIMESTAMP` columns through
//! [`convert_mysql_timestamps_to_datetime`].

use sea_orm_migration::SchemaManagerConnection;
use sea_orm_migration::prelude::{Alias, DbErr, IndexCreateStatement, SchemaManager};
use sea_orm_migration::sea_orm::{ConnectionTrait, DbBackend, Statement, TransactionTrait};

/// Create `index` on `table` unless the table already has an index of
/// that name.
///
/// sea-query renders `CREATE INDEX IF NOT EXISTS` on SQLite and Postgres
/// but drops the clause on MySQL, which has none. There, `up` over a table
/// that already had the index failed with error 1061 and blocked every
/// later migration. Asking the catalogue first works the same on every
/// backend. Callers leave `if_not_exists` off the statement: with it,
/// SQLite would skip a duplicate on its own, and the SQLite suite could no
/// longer catch a broken guard that MySQL depends on.
///
/// `index` carries the name and the columns but no table. This sets the
/// table from `table`, so the index it checks for and the one it creates
/// cannot name different tables.
///
/// # Errors
///
/// Returns [`DbErr`] when `index` has no name, or when the catalogue query
/// or the `CREATE INDEX` fails.
pub(crate) async fn create_index_if_missing(
    manager: &SchemaManager<'_>,
    table: &str,
    mut index: IndexCreateStatement,
) -> Result<(), DbErr> {
    let Some(name) = index.get_index_spec().get_name().map(str::to_owned) else {
        return Err(DbErr::Custom(format!(
            "an index on {table} needs a name before it can be created only when missing"
        )));
    };
    if manager.has_index(table, &name).await? {
        return Ok(());
    }
    index.table(Alias::new(table));
    manager.create_index(index).await
}

/// Convert each of `columns` of `table` that MySQL or MariaDB stores as
/// `TIMESTAMP` to `DATETIME`, keeping its nullability and every stored time
/// in UTC.
///
/// MySQL's `TIMESTAMP` refuses any time after 2038-01-19 03:14:07 UTC,
/// and a framework table an older migration created with `.timestamp()`
/// keeps it until something alters it. Every other backend, a missing
/// table, and a column that is already `DATETIME` are left alone, so a
/// migration built on this runs again harmlessly.
///
/// `ALTER TABLE ... MODIFY` turns each `TIMESTAMP` into the wall-clock time
/// of the session's time zone. The driver sets that zone to UTC on connect
/// unless the connection URL names another, so the statement runs on one
/// connection whose zone is set to UTC for it and restored afterwards: the
/// stored times stay the UTC times the framework wrote, and the pooled
/// connection goes back as it was.
///
/// # Errors
///
/// Returns [`DbErr`] when the catalogue query, the time zone change or the
/// `ALTER TABLE` fails, or when `table` or a column is not a plain lowercase
/// identifier.
pub(crate) async fn convert_mysql_timestamps_to_datetime(
    manager: &SchemaManager<'_>,
    table: &str,
    columns: &[&str],
) -> Result<(), DbErr> {
    let connection = manager.get_connection();
    if connection.get_database_backend() != DbBackend::MySql {
        return Ok(());
    }
    for identifier in std::iter::once(&table).chain(columns) {
        if identifier.is_empty()
            || !identifier
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(DbErr::Custom(format!(
                "{identifier:?} is not a plain lowercase identifier"
            )));
        }
    }
    let rows = connection
        .query_all_raw(Statement::from_sql_and_values(
            DbBackend::MySql,
            "SELECT COLUMN_NAME AS name, DATA_TYPE AS kind, IS_NULLABLE AS nullable \
             FROM information_schema.COLUMNS \
             WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ?",
            [table.into()],
        ))
        .await?;
    let mut changes = Vec::new();
    for column in columns {
        for row in &rows {
            let name: String = row.try_get("", "name")?;
            if !name.eq_ignore_ascii_case(column) {
                continue;
            }
            let kind: String = row.try_get("", "kind")?;
            if !kind.eq_ignore_ascii_case("timestamp") {
                continue;
            }
            let nullable: String = row.try_get("", "nullable")?;
            let null = if nullable.eq_ignore_ascii_case("yes") {
                "NULL"
            } else {
                "NOT NULL"
            };
            changes.push(format!("MODIFY `{column}` DATETIME {null}"));
        }
    }
    if changes.is_empty() {
        return Ok(());
    }
    let alter = format!("ALTER TABLE `{table}` {}", changes.join(", "));
    match connection {
        // A pool hands each statement any connection; one transaction keeps
        // the zone change, the ALTER and the restore on the same one. The
        // ALTER commits it implicitly, which ends nothing but the pin.
        SchemaManagerConnection::Connection(pool) => {
            let pinned = pool.begin().await?;
            alter_in_utc(&pinned, &alter).await?;
            pinned.commit().await
        }
        // A transaction is one connection already.
        pinned => alter_in_utc(pinned, &alter).await,
    }
}

/// Run `alter` on `connection` with its session time zone set to UTC, and
/// set the zone back afterwards, whether `alter` succeeded or not.
async fn alter_in_utc(connection: &impl ConnectionTrait, alter: &str) -> Result<(), DbErr> {
    connection
        .execute_unprepared(
            "SET @suprnova_previous_time_zone = @@session.time_zone, time_zone = '+00:00'",
        )
        .await?;
    let altered = connection.execute_unprepared(alter).await;
    let restored = connection
        .execute_unprepared("SET time_zone = @suprnova_previous_time_zone")
        .await;
    altered?;
    restored?;
    Ok(())
}
