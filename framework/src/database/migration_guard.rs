//! Schema guards for the framework's own migrations.
//!
//! An app registers a framework migration in its `Migrator` and may run
//! it over tables that already exist: created by an earlier, differently
//! named migration, by a scaffolded copy of the same schema, or by hand.
//! Each framework `up` therefore creates its tables with `if_not_exists`
//! and its indexes through [`create_index_if_missing`], an upgrade moves an
//! older table's MySQL `TIMESTAMP` columns through
//! [`convert_mysql_timestamps_to_datetime`], and an upgrade that reshapes a
//! table into a new layout moves its rows through [`set_aside`] and
//! [`move_earlier_rows`].

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
/// `TIMESTAMP` to `DATETIME`, keeping its nullability, its fractional-second
/// precision, its comment, its `DEFAULT CURRENT_TIMESTAMP` and every stored
/// time in UTC.
///
/// MySQL's `TIMESTAMP` refuses any time after 2038-01-19 03:14:07 UTC,
/// and a framework table an older migration created with `.timestamp()`
/// keeps it until something alters it. Every other backend, a missing
/// table, and a column that is already `DATETIME` are left alone, so a
/// migration built on this runs again harmlessly.
///
/// `MODIFY` restates the whole column, so whatever it does not repeat is
/// dropped. It repeats a `CURRENT_TIMESTAMP` default, which some framework
/// tables declare and their inserts rely on. It does not repeat any other
/// default: no framework column declares one, and the zero date that older
/// MariaDB versions add on their own to a `TIMESTAMP NOT NULL` column is not a
/// valid `DATETIME` under strict mode. Nor the `ON UPDATE CURRENT_TIMESTAMP`
/// those versions add to the first such column: the framework writes its own
/// times, and that clause would rewrite one on every update.
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
            "SELECT COLUMN_NAME AS name, DATA_TYPE AS kind, IS_NULLABLE AS nullable, \
             COLUMN_DEFAULT AS column_default, COLUMN_COMMENT AS column_comment, \
             CAST(COALESCE(DATETIME_PRECISION, 0) AS SIGNED) AS fsp \
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
            let fsp: i64 = row.try_get("", "fsp")?;
            let precision = if fsp > 0 {
                format!("({fsp})")
            } else {
                String::new()
            };
            let default: Option<String> = row.try_get("", "column_default")?;
            // MySQL reports the default as `CURRENT_TIMESTAMP`, MariaDB as
            // `current_timestamp()`, each with the precision when it has one.
            let default = if default.is_some_and(|default| {
                default
                    .to_ascii_lowercase()
                    .starts_with("current_timestamp")
            }) {
                format!(" DEFAULT CURRENT_TIMESTAMP{precision}")
            } else {
                String::new()
            };
            let comment: String = row.try_get("", "column_comment")?;
            let comment = if comment.is_empty() {
                String::new()
            } else {
                format!(
                    " COMMENT '{}'",
                    comment.replace('\\', "\\\\").replace('\'', "''")
                )
            };
            changes.push(format!(
                "MODIFY `{column}` DATETIME{precision} {null}{default}{comment}"
            ));
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

/// The table an upgrade keeps a table's earlier rows in while it reshapes
/// them into a new layout under the original name: in the same schema when
/// the name is schema-qualified.
pub(crate) fn earlier_table_name(table: &str) -> String {
    match table.rsplit_once('.') {
        Some((schema, name)) => format!("{schema}.suprnova_earlier_{name}"),
        None => format!("suprnova_earlier_{table}"),
    }
}

/// Where an upgrade of `table` stands, read from the catalog: the earlier
/// layout is recognized by `is_earlier`, which a caller writes for its
/// table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpgradeState {
    /// Neither `table` nor its earlier rows exist: create the new layout.
    Fresh,
    /// `table` is in the earlier layout: set it aside, create the new
    /// layout, move the rows.
    Earlier,
    /// A previous run set the rows aside and stopped: create the new layout
    /// if it is missing, then move what is left.
    Resume,
    /// `table` is in some other layout (Laravel's, or one this upgrade
    /// already produced) and nothing is set aside: leave it alone.
    Untouched,
}

/// Read [`UpgradeState`] for `table`.
///
/// # Errors
///
/// Returns [`DbErr`] when the catalog cannot be read.
pub(crate) async fn upgrade_state(
    manager: &SchemaManager<'_>,
    table: &str,
    is_earlier: impl Fn(&[crate::database::catalog::CatalogColumn]) -> bool,
) -> Result<UpgradeState, DbErr> {
    let connection = manager.get_connection();
    let columns = crate::database::catalog::table_columns(connection, table).await?;
    let earlier =
        crate::database::catalog::table_columns(connection, &earlier_table_name(table)).await?;
    Ok(match (columns.is_empty(), earlier.is_empty()) {
        (true, true) => UpgradeState::Fresh,
        (false, true) if is_earlier(&columns) => UpgradeState::Earlier,
        (false, true) => UpgradeState::Untouched,
        (_, false) => UpgradeState::Resume,
    })
}

