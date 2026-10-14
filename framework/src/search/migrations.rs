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
    vec![Box::new(CreateSearchTables), Box::new(CreateSearchJobs)]
}

/// Durable pending references let every queue driver participate in generation publication.
pub struct CreateSearchJobs;

impl MigrationName for CreateSearchJobs {
    fn name(&self) -> &str {
        "m20261014_000002_create_search_jobs"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateSearchJobs {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut payload = ColumnDef::new(Alias::new("payload_json"));
        if manager.get_database_backend() == DbBackend::MySql {
            payload.custom(Alias::new("LONGTEXT"));
        } else {
            payload.text();
        }
        payload.not_null();
        let mut jobs = Table::create()
            .table(Alias::new("search_jobs_pending"))
            .if_not_exists()
            .col(
                ColumnDef::new(Alias::new("job_id"))
                    .string_len(64)
                    .not_null()
                    .primary_key(),
            )
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
                ColumnDef::new(Alias::new("sequence"))
                    .big_integer()
                    .not_null(),
            )
            .col(
                ColumnDef::new(Alias::new("job_name"))
                    .string_len(64)
                    .not_null(),
            )
            .col(&mut payload)
            .col(
                ColumnDef::new(Alias::new("published"))
                    .boolean()
                    .not_null()
                    .default(false),
            )
            .col(
                ColumnDef::new(Alias::new("completed"))
                    .boolean()
                    .not_null()
                    .default(false),
            )
            .col(ColumnDef::new(Alias::new("completed_at")).big_integer())
            .to_owned();
        if manager.get_database_backend() == DbBackend::MySql {
            jobs.character_set("utf8mb4").collate("utf8mb4_bin");
        }
        manager.create_table(jobs).await?;
        if !manager
            .has_column("search_generations", "switch_watermark")
            .await?
        {
            manager
                .alter_table(
                    Table::alter()
                        .table(Alias::new("search_generations"))
                        .add_column(ColumnDef::new(Alias::new("switch_watermark")).big_integer())
                        .to_owned(),
                )
                .await?;
        }
        for (name, len) in [
            ("building_generation", 64),
            ("building_digest", 64),
            ("creation_owner", 36),
        ] {
            if !manager.has_column("search_generations", name).await? {
                manager
                    .alter_table(
                        Table::alter()
                            .table(Alias::new("search_generations"))
                            .add_column(ColumnDef::new(Alias::new(name)).string_len(len))
                            .to_owned(),
                    )
                    .await?;
            }
        }
        if !manager
            .has_column("search_generations", "creation_expires_at")
            .await?
        {
            manager
                .alter_table(
                    Table::alter()
                        .table(Alias::new("search_generations"))
                        .add_column(ColumnDef::new(Alias::new("creation_expires_at")).big_integer())
                        .to_owned(),
                )
                .await?;
        }
        for name in ["flush_version", "flush_removed_at"] {
            if !manager.has_column("search_generations", name).await? {
                manager
                    .alter_table(
                        Table::alter()
                            .table(Alias::new("search_generations"))
                            .add_column(ColumnDef::new(Alias::new(name)).big_integer())
                            .to_owned(),
                    )
                    .await?;
            }
        }
        let mut intent_payload = ColumnDef::new(Alias::new("payload_json"));
        if manager.get_database_backend() == DbBackend::MySql {
            intent_payload.custom(Alias::new("LONGTEXT"));
        } else {
            intent_payload.text();
        }
        intent_payload.not_null();
        let mut intents = Table::create()
            .table(Alias::new("search_job_intents"))
            .if_not_exists()
            .col(
                ColumnDef::new(Alias::new("intent_id"))
                    .string_len(64)
                    .not_null()
                    .primary_key(),
            )
            .col(
                ColumnDef::new(Alias::new("index_name"))
                    .string_len(128)
                    .not_null(),
            )
            .col(
                ColumnDef::new(Alias::new("job_name"))
                    .string_len(64)
                    .not_null(),
            )
            .col(&mut intent_payload)
            .col(
                ColumnDef::new(Alias::new("created_at"))
                    .big_integer()
                    .not_null(),
            )
            .to_owned();
        if manager.get_database_backend() == DbBackend::MySql {
            intents.character_set("utf8mb4").collate("utf8mb4_bin");
        }
        manager.create_table(intents).await?;
        if !manager
            .has_index("search_jobs_pending", "search_jobs_generation_sequence")
            .await?
        {
            manager
                .create_index(
                    Index::create()
                        .name("search_jobs_generation_sequence")
                        .table(Alias::new("search_jobs_pending"))
                        .col(Alias::new("index_name"))
                        .col(Alias::new("generation"))
                        .col(Alias::new("sequence"))
                        .to_owned(),
                )
                .await?;
        }
        manager.get_connection().execute_unprepared("UPDATE search_generations SET state=CASE state WHEN 'active' THEN 'current' WHEN 'inactive' THEN 'building' ELSE state END WHERE state IN ('active','inactive')").await?;
        Ok(())
    }
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(Alias::new("search_job_intents"))
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(Alias::new("search_jobs_pending"))
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        for name in [
            "building_generation",
            "building_digest",
            "creation_owner",
            "creation_expires_at",
            "flush_version",
            "flush_removed_at",
        ] {
            if manager.has_column("search_generations", name).await? {
                manager
                    .alter_table(
                        Table::alter()
                            .table(Alias::new("search_generations"))
                            .drop_column(Alias::new(name))
                            .to_owned(),
                    )
                    .await?;
            }
        }
        if manager
            .has_column("search_generations", "switch_watermark")
            .await?
        {
            manager
                .alter_table(
                    Table::alter()
                        .table(Alias::new("search_generations"))
                        .drop_column(Alias::new("switch_watermark"))
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }
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
        CreateSearchJobs.up(&manager).await?;
        CreateSearchJobs.up(&manager).await?;
        for table in [
            "search_generations",
            "search_tombstones",
            "search_vocabulary",
            "search_jobs_pending",
            "search_job_intents",
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

        CreateSearchJobs.down(&manager).await?;
        CreateSearchJobs.down(&manager).await?;
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
