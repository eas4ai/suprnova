#![cfg(feature = "testing")]

//! Framework-hosted Magnetar authentication integration.

use std::{any::TypeId, sync::Arc};

use async_trait::async_trait;
use magnetar::{
    Error, Result as MagnetarResult,
    auth::FactorVerifier,
    crypto::AeadEncryptor,
    passkey::PasskeyConfig,
    password::{PasswordHashConfig, PasswordVerifier, StandardPasswordHashDriver},
    plugin::{LifecycleEvent, LifecycleEventKind},
    plugins::password::{PasswordAttempt, PasswordAuthProvider, PasswordAuthService},
    schema::{
        AuthSchema, CeremonyFields, EntityBinding, LinkedAccountFields, PasskeyFields,
        SessionEpoch, TokenFields, UserBinding, UserFields, UserOptionalFields,
    },
    sessions::{
        OpaqueConfig, OpaqueSessionStore, SessionMetadata, SessionQueries, StoredSession,
        WebSessionBinding,
    },
    storage::{CeremonyStore, PasskeyStore, SeaOrmStorage, UserStore},
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, Database, DatabaseBackend,
    EntityTrait, QueryFilter, Statement, sea_query::Expr,
};
#[cfg(feature = "magnetar-oauth")]
use secrecy::ExposeSecret;
use serial_test::serial;
use suprnova::rate_limit::{RateLimiterDriver, SlidingWindowConfig};
use suprnova::testing::TestContainer;
use suprnova::{
    EventFacade, User, UserId,
    events::testing::{assert_dispatched_once, dispatched},
    magnetar_integration::engine::{
        HostLifecycleDeduplication, HostPasswordLockout, HostUserAdapter, LifecycleDeliveryClaim,
        LifecycleForwardResult, MagnetarAuthStore, MagnetarBinding, MagnetarHostEngine,
        MagnetarHostEngineParts, MagnetarLifecycleEvent, MagnetarPasskeyAuthEngine,
        MagnetarPasswordAuthEngine,
    },
    model,
};

#[model(table = "framework_magnetar_engine_users", timestamps = false)]
pub struct FrameworkMagnetarEngineUser {
    pub id: i64,
    pub login_email: String,
    pub password_hash: String,
    pub display_name: Option<String>,
    pub email_verified_at: Option<String>,
    pub locked_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub session_version: i64,
}

#[model(table = "framework_magnetar_engine_sessions", timestamps = false)]
pub struct FrameworkMagnetarEngineSession {
    pub id: i64,
    pub session_id: String,
    pub user_id: String,
    pub token_hash: String,
    pub token_digest: String,
    pub expires_at: String,
    pub revoked_at: Option<String>,
    pub user_agent: Option<String>,
    pub ip_address: Option<String>,
}

#[model(
    table = "framework_magnetar_engine_linked_accounts",
    timestamps = false
)]
pub struct FrameworkMagnetarEngineLinkedAccount {
    pub id: i64,
    pub account_id: String,
    pub user_id: String,
    pub provider: String,
    pub provider_account_id: String,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub expires_at: Option<String>,
}

#[model(table = "framework_magnetar_engine_passkeys", timestamps = false)]
pub struct FrameworkMagnetarEnginePasskey {
    pub id: i64,
    pub passkey_id: String,
    pub user_id: String,
    pub credential_id: String,
    pub public_key: String,
    pub sign_count: i64,
    pub transports: Option<String>,
    pub created_at: String,
}

#[model(table = "framework_magnetar_engine_tokens", timestamps = false)]
pub struct FrameworkMagnetarEngineToken {
    pub id: i64,
    pub token_id: String,
    pub user_id: Option<String>,
    pub purpose: String,
    pub digest: String,
    pub expires_at: String,
    pub used_at: Option<String>,
}

#[model(table = "framework_magnetar_engine_ceremonies", timestamps = false)]
pub struct FrameworkMagnetarEngineCeremony {
    pub id: i64,
    pub ceremony_id: String,
    pub kind: String,
    pub selector: String,
    pub payload: Vec<u8>,
    pub state: String,
    pub expires_at: String,
    pub used_at: Option<String>,
}

#[model(table = "framework_magnetar_engine_lockouts", timestamps = false)]
pub struct FrameworkMagnetarEngineLockout {
    pub id: i64,
    pub email: String,
}

#[model(table = "framework_magnetar_engine_token_records", timestamps = false)]
pub struct FrameworkMagnetarEngineTokenRecord {
    pub id: i64,
    pub user_id: String,
}

macro_rules! bind_entity {
    ($module:ident) => {
        impl EntityBinding for $module::Entity {
            type Entity = $module::Entity;
            type Column = $module::Column;
            type PrimaryKey = $module::PrimaryKey;
            type Model = $module::Model;
            type ActiveModel = $module::ActiveModel;
        }
    };
}

bind_entity!(framework_magnetar_engine_user);
bind_entity!(framework_magnetar_engine_session);
bind_entity!(framework_magnetar_engine_linked_account);
bind_entity!(framework_magnetar_engine_passkey);
bind_entity!(framework_magnetar_engine_token);
bind_entity!(framework_magnetar_engine_ceremony);
bind_entity!(framework_magnetar_engine_lockout);
bind_entity!(framework_magnetar_engine_token_record);

impl UserFields for framework_magnetar_engine_user::Entity {
    fn read_user_id(model: &Self::Model) -> String {
        model.id.to_string()
    }
    fn user_id_column() -> Self::Column {
        framework_magnetar_engine_user::Column::Id
    }
    fn write_user_id(model: &mut Self::ActiveModel, value: &str) {
        model.id = Set(value.parse().expect("test storage emits i64 identifiers"));
    }
    fn read_email(model: &Self::Model) -> String {
        model.login_email.clone()
    }
    fn email_column() -> Self::Column {
        framework_magnetar_engine_user::Column::LoginEmail
    }
    fn write_email(model: &mut Self::ActiveModel, value: &str) {
        model.login_email = Set(value.to_owned());
    }
    fn read_password_hash(model: &Self::Model) -> Option<String> {
        (!model.password_hash.is_empty()).then(|| model.password_hash.clone())
    }
    fn password_hash_column() -> Self::Column {
        framework_magnetar_engine_user::Column::PasswordHash
    }
    fn write_password_hash(model: &mut Self::ActiveModel, value: Option<&str>) {
        model.password_hash = Set(value.unwrap_or_default().to_owned());
    }
    fn read_locked_at(
        model: &Self::Model,
    ) -> Option<suprnova::chrono::DateTime<suprnova::chrono::Utc>> {
        parse_optional_timestamp(model.locked_at.as_deref())
    }
    fn write_locked_at(
        model: &mut Self::ActiveModel,
        value: Option<suprnova::chrono::DateTime<suprnova::chrono::Utc>>,
    ) {
        model.locked_at = Set(value.map(|value| value.to_rfc3339()));
    }
}

impl UserOptionalFields for framework_magnetar_engine_user::Entity {
    fn read_name(_: &Self::Model) -> Option<String> {
        None
    }
    fn read_email_verified_at(
        model: &Self::Model,
    ) -> Option<suprnova::chrono::DateTime<suprnova::chrono::Utc>> {
        parse_optional_timestamp(model.email_verified_at.as_deref())
    }
    fn write_email_verified_at(
        model: &mut Self::ActiveModel,
        value: Option<suprnova::chrono::DateTime<suprnova::chrono::Utc>>,
    ) {
        model.email_verified_at = Set(value.map(|value| value.to_rfc3339()));
    }
    fn read_remember_token(_: &Self::Model) -> Option<String> {
        None
    }
    fn write_remember_token(_: &mut Self::ActiveModel, _: Option<&str>) {}
}

impl SessionEpoch for framework_magnetar_engine_user::Entity {
    fn auth_epoch(model: &Self::Model) -> u64 {
        model.session_version as u64
    }
    fn auth_epoch_column() -> Self::Column {
        framework_magnetar_engine_user::Column::SessionVersion
    }
    fn write_auth_epoch(model: &mut Self::ActiveModel, value: u64) {
        model.session_version = Set(value as i64);
    }
}

impl LinkedAccountFields for framework_magnetar_engine_linked_account::Entity {
    fn read_account_id(model: &Self::Model) -> String {
        model.account_id.clone()
    }
    fn account_id_column() -> Self::Column {
        framework_magnetar_engine_linked_account::Column::AccountId
    }
    fn write_account_id(model: &mut Self::ActiveModel, value: &str) {
        model.account_id = Set(value.to_owned());
    }
    fn read_user_id(model: &Self::Model) -> String {
        model.user_id.clone()
    }
    fn user_id_column() -> Self::Column {
        framework_magnetar_engine_linked_account::Column::UserId
    }
    fn write_user_id(model: &mut Self::ActiveModel, value: &str) {
        model.user_id = Set(value.to_owned());
    }
    fn read_provider(model: &Self::Model) -> String {
        model.provider.clone()
    }
    fn provider_column() -> Self::Column {
        framework_magnetar_engine_linked_account::Column::Provider
    }
    fn write_provider(model: &mut Self::ActiveModel, value: &str) {
        model.provider = Set(value.to_owned());
    }
    fn read_provider_account_id(model: &Self::Model) -> String {
        model.provider_account_id.clone()
    }
    fn provider_account_id_column() -> Self::Column {
        framework_magnetar_engine_linked_account::Column::ProviderAccountId
    }
    fn write_provider_account_id(model: &mut Self::ActiveModel, value: &str) {
        model.provider_account_id = Set(value.to_owned());
    }
    fn read_access_token(model: &Self::Model) -> Option<String> {
        model.access_token.clone()
    }
    fn read_refresh_token(model: &Self::Model) -> Option<String> {
        model.refresh_token.clone()
    }
    fn read_expires_at(
        model: &Self::Model,
    ) -> Option<suprnova::chrono::DateTime<suprnova::chrono::Utc>> {
        parse_optional_timestamp(model.expires_at.as_deref())
    }
}