/// Copy `table` into its earlier-rows table and drop it, so the caller can
/// create the new layout under the same name.
///
/// `CREATE TABLE ... AS SELECT` copies the rows and none of the indexes,
/// keys or sequences, so the new table's names never collide with the
/// earlier table's. Each statement is atomic, so a run that stops between
/// them is resumed by [`resume_set_aside`]: the copy is either whole or
/// absent.
///
/// # Errors
///
/// Returns [`DbErr`] when a statement fails.
pub(crate) async fn set_aside(manager: &SchemaManager<'_>, table: &str) -> Result<(), DbErr> {
    let connection = manager.get_connection();
    let backend = connection.get_database_backend();
    let earlier = earlier_table_name(table);
    connection
        .execute_unprepared(&format!(
            "CREATE TABLE {} AS SELECT * FROM {}",
            quote(backend, &earlier),
            quote(backend, table)
        ))
        .await?;
    connection
        .execute_unprepared(&format!("DROP TABLE {}", quote(backend, table)))
        .await?;
    Ok(())
}

/// Finish what [`set_aside`] started when a run stopped after the copy:
/// drop `table` if it is still in the earlier layout. The copy is whole,
/// since `CREATE TABLE ... AS SELECT` is one statement.
///
/// # Errors
///
/// Returns [`DbErr`] when the catalog cannot be read or the drop fails.
pub(crate) async fn resume_set_aside(
    manager: &SchemaManager<'_>,
    table: &str,
    is_earlier: impl Fn(&[crate::database::catalog::CatalogColumn]) -> bool,
) -> Result<(), DbErr> {
    let connection = manager.get_connection();
    let columns = crate::database::catalog::table_columns(connection, table).await?;
    if !columns.is_empty() && is_earlier(&columns) {
        let backend = connection.get_database_backend();
        connection
            .execute_unprepared(&format!("DROP TABLE {}", quote(backend, table)))
            .await?;
    }
    Ok(())
}

/// How many rows one read of [`first_misfit`] takes.
const SCAN_CHUNK: u64 = 1000;

/// The first row of `table`, in the order of its `key` column, whose text
/// `column` `fits` refuses, as `(key, value)`; `None` when every row fits.
///
/// An upgrade whose layout takes a setting (a key type) runs this over the
/// rows it is about to move, before it changes any table: a row the setting
/// cannot hold then stops the migration with nothing set aside and no new
/// table created in the wrong form, and running it again under the setting
/// the error advises starts from the earlier layout. `key` is a unique
/// column, an integer one when `integer_key` says so and text otherwise;
/// the rows are read in chunks from the last key seen.
///
/// # Errors
///
/// Returns [`DbErr`] when a statement fails or a row cannot be read.
pub(crate) async fn first_misfit(
    manager: &SchemaManager<'_>,
    table: &str,
    key: &str,
    integer_key: bool,
    column: &str,
    fits: impl Fn(&str) -> bool,
) -> Result<Option<(String, String)>, DbErr> {
    let connection = manager.get_connection();
    let backend = connection.get_database_backend();
    let mut after: Option<sea_orm_migration::sea_orm::Value> = None;
    loop {
        let (filter, values) = match &after {
            Some(last) => (
                format!(
                    " WHERE {} > {}",
                    quote(backend, key),
                    crate::database::placeholder::placeholder(backend, 1)
                        .map_err(|e| DbErr::Migration(e.to_string()))?
                ),
                vec![last.clone()],
            ),
            None => (String::new(), Vec::new()),
        };
        let rows = connection
            .query_all_raw(Statement::from_sql_and_values(
                backend,
                format!(
                    "SELECT {key_column} AS row_key, {value_column} AS row_value FROM {table}\
                     {filter} ORDER BY {key_column} LIMIT {SCAN_CHUNK}",
                    key_column = quote(backend, key),
                    value_column = quote(backend, column),
                    table = quote(backend, table),
                ),
                values,
            ))
            .await?;
        let Some(last) = rows.last() else {
            return Ok(None);
        };
        let read_key = |row: &sea_orm_migration::sea_orm::QueryResult| {
            if integer_key {
                row.try_get::<i64>("", "row_key")
                    .map(sea_orm_migration::sea_orm::Value::from)
            } else {
                row.try_get::<String>("", "row_key")
                    .map(sea_orm_migration::sea_orm::Value::from)
            }
        };
        for row in &rows {
            let value: String = row.try_get("", "row_value")?;
            if !fits(&value) {
                let key = match read_key(row)? {
                    sea_orm_migration::sea_orm::Value::BigInt(Some(key)) => key.to_string(),
                    sea_orm_migration::sea_orm::Value::String(Some(key)) => key,
                    other => format!("{other:?}"),
                };
                return Ok(Some((key, value)));
            }
        }
        after = Some(read_key(last)?);
    }
}

