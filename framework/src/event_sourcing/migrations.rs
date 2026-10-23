//! Applications register these tables with their own migrator.
use sea_orm::{ConnectionTrait, DbBackend};
use sea_orm_migration::prelude::*;

/// Creates durable histories, snapshots and ordered projector checkpoints.
pub struct CreateEventSourcingTables;

impl MigrationName for CreateEventSourcingTables {
    fn name(&self) -> &str {
        "m20261022_000001_create_event_sourcing_tables"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateEventSourcingTables {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut events = Table::create();
        events
            .table(Alias::new("stored_events"))
            .col(
                ColumnDef::new(Alias::new("id"))
                    .big_integer()
                    .not_null()
                    .auto_increment()
                    .primary_key(),
            )
            .col(
                ColumnDef::new(Alias::new("aggregate_uuid"))
                    .string_len(36)
                    .not_null(),
            )
            .col(
                ColumnDef::new(Alias::new("aggregate_version"))
                    .big_integer()
                    .not_null(),
            )
            .col(
                ColumnDef::new(Alias::new("event_version"))
                    .integer()
                    .not_null(),
            )
            .col(
                ColumnDef::new(Alias::new("event_type"))
                    .string_len(191)
                    .not_null(),
            )
            .col(
                ColumnDef::new(Alias::new("payload"))
                    .json_binary()
                    .not_null(),
            )
            .col(
                ColumnDef::new(Alias::new("metadata"))
                    .json_binary()
                    .not_null(),
            )
            .col(
                ColumnDef::new(Alias::new("created_at"))
                    .date_time()
                    .not_null(),
            )
            .index(
                Index::create()
                    .name("stored_events_aggregate_version_unique")
                    .unique()
                    .col(Alias::new("aggregate_uuid"))
                    .col(Alias::new("aggregate_version")),
            );
        let mut snapshots = Table::create();
        snapshots
            .table(Alias::new("snapshots"))
            .col(ColumnDef::new(Alias::new("uuid")).string_len(36).not_null())
            .col(
                ColumnDef::new(Alias::new("aggregate_version"))
                    .big_integer()
                    .not_null(),
            )
            .col(ColumnDef::new(Alias::new("state")).json_binary().not_null())
            .col(
                ColumnDef::new(Alias::new("created_at"))
                    .date_time()
                    .not_null(),
            )
            .primary_key(
                Index::create()
                    .col(Alias::new("uuid"))
                    .col(Alias::new("aggregate_version")),
            );
        let mut checkpoints = Table::create();
        checkpoints
            .table(Alias::new("projector_checkpoints"))
            .col(
                ColumnDef::new(Alias::new("name"))
                    .string_len(191)
                    .not_null()
                    .primary_key(),
            )
            .col(
                ColumnDef::new(Alias::new("last_event_id"))
                    .big_integer()
                    .not_null(),
            );
        if manager.get_database_backend() == DbBackend::MySql {
            // Names are identities, so letter case must not merge two handlers.
            events
                .engine("InnoDB")
                .character_set("utf8mb4")
                .collate("utf8mb4_bin");
            snapshots
                .engine("InnoDB")
                .character_set("utf8mb4")
                .collate("utf8mb4_bin");
            checkpoints
                .engine("InnoDB")
                .character_set("utf8mb4")
                .collate("utf8mb4_bin");
        }
        manager.create_table(events.to_owned()).await?;
        manager.create_table(snapshots.to_owned()).await?;
        manager.create_table(checkpoints.to_owned()).await?;
        let initial = Query::insert()
            .into_table(Alias::new("projector_checkpoints"))
            .columns([Alias::new("name"), Alias::new("last_event_id")])
            .values(["__event_store__".into(), 0i64.into()])
            .map_err(|e| DbErr::Migration(e.to_string()))?
            .to_owned();
        manager
            .get_connection()
            .execute_raw(manager.get_database_backend().build(&initial))
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in ["projector_checkpoints", "snapshots", "stored_events"] {
            manager
                .drop_table(Table::drop().table(Alias::new(table)).to_owned())
                .await?;
        }
        Ok(())
    }
}

/// Returns the framework migration for application registration.
pub fn migrations() -> Vec<Box<dyn MigrationTrait>> {
    vec![Box::new(CreateEventSourcingTables)]
}
