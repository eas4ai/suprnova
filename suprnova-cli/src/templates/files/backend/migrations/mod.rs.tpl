pub use sea_orm_migration::prelude::*;

mod m20240101_000001_create_users_table;
mod m20240101_000002_create_sessions_table;
mod m20240101_000003_create_remember_tokens_table;
mod m20240101_000004_create_auth_flow_tokens_table;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20240101_000001_create_users_table::Migration),
            Box::new(m20240101_000002_create_sessions_table::Migration),
            Box::new(m20240101_000003_create_remember_tokens_table::Migration),
            Box::new(m20240101_000004_create_auth_flow_tokens_table::Migration),
            // The RenderCache Tier 0 generation ledger. The framework ships
            // the migration; listing it here provisions the
            // `suprnova_render_*` tables alongside this project's own
            // schema. `live::routes_with_render_cache` fails at boot without
            // them.
            Box::new(suprnova::render_cache::migration::Migration),
        ]
    }
}
