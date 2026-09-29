//! An opt-in schema builder for migrations, layered over SeaORM's
//! `SchemaManager`.
//!
//! It is the shorter way to write the common migration: a `Blueprint`
//! closure records columns, indexes and foreign keys, and the builder turns
//! the description into `CREATE TABLE`, `ALTER TABLE`, `CREATE INDEX` and
//! foreign key statements and runs them through the `manager` it is given.
//! It never opens a connection of its own, so every statement runs on the
//! migration's connection and inside its transaction.
//!
//! SeaORM migrations keep working. Both styles can share one `Migrator` and
//! one `up()`; the builder adds no migration engine, no macro and no table
//! rebuild on SQLite.
//!
//! The module is not re-exported at the crate root, because
//! `suprnova::Schema` already names SeaORM's `Schema`. Import the builder by
//! its module path:
//!
//! ```no_run
//! use sea_orm_migration::prelude::*;
//! use suprnova::schema::Schema;
//!
//! #[derive(DeriveMigrationName)]
//! pub struct Migration;
//!
//! #[async_trait::async_trait]
//! impl MigrationTrait for Migration {
//!     async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
//!         Schema::create(manager, "posts", |t| {
//!             t.id();
//!             t.string("title");
//!             t.text("body").nullable();
//!             t.string("slug").length(120).unique();
//!             t.boolean("published").default(false);
//!             t.foreign_id("author_id")
//!                 .constrained("users")
//!                 .on_delete(ForeignKeyAction::Cascade);
//!             t.index(&["published", "created_at"]);
//!             t.timestamps();
//!             t.soft_deletes();
//!         })
//!         .await
//!     }
//!
//!     async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
//!         Schema::drop_if_exists(manager, "posts").await
//!     }
//! }
//! ```
//!
//! # Entry points
//!
//! Every function is `async`, takes `manager: &SchemaManager`, and returns
//! `Result<_, DbErr>`, so it fits `MigrationTrait::up` and `down`.
//!
//! | Function | Effect |
//! |---|---|
//! | [`Schema::create`] | Creates the table, then its indexes. Foreign keys are part of `CREATE TABLE`. |
//! | [`Schema::table`] | Alters an existing table. |
//! | [`Schema::drop`] | Drops the table; an error if it does not exist. |
//! | [`Schema::drop_if_exists`] | Drops the table if it exists. |
//! | [`Schema::rename`] | Renames a table. |
//! | [`Schema::has_table`] | Returns whether the table exists. |
//! | [`Schema::has_column`] | Returns whether the column exists. |
//!
//! # Column types
//!
//! A column is `NOT NULL` unless it is marked `.nullable()`.
//!
//! | Method | Column |
//! |---|---|
//! | `id()` | `id`, `BIGINT`, auto-increment, primary key |
//! | `foreign_id(name)` | `BIGINT`, the type of `id()` |
//! | `big_integer(name)`, `integer(name)`, `small_integer(name)` | 64-, 32- and 16-bit integers |
//! | `boolean(name)` | boolean |
//! | `string(name)` | `VARCHAR(255)`; `.length(n)` sets the length |
//! | `char(name, length)` | fixed length |
//! | `text(name)` | unbounded text |
//! | `float(name)`, `double(name)` | 32- and 64-bit floating point |
//! | `decimal(name, precision, scale)` | exact decimal |
//! | `date(name)`, `time(name)` | date, time of day |
//! | `date_time(name)` | date and time without a time zone |
//! | `timestamp_tz(name)` | date and time with a time zone |
//! | `json(name)` | `jsonb` on Postgres, `JSON` on MySQL, text on SQLite |
//! | `uuid(name)` | `uuid` on Postgres, `CHAR(36)` on MySQL and SQLite |
//! | `ulid(name)` | `CHAR(26)` |
//! | `binary(name)` | `bytea` on Postgres, `BLOB` elsewhere |
//!
//! `timestamps()` and `soft_deletes()` create string columns, because that
//! is the storage a `#[suprnova::model]` uses for a `DateTime<Utc>` field
//! with no declared cast. Their documentation gives the reason.
//!
//! # Indexes and foreign keys
//!
//! `t.index(&["a", "b"])` is named `{table}_{columns}_index` and
//! `t.unique(&["a"])` is named `{table}_{columns}_unique`, with the columns
//! joined by `_`. A foreign key is named `{table}_{column}_foreign`. Indexes
//! are separate `CREATE INDEX` statements that run after the table.
//!
//! # Altering a table
//!
//! [`Schema::table`] accepts new columns of any type, `rename_column`,
//! `drop_column`, `index`, `unique`, `drop_index`, `foreign_id(..)
//! .constrained(..)` and `drop_foreign`. It runs the operations in the order
//! the closure recorded them, each as its own statement. Changing the type
//! of an existing column is not supported.
//!
//! SQLite cannot add or drop a foreign key on an existing table. On SQLite
//! `Schema::table` returns an error for either operation before it runs any
//! statement of the call; create the key with the table, or write the step
//! with SeaORM. It also refuses to add a primary key column, and a `NOT
//! NULL` column with no default, because SQLite does. A column added to an
//! existing table on SQLite needs a constant default; SQLite refuses
//! `CURRENT_TIMESTAMP` there.
//!
//! The builder checks a description for the mistakes it can see before it
//! runs the first statement. A statement the database refuses stops the
//! call. On MySQL and SQLite the migrator runs a migration without a
//! transaction, so the statements before the refused one stay applied; on
//! Postgres the migration's transaction rolls them back.
//!
//! # Errors
//!
//! Every error is a `DbErr`. A refusal made by this module is
//! `DbErr::Migration` with a text that names the table, the column and the
//! operation. No text contains a connection URL or a value from a row.

