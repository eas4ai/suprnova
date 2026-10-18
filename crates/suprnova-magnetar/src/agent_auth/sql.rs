//! SeaORM tables and store for agent auth on SQLite, PostgreSQL and
//! MySQL/MariaDB.
//!
//! Three tables belong to the plugin: `auth_agent_registrations` holds one
//! row per registration with its claim ceremony, `auth_agent_grants` the
//! one authorization server grant a registration holds per resource, and
//! `auth_agent_replays` the identifiers of ID-JAGs and Security Event
//! Tokens already seen. [`migrate`] creates them, and
//! [`crate::default_schema::migrate`] calls it when the `agent-auth`
//! feature is on.
//!
//! Every column has an explicit type. Anything that can pass 255
//! characters is `Text`, because MySQL in strict mode refuses a longer
//! write to the `varchar(255)` a plain string becomes; indexed columns hold
//! digests and identifiers, which stay short.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sea_orm::sea_query::{Alias, Expr, Index};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, Condition, ConnectionTrait, DatabaseConnection, DbBackend,
    EntityTrait, ExprTrait, QueryFilter, QueryOrder, QuerySelect, Schema,
};

use super::store::{
    AgentAuthStore, AgentGrantRecord, IDENTITY_ASSERTION, NewAttempt, RegistrationRecord,
    STATUS_ACTIVE, STATUS_CLAIMED, STATUS_EXPIRED, STATUS_PENDING, STATUS_REVOKED,
    STATUS_UNCLAIMED,
};
use crate::storage::db_error;
use crate::storage::migrations::has_index;
use crate::{Error, Result};

/// SeaORM entity for the `auth_agent_registrations` table.
#[allow(missing_docs)]
pub mod registrations {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_agent_registrations")]
    pub struct Model {
        #[sea_orm(
            primary_key,
            auto_increment = false,
            column_type = "String(StringLen::N(64))"
        )]
        pub id: String,
        #[sea_orm(column_type = "String(StringLen::N(32))")]
        pub kind: String,
        #[sea_orm(column_type = "String(StringLen::N(32))")]
        pub status: String,
        #[sea_orm(column_type = "String(StringLen::N(191))", nullable)]
        pub user_id: Option<String>,
        #[sea_orm(column_type = "Text", nullable)]
        pub claim_email: Option<String>,
        #[sea_orm(column_type = "Text", nullable)]
        pub claim_user_id: Option<String>,
        #[sea_orm(column_type = "String(StringLen::N(64))", nullable)]
        pub claim_token_hash: Option<String>,
        #[sea_orm(nullable)]
        pub claim_expires_at: Option<DateTimeUtc>,
        #[sea_orm(column_type = "String(StringLen::N(64))", nullable)]
        pub attempt_id: Option<String>,
        #[sea_orm(column_type = "String(StringLen::N(64))", nullable)]
        pub attempt_token_hash: Option<String>,
        #[sea_orm(column_type = "String(StringLen::N(64))", nullable)]
        pub user_code_hash: Option<String>,
        #[sea_orm(nullable)]
        pub user_code_expires_at: Option<DateTimeUtc>,
        #[sea_orm(column_type = "Integer")]
        pub user_code_failures: i32,
        #[sea_orm(nullable)]
        pub last_polled_at: Option<DateTimeUtc>,
        #[sea_orm(column_type = "String(StringLen::N(64))", nullable)]
        pub delegation_key: Option<String>,
        #[sea_orm(column_type = "Text", nullable)]
        pub issuer: Option<String>,
        #[sea_orm(column_type = "Text", nullable)]
        pub subject: Option<String>,
        #[sea_orm(column_type = "Text")]
        pub scopes: String,
        #[sea_orm(column_type = "Integer")]
        pub assertion_version: i32,
        #[sea_orm(column_type = "BigInteger", nullable)]
        pub auth_epoch: Option<i64>,
        #[sea_orm(column_type = "Text", nullable)]
        pub client_address: Option<String>,
        #[sea_orm(column_type = "Boolean")]
        pub cleanup_pending: bool,
        pub created_at: DateTimeUtc,
        #[sea_orm(nullable)]
        pub claimed_at: Option<DateTimeUtc>,
        #[sea_orm(nullable)]
        pub revoked_at: Option<DateTimeUtc>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// SeaORM entity for the `auth_agent_grants` table.
