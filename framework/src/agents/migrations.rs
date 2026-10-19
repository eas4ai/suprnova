//! The tables the two Magnetar plugins keep, as migrations an application
//! appends to its migrator (AGENT-017).
//!
//! The OAuth server and agent auth can create their tables at boot
//! ([`OAuthServer::migrate`](crate::agents::OAuthServer::migrate),
//! [`AgentAuth::migrate`](crate::agents::AgentAuth::migrate)), and the
//! Magnetar default engine's migrations create them too. These migrations
//! let an application that runs its own migrator create them in the same
//! run as its other tables, recorded and reversible like the rest:
//!
//! ```rust,ignore
//! impl MigratorTrait for Migrator {
//!     fn migrations() -> Vec<Box<dyn MigrationTrait>> {
//!         let mut migrations: Vec<Box<dyn MigrationTrait>> = vec![/* ... */];
//!         migrations.extend(suprnova::agents::migrations());
//!         migrations
//!     }
//! }
//! ```
//!
//! Each migration creates only what is missing, so it is safe beside
//! [`init_magnetar`](crate::init_magnetar) or a boot-time `migrate`, which
//! create the same tables.

use sea_orm_migration::prelude::*;

/// The authorization server's four tables and their indexes:
/// `auth_oauth_clients`, `auth_oauth_authorizations`, `auth_oauth_grants`
/// and `auth_oauth_tokens`.
pub struct CreateOAuthServerTables;

impl MigrationName for CreateOAuthServerTables {
    fn name(&self) -> &str {
        "m20261019_000001_create_oauth_server_tables"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateOAuthServerTables {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // On PostgreSQL the migrator hands a transaction: the tables and
        // indexes commit with the migration's record, or not at all.
        magnetar::authorization_server::migrate(manager.get_connection())
            .await
            .map_err(|error| {
                DbErr::Migration(format!("create the authorization server's tables: {error}"))
            })
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_tables(
            manager,
            &[
                "auth_oauth_tokens",
                "auth_oauth_grants",
                "auth_oauth_authorizations",
                "auth_oauth_clients",
            ],
        )
        .await
    }
}

/// Agent auth's four tables and their indexes: `auth_agent_registrations`,
/// `auth_agent_grants`, `auth_agent_replays` and `auth_agent_events`, the
/// outbox of claim events.
pub struct CreateAgentAuthTables;

impl MigrationName for CreateAgentAuthTables {
    fn name(&self) -> &str {
        "m20261019_000002_create_agent_auth_tables"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateAgentAuthTables {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        magnetar::agent_auth::migrate(manager.get_connection())
            .await
            .map_err(|error| DbErr::Migration(format!("create agent auth's tables: {error}")))
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_tables(
            manager,
            &[
                "auth_agent_events",
                "auth_agent_replays",
                "auth_agent_grants",
                "auth_agent_registrations",
            ],
        )
        .await
    }
}

/// Drop `tables` in order, each only if it exists. The lists above name
/// the tables that hold references to others first.
async fn drop_tables(manager: &SchemaManager<'_>, tables: &[&str]) -> Result<(), DbErr> {
    for table in tables {
        manager
            .drop_table(
                Table::drop()
                    .table(Alias::new(*table))
                    .if_exists()
                    .to_owned(),
            )
            .await?;
    }
    Ok(())
}

/// Both plugins' tables, the authorization server's first: agent auth
/// issues its tokens on the authorization server.
pub fn migrations() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(CreateOAuthServerTables),
        Box::new(CreateAgentAuthTables),
    ]
}
