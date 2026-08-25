//! Default Magnetar engine composition for Suprnova applications.

use std::sync::Arc;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use magnetar::crypto::{CryptoPurpose, Encryptor};
use magnetar::default_first_email_proof::SqlFirstEmailProofStore;
use magnetar::default_schema::sql_stores::{SqlRememberStore, SqlSessionStore};
use magnetar::default_schema::sql_two_factor::SqlTwoFactorStore;
use magnetar::default_schema::{DefaultAuthSchema, lifecycle_deliveries, users};
use magnetar::passkey::PasskeyConfig;
use magnetar::password::hash::{PasswordHashConfig, PasswordVerifier, StandardPasswordHashDriver};
use magnetar::password::lockout::{LockoutConfig, LockoutService};
use magnetar::plugins::password::PasswordAuthService;
use magnetar::sessions::OpaqueConfig;
use magnetar::storage::SeaOrmStorage;
use magnetar::two_factor::{TwoFactorConfig, TwoFactorService};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, Condition, DatabaseConnection, EntityTrait,
    QueryFilter,
};

use super::engine::{
    HostLifecycleDeduplication, HostPasswordLockout, HostUserAdapter, LifecycleDeliveryClaim,
    MagnetarBinding, MagnetarHostEngine, MagnetarHostEngineParts,
};
#[cfg(feature = "magnetar-oauth")]
use super::engine::{HostOAuthError, MagnetarOAuthAuthEngine, MagnetarOAuthHostConfig};
use super::{LockoutStatus, User, UserId};
use crate::crypto::{Crypt, CryptPurpose as FrameworkCryptPurpose};
use crate::error::FrameworkError;

/// Configuration for the framework's default Magnetar engine.
pub struct MagnetarConfig {
    connection: DatabaseConnection,
    apply_migrations: bool,
    passkey: PasskeyConfig,
    sessions: OpaqueConfig,
    lockout: LockoutConfig,
    two_factor: TwoFactorConfig,
    #[cfg(feature = "magnetar-oauth")]
    oauth: Option<MagnetarOAuthHostConfig>,
}

impl MagnetarConfig {
    /// Bind Magnetar to the application's SeaORM connection.
    #[must_use]
    pub fn from_sea_orm(connection: DatabaseConnection) -> Self {
        Self {
            connection,
            apply_migrations: true,
            passkey: PasskeyConfig::default(),
            sessions: OpaqueConfig::default(),
            lockout: LockoutConfig::default(),
            two_factor: TwoFactorConfig::default(),
            #[cfg(feature = "magnetar-oauth")]
            oauth: None,
        }
    }

    /// Control whether default auth tables are created during initialization.
    #[must_use]
    pub fn apply_migrations(mut self, apply: bool) -> Self {
        self.apply_migrations = apply;
        self
    }

    /// Set the passkey relying-party configuration.
    #[must_use]
    pub fn passkey_config(mut self, passkey: PasskeyConfig) -> Self {
        self.passkey = passkey;
        self
    }

    /// Configure the OAuth providers published with the default engine.
    #[cfg(feature = "magnetar-oauth")]
    #[must_use]
    pub fn oauth(mut self, oauth: MagnetarOAuthHostConfig) -> Self {
        self.oauth = Some(oauth);
        self
    }

    /// Set opaque-session policy.
    #[must_use]
    pub fn session_config(mut self, sessions: OpaqueConfig) -> Self {
        self.sessions = sessions;
        self
    }

    /// Set password lockout policy.
    #[must_use]
    pub fn lockout_config(mut self, lockout: LockoutConfig) -> Self {
        self.lockout = lockout;
        self
    }

    /// Set two-factor policy.
    #[must_use]
    pub fn two_factor_config(mut self, two_factor: TwoFactorConfig) -> Self {
        self.two_factor = two_factor;
        self
    }
}

struct FrameworkEncryptor;

impl Encryptor for FrameworkEncryptor {
    fn encrypt(&self, purpose: CryptoPurpose, plaintext: &[u8]) -> magnetar::Result<Vec<u8>> {
        let encoded = URL_SAFE_NO_PAD.encode(plaintext);
        Crypt::encrypt_string(framework_purpose(purpose), &encoded)
            .map(String::into_bytes)
            .map_err(crypto_error)
    }

    fn decrypt(&self, purpose: CryptoPurpose, ciphertext: &[u8]) -> magnetar::Result<Vec<u8>> {
        let wire = std::str::from_utf8(ciphertext).map_err(|_| magnetar::Error::InvalidInput {
            field: "ciphertext".to_owned(),
            message: "encrypted Magnetar value is not UTF-8".to_owned(),
        })?;
        let encoded =
            Crypt::decrypt_string(framework_purpose(purpose), wire).map_err(crypto_error)?;
        URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| magnetar::Error::InvalidInput {
                field: "ciphertext".to_owned(),
                message: "encrypted Magnetar value has invalid payload encoding".to_owned(),
            })
    }
}

