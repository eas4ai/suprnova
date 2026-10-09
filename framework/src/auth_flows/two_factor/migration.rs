//! Migration that creates the `two_factor_credentials` table consumed
//! by [`crate::auth_flows::TwoFactor`].
//!
//! Consumer apps include this migration in their `Migrator`'s
//! `migrations()` list - the framework owns the schema, the app owns
//! when to apply it.

use sea_orm::DbBackend;
use sea_orm_migration::prelude::*;

/// Migration that creates the `two_factor_credentials` table.
pub struct Migration;

impl MigrationName for Migration {
    // Explicit, file-stable name. `DeriveMigrationName` derives from
    // the parent module path, which produces just `migration` here -
    // not unique enough for the `seaql_migrations` table once a
    // second framework-owned migration lands in another module with
    // the same file name. The date prefix matches the convention the
    // example app uses for its own migrations.
    fn name(&self) -> &str {
        "m20260101_000001_create_two_factor_credentials"
    }
}

#[derive(DeriveIden)]
enum TwoFactorCredentials {
    Table,
    UserId,
    Secret,
    ConfirmedAt,
    RecoveryCodes,
    CreatedAt,
    UpdatedAt,
}

/// The key column. MySQL and MariaDB can't index a `TEXT` column without a
/// prefix length, so the table can't be created there with a `TEXT` primary
/// key; they get a bounded `VARCHAR(255)`, the width of the attempt table's
/// user id. PostgreSQL and SQLite keep `TEXT`, so their DDL stays exactly
/// what apps already ran under this migration's name.
fn user_id_column(backend: DbBackend) -> ColumnDef {
    let mut column = ColumnDef::new(TwoFactorCredentials::UserId);
    if backend == DbBackend::MySql {
        column.string_len(255);
    } else {
        column.text();
    }
    column.not_null().primary_key();
    column.take()
}

/// The `CREATE TABLE IF NOT EXISTS` statement for `backend`.
fn create_table(backend: DbBackend) -> TableCreateStatement {
    Table::create()
        .table(TwoFactorCredentials::Table)
        .if_not_exists()
        .col(user_id_column(backend))
        .col(
            ColumnDef::new(TwoFactorCredentials::Secret)
                .text()
                .not_null(),
        )
        .col(
            ColumnDef::new(TwoFactorCredentials::ConfirmedAt)
                .timestamp_with_time_zone()
                .null(),
        )
        .col(
            ColumnDef::new(TwoFactorCredentials::RecoveryCodes)
                .text()
                .null(),
        )
        .col(
            ColumnDef::new(TwoFactorCredentials::CreatedAt)
                .timestamp_with_time_zone()
                .not_null()
                .default(Expr::current_timestamp()),
        )
        .col(
            ColumnDef::new(TwoFactorCredentials::UpdatedAt)
                .timestamp_with_time_zone()
                .not_null()
                .default(Expr::current_timestamp()),
        )
        .to_owned()
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(create_table(manager.get_database_backend()))
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(TwoFactorCredentials::Table).to_owned())
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // PostgreSQL and SQLite must render exactly what this migration rendered
    // before MySQL got its own key type: apps on those engines already ran
    // it under this name, and a fresh install must build the same table.

    #[test]
    fn postgres_ddl_is_unchanged() {
        assert_eq!(
            create_table(DbBackend::Postgres).to_string(PostgresQueryBuilder),
            "CREATE TABLE IF NOT EXISTS \"two_factor_credentials\" ( \
             \"user_id\" text NOT NULL PRIMARY KEY, \
             \"secret\" text NOT NULL, \
             \"confirmed_at\" timestamp with time zone NULL, \
             \"recovery_codes\" text NULL, \
             \"created_at\" timestamp with time zone NOT NULL DEFAULT CURRENT_TIMESTAMP, \
             \"updated_at\" timestamp with time zone NOT NULL DEFAULT CURRENT_TIMESTAMP )"
        );
    }

    #[test]
    fn sqlite_ddl_is_unchanged() {
        assert_eq!(
            create_table(DbBackend::Sqlite).to_string(SqliteQueryBuilder),
            "CREATE TABLE IF NOT EXISTS \"two_factor_credentials\" ( \
             \"user_id\" text NOT NULL PRIMARY KEY, \
             \"secret\" text NOT NULL, \
             \"confirmed_at\" timestamp_with_timezone_text NULL, \
             \"recovery_codes\" text NULL, \
             \"created_at\" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP, \
             \"updated_at\" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP )"
        );
    }

    #[test]
    fn mysql_keys_the_table_on_a_bounded_varchar() {
        assert_eq!(
            create_table(DbBackend::MySql).to_string(MysqlQueryBuilder),
            "CREATE TABLE IF NOT EXISTS `two_factor_credentials` ( \
             `user_id` varchar(255) NOT NULL PRIMARY KEY, \
             `secret` text NOT NULL, \
             `confirmed_at` timestamp NULL, \
             `recovery_codes` text NULL, \
             `created_at` timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP, \
             `updated_at` timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP )"
        );
    }
}