impl PasskeyFields for framework_magnetar_engine_passkey::Entity {
    fn read_passkey_id(model: &Self::Model) -> String {
        model.passkey_id.clone()
    }
    fn passkey_id_column() -> Self::Column {
        framework_magnetar_engine_passkey::Column::PasskeyId
    }
    fn write_passkey_id(model: &mut Self::ActiveModel, value: &str) {
        model.passkey_id = Set(value.to_owned());
    }
    fn read_user_id(model: &Self::Model) -> String {
        model.user_id.clone()
    }
    fn user_id_column() -> Self::Column {
        framework_magnetar_engine_passkey::Column::UserId
    }
    fn write_user_id(model: &mut Self::ActiveModel, value: &str) {
        model.user_id = Set(value.to_owned());
    }
    fn read_credential_id(model: &Self::Model) -> String {
        model.credential_id.clone()
    }
    fn credential_id_column() -> Self::Column {
        framework_magnetar_engine_passkey::Column::CredentialId
    }
    fn write_credential_id(model: &mut Self::ActiveModel, value: &str) {
        model.credential_id = Set(value.to_owned());
    }
    fn read_public_key(model: &Self::Model) -> String {
        model.public_key.clone()
    }
    fn write_public_key(model: &mut Self::ActiveModel, value: &str) {
        model.public_key = Set(value.to_owned());
    }
    fn read_sign_count(model: &Self::Model) -> i64 {
        model.sign_count
    }
    fn read_transports(model: &Self::Model) -> Option<String> {
        model.transports.clone()
    }
    fn read_created_at(model: &Self::Model) -> suprnova::chrono::DateTime<suprnova::chrono::Utc> {
        parse_timestamp(&model.created_at)
    }
}

impl TokenFields for framework_magnetar_engine_token::Entity {
    fn read_token_id(model: &Self::Model) -> String {
        model.token_id.clone()
    }
    fn token_id_column() -> Self::Column {
        framework_magnetar_engine_token::Column::TokenId
    }
    fn read_user_id(model: &Self::Model) -> Option<String> {
        model.user_id.clone()
    }
    fn user_id_column() -> Self::Column {
        framework_magnetar_engine_token::Column::UserId
    }
    fn read_purpose(model: &Self::Model) -> String {
        model.purpose.clone()
    }
    fn purpose_column() -> Self::Column {
        framework_magnetar_engine_token::Column::Purpose
    }
    fn purpose_column_name() -> &'static str {
        "purpose"
    }
    fn read_digest(model: &Self::Model) -> String {
        model.digest.clone()
    }
    fn digest_column() -> Self::Column {
        framework_magnetar_engine_token::Column::Digest
    }
    fn digest_column_name() -> &'static str {
        "digest"
    }
    fn read_expires_at(model: &Self::Model) -> suprnova::chrono::DateTime<suprnova::chrono::Utc> {
        parse_timestamp(&model.expires_at)
    }
    fn expires_at_column() -> Self::Column {
        framework_magnetar_engine_token::Column::ExpiresAt
    }
    fn read_used_at(
        model: &Self::Model,
    ) -> Option<suprnova::chrono::DateTime<suprnova::chrono::Utc>> {
        parse_optional_timestamp(model.used_at.as_deref())
    }
    fn used_at_column() -> Self::Column {
        framework_magnetar_engine_token::Column::UsedAt
    }
    fn used_at_column_name() -> &'static str {
        "used_at"
    }
    fn write_used_at(
        model: &mut Self::ActiveModel,
        value: Option<suprnova::chrono::DateTime<suprnova::chrono::Utc>>,
    ) {
        model.used_at = Set(value.map(|value| value.to_rfc3339()));
    }
    fn write_token_id(model: &mut Self::ActiveModel, value: &str) {
        model.token_id = Set(value.to_owned());
    }
    fn write_user_id(model: &mut Self::ActiveModel, value: Option<&str>) {
        model.user_id = Set(value.map(str::to_owned));
    }
    fn write_purpose(model: &mut Self::ActiveModel, value: &str) {
        model.purpose = Set(value.to_owned());
    }
    fn write_digest(model: &mut Self::ActiveModel, value: &str) {
        model.digest = Set(value.to_owned());
    }
    fn write_expires_at(
        model: &mut Self::ActiveModel,
        value: suprnova::chrono::DateTime<suprnova::chrono::Utc>,
    ) {
        model.expires_at = Set(value.to_rfc3339());
    }
}

impl CeremonyFields for framework_magnetar_engine_ceremony::Entity {
    fn read_ceremony_id(model: &Self::Model) -> String {
        model.ceremony_id.clone()
    }
    fn ceremony_id_column() -> Self::Column {
        framework_magnetar_engine_ceremony::Column::CeremonyId
    }
    fn read_kind(model: &Self::Model) -> String {
        model.kind.clone()
    }
    fn kind_column() -> Self::Column {
        framework_magnetar_engine_ceremony::Column::Kind
    }
    fn kind_column_name() -> &'static str {
        "kind"
    }
    fn read_selector(model: &Self::Model) -> String {
        model.selector.clone()
    }
    fn selector_column() -> Self::Column {
        framework_magnetar_engine_ceremony::Column::Selector
    }
    fn selector_column_name() -> &'static str {
        "selector"
    }
    fn read_payload(model: &Self::Model) -> Vec<u8> {
        model.payload.clone()
    }
    fn read_state(model: &Self::Model) -> String {
        model.state.clone()
    }
    fn state_column() -> Self::Column {
        framework_magnetar_engine_ceremony::Column::State
    }
    fn state_column_name() -> &'static str {
        "state"
    }
    fn read_expires_at(model: &Self::Model) -> suprnova::chrono::DateTime<suprnova::chrono::Utc> {
        parse_timestamp(&model.expires_at)
    }
    fn expires_at_column() -> Self::Column {
        framework_magnetar_engine_ceremony::Column::ExpiresAt
    }
    fn read_used_at(
        model: &Self::Model,
    ) -> Option<suprnova::chrono::DateTime<suprnova::chrono::Utc>> {
        parse_optional_timestamp(model.used_at.as_deref())
    }
    fn used_at_column() -> Self::Column {
        framework_magnetar_engine_ceremony::Column::UsedAt
    }
    fn write_state(model: &mut Self::ActiveModel, state: &str) {
        model.state = Set(state.to_owned());
    }
    fn write_used_at(
        model: &mut Self::ActiveModel,
        value: Option<suprnova::chrono::DateTime<suprnova::chrono::Utc>>,
    ) {
        model.used_at = Set(value.map(|value| value.to_rfc3339()));
    }
    fn write_ceremony_id(model: &mut Self::ActiveModel, value: &str) {
        model.ceremony_id = Set(value.to_owned());
    }
    fn write_kind(model: &mut Self::ActiveModel, value: &str) {
        model.kind = Set(value.to_owned());
    }
    fn write_selector(model: &mut Self::ActiveModel, value: &str) {
        model.selector = Set(value.to_owned());
    }
    fn write_payload(model: &mut Self::ActiveModel, value: &[u8]) {
        model.payload = Set(value.to_vec());
    }
    fn write_expires_at(
        model: &mut Self::ActiveModel,
        value: suprnova::chrono::DateTime<suprnova::chrono::Utc>,
    ) {
        model.expires_at = Set(value.to_rfc3339());
    }
}

struct FrameworkAuthSchema;

impl AuthSchema for FrameworkAuthSchema {
    type User = framework_magnetar_engine_user::Entity;
    type Session = framework_magnetar_engine_session::Entity;
    type LinkedAccount = framework_magnetar_engine_linked_account::Entity;
    type Passkey = framework_magnetar_engine_passkey::Entity;
    type Token = framework_magnetar_engine_token::Entity;
    type Ceremony = framework_magnetar_engine_ceremony::Entity;
    type Lockout = framework_magnetar_engine_lockout::Entity;
    type TokenRecord = framework_magnetar_engine_token_record::Entity;
}

struct FrameworkSessionStore {
    database: sea_orm::DatabaseConnection,
}

