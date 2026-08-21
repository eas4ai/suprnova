//! Transactional migration bindings for Magnetar's default SeaORM schema.

use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use sea_orm::{
    AccessMode, ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait,
    DatabaseConnection, DatabaseTransaction, DbBackend, EntityTrait, IntoActiveModel,
    IsolationLevel, QueryFilter, Statement, TransactionTrait,
};
use secrecy::ExposeSecret;
use sha2::{Digest, Sha256};

use crate::default_schema::{
    accounts, lockouts, methods, migration_identities, migration_runs, migration_state, tokens,
    two_factor, users,
};
use crate::migration::{
    AppUser, DurableAuthRecord, ImportedUser, MigrationBindings, MigrationTransaction,
};
use crate::password::normalize_email;
use crate::{Error, Result};

/// Application bindings backed by Magnetar's default application-owned schema.
#[derive(Clone)]
pub struct DefaultMigrationBindings {
    database: DatabaseConnection,
    share_source_transaction: bool,
}

impl DefaultMigrationBindings {
    /// Creates default-schema migration bindings over one application database.
    #[must_use]
    pub fn new(database: DatabaseConnection) -> Self {
        Self {
            database,
            share_source_transaction: false,
        }
    }

    /// Use the coordinator's source transaction for an in-place migration.
    ///
    /// Enable this only when the source and application bindings address the
    /// same physical database.
    #[must_use]
    pub fn sharing_source_database(mut self) -> Self {
        self.share_source_transaction = true;
        self
    }
}

#[async_trait]
impl MigrationBindings for DefaultMigrationBindings {
    async fn app_users(&self) -> Result<Vec<AppUser>> {
        Ok(users::Entity::find()
            .all(&self.database)
            .await
            .map_err(database_error)?
            .into_iter()
            .map(app_user)
            .collect())
    }

    async fn shares_source_database(&self, _source: &DatabaseConnection) -> Result<bool> {
        Ok(self.share_source_transaction)
    }

    async fn app_users_in_source(
        &self,
        source_transaction: &DatabaseTransaction,
    ) -> Result<Vec<AppUser>> {
        if !self.share_source_transaction {
            return self.app_users().await;
        }
        Ok(users::Entity::find()
            .all(source_transaction)
            .await
            .map_err(database_error)?
            .into_iter()
            .map(app_user)
            .collect())
    }
    async fn begin_transaction<'a>(
        &'a self,
        source_transaction: Option<&'a DatabaseTransaction>,
    ) -> Result<Box<dyn MigrationTransaction + 'a>> {
        let connection = match source_transaction {
            Some(transaction) if self.share_source_transaction => {
                DefaultMigrationConnection::Shared(transaction)
            }
            Some(_) => {
                return Err(Error::InvalidInput {
                    field: "migration database".to_owned(),
                    message: "source transaction supplied to a separate-database binding"
                        .to_owned(),
                });
            }
            None if self.share_source_transaction => {
                return Err(Error::InvalidInput {
                    field: "migration database".to_owned(),
                    message: "same-database binding requires the coordinator transaction"
                        .to_owned(),
                });
            }
            None => DefaultMigrationConnection::Owned(Some(
                self.database
                    .begin_with_config(
                        Some(IsolationLevel::Serializable),
                        Some(AccessMode::ReadWrite),
                    )
                    .await
                    .map_err(database_error)?,
            )),
        };
        Ok(Box::new(DefaultMigrationTransaction { connection }))
    }

    async fn mark_migration_completed(&self, plan_id: &str) -> Result<()> {
        let transaction = self
            .database
            .begin_with_config(
                Some(IsolationLevel::Serializable),
                Some(AccessMode::ReadWrite),
            )
            .await
            .map_err(database_error)?;
        let run = migration_runs::Entity::find_by_id(plan_id)
            .one(&transaction)
            .await
            .map_err(database_error)?
            .ok_or_else(|| Error::Conflict {
                resource: "migration run ledger".to_owned(),
                message: "completed migration has no committed import ledger".to_owned(),
            })?;
        if !run.imports_committed {
            return Err(Error::Conflict {
                resource: "migration run ledger".to_owned(),
                message: "migration imports are not committed".to_owned(),
            });
        }
        if run.completed_at.is_none() {
            let mut active = run.into_active_model();
            active.completed_at = Set(Some(Utc::now()));
            active.update(&transaction).await.map_err(database_error)?;
        }
        repair_app_user_sequence(&transaction).await?;
        match migration_state::Entity::find_by_id("schema_version")
            .one(&transaction)
            .await
            .map_err(database_error)?
        {
            Some(marker) if marker.value == "1" => {}
            Some(_) => {
                return Err(Error::Conflict {
                    resource: "Magnetar migration marker".to_owned(),
                    message: "unsupported existing marker version".to_owned(),
                });
            }
            None => {
                migration_state::ActiveModel {
                    key: Set("schema_version".to_owned()),
                    value: Set("1".to_owned()),
                }
                .insert(&transaction)
                .await
                .map_err(database_error)?;
            }
        }
        migration_state::Entity::delete_by_id("source_pending")
            .exec(&transaction)
            .await
            .map_err(database_error)?;
        transaction.commit().await.map_err(database_error)
    }

    fn migration_target_tables(&self) -> Vec<String> {
        vec![
            "app_users".to_owned(),
            "auth_sessions".to_owned(),
            "auth_linked_accounts".to_owned(),
            "auth_methods".to_owned(),
            "auth_tokens".to_owned(),
            "auth_ceremonies".to_owned(),
            "auth_lockouts".to_owned(),
            "auth_remember_tokens".to_owned(),
            "auth_two_factor".to_owned(),
            "auth_lifecycle_deliveries".to_owned(),
            "auth_migration_runs".to_owned(),
            "auth_migration_identities".to_owned(),
            "auth_provider_tokens".to_owned(),
            "magnetar_migration_state".to_owned(),
        ]
    }
}

