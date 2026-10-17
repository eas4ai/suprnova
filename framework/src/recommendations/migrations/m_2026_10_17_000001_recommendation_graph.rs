//! Version two adds the graph engine's generations, nodes and edges (REC-005, REC-009).
use super::m_2026_10_14_000001_recommendations::{index, integer, json, text};
use sea_orm::{ConnectionTrait, Statement};
use sea_orm_migration::prelude::*;

/// The graph engine's facts in the application's database, for the `database` store.
#[derive(DeriveMigrationName)]
pub struct Migration;

fn optional(name: &str, size: u32) -> ColumnDef {
    ColumnDef::new(Alias::new(name))
        .string_len(size)
        .null()
        .to_owned()
}

async fn schema_version(manager: &SchemaManager<'_>, version: i64) -> Result<(), DbErr> {
    let backend = manager.get_database_backend();
    let placeholder = if backend == sea_orm::DbBackend::Postgres {
        "$1"
    } else {
        "?"
    };
    manager
        .get_connection()
        .execute_raw(Statement::from_sql_and_values(
            backend,
            format!(
                "UPDATE recommendation_schema SET recommendation_schema_version = {placeholder} WHERE id = 1"
            ),
            vec![version.into()],
        ))
        .await?;
    Ok(())
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        manager
            .create_table(
                Table::create()
                    .table(Alias::new("recommendation_graph_generations"))
                    .col(
                        ColumnDef::new(Alias::new("id"))
                            .string_len(36)
                            .not_null()
                            .primary_key(),
                    )
                    .col(text("state", 16))
                    .col(integer("created_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Alias::new("recommendation_graph_nodes"))
                    .col(text("generation_id", 36))
                    .col(text("node_key", 64))
                    .col(text("kind", 128))
                    .col(optional("target_index", 128))
                    .col(optional("target_key", 255))
                    .col(json("scope_json", backend))
                    .col(integer("source_version"))
                    .col(json("provenance_json", backend))
                    .col(
                        ColumnDef::new(Alias::new("provenance_event"))
                            .big_integer()
                            .null(),
                    )
                    .col(optional("provenance_source", 400))
                    .col(
                        ColumnDef::new(Alias::new("provenance_version"))
                            .big_integer()
                            .null(),
                    )
                    .primary_key(
                        Index::create()
                            .col(Alias::new("generation_id"))
                            .col(Alias::new("node_key")),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Alias::new("recommendation_graph_edges"))
                    .col(text("generation_id", 36))
                    .col(text("edge_key", 64))
                    .col(text("source_key", 64))
                    .col(text("target_key", 64))
                    .col(text("kind", 160))
                    .col(
                        ColumnDef::new(Alias::new("confidence_bps"))
                            .integer()
                            .not_null(),
                    )
                    .col(json("scope_json", backend))
                    .col(integer("source_version"))
                    .col(json("provenance_json", backend))
                    .col(
                        ColumnDef::new(Alias::new("provenance_event"))
                            .big_integer()
                            .null(),
                    )
                    .col(optional("provenance_source", 400))
                    .col(
                        ColumnDef::new(Alias::new("provenance_version"))
                            .big_integer()
                            .null(),
                    )
                    .col(integer("created_at"))
                    .col(integer("updated_at"))
                    .primary_key(
                        Index::create()
                            .col(Alias::new("generation_id"))
                            .col(Alias::new("edge_key")),
                    )
                    .to_owned(),
            )
            .await?;
        for (table, name, columns) in [
            (
                "recommendation_graph_nodes",
                "recommendation_graph_nodes_event",
                &["generation_id", "provenance_event"][..],
            ),
            (
                "recommendation_graph_nodes",
                "recommendation_graph_nodes_source",
                &["generation_id", "provenance_source"][..],
            ),
            (
                "recommendation_graph_edges",
                "recommendation_graph_edges_hop",
                &["generation_id", "source_key", "confidence_bps"][..],
            ),
            (
                "recommendation_graph_edges",
                "recommendation_graph_edges_target",
                &["generation_id", "target_key"][..],
            ),
            (
                "recommendation_graph_edges",
                "recommendation_graph_edges_event",
                &["generation_id", "provenance_event"][..],
            ),
            (
                "recommendation_graph_edges",
                "recommendation_graph_edges_source",
                &["generation_id", "provenance_source"][..],
            ),
        ] {
            index(manager, table, name, columns, false).await?;
        }
        schema_version(manager, 2).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            "recommendation_graph_edges",
            "recommendation_graph_nodes",
            "recommendation_graph_generations",
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
        schema_version(manager, 1).await
    }
}
