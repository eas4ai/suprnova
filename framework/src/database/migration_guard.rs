//! Index creation for the framework's own migrations.
//!
//! An app registers a framework migration in its `Migrator` and may run
//! it over tables that already exist: created by an earlier, differently
//! named migration, by a scaffolded copy of the same schema, or by hand.
//! Each framework `up` therefore creates its tables with `if_not_exists`
//! and its indexes through [`create_index_if_missing`].

use sea_orm_migration::prelude::{Alias, DbErr, IndexCreateStatement, SchemaManager};

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