enum DefaultMigrationConnection<'a> {
    Owned(Option<DatabaseTransaction>),
    Shared(&'a DatabaseTransaction),
}

struct DefaultMigrationTransaction<'a> {
    connection: DefaultMigrationConnection<'a>,
}

impl DefaultMigrationTransaction<'_> {
    fn connection(&self) -> Result<&DatabaseTransaction> {
        match &self.connection {
            DefaultMigrationConnection::Owned(Some(transaction)) => Ok(transaction),
            DefaultMigrationConnection::Shared(transaction) => Ok(transaction),
            DefaultMigrationConnection::Owned(None) => Err(Error::Internal {
                message: "default migration transaction is already closed".to_owned(),
            }),
        }
    }

    fn take_owned_connection(&mut self) -> Result<Option<DatabaseTransaction>> {
        match &mut self.connection {
            DefaultMigrationConnection::Owned(transaction) => {
                transaction.take().map(Some).ok_or_else(|| Error::Internal {
                    message: "default migration transaction is already closed".to_owned(),
                })
            }
            DefaultMigrationConnection::Shared(_) => Ok(None),
        }
    }

    async fn find_user(&self, imported: &ImportedUser) -> Result<Option<users::Model>> {
        let transaction = self.connection()?;
        if let Some(id) = imported.preferred_app_user_id
            && let Some(user) = users::Entity::find_by_id(id)
                .one(transaction)
                .await
                .map_err(database_error)?
        {
            if normalize_email(&user.email) != normalize_email(&imported.email) {
                return Err(Error::Conflict {
                    resource: "application user identity".to_owned(),
                    message: format!("application id {id} belongs to a different normalized email"),
                });
            }
            return Ok(Some(user));
        }

        let normalized = normalize_email(&imported.email);
        let matches = users::Entity::find()
            .all(transaction)
            .await
            .map_err(database_error)?
            .into_iter()
            .filter(|user| normalize_email(&user.email) == normalized)
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [] => Ok(None),
            [user] => {
                if imported
                    .preferred_app_user_id
                    .is_some_and(|id| id != user.id)
                {
                    return Err(Error::Conflict {
                        resource: "application user identity".to_owned(),
                        message: format!(
                            "source requested application id {} but normalized email belongs to {}",
                            imported.preferred_app_user_id.unwrap_or_default(),
                            user.id
                        ),
                    });
                }
                Ok(Some(user.clone()))
            }
            _ => Err(Error::Conflict {
                resource: "normalized application email".to_owned(),
                message: format!("multiple application users own {normalized}"),
            }),
        }
    }
}