#[async_trait]
impl OpaqueSessionStore for FrameworkSessionStore {
    async fn insert_session(&self, session: StoredSession) -> MagnetarResult<()> {
        let row = framework_magnetar_engine_session::ActiveModel {
            session_id: Set(session.session_id),
            user_id: Set(session.user_id),
            token_hash: Set(hex_digest(&session.token_hash)),
            token_digest: Set(hex_digest(&session.token_digest)),
            expires_at: Set(session.expires_at.to_rfc3339()),
            revoked_at: Set(session.revoked_at.map(|value| value.to_rfc3339())),
            user_agent: Set(session.metadata.user_agent),
            ip_address: Set(session.metadata.ip_address),
            ..Default::default()
        };
        row.insert(&self.database).await.map_err(database_error)?;
        Ok(())
    }

    async fn find_by_token_hash(
        &self,
        token_hash: [u8; 32],
    ) -> MagnetarResult<Option<StoredSession>> {
        framework_magnetar_engine_session::Entity::find()
            .filter(
                framework_magnetar_engine_session::Column::TokenHash.eq(hex_digest(&token_hash)),
            )
            .filter(framework_magnetar_engine_session::Column::RevokedAt.is_null())
            .one(&self.database)
            .await
            .map_err(database_error)?
            .map(stored_session)
            .transpose()
    }

    async fn find_by_web_binding(
        &self,
        binding: &WebSessionBinding,
    ) -> MagnetarResult<Option<StoredSession>> {
        framework_magnetar_engine_session::Entity::find()
            .filter(
                framework_magnetar_engine_session::Column::SessionId.eq(binding.session_id.clone()),
            )
            .filter(
                framework_magnetar_engine_session::Column::TokenDigest
                    .eq(hex_digest(&binding.token_digest)),
            )
            .filter(framework_magnetar_engine_session::Column::RevokedAt.is_null())
            .one(&self.database)
            .await
            .map_err(database_error)?
            .map(stored_session)
            .transpose()
    }

    async fn revoke_all_sessions(
        &self,
        user_id: &str,
        at: suprnova::chrono::DateTime<suprnova::chrono::Utc>,
    ) -> MagnetarResult<u64> {
        let result = framework_magnetar_engine_session::Entity::update_many()
            .col_expr(
                framework_magnetar_engine_session::Column::RevokedAt,
                Expr::value(at.to_rfc3339()),
            )
            .filter(framework_magnetar_engine_session::Column::UserId.eq(user_id.to_owned()))
            .filter(framework_magnetar_engine_session::Column::RevokedAt.is_null())
            .exec(&self.database)
            .await
            .map_err(database_error)?;
        Ok(result.rows_affected)
    }

    async fn revoke_session(
        &self,
        session_id: &str,
        at: suprnova::chrono::DateTime<suprnova::chrono::Utc>,
    ) -> MagnetarResult<bool> {
        let result = framework_magnetar_engine_session::Entity::update_many()
            .col_expr(
                framework_magnetar_engine_session::Column::RevokedAt,
                Expr::value(at.to_rfc3339()),
            )
            .filter(framework_magnetar_engine_session::Column::SessionId.eq(session_id.to_owned()))
            .filter(framework_magnetar_engine_session::Column::RevokedAt.is_null())
            .exec(&self.database)
            .await
            .map_err(database_error)?;
        Ok(result.rows_affected == 1)
    }

    async fn list_active_sessions(
        &self,
        user_id: &str,
        now: suprnova::chrono::DateTime<suprnova::chrono::Utc>,
    ) -> MagnetarResult<Vec<StoredSession>> {
        framework_magnetar_engine_session::Entity::find()
            .filter(framework_magnetar_engine_session::Column::UserId.eq(user_id.to_owned()))
            .filter(framework_magnetar_engine_session::Column::RevokedAt.is_null())
            .filter(framework_magnetar_engine_session::Column::ExpiresAt.gt(now.to_rfc3339()))
            .all(&self.database)
            .await
            .map_err(database_error)?
            .into_iter()
            .map(stored_session)
            .collect()
    }
}

struct FrameworkFactorVerifier {
    enrolled: std::sync::atomic::AtomicBool,
}

impl FrameworkFactorVerifier {
    fn enrolled() -> Self {
        Self {
            enrolled: std::sync::atomic::AtomicBool::new(true),
        }
    }
}

#[async_trait]
impl FactorVerifier for FrameworkFactorVerifier {
    async fn has_confirmed_enrollment(&self, _: &str) -> MagnetarResult<bool> {
        Ok(self.enrolled.load(std::sync::atomic::Ordering::SeqCst))
    }
    async fn verify_code(&self, _: &str, code: &str) -> MagnetarResult<bool> {
        Ok(code == "654321")
    }
}

struct AllowingLimiter;

#[async_trait]
impl RateLimiterDriver for AllowingLimiter {
    async fn try_acquire(
        &self,
        _: &str,
        _: &SlidingWindowConfig,
    ) -> Result<bool, suprnova::FrameworkError> {
        Ok(true)
    }

    async fn retry_after(
        &self,
        _: &str,
        _: &SlidingWindowConfig,
    ) -> Result<Option<std::time::Duration>, suprnova::FrameworkError> {
        Ok(None)
    }
}

#[derive(Default)]
struct RecordingPasswordLockout {
    locked: std::sync::atomic::AtomicBool,
    attempts: std::sync::atomic::AtomicU32,
    events: std::sync::Mutex<Vec<String>>,
}

impl RecordingPasswordLockout {
    fn current_status(&self, identity: &str) -> suprnova::LockoutStatus {
        suprnova::LockoutStatus {
            email: identity.to_owned(),
            failed_attempts: self.attempts.load(std::sync::atomic::Ordering::SeqCst),
            is_locked: self.locked.load(std::sync::atomic::Ordering::SeqCst),
            locked_until: None,
        }
    }
}

#[async_trait]
impl HostPasswordLockout for RecordingPasswordLockout {
    async fn status(&self, identity: &str) -> MagnetarResult<suprnova::LockoutStatus> {
        self.events
            .lock()
            .expect("lockout events mutex")
            .push(format!("check:{identity}"));
        Ok(self.current_status(identity))
    }

    async fn record_failure(
        &self,
        identity: &str,
        _: Option<&str>,
    ) -> MagnetarResult<suprnova::LockoutStatus> {
        self.events
            .lock()
            .expect("lockout events mutex")
            .push(format!("failure:{identity}"));
        self.attempts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(self.current_status(identity))
    }

    async fn reset_after_success(&self, identity: &str) -> MagnetarResult<()> {
        self.events
            .lock()
            .expect("lockout events mutex")
            .push(format!("success:{identity}"));
        self.attempts.store(0, std::sync::atomic::Ordering::SeqCst);
        self.locked
            .store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    async fn unlock(&self, _: &str) -> MagnetarResult<bool> {
        self.attempts.store(0, std::sync::atomic::Ordering::SeqCst);
        Ok(self.locked.swap(false, std::sync::atomic::Ordering::SeqCst))
    }
}

struct FrameworkUsers {
    database: sea_orm::DatabaseConnection,
}

#[async_trait]
impl HostUserAdapter for FrameworkUsers {
    type User = User;

    async fn user_for_id(&self, user_id: &str) -> MagnetarResult<Self::User> {
        let id = user_id.parse::<i64>().map_err(|_| Error::NotFound {
            resource: "application user".to_owned(),
            identifier: user_id.to_owned(),
        })?;
        let row = framework_magnetar_engine_user::Entity::find_by_id(id)
            .one(&self.database)
            .await
            .map_err(database_error)?
            .ok_or_else(|| Error::NotFound {
                resource: "application user".to_owned(),
                identifier: user_id.to_owned(),
            })?;
        User::builder()
            .id(UserId::new(user_id))
            .name(row.display_name)
            .email(row.login_email)
            .email_verified_at(parse_optional_timestamp(row.email_verified_at.as_deref()))
            .locked_at(parse_optional_timestamp(row.locked_at.as_deref()))
            .created_at(parse_timestamp(&row.created_at))
            .updated_at(parse_timestamp(&row.updated_at))
            .build()
            .map_err(|error| Error::Internal {
                message: format!("map framework user: {error}"),
            })
    }
}

struct SqliteLifecycleDeduplication {
    database: sea_orm::DatabaseConnection,
}

#[async_trait]
impl HostLifecycleDeduplication for SqliteLifecycleDeduplication {
    async fn claim(
        &self,
        mutation_id: &str,
        lease_id: &str,
        now: suprnova::chrono::DateTime<suprnova::chrono::Utc>,
        lease_until: suprnova::chrono::DateTime<suprnova::chrono::Utc>,
    ) -> MagnetarResult<LifecycleDeliveryClaim> {
        let inserted = self.database.execute(sqlite_statement(
            "INSERT INTO framework_magnetar_lifecycle_delivery (mutation_id, state, lease_id, lease_until) VALUES (?, 'in_flight', ?, ?) ON CONFLICT(mutation_id) DO NOTHING",
            vec![mutation_id, lease_id, &lease_until.to_rfc3339()],
        )).await.map_err(database_error)?;
        if inserted.rows_affected() == 1 {
            return Ok(LifecycleDeliveryClaim::Deliver);
        }

        let row = self
            .database
            .query_one(sqlite_statement(
                "SELECT state FROM framework_magnetar_lifecycle_delivery WHERE mutation_id = ?",
                vec![mutation_id],
            ))
            .await
            .map_err(database_error)?
            .ok_or_else(|| Error::Internal {
                message: "lifecycle delivery row disappeared after an insert conflict".to_owned(),
            })?;
        let state: String = row.try_get("", "state").map_err(database_error)?;
        if state == "delivered" {
            return Ok(LifecycleDeliveryClaim::AlreadyDelivered);
        }

        let reclaimed = self.database.execute(sqlite_statement(
            "UPDATE framework_magnetar_lifecycle_delivery SET lease_id = ?, lease_until = ? WHERE mutation_id = ? AND state = 'in_flight' AND lease_until <= ?",
            vec![lease_id, &lease_until.to_rfc3339(), mutation_id, &now.to_rfc3339()],
        )).await.map_err(database_error)?;
        Ok(if reclaimed.rows_affected() == 1 {
            LifecycleDeliveryClaim::Deliver
        } else {
            LifecycleDeliveryClaim::InFlight
        })
    }