mod blueprint;
mod column;
mod foreign;
mod plan;

use sea_orm::DbErr;
use sea_orm::sea_query::{Alias, Table};
use sea_orm_migration::SchemaManager;

pub use blueprint::Blueprint;
pub use column::ColumnBuilder;
pub use foreign::ForeignIdBuilder;

use plan::{Step, plan_alter, plan_create};

/// The identifier SeaQuery quotes for `name`. `Alias` carries an arbitrary
/// string, which is what a table or column named at run time needs.
fn sea_ident(name: &str) -> Alias {
    Alias::new(name)
}

/// Runs the statements of a plan in order and stops at the first error.
async fn run(manager: &SchemaManager<'_>, steps: Vec<Step>) -> Result<(), DbErr> {
    for step in steps {
        match step {
            Step::CreateTable(statement) => manager.create_table(statement).await?,
            Step::AlterTable(statement) => manager.alter_table(statement).await?,
            Step::CreateIndex(statement) => manager.create_index(statement).await?,
            Step::DropIndex(statement) => manager.drop_index(statement).await?,
            Step::CreateForeignKey(statement) => manager.create_foreign_key(statement).await?,
            Step::DropForeignKey(statement) => manager.drop_foreign_key(statement).await?,
        }
    }
    Ok(())
}

/// The entry points of the schema builder. It has no state: every function
/// takes the `SchemaManager` of the migration that calls it.
pub struct Schema;

impl Schema {
    /// Creates `table` with the columns, indexes and foreign keys `define`
    /// records: the table first (foreign keys inline), then one statement
    /// per index.
    ///
    /// The closure only records. The description is checked for the mistakes
    /// the builder can see (a duplicate column, an index over a column the
    /// table does not declare, an operation that belongs to
    /// [`Schema::table`]) before the first statement runs. A statement the
    /// database refuses stops the call. On MySQL and SQLite the migrator runs
    /// a migration without a transaction, so the statements before the
    /// refused one stay applied; on Postgres the migration's transaction
    /// rolls them back. A closure that records `rename_column`, `drop_column`, `drop_index` or
    /// `drop_foreign` is an error: those belong to [`Schema::table`].
    pub async fn create<F>(manager: &SchemaManager<'_>, table: &str, define: F) -> Result<(), DbErr>
    where
        F: FnOnce(&mut Blueprint) + Send,
    {
        let steps = {
            let mut blueprint = Blueprint::new(table);
            define(&mut blueprint);
            plan_create(&blueprint, manager.get_database_backend())?
        };
        run(manager, steps).await
    }

    /// Alters the existing `table` with the operations `define` records.
    ///
    /// Each operation runs as its own statement, in the order the closure
    /// recorded it, because SQLite accepts one alteration per statement. On
    /// SQLite, adding or dropping a foreign key returns an error before any
    /// statement of the call runs, and so does a column added twice.
    ///
    /// A statement the database refuses stops the call. On MySQL and SQLite
    /// the migrator runs a migration without a transaction, so the statements
    /// before the refused one stay applied; on Postgres the migration's
    /// transaction rolls them back. On SQLite a column added to an existing
    /// table needs a constant default: SQLite refuses `CURRENT_TIMESTAMP`
    /// there, and the plan cannot see inside an `Expr` to refuse it first.
    pub async fn table<F>(manager: &SchemaManager<'_>, table: &str, define: F) -> Result<(), DbErr>
    where
        F: FnOnce(&mut Blueprint) + Send,
    {
        let steps = {
            let mut blueprint = Blueprint::new(table);
            define(&mut blueprint);
            plan_alter(&blueprint, manager.get_database_backend())?
        };
        run(manager, steps).await
    }

    /// Drops `table`. It is an error if the table does not exist; use
    /// [`Schema::drop_if_exists`] when that is acceptable.
    pub async fn drop(manager: &SchemaManager<'_>, table: &str) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(sea_ident(table)).to_owned())
            .await
    }

    /// Drops `table` if it exists, and does nothing if it does not.
    pub async fn drop_if_exists(manager: &SchemaManager<'_>, table: &str) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(sea_ident(table)).if_exists().to_owned())
            .await
    }

    /// Renames the table `from` to `to`.
    pub async fn rename(manager: &SchemaManager<'_>, from: &str, to: &str) -> Result<(), DbErr> {
        manager
            .rename_table(
                Table::rename()
                    .table(sea_ident(from), sea_ident(to))
                    .to_owned(),
            )
            .await
    }

    /// Returns whether `table` exists.
    pub async fn has_table(manager: &SchemaManager<'_>, table: &str) -> Result<bool, DbErr> {
        manager.has_table(table).await
    }

    /// Returns whether `table` has a column called `column`.
    pub async fn has_column(
        manager: &SchemaManager<'_>,
        table: &str,
        column: &str,
    ) -> Result<bool, DbErr> {
        manager.has_column(table, column).await
    }
}
