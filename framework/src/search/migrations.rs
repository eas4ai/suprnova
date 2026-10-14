//! Shared search metadata survives generation rebuilds on the application's database.
//!
//! Applications append [`migrations()`] to their migrator. Engines create and drop
//! the per-generation document and full-text tables separately.

use sea_orm::{ConnectionTrait, DbBackend, Statement};
use sea_orm_migration::prelude::*;

/// Persist generation settings, removal fences and vocabulary independently of side tables.
pub struct CreateSearchTables;

impl MigrationName for CreateSearchTables {
    fn name(&self) -> &str {
        "m20261014_000001_create_search_tables"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateSearchTables {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let mysql_collation = if backend == DbBackend::MySql {
            let row = manager
                .get_connection()
                .query_one_raw(Statement::from_string(backend, "SELECT VERSION()"))
                .await?
                .ok_or_else(|| {
                    DbErr::Migration("search migration SELECT VERSION() returned no row".into())
                })?;
            let version: String = row.try_get_by_index(0)?;
            Some(if version.to_ascii_lowercase().contains("mariadb") {
                "utf8mb4_nopad_bin"
            } else {
                "utf8mb4_0900_bin"
            })
        } else {
            None
        };
        let mut settings_json = ColumnDef::new(Alias::new("settings_json"));
        let mut pending_vectors_json = ColumnDef::new(Alias::new("pending_vectors_json"));
        if backend == DbBackend::MySql {
            settings_json.custom(Alias::new("LONGTEXT"));
            pending_vectors_json.custom(Alias::new("LONGTEXT"));
        } else {
            settings_json.text();
            pending_vectors_json.text();
        }
        settings_json.not_null();
        pending_vectors_json
            .not_null()
            .default(Expr::cust("('[]')"));
        let mut generations = Table::create();
        generations
            .table(Alias::new("search_generations"))
            .if_not_exists()
            .col(
                ColumnDef::new(Alias::new("index_name"))
                    .string_len(128)
                    .not_null(),
            )
            .col(
                ColumnDef::new(Alias::new("generation"))
                    .string_len(64)
                    .not_null(),
            )
            .col(
                ColumnDef::new(Alias::new("state"))
                    .string_len(16)
                    .not_null(),
            )
            .col(&mut settings_json)
            .col(
                ColumnDef::new(Alias::new("digest"))
                    .string_len(64)
                    .not_null(),
            )
            .col(ColumnDef::new(Alias::new("model")).text())
            .col(ColumnDef::new(Alias::new("dimensions")).big_integer())
            .col(
                ColumnDef::new(Alias::new("write_count"))
                    .big_integer()
                    .not_null()
                    .default(0),
            )
            .col(
                ColumnDef::new(Alias::new("refreshed_count"))
                    .big_integer()
                    .not_null()
                    .default(0),
            )
            .col(&mut pending_vectors_json)
            .primary_key(
                Index::create()
                    .col(Alias::new("index_name"))
                    .col(Alias::new("generation")),
            );

        let mut tombstones = Table::create();
        tombstones
            .table(Alias::new("search_tombstones"))
            .if_not_exists()
            .col(
                ColumnDef::new(Alias::new("index_name"))
                    .string_len(128)
                    .not_null(),
            )
            .col(
                ColumnDef::new(Alias::new("generation"))
                    .string_len(64)
                    .not_null(),
            )
            .col(
                ColumnDef::new(Alias::new("document_key"))
                    .string_len(255)
                    .not_null(),
            )
            .col(
                ColumnDef::new(Alias::new("source_version"))
                    .big_integer()
                    .not_null(),
            )
            .col(
                ColumnDef::new(Alias::new("removed_at"))
                    .big_integer()
                    .not_null(),
            )
            .primary_key(
                Index::create()
                    .col(Alias::new("index_name"))
                    .col(Alias::new("generation"))
                    .col(Alias::new("document_key")),
            );

        let mut vocabulary = Table::create();
        vocabulary
            .table(Alias::new("search_vocabulary"))
            .if_not_exists()
            .col(
                ColumnDef::new(Alias::new("index_name"))
                    .string_len(128)
                    .not_null(),
            )
            .col(
                ColumnDef::new(Alias::new("generation"))
                    .string_len(64)
                    .not_null(),
            )
            .col(
                ColumnDef::new(Alias::new("word"))
                    .string_len(255)
                    .not_null(),
            )
            .col(
                ColumnDef::new(Alias::new("document_frequency"))
                    .big_integer()
                    .not_null(),
            )
            .primary_key(
                Index::create()
                    .col(Alias::new("index_name"))
                    .col(Alias::new("generation"))
                    .col(Alias::new("word")),
            );

        for mut table in [generations, tombstones, vocabulary] {
            if let Some(collation) = mysql_collation {
                // No-pad identity comparisons preserve both case and trailing spaces.
                // The widest key is 1788 bytes, below InnoDB's 3072-byte limit.
                table.character_set("utf8mb4").collate(collation);
            }
            manager.create_table(table).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            "search_vocabulary",
            "search_tombstones",
            "search_generations",
        ] {
            manager
                .drop_table(
                    Table::drop()
                        .table(Alias::new(table))
                        .if_exists()
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }
}

/// Append these migrations so persistent search can share the application's connection.
pub fn migrations() -> Vec<Box<dyn MigrationTrait>> {
    vec![Box::new(CreateSearchTables)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};

    #[tokio::test]
    async fn sqlite_search_metadata_migration_round_trip() -> Result<(), DbErr> {
        let database = Database::connect("sqlite::memory:").await?;
        let manager = SchemaManager::new(&database);
        CreateSearchTables.up(&manager).await?;
        CreateSearchTables.up(&manager).await?;
        for table in [
            "search_generations",
            "search_tombstones",
            "search_vocabulary",
        ] {
            assert!(manager.has_table(table).await?);
        }

        let insert = Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "INSERT INTO search_generations (index_name, generation, state, settings_json, digest) VALUES (?, ?, ?, ?, ?)",
            [
                "catalog".into(),
                "default".into(),
                "active".into(),
                "{}".into(),
                "digest".into(),
            ],
        );
        database.execute_raw(insert.clone()).await?;
        assert!(database.execute_raw(insert).await.is_err());
        let row = database.query_one_raw(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "SELECT write_count, refreshed_count, pending_vectors_json, model, dimensions FROM search_generations WHERE index_name = ? AND generation = ?",
            ["catalog".into(), "default".into()],
        )).await?.ok_or_else(|| DbErr::Custom("search generation catalog/default was not persisted".into()))?;
        assert_eq!(row.try_get::<i64>("", "write_count")?, 0);
        assert_eq!(row.try_get::<i64>("", "refreshed_count")?, 0);
        assert_eq!(row.try_get::<String>("", "pending_vectors_json")?, "[]");
        assert_eq!(row.try_get::<Option<String>>("", "model")?, None);
        assert_eq!(row.try_get::<Option<i64>>("", "dimensions")?, None);

        CreateSearchTables.down(&manager).await?;
        CreateSearchTables.down(&manager).await?;
        for table in [
            "search_generations",
            "search_tombstones",
            "search_vocabulary",
        ] {
            assert!(!manager.has_table(table).await?);
        }
        Ok(())
    }
}
