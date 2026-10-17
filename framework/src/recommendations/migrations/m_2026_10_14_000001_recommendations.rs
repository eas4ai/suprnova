//! Version one stores immutable events, private evidence and atomically published runs.
use sea_orm::{ConnectionTrait, Statement};
use sea_orm_migration::prelude::*;

/// The starter and future graph engine share this framework-owned storage contract.
#[derive(DeriveMigrationName)]
pub struct Migration;

pub(super) fn text(name: &str, size: u32) -> ColumnDef {
    ColumnDef::new(Alias::new(name))
        .string_len(size)
        .not_null()
        .to_owned()
}
pub(super) fn integer(name: &str) -> ColumnDef {
    ColumnDef::new(Alias::new(name))
        .big_integer()
        .not_null()
        .to_owned()
}
pub(super) fn json(name: &str, backend: sea_orm::DbBackend) -> ColumnDef {
    // JSON text has identical canonical bytes on all three dialects for digest verification.
    let mut column = ColumnDef::new(Alias::new(name));
    if backend == sea_orm::DbBackend::MySql {
        column.custom(Alias::new("LONGTEXT"));
    } else {
        column.text();
    }
    column.not_null().to_owned()
}
pub(super) async fn index(
    manager: &SchemaManager<'_>,
    table: &str,
    name: &str,
    columns: &[&str],
    unique: bool,
) -> Result<(), DbErr> {
    let mut statement = Index::create();
    statement.name(name).table(Alias::new(table));
    for column in columns {
        statement.col(Alias::new(*column));
    }
    if unique {
        statement.unique();
    }
    crate::database::migration_guard::create_index_if_missing(manager, table, statement.to_owned())
        .await
}
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        manager
            .create_table(
                Table::create()
                    .table(Alias::new("recommendation_events"))
                    .col(
                        ColumnDef::new(Alias::new("id"))
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(text("actor_kind", 16))
                    .col(text("actor_id", 128))
                    .col(text("target_index", 128))
                    .col(text("target_key", 255))
                    .col(text("event_type", 128))
                    .col(integer("weight"))
                    .col(text("surface", 128))
                    .col(integer("occurred_at"))
                    .col(json("access_scope", backend))
                    .col(integer("source_version"))
                    .col(text("dedupe_key", 128))
                    .col(text("event_class", 16))
                    .col(integer("score"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "recommendation_events",
            "recommendation_events_dedupe",
            &["actor_kind", "actor_id", "dedupe_key"],
            true,
        )
        .await?;
        index(
            manager,
            "recommendation_events",
            "recommendation_events_actor_time",
            &["actor_kind", "actor_id", "occurred_at", "id"],
            false,
        )
        .await?;
        index(
            manager,
            "recommendation_events",
            "recommendation_events_target",
            &["target_index", "target_key", "occurred_at"],
            false,
        )
        .await?;
        index(
            manager,
            "recommendation_events",
            "recommendation_events_popularity",
            &["event_class", "occurred_at"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table(Alias::new("recommendation_runs"))
                    .col(
                        ColumnDef::new(Alias::new("id"))
                            .string_len(36)
                            .not_null()
                            .primary_key(),
                    )
                    .col(integer("started_at"))
                    .col(integer("duration_ms"))
                    .col(integer("events_read"))
                    .col(integer("new_events"))
                    .col(integer("users_covered"))
                    .col(integer("items_scored"))
                    .col(ColumnDef::new(Alias::new("coverage")).double().not_null())
                    .col(
                        ColumnDef::new(Alias::new("fallback_share"))
                            .double()
                            .not_null(),
                    )
                    .col(integer("high_water_mark"))
                    .col(text("catalog_digest", 64).default(""))
                    .col(json("actor_history_snapshots", backend).null())
                    .col(text("state", 16))
                    .col(ColumnDef::new(Alias::new("error")).text().null())
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "recommendation_runs",
            "recommendation_runs_latest",
            &["state", "started_at", "id"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table(Alias::new("recommendation_explanations"))
                    .col(
                        ColumnDef::new(Alias::new("id"))
                            .string_len(36)
                            .not_null()
                            .primary_key(),
                    )
                    .col(text("run_id", 36))
                    .col(json("event_ids", backend))
                    .col(json("attribute_values", backend))
                    .col(integer("peer_count"))
                    .col(integer("shared_target_count"))
                    .col(json("shared_targets", backend))
                    .col(text("fallback_tier", 32))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "recommendation_explanations",
            "recommendation_explanations_run",
            &["run_id"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table(Alias::new("starter_recommendations"))
                    .col(
                        ColumnDef::new(Alias::new("id"))
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(text("run_id", 36))
                    .col(text("actor_kind", 16))
                    .col(text("actor_id", 128))
                    .col(text("surface", 128))
                    .col(text("target_index", 128))
                    .col(text("target_key", 255))
                    .col(integer("score"))
                    .col(integer("rank"))
                    .col(text("explanation_id", 36))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "starter_recommendations",
            "starter_recommendations_actor_rank",
            &["run_id", "actor_kind", "actor_id", "surface", "rank"],
            true,
        )
        .await?;
        index(
            manager,
            "starter_recommendations",
            "starter_recommendations_target",
            &["target_index", "target_key"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table(Alias::new("recommendation_schema"))
                    .col(
                        ColumnDef::new(Alias::new("id"))
                            .integer()
                            .not_null()
                            .primary_key(),
                    )
                    .col(integer("recommendation_schema_version"))
                    .to_owned(),
            )
            .await?;
        let backend = manager.get_database_backend();
        let placeholders = if backend == sea_orm::DbBackend::Postgres {
            "$1, $2"
        } else {
            "?, ?"
        };
        manager.get_connection().execute_raw(Statement::from_sql_and_values(backend,
            format!("INSERT INTO recommendation_schema (id, recommendation_schema_version) VALUES ({placeholders})"),
            vec![1.into(), 1_i64.into()])).await?;
        Ok(())
    }
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            "starter_recommendations",
            "recommendation_explanations",
            "recommendation_runs",
            "recommendation_events",
            "recommendation_schema",
        ] {
            manager
                .drop_table(Table::drop().table(Alias::new(table)).to_owned())
                .await?;
        }
        Ok(())
    }
}