#[allow(missing_docs)]
pub mod agent_grants {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_agent_grants")]
    pub struct Model {
        /// Digest of the registration and the resource: one row per pair.
        #[sea_orm(
            primary_key,
            auto_increment = false,
            column_type = "String(StringLen::N(64))"
        )]
        pub id: String,
        #[sea_orm(column_type = "String(StringLen::N(64))")]
        pub registration_id: String,
        #[sea_orm(column_type = "Text")]
        pub resource: String,
        #[sea_orm(column_type = "String(StringLen::N(64))")]
        pub grant_id: String,
        #[sea_orm(column_type = "Text", nullable)]
        pub user_id: Option<String>,
        #[sea_orm(column_type = "Text")]
        pub scopes: String,
        pub created_at: DateTimeUtc,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// SeaORM entity for the `auth_agent_replays` table.
#[allow(missing_docs)]
pub mod replays {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_agent_replays")]
    pub struct Model {
        #[sea_orm(
            primary_key,
            auto_increment = false,
            column_type = "String(StringLen::N(64))"
        )]
        pub id: String,
        pub expires_at: DateTimeUtc,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// Create agent auth's tables and indexes when they are missing. Running
/// it again changes nothing, and two processes running it at once both
/// succeed.
///
/// # Errors
///
/// Returns an error for a backend other than SQLite, PostgreSQL or MySQL,
/// or when the database refuses a statement.
pub async fn migrate(db: &DatabaseConnection) -> Result<()> {
    let backend = db.get_database_backend();
    if !matches!(
        backend,
        DbBackend::Sqlite | DbBackend::Postgres | DbBackend::MySql
    ) {
        return Err(Error::DependencyUnavailable {
            dependency: "database backend".to_owned(),
            message: format!("unsupported SeaORM database backend: {backend:?}"),
        });
    }
    let schema = Schema::new(backend);
    for statement in [
        schema
            .create_table_from_entity(registrations::Entity)
            .if_not_exists()
            .to_owned(),
        schema
            .create_table_from_entity(agent_grants::Entity)
            .if_not_exists()
            .to_owned(),
        schema
            .create_table_from_entity(replays::Entity)
            .if_not_exists()
            .to_owned(),
    ] {
        db.execute(&statement)
            .await
            .map_err(|error| Error::Internal {
                message: format!("create agent auth table: {error}"),
            })?;
    }
    // The one-time tokens and the delegation are looked up by digest, and a
    // digest must name one registration: unique keys, so the database
    // refuses a second row rather than a lookup reading either. Each allows
    // any number of NULLs, the value a spent token leaves.
    for (name, column) in [
        (
            "auth_agent_registrations_claim_token_unique",
            "claim_token_hash",
        ),
        (
            "auth_agent_registrations_attempt_token_unique",
            "attempt_token_hash",
        ),
        (
            "auth_agent_registrations_delegation_unique",
            "delegation_key",
        ),
    ] {
        ensure_index(db, "auth_agent_registrations", name, &[column], true).await?;
    }
    ensure_index(
        db,
        "auth_agent_registrations",
        "auth_agent_registrations_user_id",
        &["user_id"],
        false,
    )
    .await?;
    ensure_index(
        db,
        "auth_agent_registrations",
        "auth_agent_registrations_status",
        &["status", "claim_expires_at"],
        false,
    )
    .await?;
    ensure_index(
        db,
        "auth_agent_grants",
        "auth_agent_grants_registration_id",
        &["registration_id"],
        false,
    )
    .await?;
    ensure_index(
        db,
        "auth_agent_replays",
        "auth_agent_replays_expires_at",
        &["expires_at"],
        false,
    )
    .await
}

/// Create an index unless it exists; another process creating it between
/// the check and the create is not a failure.
async fn ensure_index(
    db: &DatabaseConnection,
    table: &str,
    name: &str,
    columns: &[&'static str],
    unique: bool,
) -> Result<()> {
    if has_index(db, None, table, name).await? {
        return Ok(());
    }
    let mut index = Index::create();
    index.name(name).table(Alias::new(table));
    for column in columns {
        index.col(Alias::new(*column));
    }
    if unique {
        index.unique();
    }
    match db.execute(&index).await {
        Ok(_) => Ok(()),
        Err(error) => {
            if has_index(db, None, table, name).await? {
                Ok(())
            } else {
                Err(Error::Internal {
                    message: format!("create agent auth index {name}: {error}"),
                })
            }
        }
    }
}

/// The SeaORM implementation of [`AgentAuthStore`] over the tables
/// [`migrate`] creates.
#[derive(Clone)]
pub struct SqlAgentAuthStore {
    db: DatabaseConnection,
}

impl SqlAgentAuthStore {
    /// Bind the store to the application's database.
    #[must_use]
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }
}