/// One earlier row, converted: the statements that write it into the new
/// layout, and the earlier table's key for the row, by which it is removed
/// from the earlier-rows table in the same transaction.
pub(crate) struct MovedRow {
    pub(crate) key: sea_orm_migration::sea_orm::Value,
    pub(crate) writes: Vec<Statement>,
}

/// How many earlier rows one transaction moves.
const MOVE_CHUNK: u64 = 500;

/// Move every row of `table`'s earlier-rows table into the new layout, then
/// drop the earlier-rows table.
///
/// Rows move in chunks. Each chunk's writes and the removal of its rows
/// from the earlier-rows table commit together, so a run that stops part
/// way loses nothing and moves nothing twice: the next run starts from the
/// rows still set aside.
///
/// `select` names the earlier columns `convert` reads, and `key` the
/// earlier table's key column.
///
/// # Errors
///
/// Returns [`DbErr`] when a statement fails or `convert` refuses a row.
pub(crate) async fn move_earlier_rows(
    manager: &SchemaManager<'_>,
    table: &str,
    select: &str,
    key: &str,
    convert: impl FnMut(&sea_orm_migration::sea_orm::QueryResult) -> Result<MovedRow, DbErr>,
) -> Result<u64, DbErr> {
    move_rows(manager, &earlier_table_name(table), select, key, convert).await
}

/// [`move_earlier_rows`] from any table `source`, which is dropped once it
/// is empty: for an earlier table whose rows move to a table of another
/// name.
///
/// # Errors
///
/// Returns [`DbErr`] when a statement fails or `convert` refuses a row.
pub(crate) async fn move_rows(
    manager: &SchemaManager<'_>,
    source: &str,
    select: &str,
    key: &str,
    mut convert: impl FnMut(&sea_orm_migration::sea_orm::QueryResult) -> Result<MovedRow, DbErr>,
) -> Result<u64, DbErr> {
    let connection = manager.get_connection();
    let backend = connection.get_database_backend();
    let earlier = source.to_owned();
    let mut moved = 0;
    loop {
        let rows = connection
            .query_all_raw(Statement::from_string(
                backend,
                format!(
                    "SELECT {select} FROM {} LIMIT {MOVE_CHUNK}",
                    quote(backend, &earlier)
                ),
            ))
            .await?;
        if rows.is_empty() {
            break;
        }
        let mut converted = Vec::with_capacity(rows.len());
        for row in &rows {
            converted.push(convert(row)?);
        }
        let placeholders = (1..=converted.len())
            .map(|ordinal| {
                if backend == DbBackend::Postgres {
                    format!("${ordinal}")
                } else {
                    "?".to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        let keys: Vec<_> = converted.iter().map(|row| row.key.clone()).collect();
        let remove = Statement::from_sql_and_values(
            backend,
            format!(
                "DELETE FROM {} WHERE {} IN ({placeholders})",
                quote(backend, &earlier),
                quote(backend, key)
            ),
            keys,
        );
        let writes: Vec<Statement> = converted.into_iter().flat_map(|row| row.writes).collect();
        match connection {
            SchemaManagerConnection::Connection(pool) => {
                let txn = pool.begin().await?;
                for write in writes {
                    txn.execute_raw(write).await?;
                }
                txn.execute_raw(remove).await?;
                txn.commit().await?;
            }
            // Already inside the migration's transaction (Postgres).
            pinned => {
                for write in writes {
                    pinned.execute_raw(write).await?;
                }
                pinned.execute_raw(remove).await?;
            }
        }
        moved += rows.len() as u64;
    }
    connection
        .execute_unprepared(&format!("DROP TABLE {}", quote(backend, &earlier)))
        .await?;
    Ok(moved)
}

/// `name` quoted as an identifier for `backend`. Callers pass plain
/// identifiers (letters, digits, underscores); a schema-qualified name is
/// quoted segment by segment.
pub(crate) fn quote(backend: DbBackend, name: &str) -> String {
    let mark = if backend == DbBackend::MySql {
        '`'
    } else {
        '"'
    };
    name.split('.')
        .map(|segment| format!("{mark}{segment}{mark}"))
        .collect::<Vec<_>>()
        .join(".")
}

/// After rows were inserted with explicit ids, move a Postgres `id`
/// sequence past them, so the next insert does not reuse one. Every other
/// engine advances its counter on its own.
///
/// # Errors
///
/// Returns [`DbErr`] when the statement fails.
pub(crate) async fn advance_id_sequence(
    manager: &SchemaManager<'_>,
    table: &str,
) -> Result<(), DbErr> {
    let connection = manager.get_connection();
    if connection.get_database_backend() != DbBackend::Postgres {
        return Ok(());
    }
    connection
        .execute_unprepared(&format!(
            "SELECT setval(pg_get_serial_sequence('{table}', 'id'), COALESCE(MAX(id), 1), \
             MAX(id) IS NOT NULL) FROM {}",
            quote(DbBackend::Postgres, table)
        ))
        .await?;
    Ok(())
}