    async fn mark_delivered(&self, mutation_id: &str, lease_id: &str) -> MagnetarResult<()> {
        let updated = self.database.execute(sqlite_statement(
            "UPDATE framework_magnetar_lifecycle_delivery SET state = 'delivered', lease_id = NULL, lease_until = NULL WHERE mutation_id = ? AND state = 'in_flight' AND lease_id = ?",
            vec![mutation_id, lease_id],
        )).await.map_err(database_error)?;
        if updated.rows_affected() == 1 {
            Ok(())
        } else {
            Err(Error::Conflict {
                resource: "lifecycle delivery".to_owned(),
                message: "the forwarder no longer owns this delivery lease".to_owned(),
            })
        }
    }

    async fn release(&self, mutation_id: &str, lease_id: &str) -> MagnetarResult<()> {
        self.database.execute(sqlite_statement(
            "DELETE FROM framework_magnetar_lifecycle_delivery WHERE mutation_id = ? AND state = 'in_flight' AND lease_id = ?",
            vec![mutation_id, lease_id],
        )).await.map_err(database_error)?;
        Ok(())
    }
}

#[tokio::test]
#[serial]
async fn host_engine_issues_queries_revokes_and_deduplicates_lifecycle_with_real_sqlite_rows() {
    assert_ne!(
        TypeId::of::<framework_magnetar_engine_user::Entity>(),
        TypeId::of::<framework_magnetar_engine_session::Entity>()
    );
    assert_ne!(
        TypeId::of::<framework_magnetar_engine_session::Entity>(),
        TypeId::of::<framework_magnetar_engine_ceremony::Entity>()
    );
    assert_ne!(
        TypeId::of::<framework_magnetar_engine_linked_account::Entity>(),
        TypeId::of::<framework_magnetar_engine_passkey::Entity>()
    );

    let connection = Database::connect("sqlite::memory:")
        .await
        .expect("connect application-owned SQLite database");
    create_application_auth_tables(&connection).await;

    let binding = MagnetarBinding::<FrameworkAuthSchema>::new(connection);
    let storage = Arc::new(SeaOrmStorage::<FrameworkAuthSchema>::new(
        binding.database().clone(),
    ));
    let password = Arc::new(PasswordAuthService::new(
        storage.clone(),
        storage.clone(),
        Arc::new(test_password_verifier()),
    ));

    let users = Arc::new(FrameworkUsers {
        database: storage.database().clone(),
    });
    let password_lockout = Arc::new(RecordingPasswordLockout::default());
    let event_fake = EventFacade::fake();
    let factor_verifier = Arc::new(FrameworkFactorVerifier::enrolled());
    let engine = Arc::new(
        MagnetarHostEngine::new(MagnetarHostEngineParts {
            binding,
            session_store: Arc::new(FrameworkSessionStore {
                database: storage.database().clone(),
            }),
            ceremonies: storage.clone(),
            factors: factor_verifier.clone(),
            password,
            password_lockout: password_lockout.clone(),
            encryptor: Arc::new(AeadEncryptor::new([7; 32])),
            session_config: OpaqueConfig::default(),
            users,
            lifecycle_deliveries: Arc::new(SqliteLifecycleDeduplication {
                database: storage.database().clone(),
            }),
            lifecycle_lease_duration: suprnova::chrono::Duration::seconds(30),
        })
        .expect("compose a real host engine"),
    );
    suprnova::magnetar_integration::install_magnetar_password_engine_for_test(engine.clone())
        .expect("install the only password dispatcher before the facade is used");

    let rejected = TestContainer::scope(async {
        TestContainer::bind::<dyn RateLimiterDriver>(Arc::new(AllowingLimiter));
        suprnova::Auth::password()
            .authenticate(" HOST-ENGINE@EXAMPLE.TEST ", "wrong password", None, None)
            .await
    })
    .await;
    assert!(matches!(
        rejected,
        Err(suprnova::FrameworkError::Domain {
            status_code: 401,
            ..
        })
    ));
    assert_eq!(
        password_lockout
            .events
            .lock()
            .expect("lockout events mutex")
            .as_slice(),
        [
            "check:host-engine@example.test".to_owned(),
            "failure:host-engine@example.test".to_owned(),
        ]
    );

    let registered = TestContainer::scope(async {
        TestContainer::bind::<dyn RateLimiterDriver>(Arc::new(AllowingLimiter));
        suprnova::Auth::password()
            .register("host-engine@example.test", "correct horse battery staple")
            .await
    })
    .await
    .expect("password facade registers through real Magnetar storage");
    assert_eq!(registered.email, "host-engine@example.test");
    let created = storage
        .find_by_email("host-engine@example.test")
        .await
        .expect("reload registered Magnetar row")
        .expect("registration inserted the user row");
    let lookup_user = suprnova::magnetar_integration::find_user_by_id(&created.user_id)
        .await
        .expect("public direct lookup delegates to installed Magnetar engine")
        .expect("registered Magnetar user is discoverable by id");
    assert_eq!(lookup_user.email, "host-engine@example.test");
    let converted_user = engine
        .users()
        .user_for_id(&created.user_id)
        .await
        .expect("host loads and maps the persisted application row");
    assert_eq!(converted_user.name.as_deref(), Some("Framework Host"));
    assert_eq!(
        converted_user.created_at,
        parse_timestamp("2025-01-02T03:04:05+00:00")
    );
    assert_eq!(converted_user.email, "host-engine@example.test");
    let factor_required = TestContainer::scope(async {
        TestContainer::bind::<dyn RateLimiterDriver>(Arc::new(AllowingLimiter));
        suprnova::Auth::password()
            .authenticate(
                "host-engine@example.test",
                "correct horse battery staple",
                Some("framework-host-engine-test".to_owned()),
                Some("127.0.0.1".to_owned()),
            )
            .await
    })
    .await;
    assert!(matches!(
        factor_required,
        Err(suprnova::FrameworkError::Domain {
            status_code: 401,
            ..
        })
    ));

    let first = issue_factor_approved_session(
        &engine,
        engine
            .authenticate_password(password_attempt())
            .await
            .expect("host engine authenticates through Magnetar password storage"),
    )
    .await;
    let first_token = first
        .session
        .token
        .as_ref()
        .expect("freshly issued users session carries a bearer")
        .expose_secret()
        .to_owned();
    assert_eq!(
        engine
            .bearer_user_id(&first_token)
            .await
            .expect("host engine verifies the Magnetar-issued bearer"),
        Some(created.user_id.clone())
    );
    let first_web_row = engine
        .session_store()
        .find_by_web_binding(&first.web_binding)
        .await
        .expect("look up row by web binding")
        .expect("web binding resolves issued row");
    assert_eq!(first_web_row.session_id, first.session_id);
    assert_eq!(
        first_web_row.metadata.user_agent.as_deref(),
        Some("framework-host-engine-test")
    );
    assert_eq!(
        first_web_row.metadata.ip_address.as_deref(),
        Some("127.0.0.1")
    );
    assert_ne!(first_web_row.token_hash, [0; 32]);
    assert_eq!(
        engine
            .session_provider()
            .verify_bearer(&first_token)
            .await
            .expect("hashed bearer lookup resolves session")
            .user_id,
        created.user_id
    );

    let second = issue_factor_approved_session(
        &engine,
        engine
            .authenticate_password(password_attempt())
            .await
            .expect("host engine authenticates second real sign-in"),
    )
    .await;
    let second_token = second
        .session
        .token
        .as_ref()
        .expect("freshly issued users session carries a bearer")
        .expose_secret()
        .to_owned();
    assert!(
        suprnova::magnetar_integration::revoke_session(&first.session_id)
            .await
            .expect("public integration atomically revokes one session")
    );
    assert!(
        !suprnova::magnetar_integration::revoke_session(&first.session_id)
            .await
            .expect("second public revoke observes no live row")
    );
    assert!(
        engine
            .session_provider()
            .verify_bearer(&first_token)
            .await
            .is_err()
    );
    assert_eq!(
        suprnova::magnetar_integration::list_sessions(&created.user_id)
            .await
            .expect("public integration lists active sessions")
            .len(),
        1
    );
    assert_eq!(
        suprnova::magnetar_integration::revoke_all_sessions(&created.user_id)
            .await
            .expect("public integration atomically revokes remaining sessions"),
        1
    );
    assert!(
        engine
            .session_provider()
            .verify_bearer(&second_token)
            .await
            .is_err()
    );

    factor_verifier
        .enrolled
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let magic_token = TestContainer::scope(async {
        TestContainer::bind::<dyn RateLimiterDriver>(Arc::new(AllowingLimiter));
        suprnova::Auth::magic_link()
            .send("magic-link@example.test", "http://localhost/auth/magic")
            .await
    })
    .await
    .expect("public magic-link facade mints a Magnetar plaintext token");
    assert!(!magic_token.is_empty());
    let (magic_user, magic_session) = suprnova::Auth::magic_link()
        .consume(&magic_token)
        .await
        .expect("public magic-link facade consumes the Magnetar token once");
    assert_eq!(magic_user.email, "magic-link@example.test");
    let magic_bearer = magic_session
        .token
        .as_ref()
        .expect("magic-link SessionAllowed has a bearer")
        .expose_secret()
        .to_owned();
    assert_eq!(
        engine
            .bearer_user_id(&magic_bearer)
            .await
            .expect("engine verifies a magic-link-issued bearer"),
        Some(magic_user.id.as_str().to_owned())
    );
    assert!(matches!(
        suprnova::Auth::magic_link().consume(&magic_token).await,
        Err(suprnova::FrameworkError::Domain {
            status_code: 401,
            ..
        })
    ));

    let (facade_user, facade_session) = TestContainer::scope(async {
        TestContainer::bind::<dyn RateLimiterDriver>(Arc::new(AllowingLimiter));
        suprnova::Auth::password()
            .authenticate(
                "host-engine@example.test",
                "correct horse battery staple",
                Some("framework-host-engine-test".to_owned()),
                Some("127.0.0.1".to_owned()),
            )
            .await
    })
    .await
    .expect("unchanged password facade converts a Magnetar SessionAllowed result");
    assert_eq!(facade_user.id.as_str(), created.user_id);
    assert_eq!(
        facade_session.user_agent.as_deref(),
        Some("framework-host-engine-test")
    );
    let facade_token = facade_session
        .token
        .as_ref()
        .expect("facade session contains the newly issued bearer")
        .expose_secret()
        .to_owned();
    assert_eq!(
        engine
            .bearer_user_id(&facade_token)
            .await
            .expect("installed engine verifies the facade-issued bearer"),
        Some(created.user_id.clone())
    );

    let request = build_bearer_request(&format!("Bearer {facade_token}")).await;
    let slot = suprnova::session::new_session_slot_for_test();
    let bound_user_id = suprnova::session::session_scope_for_test(slot, async {
        use suprnova::Middleware;

        let next: suprnova::Next =
            Arc::new(|_| Box::pin(async { Ok(suprnova::HttpResponse::text("ok")) }));
        let _ = suprnova::magnetar_integration::middleware::BearerTokenMiddleware
            .handle(request, next)
            .await;
        suprnova::Auth::id()
    })
    .await;
    assert_eq!(bound_user_id.as_deref(), Some(created.user_id.as_str()));

    let lifecycle = LifecycleEvent::new(
        "committed-session-mutation",
        LifecycleEventKind::SessionCreated,
        created.user_id.clone(),
    );
    assert_eq!(
        engine
            .forward_lifecycle(lifecycle.clone())
            .await
            .expect("forward committed lifecycle event"),
        LifecycleForwardResult::Delivered
    );
    assert_eq!(
        engine
            .forward_lifecycle(lifecycle)
            .await
            .expect("replay uses durable mutation idempotency"),
        LifecycleForwardResult::AlreadyDelivered
    );
    assert_dispatched_once::<MagnetarLifecycleEvent>();
    assert_eq!(
        dispatched::<MagnetarLifecycleEvent>(
            |event| event.mutation_id == "committed-session-mutation"
        ),
        vec![MagnetarLifecycleEvent {
            mutation_id: "committed-session-mutation".to_owned(),
            kind: LifecycleEventKind::SessionCreated,
            user_id: created.user_id,
        }]
    );
    password_lockout
        .locked
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let events_before_locked_attempt = password_lockout
        .events
        .lock()
        .expect("lockout events mutex")
        .len();
    assert!(
        engine
            .authenticate_password(password_attempt())
            .await
            .is_err()
    );
    let lockout_events = password_lockout
        .events
        .lock()
        .expect("lockout events mutex");
    assert_eq!(
        lockout_events[events_before_locked_attempt],
        "check:host-engine@example.test"
    );
    assert_eq!(lockout_events.len(), events_before_locked_attempt + 1);
    let normalized_success = "success:host-engine@example.test".to_owned();
    assert!(lockout_events.contains(&normalized_success));
    drop(event_fake);
}

#[tokio::test]
#[serial]
async fn host_engine_passkey_delegate_uses_real_ceremonies_envelopes_factors_and_sessions() {
    const EMAIL: &str = "passkey-host-engine@example.test";

    let connection = Database::connect("sqlite::memory:")
        .await
        .expect("connect application-owned SQLite database");
    create_application_auth_tables(&connection).await;

    let binding = MagnetarBinding::<FrameworkAuthSchema>::new(connection);
    let storage = Arc::new(SeaOrmStorage::<FrameworkAuthSchema>::new(
        binding.database().clone(),
    ));
    let password = Arc::new(PasswordAuthService::new(
        storage.clone(),
        storage.clone(),
        Arc::new(test_password_verifier()),
    ));
    let factor_verifier = Arc::new(FrameworkFactorVerifier::enrolled());
    let engine = Arc::new(
        MagnetarHostEngine::new(MagnetarHostEngineParts {
            binding,
            session_store: Arc::new(FrameworkSessionStore {
                database: storage.database().clone(),
            }),
            ceremonies: storage.clone(),
            factors: factor_verifier.clone(),
            password,
            password_lockout: Arc::new(RecordingPasswordLockout::default()),
            encryptor: Arc::new(AeadEncryptor::new([9; 32])),
            session_config: OpaqueConfig::default(),
            users: Arc::new(FrameworkUsers {
                database: storage.database().clone(),
            }),
            lifecycle_deliveries: Arc::new(SqliteLifecycleDeduplication {
                database: storage.database().clone(),
            }),
            lifecycle_lease_duration: suprnova::chrono::Duration::seconds(30),
        })
        .expect("compose a real passkey host engine"),
    );
    let passkey_engine = Arc::new(
        engine
            .passkey_service(&PasskeyConfig::default())
            .expect("compose the real passkey adapter over the host engine"),
    );
    suprnova::magnetar_integration::install_magnetar_passkey_engine_for_test(
        passkey_engine.clone(),
    )
    .expect("install the only passkey dispatcher before the facade is used");

    let session_slot = suprnova::session::new_session_slot_for_test();
    let registration = suprnova::session::session_scope_for_test(session_slot.clone(), async {
        suprnova::Auth::passkey().begin_registration(EMAIL).await
    })
    .await
    .expect("public passkey facade delegates registration start to Magnetar");
    assert_eq!(registration.user_email, EMAIL);
    assert_eq!(registration.rp_id, "localhost");

    let origin = webauthn_authenticator_rs::prelude::Url::parse("http://localhost")
        .expect("test WebAuthn origin is valid");
    let mut authenticator = webauthn_authenticator_rs::WebauthnAuthenticator::new(
        webauthn_authenticator_rs::softpasskey::SoftPasskey::new(true),
    );
    let registration_response = authenticator
        .do_registration(origin.clone(), registration.raw_options)
        .expect("software authenticator completes WebAuthn registration");
    let user = suprnova::session::session_scope_for_test(session_slot.clone(), async {
        suprnova::Auth::passkey()
            .finish_registration(EMAIL, registration_response)
            .await
    })
    .await
    .expect("public passkey facade stores the verified Magnetar credential");

    let stored_before = storage
        .passkeys_for_user(user.id.as_str())
        .await
        .expect("load the application-owned credential row");
    assert_eq!(stored_before.len(), 1);
    let envelope_before: serde_json::Value = serde_json::from_str(&stored_before[0].envelope_json)
        .expect("Magnetar stores Magnetar-compatible data_json");
    assert_eq!(
        envelope_before["credential_id"],
        serde_json::Value::String(stored_before[0].credential_id.clone())
    );
    assert!(envelope_before["public_key"].as_str().is_some());
    assert!(envelope_before["created_at"].as_str().is_some());
    assert!(envelope_before["last_used_at"].is_null());

    let anonymous_enrollment =
        suprnova::session::session_scope_for_test(session_slot.clone(), async {
            suprnova::Auth::passkey().begin_registration(EMAIL).await
        })
        .await;
    assert!(matches!(
        anonymous_enrollment,
        Err(suprnova::FrameworkError::Domain {
            status_code: 401,
            ..
        })
    ));

    let authentication = suprnova::session::session_scope_for_test(session_slot.clone(), async {
        suprnova::Auth::passkey().begin_authentication(EMAIL).await
    })
    .await
    .expect("public passkey facade delegates assertion start to Magnetar");
    let authentication_response = authenticator
        .do_authentication(origin.clone(), authentication.raw_options)
        .expect("software authenticator completes WebAuthn assertion");
    let public_factor_required = suprnova::session::session_scope_for_test(session_slot, async {
        suprnova::Auth::passkey()
            .finish_authentication(EMAIL, authentication_response)
            .await
    })
    .await;
    assert!(matches!(
        public_factor_required,
        Err(suprnova::FrameworkError::Domain {
            status_code: 401,
            ..
        })
    ));

    let direct_authentication = passkey_engine
        .passkey_begin_authentication(EMAIL)
        .await
        .expect("host engine begins a second real Magnetar assertion ceremony");
    let direct_response = authenticator
        .do_authentication(origin.clone(), direct_authentication.options)
        .expect("software authenticator completes a second assertion");
    let factor_selector = match passkey_engine
        .passkey_finish_authentication(
            &direct_authentication.selector,
            EMAIL,
            &direct_response,
            SessionMetadata::default(),
        )
        .await
        .expect("host engine verifies the assertion through Magnetar")
    {
        suprnova::magnetar_integration::engine::HostSignInDecision::FactorRequired {
            challenge_selector,
        } => challenge_selector,
        suprnova::magnetar_integration::engine::HostSignInDecision::SessionAllowed(_) => {
            panic!("the configured host factor verifier must require a ceremony")
        }
    };
    let issued = engine
        .complete_challenge(&factor_selector, "654321")
        .await
        .expect("the passkey assertion receives an opaque Magnetar session after factor approval");
    assert_eq!(issued.session.user_id.as_str(), user.id.as_str());
    assert!(issued.session.token.is_some());

    factor_verifier
        .enrolled
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let success_slot = suprnova::session::new_session_slot_for_test();
    let successful_authentication =
        suprnova::session::session_scope_for_test(success_slot.clone(), async {
            suprnova::Auth::passkey().begin_authentication(EMAIL).await
        })
        .await
        .expect("public facade begins an authentication with no enrolled second factor");
    let successful_response = authenticator
        .do_authentication(origin, successful_authentication.raw_options)
        .expect("software authenticator completes the public success assertion");
    let (successful_user, successful_session) =
        suprnova::session::session_scope_for_test(success_slot, async {
            suprnova::Auth::passkey()
                .finish_authentication(EMAIL, successful_response)
                .await
        })
        .await
        .expect("public facade converts an allowed Magnetar session");
    assert_eq!(successful_user.id, user.id);
    assert!(successful_session.token.is_some());

    let stored_after = storage
        .passkeys_for_user(user.id.as_str())
        .await
        .expect("reload the rewritten credential envelope");
    let envelope_after: serde_json::Value = serde_json::from_str(&stored_after[0].envelope_json)
        .expect("updated data_json remains Magnetar-compatible");
    assert_eq!(
        envelope_after["credential_id"],
        envelope_before["credential_id"]
    );
    assert_eq!(envelope_after["name"], envelope_before["name"]);
    assert_eq!(envelope_after["created_at"], envelope_before["created_at"]);
    assert!(envelope_after["last_used_at"].as_str().is_some());
}

#[cfg(feature = "magnetar-oauth")]
struct OfflineOAuthProvider;

#[cfg(feature = "magnetar-oauth")]
#[async_trait]
impl magnetar::oauth::provider::OAuthProvider for OfflineOAuthProvider {
    fn name(&self) -> &'static str {
        "offline"
    }