fn is_unique_violation(error: &sea_orm::DbErr) -> bool {
    matches!(
        error.sql_err(),
        Some(sea_orm::SqlErr::UniqueConstraintViolation(_))
    )
}

fn record(model: registrations::Model) -> RegistrationRecord {
    RegistrationRecord {
        id: model.id,
        kind: model.kind,
        status: model.status,
        user_id: model.user_id,
        claim_email: model.claim_email,
        claim_user_id: model.claim_user_id,
        claim_token_hash: model.claim_token_hash,
        claim_expires_at: model.claim_expires_at,
        attempt_id: model.attempt_id,
        attempt_token_hash: model.attempt_token_hash,
        user_code_hash: model.user_code_hash,
        user_code_expires_at: model.user_code_expires_at,
        // A negative count was never written; reading it as the largest
        // count keeps the attempt closed.
        user_code_failures: u32::try_from(model.user_code_failures).unwrap_or(u32::MAX),
        last_polled_at: model.last_polled_at,
        delegation_key: model.delegation_key,
        issuer: model.issuer,
        subject: model.subject,
        scopes: model.scopes.split_whitespace().map(str::to_owned).collect(),
        // Likewise a version no assertion carries.
        assertion_version: u32::try_from(model.assertion_version).unwrap_or(u32::MAX),
        // A negative epoch was never written; it reads as one no user
        // holds, which ends the registration rather than skipping the check.
        auth_epoch: model
            .auth_epoch
            .map(|epoch| u64::try_from(epoch).unwrap_or(u64::MAX)),
        client_address: model.client_address,
        cleanup_pending: model.cleanup_pending,
        created_at: model.created_at,
        claimed_at: model.claimed_at,
        revoked_at: model.revoked_at,
    }
}

/// The key of a registration's grant row for one resource: short and
/// fixed-length whatever the resource's length, so it indexes on MySQL.
fn grant_key(registration_id: &str, resource: &str) -> String {
    crate::authorization_server::token_digest(&format!("{registration_id}\n{resource}"))
}

fn grant_record(model: agent_grants::Model) -> AgentGrantRecord {
    AgentGrantRecord {
        registration_id: model.registration_id,
        resource: model.resource,
        grant_id: model.grant_id,
        user_id: model.user_id,
        scopes: model.scopes.split_whitespace().map(str::to_owned).collect(),
    }
}

fn epoch_to_db(epoch: Option<u64>) -> Result<Option<i64>> {
    epoch
        .map(|value| {
            i64::try_from(value).map_err(|_| Error::InvalidInput {
                field: "auth_epoch".to_owned(),
                message: "exceeds the database integer range".to_owned(),
            })
        })
        .transpose()
}

fn to_i32(value: u32, field: &str) -> Result<i32> {
    i32::try_from(value).map_err(|_| Error::InvalidInput {
        field: field.to_owned(),
        message: "exceeds the database integer range".to_owned(),
    })
}

/// The statuses in which a registration still waits for its claim.
fn claimable() -> Condition {
    Condition::any()
        .add(registrations::Column::Status.eq(STATUS_UNCLAIMED))
        .add(registrations::Column::Status.eq(STATUS_PENDING))
}

impl SqlAgentAuthStore {
    async fn find_one(&self, condition: Condition) -> Result<Option<RegistrationRecord>> {
        Ok(registrations::Entity::find()
            .filter(condition)
            .one(&self.db)
            .await
            .map_err(db_error)?
            .map(record))
    }
}