fn framework_purpose(purpose: CryptoPurpose) -> FrameworkCryptPurpose {
    match purpose {
        CryptoPurpose::CeremonyState => FrameworkCryptPurpose::MagnetarCeremonyState,
        CryptoPurpose::TwoFactorSecret => FrameworkCryptPurpose::TwoFactorSecret,
        CryptoPurpose::TwoFactorRecovery => FrameworkCryptPurpose::TwoFactorRecovery,
        CryptoPurpose::ProviderToken => FrameworkCryptPurpose::MagnetarProviderToken,
        CryptoPurpose::RefreshToken => FrameworkCryptPurpose::MagnetarRefreshToken,
        CryptoPurpose::SessionGrant => FrameworkCryptPurpose::MagnetarSessionGrant,
    }
}

fn crypto_error(error: FrameworkError) -> magnetar::Error {
    magnetar::Error::Internal {
        message: format!("framework encryption: {error}"),
    }
}

struct DefaultUsers {
    connection: DatabaseConnection,
}

#[async_trait]
impl HostUserAdapter for DefaultUsers {
    type User = User;

    async fn user_for_id(&self, user_id: &str) -> magnetar::Result<User> {
        let id = user_id
            .parse::<i64>()
            .map_err(|_| magnetar::Error::NotFound {
                resource: "application user".to_owned(),
                identifier: user_id.to_owned(),
            })?;
        let row = users::Entity::find_by_id(id)
            .one(&self.connection)
            .await
            .map_err(database_error)?
            .ok_or_else(|| magnetar::Error::NotFound {
                resource: "application user".to_owned(),
                identifier: user_id.to_owned(),
            })?;
        let created_at = row
            .created_at
            .unwrap_or(chrono::DateTime::<chrono::Utc>::UNIX_EPOCH);
        let updated_at = row.updated_at.unwrap_or(created_at);
        User::builder()
            .id(UserId::new(user_id))
            .name(row.name)
            .email(row.email)
            .email_verified_at(row.email_verified_at)
            .locked_at(row.locked_at)
            .created_at(created_at)
            .updated_at(updated_at)
            .build()
            .map_err(|error| magnetar::Error::Internal {
                message: format!("map application user: {error}"),
            })
    }
}

struct DefaultLockout {
    service: Arc<LockoutService>,
}

fn framework_lockout(status: magnetar::password::lockout::LockoutStatus) -> LockoutStatus {
    LockoutStatus {
        email: status.identity,
        failed_attempts: status.failed_attempts,
        is_locked: status.is_locked,
        locked_until: status.locked_until,
    }
}

#[async_trait]
impl HostPasswordLockout for DefaultLockout {
    async fn status(&self, identity: &str) -> magnetar::Result<LockoutStatus> {
        self.service.status(identity).await.map(framework_lockout)
    }

    async fn record_failure(
        &self,
        identity: &str,
        ip_address: Option<&str>,
    ) -> magnetar::Result<LockoutStatus> {
        self.service
            .record_failed_attempt(identity, ip_address)
            .await?;
        self.service.status(identity).await.map(framework_lockout)
    }

    async fn reset_after_success(&self, identity: &str) -> magnetar::Result<()> {
        self.service.reset_attempts(identity).await
    }

    async fn unlock(&self, identity: &str) -> magnetar::Result<bool> {
        self.service.unlock_account(identity).await
    }
}

#[derive(Clone)]
struct SqlLifecycleDedup {
    connection: DatabaseConnection,
}

impl SqlLifecycleDedup {
    fn new(connection: DatabaseConnection) -> Self {
        Self { connection }
    }
}