#[async_trait]
impl MigrationTransaction for DefaultMigrationTransaction<'_> {
    async fn app_users(&mut self) -> Result<Vec<AppUser>> {
        Ok(users::Entity::find()
            .all(self.connection()?)
            .await
            .map_err(database_error)?
            .into_iter()
            .map(app_user)
            .collect())
    }

    async fn import_user(&mut self, imported: &ImportedUser) -> Result<AppUser> {
        let existing = self.find_user(imported).await?;
        let transaction = self.connection()?;
        let model = if let Some(existing) = existing {
            if let (Some(current), Some(source)) =
                (&existing.password_hash, &imported.password_hash)
                && current != source
            {
                return Err(Error::Conflict {
                    resource: "application password credential".to_owned(),
                    message: "existing application password differs from the legacy source hash"
                        .to_owned(),
                });
            }
            let fill_name = existing.name.is_none() && imported.name.is_some();
            let fill_password =
                existing.password_hash.is_none() && imported.password_hash.is_some();
            let fill_verification =
                existing.email_verified_at.is_none() && imported.email_verified_at.is_some();
            let fill_lockout = existing.locked_at.is_none() && imported.locked_at.is_some();
            let fill_created = existing.created_at.is_none() && imported.created_at.is_some();
            let existing_epoch = existing.auth_epoch;
            let mut active = existing.into_active_model();
            if fill_name {
                active.name = Set(imported.name.clone());
            }
            if fill_password {
                active.password_hash = Set(imported.password_hash.clone());
            }
            if fill_verification {
                active.email_verified_at = Set(parse_optional(
                    imported.email_verified_at.as_deref(),
                    "email_verified_at",
                )?);
            }
            if fill_lockout {
                active.locked_at = Set(parse_optional(imported.locked_at.as_deref(), "locked_at")?);
            }
            let imported_epoch = imported
                .auth_epoch
                .into_iter()
                .chain(imported.session_version)
                .max()
                .unwrap_or(0);
            active.auth_epoch = Set(existing_epoch.max(imported_epoch));
            if fill_created {
                active.created_at = Set(parse_optional(
                    imported.created_at.as_deref(),
                    "created_at",
                )?);
            }
            active.update(transaction).await.map_err(database_error)?
        } else {
            let mut active = users::ActiveModel {
                email: Set(imported.email.clone()),
                name: Set(imported.name.clone()),
                password_hash: Set(imported.password_hash.clone()),
                remember_token: Set(None),
                email_verified_at: Set(parse_optional(
                    imported.email_verified_at.as_deref(),
                    "email_verified_at",
                )?),
                locked_at: Set(parse_optional(imported.locked_at.as_deref(), "locked_at")?),
                auth_epoch: Set(imported
                    .auth_epoch
                    .into_iter()
                    .chain(imported.session_version)
                    .max()
                    .unwrap_or(0)),
                created_at: Set(parse_optional(
                    imported.created_at.as_deref(),
                    "created_at",
                )?),
                updated_at: Set(parse_optional(
                    imported.updated_at.as_deref(),
                    "updated_at",
                )?),
                ..Default::default()
            };
            if let Some(id) = imported.preferred_app_user_id {
                active.id = Set(id);
            }
            active.insert(transaction).await.map_err(database_error)?
        };
        Ok(app_user(model))
    }

    async fn bind_external_identity(
        &mut self,
        provider: &str,
        external_user_id: &str,
        app_user_id: i64,
    ) -> Result<()> {
        let transaction = self.connection()?;
        if let Some(existing) = accounts::Entity::find()
            .filter(accounts::Column::Provider.eq(provider))
            .filter(accounts::Column::ProviderAccountId.eq(external_user_id))
            .one(transaction)
            .await
            .map_err(database_error)?
        {
            if existing.user_id != app_user_id {
                return Err(Error::Conflict {
                    resource: "external identity".to_owned(),
                    message: "provider identity is already bound to another user".to_owned(),
                });
            }
            return Ok(());
        }
        accounts::ActiveModel {
            user_id: Set(app_user_id),
            provider: Set(provider.to_owned()),
            provider_account_id: Set(external_user_id.to_owned()),
            ..Default::default()
        }
        .insert(transaction)
        .await
        .map_err(database_error)?;
        Ok(())
    }

    async fn import_passkey(
        &mut self,
        app_user_id: i64,
        credential_id: &str,
        data_json: &str,
    ) -> Result<()> {
        let transaction = self.connection()?;
        if let Some(existing) = methods::Entity::find()
            .filter(methods::Column::CredentialId.eq(credential_id))
            .one(transaction)
            .await
            .map_err(database_error)?
        {
            if existing.user_id != app_user_id || existing.public_key.as_deref() != Some(data_json)
            {
                return Err(Error::Conflict {
                    resource: "passkey credential".to_owned(),
                    message: "credential id already exists with different owner or envelope"
                        .to_owned(),
                });
            }
            return Ok(());
        }
        methods::ActiveModel {
            user_id: Set(app_user_id),
            credential_id: Set(Some(credential_id.to_owned())),
            public_key: Set(Some(data_json.to_owned())),
            ..Default::default()
        }
        .insert(transaction)
        .await
        .map_err(database_error)?;
        Ok(())
    }

    async fn import_durable_record(&mut self, record: DurableAuthRecord) -> Result<()> {
        match record {
            DurableAuthRecord::LinkedAccount(account) => {
                let transaction = self.connection()?;
                let created_at =
                    parse_optional(account.created_at.as_deref(), "linked_account.created_at")?;
                let updated_at =
                    parse_optional(account.updated_at.as_deref(), "linked_account.updated_at")?;
                if let Some(existing) = accounts::Entity::find()
                    .filter(accounts::Column::Provider.eq(&account.provider))
                    .filter(accounts::Column::ProviderAccountId.eq(&account.subject))
                    .one(transaction)
                    .await
                    .map_err(database_error)?
                {
                    if existing.user_id != account.app_user_id
                        || existing.created_at != created_at
                        || existing.updated_at != updated_at
                    {
                        return Err(Error::Conflict {
                            resource: "linked account".to_owned(),
                            message:
                                "provider identity already exists with different durable state"
                                    .to_owned(),
                        });
                    }
                    return Ok(());
                }
                accounts::ActiveModel {
                    user_id: Set(account.app_user_id),
                    provider: Set(account.provider),
                    provider_account_id: Set(account.subject),
                    created_at: Set(created_at),
                    updated_at: Set(updated_at),
                    ..Default::default()
                }
                .insert(transaction)
                .await
                .map_err(database_error)?;
                Ok(())
            }
            DurableAuthRecord::SecureToken(token) => {
                let transaction = self.connection()?;
                let digest = token_digest(token.token.expose_secret());
                let expires_at = parse_required(&token.expires_at, "expires_at")?;
                let used_at = parse_optional(token.used_at.as_deref(), "used_at")?;
                let created_at = Some(parse_required(&token.created_at, "created_at")?);
                let updated_at = Some(parse_required(&token.updated_at, "updated_at")?);
                if let Some(existing) = tokens::Entity::find()
                    .filter(tokens::Column::Digest.eq(&digest))
                    .filter(tokens::Column::Purpose.eq(&token.purpose))
                    .one(transaction)
                    .await
                    .map_err(database_error)?
                {
                    if existing.user_id != Some(token.app_user_id)
                        || existing.expires_at != expires_at
                        || existing.used_at != used_at
                        || existing.created_at != created_at
                        || existing.updated_at != updated_at
                    {
                        return Err(Error::Conflict {
                            resource: "secure token".to_owned(),
                            message: "token digest already exists with different durable state"
                                .to_owned(),
                        });
                    }
                    return Ok(());
                }
                tokens::ActiveModel {
                    user_id: Set(Some(token.app_user_id)),
                    purpose: Set(token.purpose),
                    digest: Set(digest),
                    expires_at: Set(expires_at),
                    used_at: Set(used_at),
                    created_at: Set(created_at),
                    updated_at: Set(updated_at),
                    ..Default::default()
                }
                .insert(transaction)
                .await
                .map_err(database_error)?;
                Ok(())
            }
            DurableAuthRecord::FailedLoginAttempt(attempt) => {
                let transaction = self.connection()?;
                let attempted_at = parse_required(&attempt.attempted_at, "attempted_at")?;
                let migration_source_id =
                    format!("failed_login_attempts:{}", attempt.source_record_id);
                if let Some(existing) = lockouts::Entity::find()
                    .filter(lockouts::Column::MigrationSourceId.eq(&migration_source_id))
                    .one(transaction)
                    .await
                    .map_err(database_error)?
                {
                    if existing.identity != attempt.email
                        || existing.ip_address != attempt.ip_address
                        || existing.attempted_at != attempted_at
                    {
                        return Err(Error::Conflict {
                            resource: "failed-login migration record".to_owned(),
                            message: "source row already exists with different durable state"
                                .to_owned(),
                        });
                    }
                    return Ok(());
                }
                lockouts::ActiveModel {
                    identity: Set(attempt.email),
                    attempted_at: Set(attempted_at),
                    ip_address: Set(attempt.ip_address),
                    migration_source_id: Set(Some(migration_source_id)),
                    locked_at: Set(None),
                    reason: Set(Some("legacy failed-login attempt".to_owned())),
                    ..Default::default()
                }
                .insert(transaction)
                .await
                .map_err(database_error)?;
                Ok(())
            }
            DurableAuthRecord::TwoFactorCredential(two_factor_record) => {
                let transaction = self.connection()?;
                let user_id = two_factor_record.app_user_id.to_string();
                let secret = two_factor_record.secret.expose_secret().as_bytes().to_vec();
                let recovery_codes = two_factor_record
                    .recovery_codes
                    .as_ref()
                    .map(|value| value.expose_secret().as_bytes().to_vec());
                let confirmed_at =
                    parse_optional(two_factor_record.confirmed_at.as_deref(), "confirmed_at")?;
                let created_at = Some(parse_required(&two_factor_record.created_at, "created_at")?);
                let updated_at = Some(parse_required(&two_factor_record.updated_at, "updated_at")?);
                if let Some(existing) = two_factor::Entity::find_by_id(&user_id)
                    .one(transaction)
                    .await
                    .map_err(database_error)?
                {
                    if existing.secret != secret
                        || existing.recovery_codes != recovery_codes
                        || existing.confirmed_at != confirmed_at
                        || existing.last_used_timestep != two_factor_record.last_used_timestep
                        || existing.created_at != created_at
                        || existing.updated_at != updated_at
                    {
                        return Err(Error::Conflict {
                            resource: "two-factor enrollment".to_owned(),
                            message: "application user already has a different enrollment"
                                .to_owned(),
                        });
                    }
                    return Ok(());
                }
                two_factor::ActiveModel {
                    user_id: Set(user_id),
                    secret: Set(secret),
                    recovery_codes: Set(recovery_codes),
                    confirmed_at: Set(confirmed_at),
                    last_used_timestep: Set(two_factor_record.last_used_timestep),
                    created_at: Set(created_at),
                    updated_at: Set(updated_at),
                }
                .insert(transaction)
                .await
                .map_err(database_error)?;
                Ok(())
            }
        }
    }

    async fn imports_committed(&mut self, plan_id: &str) -> Result<bool> {
        Ok(migration_runs::Entity::find_by_id(plan_id)
            .one(self.connection()?)
            .await
            .map_err(database_error)?
            .is_some_and(|run| run.imports_committed))
    }

    async fn resolved_app_user_id(
        &mut self,
        plan_id: &str,
        source_user_id: &str,
    ) -> Result<Option<i64>> {
        let id = migration_identity_id(plan_id, source_user_id);
        let row = migration_identities::Entity::find_by_id(id)
            .one(self.connection()?)
            .await
            .map_err(database_error)?;
        match row {
            Some(row) if row.plan_id == plan_id && row.source_user_id == source_user_id => {
                Ok(Some(row.app_user_id))
            }
            Some(_) => Err(Error::Conflict {
                resource: "migration identity ledger".to_owned(),
                message: "migration identity digest collision".to_owned(),
            }),
            None => Ok(None),
        }
    }

    async fn record_identity_resolution(
        &mut self,
        plan_id: &str,
        source_user_id: &str,
        app_user_id: i64,
    ) -> Result<()> {
        let id = migration_identity_id(plan_id, source_user_id);
        if let Some(existing) = migration_identities::Entity::find_by_id(&id)
            .one(self.connection()?)
            .await
            .map_err(database_error)?
        {
            if existing.plan_id != plan_id
                || existing.source_user_id != source_user_id
                || existing.app_user_id != app_user_id
            {
                return Err(Error::Conflict {
                    resource: "migration identity ledger".to_owned(),
                    message: "source identity already resolved differently".to_owned(),
                });
            }
            return Ok(());
        }
        migration_identities::ActiveModel {
            id: Set(id),
            plan_id: Set(plan_id.to_owned()),
            source_user_id: Set(source_user_id.to_owned()),
            app_user_id: Set(app_user_id),
        }
        .insert(self.connection()?)
        .await
        .map_err(database_error)?;
        Ok(())
    }

    async fn mark_imports_committed(&mut self, plan_id: &str) -> Result<()> {
        if let Some(existing) = migration_runs::Entity::find_by_id(plan_id)
            .one(self.connection()?)
            .await
            .map_err(database_error)?
        {
            if !existing.imports_committed {
                let mut active = existing.into_active_model();
                active.imports_committed = Set(true);
                active
                    .update(self.connection()?)
                    .await
                    .map_err(database_error)?;
            }
            return Ok(());
        }
        migration_runs::ActiveModel {
            plan_id: Set(plan_id.to_owned()),
            imports_committed: Set(true),
            completed_at: Set(None),
        }
        .insert(self.connection()?)
        .await
        .map_err(database_error)?;
        Ok(())
    }

    async fn commit(mut self: Box<Self>) -> Result<()> {
        match self.take_owned_connection()? {
            Some(transaction) => transaction.commit().await.map_err(database_error),
            None => Ok(()),
        }
    }

    async fn rollback(mut self: Box<Self>) -> Result<()> {
        match self.take_owned_connection()? {
            Some(transaction) => transaction.rollback().await.map_err(database_error),
            None => Ok(()),
        }
    }
}