#[async_trait]
impl AgentAuthStore for SqlAgentAuthStore {
    async fn create_registration(&self, record: RegistrationRecord) -> Result<()> {
        let model = registrations::ActiveModel {
            id: Set(record.id),
            kind: Set(record.kind),
            status: Set(record.status),
            user_id: Set(record.user_id),
            claim_email: Set(record.claim_email),
            claim_user_id: Set(record.claim_user_id),
            claim_token_hash: Set(record.claim_token_hash),
            claim_expires_at: Set(record.claim_expires_at),
            attempt_id: Set(record.attempt_id),
            attempt_token_hash: Set(record.attempt_token_hash),
            user_code_hash: Set(record.user_code_hash),
            user_code_expires_at: Set(record.user_code_expires_at),
            user_code_failures: Set(to_i32(record.user_code_failures, "user_code_failures")?),
            last_polled_at: Set(record.last_polled_at),
            delegation_key: Set(record.delegation_key),
            issuer: Set(record.issuer),
            subject: Set(record.subject),
            scopes: Set(record.scopes.join(" ")),
            assertion_version: Set(to_i32(record.assertion_version, "assertion_version")?),
            auth_epoch: Set(epoch_to_db(record.auth_epoch)?),
            client_address: Set(record.client_address),
            cleanup_pending: Set(record.cleanup_pending),
            created_at: Set(record.created_at),
            claimed_at: Set(record.claimed_at),
            revoked_at: Set(record.revoked_at),
        };
        match registrations::Entity::insert(model).exec(&self.db).await {
            Ok(_) => Ok(()),
            Err(error) if is_unique_violation(&error) => Err(Error::Conflict {
                resource: "agent registration".to_owned(),
                message: "the delegation or a token digest is already registered".to_owned(),
            }),
            Err(error) => Err(db_error(error)),
        }
    }

    async fn find_registration(&self, id: &str) -> Result<Option<RegistrationRecord>> {
        self.find_one(Condition::all().add(registrations::Column::Id.eq(id.to_owned())))
            .await
    }

    async fn find_by_claim_token(&self, token_hash: &str) -> Result<Option<RegistrationRecord>> {
        self.find_one(
            Condition::all().add(registrations::Column::ClaimTokenHash.eq(token_hash.to_owned())),
        )
        .await
    }

    async fn find_by_attempt_token(&self, token_hash: &str) -> Result<Option<RegistrationRecord>> {
        self.find_one(
            Condition::all().add(registrations::Column::AttemptTokenHash.eq(token_hash.to_owned())),
        )
        .await
    }

    async fn find_by_delegation(&self, delegation_key: &str) -> Result<Option<RegistrationRecord>> {
        self.find_one(
            Condition::all()
                .add(registrations::Column::DelegationKey.eq(delegation_key.to_owned())),
        )
        .await
    }

    async fn start_attempt(
        &self,
        id: &str,
        claim_token_hash: &str,
        claim_email: &str,
        attempt: NewAttempt,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let moved = registrations::Entity::update_many()
            .col_expr(
                registrations::Column::ClaimEmail,
                Expr::value(claim_email.to_owned()),
            )
            .col_expr(
                registrations::Column::AttemptId,
                Expr::value(attempt.attempt_id),
            )
            .col_expr(
                registrations::Column::AttemptTokenHash,
                Expr::value(attempt.attempt_token_hash),
            )
            .col_expr(
                registrations::Column::UserCodeHash,
                Expr::value(attempt.user_code_hash),
            )
            .col_expr(
                registrations::Column::UserCodeExpiresAt,
                Expr::value(attempt.user_code_expires_at),
            )
            .col_expr(registrations::Column::UserCodeFailures, Expr::value(0))
            .filter(registrations::Column::Id.eq(id.to_owned()))
            .filter(registrations::Column::ClaimTokenHash.eq(claim_token_hash.to_owned()))
            .filter(claimable())
            .filter(registrations::Column::ClaimExpiresAt.gt(now))
            .exec(&self.db)
            .await
            .map_err(db_error)?;
        Ok(moved.rows_affected == 1)
    }