    fn authorization_shape(&self) -> magnetar::oauth::request_shape::AuthorizationRequestShape {
        magnetar::oauth::request_shape::AuthorizationRequestShape::default()
    }

    fn token_shape(&self) -> magnetar::oauth::request_shape::TokenRequestShape {
        magnetar::oauth::request_shape::TokenRequestShape::default()
    }

    async fn resolve_identity(
        &self,
        response: magnetar::oauth::provider::ProviderResponse,
    ) -> magnetar::oauth::errors::OAuthResult<magnetar::oauth::provider::ProviderIdentity> {
        let magnetar::oauth::provider::ProviderResponse::UserInfo { body } = response else {
            return Err(
                magnetar::oauth::errors::OAuthProtocolError::MalformedProviderResponse {
                    provider: self.name(),
                    message: "offline fixture requires userinfo".to_owned(),
                },
            );
        };
        let (email, email_verified) = match body.as_str() {
            "verified" => (Some("oauth-host@example.test".to_owned()), true),
            "unverified" => (Some("unverified@example.test".to_owned()), false),
            _ => {
                return Err(
                    magnetar::oauth::errors::OAuthProtocolError::MalformedProviderResponse {
                        provider: self.name(),
                        message: "unknown offline userinfo fixture".to_owned(),
                    },
                );
            }
        };
        Ok(magnetar::oauth::identity::VerifiedProviderIdentity {
            provider: self.name().to_owned(),
            subject: "offline-subject".to_owned(),
            email,
            email_verified,
            display_name: Some("Offline OAuth".to_owned()),
        })
    }