#[async_trait]
impl HostLifecycleDeduplication for SqlLifecycleDedup {
    async fn claim(
        &self,
        mutation_id: &str,
        lease_id: &str,
        now: chrono::DateTime<chrono::Utc>,
        lease_until: chrono::DateTime<chrono::Utc>,
    ) -> magnetar::Result<LifecycleDeliveryClaim> {
        let inserted = lifecycle_deliveries::ActiveModel {
            mutation_id: Set(mutation_id.to_owned()),
            lease_id: Set(Some(lease_id.to_owned())),
            lease_until: Set(Some(lease_until)),
            delivered_at: Set(None),
        }
        .insert(&self.connection)
        .await;
        match inserted {
            Ok(_) => return Ok(LifecycleDeliveryClaim::Deliver),
            Err(error)
                if matches!(
                    error.sql_err(),
                    Some(sea_orm::SqlErr::UniqueConstraintViolation(_))
                ) => {}
            Err(error) => return Err(database_error(error)),
        }

        let reclaimed = lifecycle_deliveries::Entity::update_many()
            .col_expr(
                lifecycle_deliveries::Column::LeaseId,
                Expr::value(Some(lease_id.to_owned())),
            )
            .col_expr(
                lifecycle_deliveries::Column::LeaseUntil,
                Expr::value(Some(lease_until)),
            )
            .filter(lifecycle_deliveries::Column::MutationId.eq(mutation_id))
            .filter(lifecycle_deliveries::Column::DeliveredAt.is_null())
            .filter(
                Condition::any()
                    .add(lifecycle_deliveries::Column::LeaseUntil.is_null())
                    .add(lifecycle_deliveries::Column::LeaseUntil.lte(now)),
            )
            .exec(&self.connection)
            .await
            .map_err(database_error)?;
        if reclaimed.rows_affected == 1 {
            return Ok(LifecycleDeliveryClaim::Deliver);
        }

        let row = lifecycle_deliveries::Entity::find_by_id(mutation_id)
            .one(&self.connection)
            .await
            .map_err(database_error)?
            .ok_or_else(|| magnetar::Error::Internal {
                message: "lifecycle delivery row disappeared during claim".to_owned(),
            })?;
        Ok(if row.delivered_at.is_some() {
            LifecycleDeliveryClaim::AlreadyDelivered
        } else {
            LifecycleDeliveryClaim::InFlight
        })
    }

    async fn mark_delivered(&self, mutation_id: &str, lease_id: &str) -> magnetar::Result<()> {
        let updated = lifecycle_deliveries::Entity::update_many()
            .col_expr(
                lifecycle_deliveries::Column::DeliveredAt,
                Expr::value(Some(chrono::Utc::now())),
            )
            .col_expr(
                lifecycle_deliveries::Column::LeaseId,
                Expr::value(Option::<String>::None),
            )
            .col_expr(
                lifecycle_deliveries::Column::LeaseUntil,
                Expr::value(Option::<chrono::DateTime<chrono::Utc>>::None),
            )
            .filter(lifecycle_deliveries::Column::MutationId.eq(mutation_id))
            .filter(lifecycle_deliveries::Column::DeliveredAt.is_null())
            .filter(lifecycle_deliveries::Column::LeaseId.eq(lease_id))
            .exec(&self.connection)
            .await
            .map_err(database_error)?;
        if updated.rows_affected == 1 {
            Ok(())
        } else {
            Err(magnetar::Error::Conflict {
                resource: "lifecycle delivery".to_owned(),
                message: "delivery lease is no longer owned".to_owned(),
            })
        }
    }

    async fn release(&self, mutation_id: &str, lease_id: &str) -> magnetar::Result<()> {
        lifecycle_deliveries::Entity::delete_many()
            .filter(lifecycle_deliveries::Column::MutationId.eq(mutation_id))
            .filter(lifecycle_deliveries::Column::DeliveredAt.is_null())
            .filter(lifecycle_deliveries::Column::LeaseId.eq(lease_id))
            .exec(&self.connection)
            .await
            .map_err(database_error)?;
        Ok(())
    }
}