    async fn restart_step_up(
        &self,
        id: &str,
        claim_token_hash: &str,
        claim_expires_at: DateTime<Utc>,
        attempt: NewAttempt,
    ) -> Result<bool> {
        let moved = registrations::Entity::update_many()
            .col_expr(registrations::Column::Status, Expr::value(STATUS_PENDING))
            .col_expr(
                registrations::Column::ClaimTokenHash,
                Expr::value(claim_token_hash.to_owned()),
            )
            .col_expr(
                registrations::Column::ClaimExpiresAt,
                Expr::value(claim_expires_at),
            )
            .col_expr(
                registrations::Column::AttemptId,
                Expr::value(attempt.attempt_id),
            )
            .col_expr(
                registrations::Column::AttemptTokenHash,
                Expr::value(attempt.attempt_token_hash),
            )
            .col_expr(
                registrations::Column::UserCodeHash,
                Expr::value(attempt.user_code_hash),
            )
            .col_expr(
                registrations::Column::UserCodeExpiresAt,
                Expr::value(attempt.user_code_expires_at),
            )
            .col_expr(registrations::Column::UserCodeFailures, Expr::value(0))
            .col_expr(
                registrations::Column::RevokedAt,
                Expr::value(Option::<DateTime<Utc>>::None),
            )
            .filter(registrations::Column::Id.eq(id.to_owned()))
            .filter(registrations::Column::Kind.eq(IDENTITY_ASSERTION))
            .filter(
                Condition::any()
                    .add(registrations::Column::Status.eq(STATUS_PENDING))
                    .add(registrations::Column::Status.eq(STATUS_EXPIRED)),
            )
            .exec(&self.db)
            .await
            .map_err(db_error)?;
        Ok(moved.rows_affected == 1)
    }

    async fn record_code_failure(
        &self,
        id: &str,
        attempt_token_hash: &str,
        max_failures: u32,
    ) -> Result<bool> {
        let moved = registrations::Entity::update_many()
            .col_expr(
                registrations::Column::UserCodeFailures,
                Expr::col(registrations::Column::UserCodeFailures).add(1),
            )
            .filter(registrations::Column::Id.eq(id.to_owned()))
            .filter(registrations::Column::AttemptTokenHash.eq(attempt_token_hash.to_owned()))
            .filter(
                registrations::Column::UserCodeFailures
                    .lt(to_i32(max_failures, "max_user_code_failures")?),
            )
            .exec(&self.db)
            .await
            .map_err(db_error)?;
        Ok(moved.rows_affected == 1)
    }

    async fn confirm_claim(
        &self,
        id: &str,
        attempt_token_hash: &str,
        user_code_hash: &str,
        user_id: &str,
        auth_epoch: Option<u64>,
        max_failures: u32,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let none = Option::<String>::None;
        let moved = registrations::Entity::update_many()
            .col_expr(registrations::Column::Status, Expr::value(STATUS_CLAIMED))
            .col_expr(
                registrations::Column::UserId,
                Expr::value(user_id.to_owned()),
            )
            .col_expr(
                registrations::Column::AuthEpoch,
                Expr::value(epoch_to_db(auth_epoch)?),
            )
            .col_expr(registrations::Column::ClaimedAt, Expr::value(now))
            .col_expr(
                registrations::Column::AttemptTokenHash,
                Expr::value(none.clone()),
            )
            .col_expr(registrations::Column::UserCodeHash, Expr::value(none))
            .col_expr(
                registrations::Column::AssertionVersion,
                Expr::col(registrations::Column::AssertionVersion).add(1),
            )
            .filter(registrations::Column::Id.eq(id.to_owned()))
            .filter(registrations::Column::AttemptTokenHash.eq(attempt_token_hash.to_owned()))
            .filter(registrations::Column::UserCodeHash.eq(user_code_hash.to_owned()))
            .filter(
                registrations::Column::UserCodeFailures
                    .lt(to_i32(max_failures, "max_user_code_failures")?),
            )
            .filter(registrations::Column::UserCodeExpiresAt.gt(now))
            .filter(registrations::Column::ClaimExpiresAt.gt(now))
            .filter(claimable())
            .exec(&self.db)
            .await
            .map_err(db_error)?;
        Ok(moved.rows_affected == 1)
    }

    async fn record_poll(
        &self,
        id: &str,
        slot: DateTime<Utc>,
        earliest_previous: DateTime<Utc>,
    ) -> Result<bool> {
        let moved = registrations::Entity::update_many()
            .col_expr(registrations::Column::LastPolledAt, Expr::value(slot))
            .filter(registrations::Column::Id.eq(id.to_owned()))
            .filter(
                Condition::any()
                    .add(registrations::Column::LastPolledAt.is_null())
                    .add(registrations::Column::LastPolledAt.lte(earliest_previous)),
            )
            .exec(&self.db)
            .await
            .map_err(db_error)?;
        Ok(moved.rows_affected == 1)
    }