    async fn revoke(
        &self,
        _: &str,
        _: magnetar::oauth::provider::TokenHint,
    ) -> magnetar::oauth::errors::OAuthResult<()> {
        Ok(())
    }

    fn client_id(&self) -> &str {
        "offline-client"
    }

    fn token_endpoint(&self) -> String {
        "https://offline.example.test/token".to_owned()
    }

    fn authorization_endpoint(&self) -> String {
        "https://offline.example.test/authorize".to_owned()
    }

    fn userinfo_endpoint(&self) -> Option<String> {
        Some("https://offline.example.test/userinfo".to_owned())
    }

    fn refresh_policy(&self) -> magnetar::oauth::provider::RefreshPolicy {
        magnetar::oauth::provider::RefreshPolicy {
            supported: false,
            token_client_authentication:
                magnetar::oauth::provider::ClientAuthentication::RequestBody,
            extra_authorization_params: Vec::new(),
            required_scopes: Vec::new(),
            requires_reconsent_for_reissue: false,
            invalid_grant_meaning:
                magnetar::oauth::provider::InvalidGrantMeaning::OrdinaryRevocation,
        }
    }

    async fn client_authentication(
        &self,
    ) -> magnetar::oauth::errors::OAuthResult<magnetar::oauth::provider::ClientAuthenticationMaterial>
    {
        Ok(magnetar::oauth::provider::ClientAuthenticationMaterial {
            params: Vec::new(),
            headers: Vec::new(),
        })
    }
}

#[cfg(feature = "magnetar-oauth")]
struct OfflineOAuthTransport {
    responses: std::sync::Mutex<std::collections::VecDeque<magnetar::plugin::HttpResponse>>,
    requests: std::sync::Mutex<Vec<magnetar::plugin::HttpRequest>>,
}

#[cfg(feature = "magnetar-oauth")]
struct OfflineAppleProvider;

#[cfg(feature = "magnetar-oauth")]
#[async_trait]
impl magnetar::oauth::provider::OAuthProvider for OfflineAppleProvider {
    fn name(&self) -> &'static str {
        "apple"
    }

    fn authorization_shape(&self) -> magnetar::oauth::request_shape::AuthorizationRequestShape {
        magnetar::oauth::request_shape::AuthorizationRequestShape::default()
    }

    fn token_shape(&self) -> magnetar::oauth::request_shape::TokenRequestShape {
        magnetar::oauth::request_shape::TokenRequestShape::default()
    }

    async fn resolve_identity(
        &self,
        response: magnetar::oauth::provider::ProviderResponse,
    ) -> magnetar::oauth::errors::OAuthResult<magnetar::oauth::provider::ProviderIdentity> {
        let magnetar::oauth::provider::ProviderResponse::AppleIdToken {
            id_token,
            form_post_user,
            ..
        } = response
        else {
            return Err(
                magnetar::oauth::errors::OAuthProtocolError::MalformedProviderResponse {
                    provider: self.name(),
                    message: "offline Apple fixture requires an ID token".to_owned(),
                },
            );
        };
        if id_token.expose_secret() != "offline-apple-id-token"
            || form_post_user.as_deref() != Some(r#"{"name":{"firstName":"Ada"}}"#)
        {
            return Err(
                magnetar::oauth::errors::OAuthProtocolError::IdentityVerificationFailed {
                    provider: self.name(),
                    reason: "Apple callback input was not preserved".to_owned(),
                },
            );
        }
        Ok(magnetar::oauth::identity::VerifiedProviderIdentity {
            provider: self.name().to_owned(),
            subject: "offline-apple-subject".to_owned(),
            email: Some("offline-apple@example.test".to_owned()),
            email_verified: true,
            display_name: Some("Ada".to_owned()),
        })
    }

    async fn revoke(
        &self,
        _: &str,
        _: magnetar::oauth::provider::TokenHint,
    ) -> magnetar::oauth::errors::OAuthResult<()> {
        Ok(())
    }

    fn client_id(&self) -> &str {
        "offline-apple-client"
    }

    fn token_endpoint(&self) -> String {
        "https://offline.example.test/apple/token".to_owned()
    }

    fn authorization_endpoint(&self) -> String {
        "https://offline.example.test/apple/authorize".to_owned()
    }

    fn userinfo_endpoint(&self) -> Option<String> {
        None
    }

    fn refresh_policy(&self) -> magnetar::oauth::provider::RefreshPolicy {
        OfflineOAuthProvider.refresh_policy()
    }

    async fn client_authentication(
        &self,
    ) -> magnetar::oauth::errors::OAuthResult<magnetar::oauth::provider::ClientAuthenticationMaterial>
    {
        Ok(magnetar::oauth::provider::ClientAuthenticationMaterial {
            params: Vec::new(),
            headers: Vec::new(),
        })
    }
}