async fn repair_app_user_sequence(transaction: &DatabaseTransaction) -> Result<()> {
    if transaction.get_database_backend() != DbBackend::Postgres {
        return Ok(());
    }
    transaction
        .execute(Statement::from_string(
            DbBackend::Postgres,
            "SELECT setval(
                pg_get_serial_sequence('app_users', 'id'),
                COALESCE(MAX(id), 1),
                MAX(id) IS NOT NULL
             )
             FROM app_users"
                .to_owned(),
        ))
        .await
        .map_err(database_error)?;
    Ok(())
}
fn app_user(user: users::Model) -> AppUser {
    AppUser {
        id: user.id,
        email: user.email,
        auth_epoch: user.auth_epoch,
        session_version: user.auth_epoch,
    }
}

fn migration_identity_id(plan_id: &str, source_user_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update((plan_id.len() as u64).to_be_bytes());
    hasher.update(plan_id.as_bytes());
    hasher.update((source_user_id.len() as u64).to_be_bytes());
    hasher.update(source_user_id.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn token_digest(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn parse_optional(value: Option<&str>, field: &str) -> Result<Option<DateTime<Utc>>> {
    value.map(|value| parse_required(value, field)).transpose()
}

fn parse_required(value: &str, field: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .or_else(|_| {
            NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f")
                .map(|value| value.and_utc())
        })
        .or_else(|_| {
            NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S").map(|value| value.and_utc())
        })
        .map_err(|_| Error::InvalidInput {
            field: field.to_owned(),
            message: format!("unsupported legacy timestamp {value:?}"),
        })
}

fn database_error(error: sea_orm::DbErr) -> Error {
    Error::Internal {
        message: format!("migration application database: {error}"),
    }
}