    async fn spend_claim_token(&self, id: &str, claim_token_hash: &str) -> Result<bool> {
        let moved = registrations::Entity::update_many()
            .col_expr(
                registrations::Column::ClaimTokenHash,
                Expr::value(Option::<String>::None),
            )
            .filter(registrations::Column::Id.eq(id.to_owned()))
            .filter(registrations::Column::ClaimTokenHash.eq(claim_token_hash.to_owned()))
            .filter(registrations::Column::Status.eq(STATUS_CLAIMED))
            .exec(&self.db)
            .await
            .map_err(db_error)?;
        Ok(moved.rows_affected == 1)
    }

    async fn revoke_registration(&self, id: &str, at: DateTime<Utc>) -> Result<bool> {
        let none = Option::<String>::None;
        let moved = registrations::Entity::update_many()
            .col_expr(registrations::Column::Status, Expr::value(STATUS_REVOKED))
            .col_expr(registrations::Column::RevokedAt, Expr::value(at))
            .col_expr(registrations::Column::CleanupPending, Expr::value(true))
            .col_expr(
                registrations::Column::ClaimTokenHash,
                Expr::value(none.clone()),
            )
            .col_expr(
                registrations::Column::AttemptTokenHash,
                Expr::value(none.clone()),
            )
            .col_expr(registrations::Column::UserCodeHash, Expr::value(none))
            .filter(registrations::Column::Id.eq(id.to_owned()))
            .filter(registrations::Column::Status.ne(STATUS_REVOKED))
            .exec(&self.db)
            .await
            .map_err(db_error)?;
        Ok(moved.rows_affected == 1)
    }

    async fn expire_registration(&self, id: &str, now: DateTime<Utc>) -> Result<bool> {
        let none = Option::<String>::None;
        let moved = registrations::Entity::update_many()
            .col_expr(registrations::Column::Status, Expr::value(STATUS_EXPIRED))
            .col_expr(registrations::Column::RevokedAt, Expr::value(now))
            .col_expr(registrations::Column::CleanupPending, Expr::value(true))
            .col_expr(
                registrations::Column::AttemptTokenHash,
                Expr::value(none.clone()),
            )
            .col_expr(registrations::Column::UserCodeHash, Expr::value(none))
            .filter(registrations::Column::Id.eq(id.to_owned()))
            .filter(claimable())
            .filter(registrations::Column::ClaimExpiresAt.lte(now))
            .exec(&self.db)
            .await
            .map_err(db_error)?;
        Ok(moved.rows_affected == 1)
    }

    async fn finish_cleanup(&self, id: &str, release_delegation: bool) -> Result<()> {
        let mut update = registrations::Entity::update_many()
            .col_expr(registrations::Column::CleanupPending, Expr::value(false))
            .filter(registrations::Column::Id.eq(id.to_owned()))
            .filter(
                Condition::any()
                    .add(registrations::Column::Status.eq(STATUS_REVOKED))
                    .add(registrations::Column::Status.eq(STATUS_EXPIRED)),
            );
        if release_delegation {
            update = update.col_expr(
                registrations::Column::DelegationKey,
                Expr::value(Option::<String>::None),
            );
        }
        update.exec(&self.db).await.map_err(db_error)?;
        Ok(())
    }

    async fn expirable_registrations(&self, now: DateTime<Utc>, limit: u64) -> Result<Vec<String>> {
        registrations::Entity::find()
            .select_only()
            .column(registrations::Column::Id)
            .filter(
                Condition::any()
                    .add(
                        Condition::all()
                            .add(claimable())
                            .add(registrations::Column::ClaimExpiresAt.lte(now)),
                    )
                    .add(
                        Condition::all()
                            .add(registrations::Column::Status.eq(STATUS_EXPIRED))
                            .add(registrations::Column::CleanupPending.eq(true)),
                    ),
            )
            .order_by_asc(registrations::Column::ClaimExpiresAt)
            .limit(limit)
            .into_tuple::<String>()
            .all(&self.db)
            .await
            .map_err(db_error)
    }

    async fn live_registrations(&self, limit: u64) -> Result<Vec<String>> {
        registrations::Entity::find()
            .select_only()
            .column(registrations::Column::Id)
            .filter(
                Condition::any()
                    .add(registrations::Column::Status.ne(STATUS_REVOKED))
                    .add(registrations::Column::CleanupPending.eq(true)),
            )
            .order_by_asc(registrations::Column::Id)
            .limit(limit)
            .into_tuple::<String>()
            .all(&self.db)
            .await
            .map_err(db_error)
    }