#[cfg(feature = "magnetar-oauth")]
impl OfflineOAuthTransport {
    fn new(responses: Vec<magnetar::plugin::HttpResponse>) -> Self {
        Self {
            responses: std::sync::Mutex::new(responses.into()),
            requests: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn request_count(&self) -> usize {
        self.requests.lock().expect("transport lock").len()
    }
}

#[cfg(feature = "magnetar-oauth")]
#[async_trait]
impl magnetar::plugin::HttpTransport for OfflineOAuthTransport {
    async fn send(
        &self,
        request: magnetar::plugin::HttpRequest,
    ) -> MagnetarResult<magnetar::plugin::HttpResponse> {
        self.requests.lock().expect("transport lock").push(request);
        self.responses
            .lock()
            .expect("transport lock")
            .pop_front()
            .ok_or_else(|| Error::DependencyUnavailable {
                dependency: "offline OAuth transport".to_owned(),
                message: "no response fixture remains".to_owned(),
            })
    }
}

#[cfg(feature = "magnetar-oauth")]
struct AllowOAuthBegin;

#[cfg(feature = "magnetar-oauth")]
#[async_trait]
impl magnetar::abuse::AbuseLimiter for AllowOAuthBegin {
    async fn acquire(
        &self,
        _: &str,
        _: magnetar::abuse::AbusePolicy,
    ) -> MagnetarResult<magnetar::abuse::Permit> {
        Ok(magnetar::abuse::Permit::Allowed { retry_after: None })
    }
}

#[cfg(feature = "magnetar-oauth")]
fn offline_response(body: &str) -> magnetar::plugin::HttpResponse {
    magnetar::plugin::HttpResponse {
        status: 200,
        headers: Vec::new(),
        body: body.as_bytes().to_vec(),
    }
}

#[cfg(feature = "magnetar-oauth")]
fn offline_token() -> magnetar::plugin::HttpResponse {
    offline_response(r#"{"access_token":"offline-token","token_type":"Bearer"}"#)
}

#[cfg(feature = "magnetar-oauth")]
fn offline_config(
    transport: Arc<OfflineOAuthTransport>,
) -> suprnova::magnetar_integration::engine::MagnetarOAuthHostConfig {
    suprnova::magnetar_integration::engine::MagnetarOAuthHostConfig::new(
        vec![
            suprnova::magnetar_integration::engine::MagnetarOAuthProviderConfig {
                provider: Arc::new(OfflineOAuthProvider),
                redirect_uri: "https://app.example.test/oauth/callback".to_owned(),
                scopes: vec!["openid".to_owned(), "email".to_owned()],
            },
            suprnova::magnetar_integration::engine::MagnetarOAuthProviderConfig {
                provider: Arc::new(OfflineAppleProvider),
                redirect_uri: "https://app.example.test/oauth/apple/callback".to_owned(),
                scopes: vec!["name".to_owned(), "email".to_owned()],
            },
        ],
        transport,
        Arc::new(AllowOAuthBegin),
        magnetar::oauth::authorization::OAuthAuthorizationConfig::default(),
        magnetar::oauth::identity::AutoLinkPolicy::ExplicitLinkRequired,
    )
    .unwrap_or_else(|_| panic!("offline config is valid"))
}

#[cfg(feature = "magnetar-oauth")]
#[tokio::test]
#[serial]
async fn oauth_host_delegate_binds_state_resolves_outcomes_and_uses_factor_gate() {
    let connection = Database::connect("sqlite::memory:")
        .await
        .expect("connect SQLite");
    create_application_auth_tables(&connection).await;
    let binding = MagnetarBinding::<FrameworkAuthSchema>::new(connection);
    let storage = Arc::new(SeaOrmStorage::<FrameworkAuthSchema>::new(
        binding.database().clone(),
    ));
    let password = Arc::new(PasswordAuthService::new(
        storage.clone(),
        storage.clone(),
        Arc::new(test_password_verifier()),
    ));
    let factors = Arc::new(FrameworkFactorVerifier::enrolled());
    let engine = Arc::new(
        MagnetarHostEngine::new(MagnetarHostEngineParts {
            binding,
            session_store: Arc::new(FrameworkSessionStore {
                database: storage.database().clone(),
            }),
            ceremonies: storage.clone(),
            factors: factors.clone(),
            password,
            password_lockout: Arc::new(RecordingPasswordLockout::default()),
            encryptor: Arc::new(AeadEncryptor::new([33; 32])),
            session_config: OpaqueConfig::default(),
            users: Arc::new(FrameworkUsers {
                database: storage.database().clone(),
            }),
            lifecycle_deliveries: Arc::new(SqliteLifecycleDeduplication {
                database: storage.database().clone(),
            }),
            lifecycle_lease_duration: suprnova::chrono::Duration::seconds(30),
        })
        .expect("compose host engine"),
    );

    let direct_transport = Arc::new(OfflineOAuthTransport::new(vec![
        offline_token(),
        offline_response("unverified"),
    ]));
    let direct = engine
        .oauth_service(offline_config(direct_transport.clone()))
        .expect("compose real OAuth service");
    let begun = direct
        .begin(suprnova::magnetar_integration::engine::MagnetarOAuthBegin {
            provider: "offline".to_owned(),
            intent: magnetar::oauth::authorization::OAuthIntent::SignIn,
            binding: magnetar::oauth::authorization::CeremonyBinding::HostSessionDigest([9; 32]),
            limiter_identity: "direct-test".to_owned(),
        })
        .await
        .expect("begin host-bound ceremony");
    assert!(begun.authorization_url.contains("code_challenge="));
    assert!(
        direct
            .complete(
                suprnova::magnetar_integration::engine::MagnetarOAuthCallback {
                    provider: "offline".to_owned(),
                    state: begun.state.clone(),
                    code: secrecy::SecretString::from("code"),
                    host_session_digest: Some([8; 32]),
                    form_post_user: None,
                    metadata: SessionMetadata::default(),
                }
            )
            .await
            .is_err()
    );
    assert_eq!(direct_transport.request_count(), 0);
    assert!(matches!(
        direct
            .complete(
                suprnova::magnetar_integration::engine::MagnetarOAuthCallback {
                    provider: "offline".to_owned(),
                    state: begun.state,
                    code: secrecy::SecretString::from("code"),
                    host_session_digest: Some([9; 32]),
                    form_post_user: None,
                    metadata: SessionMetadata::default(),
                }
            )
            .await
            .expect("correctly bound callback"),
        suprnova::magnetar_integration::engine::MagnetarOAuthCompletion::EmailCompletionRequired { .. }
    ));
    assert_eq!(direct_transport.request_count(), 2);

    let public_transport = Arc::new(OfflineOAuthTransport::new(vec![
        offline_token(),
        offline_response("verified"),
        offline_token(),
        offline_response("verified"),
        offline_response(
            r#"{"access_token":"offline-apple-access","token_type":"Bearer","id_token":"offline-apple-id-token"}"#,
        ),
    ]));
    suprnova::magnetar_integration::install_magnetar_oauth_engine(Arc::new(
        engine
            .oauth_service(offline_config(public_transport.clone()))
            .expect("compose installed OAuth service"),
    ))
    .expect("install OAuth dispatcher");
    let slot = suprnova::session::new_session_slot_for_test();
    let kickoff = suprnova::session::session_scope_for_test(slot.clone(), async {
        suprnova::Auth::oauth("offline").begin().await
    })
    .await
    .expect("facade routes configured provider to Magnetar");
    let factor_required = suprnova::session::session_scope_for_test(slot.clone(), async {
        suprnova::Auth::oauth("offline")
            .complete("code", &kickoff.state)
            .await
    })
    .await;
    assert!(matches!(
        factor_required,
        Err(suprnova::FrameworkError::Domain {
            status_code: 401,
            ..
        })
    ));
    factors
        .enrolled
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let kickoff = suprnova::session::session_scope_for_test(slot.clone(), async {
        suprnova::Auth::oauth("offline").begin().await
    })
    .await
    .expect("second host ceremony");
    let (user, session) = suprnova::session::session_scope_for_test(slot.clone(), async {
        suprnova::Auth::oauth("offline")
            .complete("code", &kickoff.state)
            .await
    })
    .await
    .expect("factor gate converts an opaque session");
    assert_eq!(user.email, "oauth-host@example.test");
    assert!(session.token.is_some());
    assert_eq!(public_transport.request_count(), 4);
    let unsupported_provider = suprnova::session::session_scope_for_test(slot.clone(), async {
        suprnova::Auth::oauth("github").begin().await
    })
    .await;
    assert!(matches!(
        unsupported_provider,
        Err(suprnova::FrameworkError::Domain {
            status_code: 400,
            ..
        })
    ));
    let apple_kickoff = suprnova::session::session_scope_for_test(slot.clone(), async {
        suprnova::Auth::oauth("apple").begin().await
    })
    .await
    .expect("Apple ceremony starts through Magnetar");
    let (apple_user, apple_session) = suprnova::session::session_scope_for_test(slot, async {
        suprnova::Auth::oauth("apple")
            .complete_with_apple_form_post(
                "apple-code",
                &apple_kickoff.state,
                Some(r#"{"name":{"firstName":"Ada"}}"#.to_owned()),
            )
            .await
    })
    .await
    .expect("Apple form_post callback preserves ID token and user payload");
    assert_eq!(apple_user.email, "offline-apple@example.test");
    assert!(apple_session.token.is_some());
    assert_eq!(public_transport.request_count(), 5);
}

async fn issue_factor_approved_session<S, O, C, F, P, A, D>(
    engine: &MagnetarHostEngine<S, O, C, F, P, A, D>,
    principal: magnetar::auth::VerifiedPrincipal,
) -> suprnova::magnetar_integration::engine::MagnetarIssuedSession
where
    S: AuthSchema,
    O: OpaqueSessionStore + 'static,
    C: CeremonyStore + 'static,
    F: FactorVerifier + 'static,
    P: PasswordAuthProvider,
    A: HostUserAdapter,
    D: HostLifecycleDeduplication,
    S::User: UserBinding + UserOptionalFields + SessionEpoch,
    S::Token: TokenFields,
{
    let selector = match engine
        .complete_sign_in(principal)
        .await
        .expect("factor gate creates a real ceremony")
    {
        suprnova::magnetar_integration::engine::HostSignInDecision::FactorRequired {
            challenge_selector,
        } => challenge_selector,
        suprnova::magnetar_integration::engine::HostSignInDecision::SessionAllowed(_) => {
            panic!("configured host factor verifier must require a ceremony")
        }
    };
    engine
        .complete_challenge(&selector, "654321")
        .await
        .expect("factor completion issues through OpaqueSessionProvider")
}

fn password_attempt() -> PasswordAttempt {
    PasswordAttempt {
        email: "host-engine@example.test".to_owned(),
        password: secrecy::SecretString::from("correct horse battery staple"),
        metadata: SessionMetadata {
            user_agent: Some("framework-host-engine-test".to_owned()),
            ip_address: Some("127.0.0.1".to_owned()),
        },
    }
}

fn test_password_verifier() -> PasswordVerifier {
    PasswordVerifier::new(Arc::new(StandardPasswordHashDriver), fast_hash_config())
        .expect("test password verifier is constructible")
}

async fn build_bearer_request(auth_header: &str) -> suprnova::Request {
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use std::convert::Infallible;
    use tokio::io::AsyncWriteExt;
    use tokio::sync::oneshot;

    let (request_sender, request_receiver) = oneshot::channel::<suprnova::Request>();
    let request_sender = std::sync::Mutex::new(Some(request_sender));
    let (client_io, server_io) = tokio::io::duplex(4096);
    tokio::spawn(async move {
        let service = service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
            let wrapped = suprnova::Request::new(request);
            if let Ok(mut sender) = request_sender.lock()
                && let Some(sender) = sender.take()
            {
                let _ = sender.send(wrapped);
            }
            async {
                Ok::<_, Infallible>(hyper::Response::new(
                    http_body_util::Empty::<bytes::Bytes>::new(),
                ))
            }
        });
        let _ = http1::Builder::new()
            .serve_connection(hyper_util::rt::TokioIo::new(server_io), service)
            .await;
    });
    let request = format!(
        "GET /api/test HTTP/1.1\r\nHost: localhost\r\nAuthorization: {auth_header}\r\nContent-Length: 0\r\n\r\n"
    );
    let mut client_io = client_io;
    client_io
        .write_all(request.as_bytes())
        .await
        .expect("write in-memory HTTP request");
    request_receiver
        .await
        .expect("receive parsed in-memory HTTP request")
}
fn fast_hash_config() -> PasswordHashConfig {
    PasswordHashConfig {
        bcrypt_cost: 4,
        argon2_memory_kib: 8,
        argon2_iterations: 1,
        argon2_parallelism: 1,
    }
}

fn stored_session(
    model: framework_magnetar_engine_session::Model,
) -> MagnetarResult<StoredSession> {
    Ok(StoredSession {
        session_id: model.session_id,
        user_id: model.user_id,
        token_hash: parse_digest(&model.token_hash)?,
        token_digest: parse_digest(&model.token_digest)?,
        expires_at: parse_timestamp(&model.expires_at),
        revoked_at: parse_optional_timestamp(model.revoked_at.as_deref()),
        metadata: SessionMetadata {
            user_agent: model.user_agent,
            ip_address: model.ip_address,
        },
    })
}

fn parse_timestamp(value: &str) -> suprnova::chrono::DateTime<suprnova::chrono::Utc> {
    suprnova::chrono::DateTime::parse_from_rfc3339(value)
        .expect("test rows persist RFC 3339 timestamps")
        .with_timezone(&suprnova::chrono::Utc)
}

fn parse_optional_timestamp(
    value: Option<&str>,
) -> Option<suprnova::chrono::DateTime<suprnova::chrono::Utc>> {
    value.map(parse_timestamp)
}

fn hex_digest(value: &[u8; 32]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn parse_digest(value: &str) -> MagnetarResult<[u8; 32]> {
    if value.len() != 64 {
        return Err(Error::Internal {
            message: "stored token digest was not 32 bytes".to_owned(),
        });
    }
    let mut digest = [0; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).map_err(|_| {
            Error::Internal {
                message: "stored token digest contained non-hex data".to_owned(),
            }
        })?;
    }
    Ok(digest)
}

fn database_error(error: sea_orm::DbErr) -> Error {
    Error::Internal {
        message: error.to_string(),
    }
}

fn sqlite_statement(sql: &str, values: Vec<&str>) -> Statement {
    Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        sql,
        values
            .into_iter()
            .map(|value| value.into())
            .collect::<Vec<_>>(),
    )
}

async fn create_application_auth_tables(connection: &sea_orm::DatabaseConnection) {
    for statement in [
        "CREATE TABLE framework_magnetar_engine_users (id INTEGER PRIMARY KEY NOT NULL, login_email TEXT NOT NULL, password_hash TEXT NOT NULL, display_name TEXT DEFAULT 'Framework Host', email_verified_at TEXT, locked_at TEXT, created_at TEXT NOT NULL DEFAULT '2025-01-02T03:04:05+00:00', updated_at TEXT NOT NULL DEFAULT '2025-01-02T03:04:05+00:00', session_version INTEGER NOT NULL)",
        "CREATE TABLE framework_magnetar_engine_sessions (id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL UNIQUE, user_id TEXT NOT NULL, token_hash TEXT NOT NULL UNIQUE, token_digest TEXT NOT NULL, expires_at TEXT NOT NULL, revoked_at TEXT, user_agent TEXT, ip_address TEXT)",
        "CREATE TABLE framework_magnetar_engine_linked_accounts (id INTEGER PRIMARY KEY, account_id TEXT NOT NULL, user_id TEXT NOT NULL, provider TEXT NOT NULL, provider_account_id TEXT NOT NULL, access_token TEXT, refresh_token TEXT, expires_at TEXT)",
        "CREATE TABLE framework_magnetar_engine_passkeys (id INTEGER PRIMARY KEY, passkey_id TEXT NOT NULL, user_id TEXT NOT NULL, credential_id TEXT NOT NULL, public_key TEXT NOT NULL, sign_count INTEGER NOT NULL DEFAULT 0, transports TEXT, created_at TEXT NOT NULL DEFAULT '2025-01-02T03:04:05+00:00')",
        "CREATE TABLE framework_magnetar_engine_tokens (id INTEGER PRIMARY KEY, token_id TEXT NOT NULL UNIQUE, user_id TEXT, purpose TEXT NOT NULL, digest TEXT NOT NULL, expires_at TEXT NOT NULL, used_at TEXT)",
        "CREATE TABLE framework_magnetar_engine_ceremonies (id INTEGER PRIMARY KEY AUTOINCREMENT, ceremony_id TEXT NOT NULL UNIQUE, kind TEXT NOT NULL, selector TEXT NOT NULL UNIQUE, payload BLOB NOT NULL, state TEXT NOT NULL, expires_at TEXT NOT NULL, used_at TEXT)",
        "CREATE TABLE framework_magnetar_engine_lockouts (id INTEGER PRIMARY KEY, email TEXT NOT NULL)",
        "CREATE TABLE framework_magnetar_engine_token_records (id INTEGER PRIMARY KEY, user_id TEXT NOT NULL)",
        "CREATE TABLE framework_magnetar_lifecycle_delivery (mutation_id TEXT PRIMARY KEY NOT NULL, state TEXT NOT NULL, lease_id TEXT, lease_until TEXT)",
    ] {
        connection
            .execute_unprepared(statement)
            .await
            .expect("create application-owned auth table");
    }
}