/// Initialize the default Magnetar password, passkey, session, lockout,
/// two-factor, and configured OAuth engines.
///
/// Every configured adapter is built before the reservation publishes any
/// engine slot.
///
/// # Errors
///
/// Returns an error when the application key is unavailable, schema setup
/// fails, any configured engine cannot be built, or an engine was already
/// installed.
pub async fn init_magnetar(config: MagnetarConfig) -> Result<(), FrameworkError> {
    let reservation = super::reserve_magnetar_engines()?;
    if !Crypt::is_initialized() {
        return Err(FrameworkError::internal(
            "Crypt is not initialized - set APP_KEY before initializing Magnetar",
        ));
    }

    let encryptor: Arc<dyn Encryptor> = Arc::new(FrameworkEncryptor);
    let storage = Arc::new(SeaOrmStorage::<DefaultAuthSchema>::new(
        config.connection.clone(),
    ));
    let lockout = Arc::new(LockoutService::new(
        storage.clone(),
        storage.clone(),
        config.lockout,
    ));
    let factors = Arc::new(TwoFactorService::new(
        Arc::new(SqlTwoFactorStore(config.connection.clone())),
        storage.clone(),
        lockout.clone(),
        encryptor.clone(),
        config.two_factor,
    ));
    let verifier = Arc::new(
        PasswordVerifier::new(
            Arc::new(StandardPasswordHashDriver),
            PasswordHashConfig::default(),
        )
        .map_err(map_error)?,
    );
    let password = Arc::new(PasswordAuthService::new(
        storage.clone(),
        storage.clone(),
        verifier.clone(),
    ));
    let first_email_proof = Arc::new(SqlFirstEmailProofStore::new(
        config.connection.clone(),
        encryptor.clone(),
    ));
    let engine = Arc::new(
        MagnetarHostEngine::new(MagnetarHostEngineParts {
            binding: MagnetarBinding::<DefaultAuthSchema>::new(config.connection.clone()),
            session_store: Arc::new(SqlSessionStore(config.connection.clone())),
            remember_store: Arc::new(SqlRememberStore(config.connection.clone())),
            ceremonies: storage,
            factors,
            password,
            first_email_proof,
            password_verifier: verifier,
            password_lockout: Arc::new(DefaultLockout { service: lockout }),
            encryptor,
            session_config: config.sessions,
            users: Arc::new(DefaultUsers {
                connection: config.connection.clone(),
            }),
            lifecycle_deliveries: Arc::new(SqlLifecycleDedup::new(config.connection.clone())),
            lifecycle_lease_duration: chrono::Duration::seconds(30),
        })
        .map_err(map_error)?,
    );
    let passkey = Arc::new(engine.passkey_service(&config.passkey).map_err(map_error)?);
    #[cfg(feature = "magnetar-oauth")]
    let oauth: Option<Arc<dyn MagnetarOAuthAuthEngine>> = config
        .oauth
        .map(|oauth| {
            engine
                .oauth_service(oauth)
                .map(|service| Arc::new(service) as Arc<dyn MagnetarOAuthAuthEngine>)
                .map_err(|error| match error {
                    HostOAuthError::Protocol(error) => {
                        FrameworkError::internal(format!("Magnetar OAuth initialization: {error}"))
                    }
                    HostOAuthError::Auth(error) => map_error(error),
                })
        })
        .transpose()?;
    #[cfg(not(feature = "magnetar-oauth"))]
    let oauth = ();

    if config.apply_migrations {
        magnetar::default_schema::migrate(&config.connection)
            .await
            .map_err(map_error)?;
    }
    reservation.install(engine, passkey, oauth)?;
    Ok(())
}

fn database_error(error: sea_orm::DbErr) -> magnetar::Error {
    magnetar::Error::Internal {
        message: format!("default auth database: {error}"),
    }
}

fn map_error(error: magnetar::Error) -> FrameworkError {
    FrameworkError::internal(format!("Magnetar initialization: {error}"))
}

#[cfg(all(test, feature = "database-sqlite"))]
mod tests {
    use super::*;
    use sea_orm::ConnectionTrait;

    #[tokio::test]
    async fn invalid_components_fail_before_default_schema_changes() {
        let database = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
        if !Crypt::is_initialized() {
            Crypt::init(crate::crypto::EncryptionKey::generate());
        }
        let config = MagnetarConfig::from_sea_orm(database.clone()).passkey_config(PasskeyConfig {
            rp_id: String::new(),
            rp_origin: "not a URL".to_owned(),
        });

        init_magnetar(config).await.unwrap_err();
        let app_users = database
            .query_one_raw(sea_orm::Statement::from_string(
                sea_orm::DbBackend::Sqlite,
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'app_users'",
            ))
            .await
            .unwrap();
        assert!(app_users.is_none());
    }

    #[tokio::test]
    async fn sql_lifecycle_ledger_survives_instances_and_reclaims_expired_leases() {
        let database = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
        magnetar::default_schema::migrate(&database).await.unwrap();
        let now = chrono::Utc::now();
        let first = SqlLifecycleDedup::new(database.clone());
        assert_eq!(
            first
                .claim(
                    "mutation-1",
                    "lease-1",
                    now,
                    now + chrono::Duration::seconds(30),
                )
                .await
                .unwrap(),
            LifecycleDeliveryClaim::Deliver
        );
        let second = SqlLifecycleDedup::new(database.clone());
        assert_eq!(
            second
                .claim(
                    "mutation-1",
                    "lease-2",
                    now,
                    now + chrono::Duration::seconds(30),
                )
                .await
                .unwrap(),
            LifecycleDeliveryClaim::InFlight
        );
        first.mark_delivered("mutation-1", "lease-1").await.unwrap();
        assert_eq!(
            SqlLifecycleDedup::new(database.clone())
                .claim(
                    "mutation-1",
                    "lease-3",
                    now,
                    now + chrono::Duration::seconds(30),
                )
                .await
                .unwrap(),
            LifecycleDeliveryClaim::AlreadyDelivered
        );

        assert_eq!(
            first
                .claim(
                    "mutation-2",
                    "lease-old",
                    now,
                    now + chrono::Duration::seconds(1),
                )
                .await
                .unwrap(),
            LifecycleDeliveryClaim::Deliver
        );
        assert_eq!(
            second
                .claim(
                    "mutation-2",
                    "lease-new",
                    now + chrono::Duration::seconds(2),
                    now + chrono::Duration::seconds(32),
                )
                .await
                .unwrap(),
            LifecycleDeliveryClaim::Deliver
        );
    }
}