    async fn user_registrations(&self, user_id: &str) -> Result<Vec<RegistrationRecord>> {
        Ok(registrations::Entity::find()
            .filter(registrations::Column::UserId.eq(user_id.to_owned()))
            .filter(
                Condition::any()
                    .add(registrations::Column::Status.eq(STATUS_CLAIMED))
                    .add(registrations::Column::Status.eq(STATUS_ACTIVE)),
            )
            .order_by_desc(registrations::Column::CreatedAt)
            .all(&self.db)
            .await
            .map_err(db_error)?
            .into_iter()
            .map(record)
            .collect())
    }

    async fn grant_for(
        &self,
        registration_id: &str,
        resource: &str,
    ) -> Result<Option<AgentGrantRecord>> {
        Ok(
            agent_grants::Entity::find_by_id(grant_key(registration_id, resource))
                .one(&self.db)
                .await
                .map_err(db_error)?
                .map(grant_record),
        )
    }

    async fn grants_of(&self, registration_id: &str) -> Result<Vec<AgentGrantRecord>> {
        Ok(agent_grants::Entity::find()
            .filter(agent_grants::Column::RegistrationId.eq(registration_id.to_owned()))
            .all(&self.db)
            .await
            .map_err(db_error)?
            .into_iter()
            .map(grant_record)
            .collect())
    }

    async fn reserve_grant(
        &self,
        record: AgentGrantRecord,
        expected: Option<&str>,
    ) -> Result<bool> {
        let key = grant_key(&record.registration_id, &record.resource);
        let scopes = record.scopes.join(" ");
        match expected {
            // The primary key lets one first grant per resource in.
            None => match agent_grants::Entity::insert(agent_grants::ActiveModel {
                id: Set(key),
                registration_id: Set(record.registration_id),
                resource: Set(record.resource),
                grant_id: Set(record.grant_id),
                user_id: Set(record.user_id),
                scopes: Set(scopes),
                created_at: Set(Utc::now()),
            })
            .exec(&self.db)
            .await
            {
                Ok(_) => Ok(true),
                Err(error) if is_unique_violation(&error) => Ok(false),
                Err(error) => Err(db_error(error)),
            },
            // A replacement moves the row only from the grant it read.
            Some(expected) => {
                let moved = agent_grants::Entity::update_many()
                    .col_expr(agent_grants::Column::GrantId, Expr::value(record.grant_id))
                    .col_expr(agent_grants::Column::UserId, Expr::value(record.user_id))
                    .col_expr(agent_grants::Column::Scopes, Expr::value(scopes))
                    .filter(agent_grants::Column::Id.eq(key))
                    .filter(agent_grants::Column::GrantId.eq(expected.to_owned()))
                    .exec(&self.db)
                    .await
                    .map_err(db_error)?;
                Ok(moved.rows_affected == 1)
            }
        }
    }

    async fn replay_recorded(&self, key: &str, now: DateTime<Utc>) -> Result<bool> {
        Ok(replays::Entity::find()
            .filter(replays::Column::Id.eq(key.to_owned()))
            .filter(replays::Column::ExpiresAt.gt(now))
            .one(&self.db)
            .await
            .map_err(db_error)?
            .is_some())
    }

    async fn record_replay(&self, key: &str, expires_at: DateTime<Utc>) -> Result<bool> {
        // The primary key is the guard: of two nodes recording one token,
        // the database lets exactly one insert succeed. A record that
        // expired is deleted first, so an old identifier never blocks a
        // token that can no longer be a replay of it.
        replays::Entity::delete_many()
            .filter(replays::Column::Id.eq(key.to_owned()))
            .filter(replays::Column::ExpiresAt.lte(Utc::now()))
            .exec(&self.db)
            .await
            .map_err(db_error)?;
        match replays::Entity::insert(replays::ActiveModel {
            id: Set(key.to_owned()),
            expires_at: Set(expires_at),
        })
        .exec(&self.db)
        .await
        {
            Ok(_) => Ok(true),
            Err(error) if is_unique_violation(&error) => Ok(false),
            Err(error) => Err(db_error(error)),
        }
    }

    async fn prune_replays(&self, now: DateTime<Utc>) -> Result<u64> {
        Ok(replays::Entity::delete_many()
            .filter(replays::Column::ExpiresAt.lte(now))
            .exec(&self.db)
            .await
            .map_err(db_error)?
            .rows_affected)
    }
}
