#![allow(dead_code)]

use chrono::{DateTime, Utc};
use magnetar::schema::{
    AuthSchema, BrokerSchema, CeremonyFields, EntityBinding, LinkedAccountFields, PasskeyFields,
    ProviderTokenFields, SessionEpoch, SessionFields, TokenFields, UserFields, UserOptionalFields,
};
use sea_orm::entity::prelude::{ActiveModelBehavior, ActiveModelTrait};
use sea_orm::sea_query::SqliteQueryBuilder;
use sea_orm::{
    ActiveValue::Set, ConnectionTrait, Database, DatabaseConnection, DbBackend, Schema, Statement,
};
use sea_orm::{DeriveEntityModel, DeriveRelation, EnumIter};
use sha2::{Digest, Sha256};

macro_rules! entity_common {
    ($module:ident, $table:literal, { $($field:tt)* }) => {
        pub mod $module {
            use super::*;
            #[allow(unused_imports)]
            use chrono::{DateTime as ChronoDateTime, Utc as ChronoUtc};
            use sea_orm::entity::prelude::*;
            #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
            #[sea_orm(table_name = $table)]
            pub struct Model { $($field)* }
            #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
            pub enum Relation {}
            impl ActiveModelBehavior for ActiveModel {}
        }
    };
}

entity_common!(users, "storage_users", {
    #[sea_orm(primary_key)] pub id: i64,
    pub email: String,
    pub password_hash: Option<String>,
    pub remember_token: Option<String>,
    pub email_verified_at: Option<ChronoDateTime<ChronoUtc>>,
    pub locked_at: Option<ChronoDateTime<ChronoUtc>>,
    pub auth_epoch: i64,
});
entity_common!(sessions, "storage_sessions", {
    #[sea_orm(primary_key, auto_increment = false)] pub id: String,
    pub user_id: i64,
    pub auth_epoch: i64,
    pub token_digest: String,
    pub token_hash: Option<String>,
    pub user_agent: Option<String>,
    pub ip_address: Option<String>,
    pub expires_at: ChronoDateTime<ChronoUtc>,
    pub revoked_at: Option<ChronoDateTime<ChronoUtc>>,
});
entity_common!(methods, "storage_methods", {
    #[sea_orm(primary_key)] pub id: i64,
    pub user_id: i64,
    pub credential_id: Option<String>,
    pub public_key: Option<String>,
    pub created_at: Option<ChronoDateTime<ChronoUtc>>,
});
entity_common!(accounts, "storage_accounts", {
    #[sea_orm(primary_key)] pub id: i64,
    pub user_id: i64,
    pub provider: String,
    pub provider_account_id: String,
});
entity_common!(tokens, "storage_tokens", {
    #[sea_orm(primary_key)] pub id: i64,
    pub user_id: Option<i64>,
    pub purpose: String,
    pub digest: String,
    pub expires_at: ChronoDateTime<ChronoUtc>,
    pub used_at: Option<ChronoDateTime<ChronoUtc>>,
});
entity_common!(ceremonies, "storage_ceremonies", {
    #[sea_orm(primary_key)] pub id: i64,
    pub kind: String,
    pub selector: String,
    pub payload: Vec<u8>,
    pub state: String,
    pub expires_at: ChronoDateTime<ChronoUtc>,
    pub used_at: Option<ChronoDateTime<ChronoUtc>>,
});
entity_common!(lockouts, "storage_lockouts", {
    #[sea_orm(primary_key)] pub id: i64,
    pub identity: String,
    pub attempted_at: ChronoDateTime<ChronoUtc>,
    pub locked_at: Option<ChronoDateTime<ChronoUtc>>,
    pub reason: Option<String>,
});
entity_common!(two_factor, "storage_two_factor", {
    #[sea_orm(primary_key, auto_increment = false)] pub user_id: String,
    pub secret: Vec<u8>,
    pub recovery_codes: Option<Vec<u8>>,
    pub enrollment_auth_epoch: i64,
    pub enrollment_session_id: Option<String>,
    pub enrollment_expires_at: Option<ChronoDateTime<ChronoUtc>>,
    pub rotation_pending: bool,
    pub confirmed_at: Option<ChronoDateTime<ChronoUtc>>,
    pub last_used_timestep: Option<i64>,
});
entity_common!(remembers, "storage_remembers", {
    #[sea_orm(primary_key, auto_increment = false)] pub id: String,
    pub selector: String,
    pub user_id: String,
    pub auth_epoch: i64,
    pub verifier_hash: String,
    pub expires_at: ChronoDateTime<ChronoUtc>,
});
entity_common!(provider_tokens, "storage_provider_tokens", {
    #[sea_orm(primary_key, auto_increment = false)] pub id: String,
    pub provider: String,
    pub access_ciphertext: Vec<u8>,
    pub refresh_ciphertext: Option<Vec<u8>>,
    pub raw_payload_ciphertext: Vec<u8>,
    pub token_type: String,
    pub scopes: String,
    pub access_expires_at: Option<ChronoDateTime<ChronoUtc>>,
    pub generation: i64,
    pub claim_id: Option<String>,
    pub claim_deadline: Option<ChronoDateTime<ChronoUtc>>,
    pub revoked_at: Option<ChronoDateTime<ChronoUtc>>,
    pub revoked_reused: Option<bool>,
    pub created_at: ChronoDateTime<ChronoUtc>,
});

macro_rules! bind { ($($module:ident),*) => {$ (
    impl EntityBinding for $module::Entity {
        type Entity = $module::Entity;
        type Column = $module::Column;
        type PrimaryKey = $module::PrimaryKey;
        type Model = $module::Model;
        type ActiveModel = $module::ActiveModel;
    }
)*}; }
bind!(
    users,
    sessions,
    methods,
    accounts,
    tokens,
    ceremonies,
    lockouts,
    remembers,
    provider_tokens
);

#[derive(Clone, Copy, Debug, Default)]
pub struct StorageSchema;
impl AuthSchema for StorageSchema {
    type User = users::Entity;
    type Session = sessions::Entity;
    type LinkedAccount = accounts::Entity;
    type Passkey = methods::Entity;
    type Token = tokens::Entity;
    type Ceremony = ceremonies::Entity;
    type Lockout = lockouts::Entity;
    type TokenRecord = tokens::Entity;
}

impl BrokerSchema for StorageSchema {
    type ProviderToken = provider_tokens::Entity;
}

impl UserFields for users::Entity {
    fn read_user_id(m: &Self::Model) -> String {
        m.id.to_string()
    }
    fn user_id_column() -> Self::Column {
        users::Column::Id
    }
    fn write_user_id(m: &mut Self::ActiveModel, v: &str) {
        m.id = Set(v.parse().expect("fixture user ids are i64"));
    }
    fn read_email(m: &Self::Model) -> String {
        m.email.clone()
    }
    fn email_column() -> Self::Column {
        users::Column::Email
    }
    fn write_email(m: &mut Self::ActiveModel, v: &str) {
        m.email = Set(v.to_owned());
    }
    fn read_password_hash(m: &Self::Model) -> Option<String> {
        m.password_hash.clone()
    }
    fn password_hash_column() -> Self::Column {
        users::Column::PasswordHash
    }
    fn write_password_hash(m: &mut Self::ActiveModel, v: Option<&str>) {
        m.password_hash = Set(v.map(ToOwned::to_owned));
    }
    fn read_locked_at(m: &Self::Model) -> Option<DateTime<Utc>> {
        m.locked_at
    }
    fn locked_at_column() -> Self::Column {
        users::Column::LockedAt
    }
    fn write_locked_at(m: &mut Self::ActiveModel, v: Option<DateTime<Utc>>) {
        m.locked_at = Set(v);
    }
}
impl UserOptionalFields for users::Entity {
    fn read_name(_: &Self::Model) -> Option<String> {
        None
    }
    fn read_email_verified_at(m: &Self::Model) -> Option<DateTime<Utc>> {
        m.email_verified_at
    }
    fn write_email_verified_at(m: &mut Self::ActiveModel, v: Option<DateTime<Utc>>) {
        m.email_verified_at = Set(v);
    }
    fn read_remember_token(m: &Self::Model) -> Option<String> {
        m.remember_token.clone()
    }
    fn write_remember_token(m: &mut Self::ActiveModel, v: Option<&str>) {
        m.remember_token = Set(v.map(ToOwned::to_owned));
    }
}
impl SessionEpoch for users::Entity {
    fn auth_epoch(m: &Self::Model) -> u64 {
        m.auth_epoch as u64
    }
    fn auth_epoch_column() -> Self::Column {
        users::Column::AuthEpoch
    }
    fn write_auth_epoch(m: &mut Self::ActiveModel, v: u64) {
        m.auth_epoch = Set(v as i64);
    }
}
impl SessionFields for sessions::Entity {
    fn read_session_id(m: &Self::Model) -> String {
        m.id.clone()
    }
    fn session_id_column() -> Self::Column {
        sessions::Column::Id
    }
    fn read_user_id(m: &Self::Model) -> String {
        m.user_id.to_string()
    }
    fn user_id_column() -> Self::Column {
        sessions::Column::UserId
    }
    fn read_auth_epoch(m: &Self::Model) -> magnetar::Result<u64> {
        u64::try_from(m.auth_epoch).map_err(|_| magnetar::Error::Internal {
            message: "stored session auth_epoch cannot be negative".to_owned(),
        })
    }
    fn auth_epoch_column() -> Self::Column {
        sessions::Column::AuthEpoch
    }
    fn auth_epoch_value(value: u64) -> magnetar::Result<sea_orm::Value> {
        let value = i64::try_from(value).map_err(|_| magnetar::Error::InvalidInput {
            field: "auth_epoch".to_owned(),
            message: "exceeds the database integer range".to_owned(),
        })?;
        Ok(value.into())
    }
    fn write_auth_epoch(m: &mut Self::ActiveModel, v: u64) -> magnetar::Result<()> {
        let value = i64::try_from(v).map_err(|_| magnetar::Error::InvalidInput {
            field: "auth_epoch".to_owned(),
            message: "exceeds the database integer range".to_owned(),
        })?;
        m.auth_epoch = Set(value);
        Ok(())
    }
    fn read_token_digest(m: &Self::Model) -> String {
        m.token_digest.clone()
    }
    fn read_expires_at(m: &Self::Model) -> DateTime<Utc> {
        m.expires_at
    }
    fn read_revoked_at(m: &Self::Model) -> Option<DateTime<Utc>> {
        m.revoked_at
    }
    fn revoked_at_column() -> Self::Column {
        sessions::Column::RevokedAt
    }
    fn write_revoked_at(m: &mut Self::ActiveModel, v: Option<DateTime<Utc>>) {
        m.revoked_at = Set(v);
    }
}
impl LinkedAccountFields for accounts::Entity {
    fn read_account_id(m: &Self::Model) -> String {
        m.id.to_string()
    }
    fn account_id_column() -> Self::Column {
        accounts::Column::Id
    }
    fn write_account_id(m: &mut Self::ActiveModel, v: &str) {
        m.id = Set(v.parse().expect("fixture account ids are i64"));
    }
    fn read_user_id(m: &Self::Model) -> String {
        m.user_id.to_string()
    }
    fn user_id_column() -> Self::Column {
        accounts::Column::UserId
    }
    fn write_user_id(m: &mut Self::ActiveModel, v: &str) {
        m.user_id = Set(v.parse().expect("fixture user ids are i64"));
    }
    fn read_provider(m: &Self::Model) -> String {
        m.provider.clone()
    }
    fn provider_column() -> Self::Column {
        accounts::Column::Provider
    }
    fn write_provider(m: &mut Self::ActiveModel, v: &str) {
        m.provider = Set(v.to_owned());
    }
    fn read_provider_account_id(m: &Self::Model) -> String {
        m.provider_account_id.clone()
    }
    fn provider_account_id_column() -> Self::Column {
        accounts::Column::ProviderAccountId
    }
    fn write_provider_account_id(m: &mut Self::ActiveModel, v: &str) {
        m.provider_account_id = Set(v.to_owned());
    }
    fn read_access_token(_: &Self::Model) -> Option<String> {
        None
    }
    fn read_refresh_token(_: &Self::Model) -> Option<String> {
        None
    }
    fn read_expires_at(_: &Self::Model) -> Option<DateTime<Utc>> {
        None
    }
}
impl PasskeyFields for methods::Entity {
    fn read_passkey_id(m: &Self::Model) -> String {
        m.id.to_string()
    }
    fn passkey_id_column() -> Self::Column {
        methods::Column::Id
    }
    fn write_passkey_id(m: &mut Self::ActiveModel, v: &str) {
        m.id = Set(v.parse().expect("fixture passkey ids are i64"));
    }
    fn read_user_id(m: &Self::Model) -> String {
        m.user_id.to_string()
    }
    fn user_id_column() -> Self::Column {
        methods::Column::UserId
    }
    fn write_user_id(m: &mut Self::ActiveModel, v: &str) {
        m.user_id = Set(v.parse().expect("fixture user ids are i64"));
    }
    fn read_credential_id(m: &Self::Model) -> String {
        m.credential_id.clone().unwrap_or_default()
    }
    fn credential_id_column() -> Self::Column {
        methods::Column::CredentialId
    }
    fn write_credential_id(m: &mut Self::ActiveModel, v: &str) {
        m.credential_id = Set(Some(v.to_owned()));
        m.created_at = Set(Some(Utc::now()));
    }
    fn read_public_key(m: &Self::Model) -> String {
        m.public_key.clone().unwrap_or_default()
    }
    fn write_public_key(m: &mut Self::ActiveModel, v: &str) {
        m.public_key = Set(Some(v.to_owned()));
    }
    fn read_sign_count(_: &Self::Model) -> i64 {
        0
    }
    fn read_transports(_: &Self::Model) -> Option<String> {
        None
    }
    fn read_created_at(m: &Self::Model) -> DateTime<Utc> {
        m.created_at.unwrap_or_else(Utc::now)
    }
}
impl TokenFields for tokens::Entity {
    fn read_token_id(m: &Self::Model) -> String {
        m.id.to_string()
    }
    fn token_id_column() -> Self::Column {
        tokens::Column::Id
    }
    fn read_user_id(m: &Self::Model) -> Option<String> {
        m.user_id.map(|v| v.to_string())
    }
    fn user_id_column() -> Self::Column {
        tokens::Column::UserId
    }
    fn read_purpose(m: &Self::Model) -> String {
        m.purpose.clone()
    }
    fn purpose_column() -> Self::Column {
        tokens::Column::Purpose
    }
    fn purpose_column_name() -> &'static str {
        "purpose"
    }
    fn read_digest(m: &Self::Model) -> String {
        m.digest.clone()
    }
    fn digest_column() -> Self::Column {
        tokens::Column::Digest
    }
    fn digest_column_name() -> &'static str {
        "digest"
    }
    fn read_expires_at(m: &Self::Model) -> DateTime<Utc> {
        m.expires_at
    }
    fn expires_at_column() -> Self::Column {
        tokens::Column::ExpiresAt
    }
    fn read_used_at(m: &Self::Model) -> Option<DateTime<Utc>> {
        m.used_at
    }
    fn used_at_column() -> Self::Column {
        tokens::Column::UsedAt
    }
    fn used_at_column_name() -> &'static str {
        "used_at"
    }
    fn write_used_at(m: &mut Self::ActiveModel, v: Option<DateTime<Utc>>) {
        m.used_at = Set(v);
    }
    fn write_token_id(m: &mut Self::ActiveModel, v: &str) {
        m.id = Set(v.parse().unwrap());
    }
    fn write_user_id(m: &mut Self::ActiveModel, v: Option<&str>) {
        m.user_id = Set(v.map(|x| x.parse().unwrap()));
    }
    fn write_purpose(m: &mut Self::ActiveModel, v: &str) {
        m.purpose = Set(v.to_owned());
    }
    fn write_digest(m: &mut Self::ActiveModel, v: &str) {
        m.digest = Set(v.to_owned());
    }
    fn write_expires_at(m: &mut Self::ActiveModel, v: DateTime<Utc>) {
        m.expires_at = Set(v);
    }
}
impl CeremonyFields for ceremonies::Entity {
    fn read_ceremony_id(m: &Self::Model) -> String {
        m.id.to_string()
    }
    fn ceremony_id_column() -> Self::Column {
        ceremonies::Column::Id
    }
    fn read_kind(m: &Self::Model) -> String {
        m.kind.clone()
    }
    fn kind_column() -> Self::Column {
        ceremonies::Column::Kind
    }
    fn kind_column_name() -> &'static str {
        "kind"
    }
    fn read_selector(m: &Self::Model) -> String {
        m.selector.clone()
    }
    fn selector_column() -> Self::Column {
        ceremonies::Column::Selector
    }
    fn selector_column_name() -> &'static str {
        "selector"
    }
    fn read_payload(m: &Self::Model) -> Vec<u8> {
        m.payload.clone()
    }
    fn read_state(m: &Self::Model) -> String {
        m.state.clone()
    }
    fn state_column() -> Self::Column {
        ceremonies::Column::State
    }
    fn state_column_name() -> &'static str {
        "state"
    }
    fn read_expires_at(m: &Self::Model) -> DateTime<Utc> {
        m.expires_at
    }
    fn expires_at_column() -> Self::Column {
        ceremonies::Column::ExpiresAt
    }
    fn read_used_at(m: &Self::Model) -> Option<DateTime<Utc>> {
        m.used_at
    }
    fn used_at_column() -> Self::Column {
        ceremonies::Column::UsedAt
    }
    fn write_state(m: &mut Self::ActiveModel, v: &str) {
        m.state = Set(v.to_owned());
    }
    fn write_used_at(m: &mut Self::ActiveModel, v: Option<DateTime<Utc>>) {
        m.used_at = Set(v);
    }
    fn write_ceremony_id(m: &mut Self::ActiveModel, v: &str) {
        m.id = Set(v.parse().unwrap());
    }
    fn write_kind(m: &mut Self::ActiveModel, v: &str) {
        m.kind = Set(v.to_owned());
    }
    fn write_selector(m: &mut Self::ActiveModel, v: &str) {
        m.selector = Set(v.to_owned());
    }
    fn write_payload(m: &mut Self::ActiveModel, v: &[u8]) {
        m.payload = Set(v.to_vec());
    }
    fn write_expires_at(m: &mut Self::ActiveModel, v: DateTime<Utc>) {
        m.expires_at = Set(v);
    }
}

impl ProviderTokenFields for provider_tokens::Entity {
    fn read_id(m: &Self::Model) -> String {
        m.id.clone()
    }
    fn id_column() -> Self::Column {
        provider_tokens::Column::Id
    }
    fn write_id(m: &mut Self::ActiveModel, v: &str) {
        m.id = Set(v.to_owned());
    }
    fn read_provider(m: &Self::Model) -> String {
        m.provider.clone()
    }
    fn provider_column() -> Self::Column {
        provider_tokens::Column::Provider
    }
    fn write_provider(m: &mut Self::ActiveModel, v: &str) {
        m.provider = Set(v.to_owned());
    }
    fn read_access_ciphertext(m: &Self::Model) -> Vec<u8> {
        m.access_ciphertext.clone()
    }
    fn access_ciphertext_column() -> Self::Column {
        provider_tokens::Column::AccessCiphertext
    }
    fn write_access_ciphertext(m: &mut Self::ActiveModel, v: &[u8]) {
        m.access_ciphertext = Set(v.to_vec());
    }
    fn read_refresh_ciphertext(m: &Self::Model) -> Option<Vec<u8>> {
        m.refresh_ciphertext.clone()
    }
    fn refresh_ciphertext_column() -> Self::Column {
        provider_tokens::Column::RefreshCiphertext
    }
    fn write_refresh_ciphertext(m: &mut Self::ActiveModel, v: Option<&[u8]>) {
        m.refresh_ciphertext = Set(v.map(|bytes| bytes.to_vec()));
    }
    fn read_raw_payload_ciphertext(m: &Self::Model) -> Vec<u8> {
        m.raw_payload_ciphertext.clone()
    }
    fn raw_payload_ciphertext_column() -> Self::Column {
        provider_tokens::Column::RawPayloadCiphertext
    }
    fn write_raw_payload_ciphertext(m: &mut Self::ActiveModel, v: &[u8]) {
        m.raw_payload_ciphertext = Set(v.to_vec());
    }
    fn read_token_type(m: &Self::Model) -> String {
        m.token_type.clone()
    }
    fn token_type_column() -> Self::Column {
        provider_tokens::Column::TokenType
    }
    fn write_token_type(m: &mut Self::ActiveModel, v: &str) {
        m.token_type = Set(v.to_owned());
    }
    fn read_scopes(m: &Self::Model) -> String {
        m.scopes.clone()
    }
    fn scopes_column() -> Self::Column {
        provider_tokens::Column::Scopes
    }
    fn write_scopes(m: &mut Self::ActiveModel, v: &str) {
        m.scopes = Set(v.to_owned());
    }
    fn read_access_expires_at(m: &Self::Model) -> Option<DateTime<Utc>> {
        m.access_expires_at
    }
    fn access_expires_at_column() -> Self::Column {
        provider_tokens::Column::AccessExpiresAt
    }
    fn write_access_expires_at(m: &mut Self::ActiveModel, v: Option<DateTime<Utc>>) {
        m.access_expires_at = Set(v);
    }
    fn read_generation(m: &Self::Model) -> i64 {
        m.generation
    }
    fn generation_column() -> Self::Column {
        provider_tokens::Column::Generation
    }
    fn write_generation(m: &mut Self::ActiveModel, v: i64) {
        m.generation = Set(v);
    }
    fn read_claim_id(m: &Self::Model) -> Option<String> {
        m.claim_id.clone()
    }
    fn claim_id_column() -> Self::Column {
        provider_tokens::Column::ClaimId
    }
    fn write_claim_id(m: &mut Self::ActiveModel, v: Option<&str>) {
        m.claim_id = Set(v.map(|s| s.to_owned()));
    }
    fn read_claim_deadline(m: &Self::Model) -> Option<DateTime<Utc>> {
        m.claim_deadline
    }
    fn claim_deadline_column() -> Self::Column {
        provider_tokens::Column::ClaimDeadline
    }
    fn write_claim_deadline(m: &mut Self::ActiveModel, v: Option<DateTime<Utc>>) {
        m.claim_deadline = Set(v);
    }
    fn read_revoked_at(m: &Self::Model) -> Option<DateTime<Utc>> {
        m.revoked_at
    }
    fn revoked_at_column() -> Self::Column {
        provider_tokens::Column::RevokedAt
    }
    fn write_revoked_at(m: &mut Self::ActiveModel, v: Option<DateTime<Utc>>) {
        m.revoked_at = Set(v);
    }
    fn read_revoked_reused(m: &Self::Model) -> Option<bool> {
        m.revoked_reused
    }
    fn revoked_reused_column() -> Self::Column {
        provider_tokens::Column::RevokedReused
    }
    fn write_revoked_reused(m: &mut Self::ActiveModel, v: Option<bool>) {
        m.revoked_reused = Set(v);
    }
    fn read_created_at(m: &Self::Model) -> DateTime<Utc> {
        m.created_at
    }
    fn write_created_at(m: &mut Self::ActiveModel, v: DateTime<Utc>) {
        m.created_at = Set(v);
    }
}

impl magnetar::schema::LockoutFields for lockouts::Entity {
    fn read_lockout_id(m: &Self::Model) -> String {
        m.id.to_string()
    }
    fn write_lockout_id(m: &mut Self::ActiveModel, v: &str) {
        m.id = Set(v.parse().expect("fixture lockout ids are i64"));
    }
    fn read_user_id(m: &Self::Model) -> String {
        m.identity.clone()
    }
    fn user_id_column() -> Self::Column {
        lockouts::Column::Identity
    }
    fn write_user_id(m: &mut Self::ActiveModel, v: &str) {
        m.identity = Set(v.to_owned());
    }
    fn read_attempted_at(m: &Self::Model) -> DateTime<Utc> {
        m.attempted_at
    }
    fn attempted_at_column() -> Self::Column {
        lockouts::Column::AttemptedAt
    }
    fn write_attempted_at(m: &mut Self::ActiveModel, v: DateTime<Utc>) {
        m.attempted_at = Set(v);
    }
    fn read_locked_at(m: &Self::Model) -> Option<DateTime<Utc>> {
        m.locked_at
    }
    fn read_reason(m: &Self::Model) -> Option<String> {
        m.reason.clone()
    }
    fn write_reason(m: &mut Self::ActiveModel, v: Option<&str>) {
        m.reason = Set(v.map(ToOwned::to_owned));
    }
    fn write_locked_at(m: &mut Self::ActiveModel, v: Option<DateTime<Utc>>) {
        m.locked_at = Set(v);
    }
}

/// SQL-backed opaque-session and remember-me stores over the fixture
/// entities, so the ported flows exercise the same database the storage
/// composites mutate.
pub mod sql_stores {
    use super::*;
    use magnetar::sessions::OpaqueSessionStore;
    use magnetar::sessions::{RememberRow, RememberStore, StoredSession, WebSessionBinding};
    use magnetar::{Error, Result};
    use sea_orm::sea_query::Expr;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, TransactionTrait};

    fn hex(bytes: [u8; 32]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn unhex(value: &str) -> [u8; 32] {
        let mut out = [0_u8; 32];
        for (index, chunk) in value.as_bytes().chunks(2).take(32).enumerate() {
            let text = std::str::from_utf8(chunk).expect("hex digest");
            out[index] = u8::from_str_radix(text, 16).expect("hex digest");
        }
        out
    }

    fn db_error(error: sea_orm::DbErr) -> magnetar::Error {
        magnetar::Error::Internal {
            message: error.to_string(),
        }
    }

    fn stale_actor() -> magnetar::Error {
        magnetar::Error::NotFound {
            resource: "credential actor".to_owned(),
            identifier: "expired or revoked".to_owned(),
        }
    }

    fn stored(row: &sessions::Model) -> Result<StoredSession> {
        Ok(StoredSession {
            session_id: row.id.clone(),
            user_id: row.user_id.to_string(),
            auth_epoch: <sessions::Entity as SessionFields>::read_auth_epoch(row)?,
            token_hash: unhex(row.token_hash.as_deref().unwrap_or_default()),
            token_digest: unhex(&row.token_digest),
            expires_at: row.expires_at,
            revoked_at: row.revoked_at,
            metadata: magnetar::sessions::SessionMetadata {
                user_agent: row.user_agent.clone(),
                ip_address: row.ip_address.clone(),
            },
        })
    }

    /// Opaque-session store persisting into the fixture sessions table.
    #[derive(Clone)]
    pub struct SqlSessionStore(pub DatabaseConnection);

    #[async_trait::async_trait]
    impl OpaqueSessionStore for SqlSessionStore {
        async fn insert_session_if_epoch_current(&self, session: StoredSession) -> Result<()> {
            let user_id =
                session
                    .user_id
                    .parse::<i64>()
                    .map_err(|_| magnetar::Error::InvalidInput {
                        field: "user_id".to_owned(),
                        message: "must be a database integer".to_owned(),
                    })?;
            let auth_epoch =
                i64::try_from(session.auth_epoch).map_err(|_| magnetar::Error::InvalidInput {
                    field: "auth_epoch".to_owned(),
                    message: "exceeds the database integer range".to_owned(),
                })?;
            let transaction = self.0.begin().await.map_err(db_error)?;
            let result = async {
                users::Entity::update_many()
                    .col_expr(
                        users::Column::AuthEpoch,
                        Expr::col(users::Column::AuthEpoch),
                    )
                    .filter(users::Column::Id.eq(user_id))
                    .filter(users::Column::AuthEpoch.eq(auth_epoch))
                    .exec(&transaction)
                    .await
                    .map_err(db_error)?;
                let current = users::Entity::find_by_id(user_id)
                    .one(&transaction)
                    .await
                    .map_err(db_error)?
                    .ok_or_else(stale_actor)?;
                if current.auth_epoch != auth_epoch {
                    return Err(stale_actor());
                }
                sessions::ActiveModel {
                    id: Set(session.session_id),
                    user_id: Set(user_id),
                    auth_epoch: Set(auth_epoch),
                    token_digest: Set(hex(session.token_digest)),
                    token_hash: Set(Some(hex(session.token_hash))),
                    user_agent: Set(session.metadata.user_agent),
                    ip_address: Set(session.metadata.ip_address),
                    expires_at: Set(session.expires_at),
                    revoked_at: Set(session.revoked_at),
                }
                .insert(&transaction)
                .await
                .map_err(db_error)?;
                Ok(())
            }
            .await;
            match result {
                Ok(()) => transaction.commit().await.map_err(db_error),
                Err(error) => {
                    let _ = transaction.rollback().await;
                    Err(error)
                }
            }
        }

        async fn find_by_token_hash(&self, token_hash: [u8; 32]) -> Result<Option<StoredSession>> {
            let rows = sessions::Entity::find()
                .filter(sessions::Column::TokenHash.eq(hex(token_hash)))
                .all(&self.0)
                .await
                .map_err(db_error)?;
            rows.first().map(stored).transpose()
        }

        async fn find_by_web_binding(
            &self,
            binding: &WebSessionBinding,
        ) -> Result<Option<StoredSession>> {
            let rows = sessions::Entity::find()
                .filter(sessions::Column::Id.eq(binding.session_id.clone()))
                .filter(sessions::Column::TokenDigest.eq(hex(binding.token_digest)))
                .all(&self.0)
                .await
                .map_err(db_error)?;
            rows.first().map(stored).transpose()
        }

        async fn revoke_all_sessions(&self, user_id: &str, at: DateTime<Utc>) -> Result<u64> {
            let id: i64 = user_id.parse().expect("fixture ids are i64");
            let update = sessions::Entity::update_many()
                .col_expr(sessions::Column::RevokedAt, Expr::value(at))
                .filter(sessions::Column::UserId.eq(id))
                .filter(sessions::Column::RevokedAt.is_null())
                .exec(&self.0)
                .await
                .map_err(db_error)?;
            Ok(update.rows_affected)
        }

        async fn revoke_session(&self, session_id: &str, at: DateTime<Utc>) -> Result<bool> {
            let update = sessions::Entity::update_many()
                .col_expr(sessions::Column::RevokedAt, Expr::value(at))
                .filter(sessions::Column::Id.eq(session_id.to_owned()))
                .filter(sessions::Column::RevokedAt.is_null())
                .exec(&self.0)
                .await
                .map_err(db_error)?;
            Ok(update.rows_affected == 1)
        }

        async fn list_active_sessions(
            &self,
            user_id: &str,
            now: DateTime<Utc>,
        ) -> Result<Vec<StoredSession>> {
            let id: i64 = user_id.parse().expect("fixture ids are i64");
            let rows = sessions::Entity::find()
                .filter(sessions::Column::UserId.eq(id))
                .filter(sessions::Column::RevokedAt.is_null())
                .filter(sessions::Column::ExpiresAt.gt(now))
                .all(&self.0)
                .await
                .map_err(db_error)?;
            rows.iter().map(stored).collect()
        }
    }

    /// Remember-me store persisting into the fixture remembers table.
    #[derive(Clone)]
    pub struct SqlRememberStore(pub DatabaseConnection);

    #[async_trait::async_trait]
    impl RememberStore for SqlRememberStore {
        async fn insert_remember(&self, row: RememberRow) -> Result<()> {
            remembers::ActiveModel {
                id: Set(row.id),
                selector: Set(row.selector),
                user_id: Set(row.user_id),
                auth_epoch: Set(i64::try_from(row.auth_epoch).map_err(|_| {
                    Error::InvalidInput {
                        field: "auth_epoch".to_owned(),
                        message: "exceeds the database integer range".to_owned(),
                    }
                })?),
                verifier_hash: Set(row.verifier_hash),
                expires_at: Set(row.expires_at),
            }
            .insert(&self.0)
            .await
            .map_err(db_error)?;
            Ok(())
        }

        async fn find_for_rotation(
            &self,
            selector: &str,
            now: DateTime<Utc>,
        ) -> Result<Option<RememberRow>> {
            let rows = remembers::Entity::find()
                .filter(remembers::Column::Selector.eq(selector.to_owned()))
                .filter(remembers::Column::ExpiresAt.gt(now))
                .all(&self.0)
                .await
                .map_err(db_error)?;
            let mut rows = rows.into_iter().filter(|row| row.selector == selector);
            let row = rows.next();
            if rows.next().is_some() {
                return Err(Error::Conflict {
                    resource: "remember credential".to_owned(),
                    message: "selector matched multiple active rows".to_owned(),
                });
            }
            row.as_ref()
                .map(|row| {
                    Ok(RememberRow {
                        id: row.id.clone(),
                        selector: row.selector.clone(),
                        user_id: row.user_id.clone(),
                        auth_epoch: u64::try_from(row.auth_epoch).map_err(|_| Error::Internal {
                            message: "stored remember auth_epoch cannot be negative".to_owned(),
                        })?,
                        verifier_hash: row.verifier_hash.clone(),
                        expires_at: row.expires_at,
                    })
                })
                .transpose()
        }

        async fn consume_for_rotation(
            &self,
            id: &str,
            selector: &str,
            now: DateTime<Utc>,
        ) -> Result<bool> {
            let deleted = remembers::Entity::delete_many()
                .filter(remembers::Column::Id.eq(id.to_owned()))
                .filter(remembers::Column::Selector.eq(selector.to_owned()))
                .filter(remembers::Column::ExpiresAt.gt(now))
                .exec(&self.0)
                .await
                .map_err(db_error)?;
            Ok(deleted.rows_affected == 1)
        }

        async fn replace_for_rotation(
            &self,
            id: &str,
            selector: &str,
            now: DateTime<Utc>,
            replacement: RememberRow,
        ) -> Result<bool> {
            let auth_epoch =
                i64::try_from(replacement.auth_epoch).map_err(|_| Error::InvalidInput {
                    field: "auth_epoch".to_owned(),
                    message: "exceeds the database integer range".to_owned(),
                })?;
            let transaction = self.0.begin().await.map_err(db_error)?;
            let result = async {
                let deleted = remembers::Entity::delete_many()
                    .filter(remembers::Column::Id.eq(id.to_owned()))
                    .filter(remembers::Column::Selector.eq(selector.to_owned()))
                    .filter(remembers::Column::ExpiresAt.gt(now))
                    .exec(&transaction)
                    .await
                    .map_err(db_error)?;
                if deleted.rows_affected != 1 {
                    return Ok(false);
                }
                remembers::ActiveModel {
                    id: Set(replacement.id),
                    selector: Set(replacement.selector),
                    user_id: Set(replacement.user_id),
                    auth_epoch: Set(auth_epoch),
                    verifier_hash: Set(replacement.verifier_hash),
                    expires_at: Set(replacement.expires_at),
                }
                .insert(&transaction)
                .await
                .map_err(db_error)?;
                Ok(true)
            }
            .await;

            match result {
                Ok(true) => transaction.commit().await.map_err(db_error).map(|()| true),
                Ok(false) => {
                    transaction.rollback().await.map_err(db_error)?;
                    Ok(false)
                }
                Err(error) => {
                    let _ = transaction.rollback().await;
                    Err(error)
                }
            }
        }

        async fn revoke_remember_selector(&self, user_id: &str, selector: &str) -> Result<bool> {
            let transaction = self.0.begin().await.map_err(db_error)?;
            let result = async {
                let rows = remembers::Entity::find()
                    .filter(remembers::Column::UserId.eq(user_id.to_owned()))
                    .filter(remembers::Column::Selector.eq(selector.to_owned()))
                    .all(&transaction)
                    .await
                    .map_err(db_error)?;
                let mut rows = rows
                    .into_iter()
                    .filter(|row| row.user_id == user_id && row.selector == selector);
                let Some(row) = rows.next() else {
                    return Ok(0);
                };
                if rows.next().is_some() {
                    return Err(Error::Conflict {
                        resource: "remember credential".to_owned(),
                        message: "owner and selector matched multiple rows".to_owned(),
                    });
                }
                remembers::Entity::delete_many()
                    .filter(remembers::Column::Id.eq(row.id))
                    .exec(&transaction)
                    .await
                    .map(|deleted| deleted.rows_affected)
                    .map_err(db_error)
            }
            .await;
            match result {
                Ok(1) => {
                    transaction.commit().await.map_err(db_error)?;
                    Ok(true)
                }
                Ok(0) => {
                    transaction.rollback().await.map_err(db_error)?;
                    Ok(false)
                }
                Ok(_) => {
                    transaction.rollback().await.map_err(db_error)?;
                    Err(magnetar::Error::Conflict {
                        resource: "remember credential".to_owned(),
                        message: "owner and selector matched multiple rows".to_owned(),
                    })
                }
                Err(error) => match transaction.rollback().await {
                    Ok(()) => Err(error),
                    Err(rollback_error) => Err(magnetar::Error::Internal {
                        message: format!(
                            "revoke remember selector failed: {error}; rollback failed: {rollback_error}"
                        ),
                    }),
                },
            }
        }

        async fn revoke_all_remember(&self, user_id: &str) -> Result<u64> {
            let deleted = remembers::Entity::delete_many()
                .filter(remembers::Column::UserId.eq(user_id.to_owned()))
                .exec(&self.0)
                .await
                .map_err(db_error)?;
            Ok(deleted.rows_affected)
        }

        async fn prune_expired_remember(&self, now: DateTime<Utc>) -> Result<u64> {
            let deleted = remembers::Entity::delete_many()
                .filter(remembers::Column::ExpiresAt.lte(now))
                .exec(&self.0)
                .await
                .map_err(db_error)?;
            Ok(deleted.rows_affected)
        }
    }
}

/// SQL-backed two-factor store over the fixture enrollment table.
#[cfg(feature = "two-factor")]
pub mod sql_two_factor {
    use super::*;
    use magnetar::Result;
    use magnetar::storage::{
        AuthTransaction, CredentialActor, SeaOrmStorage, fenced_credential_write,
    };
    use magnetar::two_factor::{TwoFactorProofClaim, TwoFactorRow, TwoFactorStore};
    use sea_orm::sea_query::Expr;
    use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter};

    fn db_error(error: sea_orm::DbErr) -> magnetar::Error {
        magnetar::Error::Internal {
            message: error.to_string(),
        }
    }

    fn stale_actor() -> magnetar::Error {
        magnetar::Error::NotFound {
            resource: "credential actor".to_owned(),
            identifier: "expired or revoked".to_owned(),
        }
    }

    fn actor_epoch(actor: &CredentialActor) -> Result<i64> {
        i64::try_from(actor.issuance_epoch()).map_err(|_| magnetar::Error::InvalidInput {
            field: "auth_epoch".to_owned(),
            message: "exceeds the database integer range".to_owned(),
        })
    }

    struct EnrollmentActorSnapshot {
        auth_epoch: i64,
        session_id: Option<String>,
        expires_at: Option<DateTime<Utc>>,
    }

    fn enrollment_actor_snapshot(actor: &CredentialActor) -> Result<EnrollmentActorSnapshot> {
        Ok(EnrollmentActorSnapshot {
            auth_epoch: actor_epoch(actor)?,
            session_id: actor.opaque_session_id().map(str::to_owned),
            expires_at: actor.expires_at(),
        })
    }

    fn enrollment_model(
        user_id: &str,
        secret: &[u8],
        recovery_codes: Option<&[u8]>,
        enrollment_auth_epoch: i64,
        enrollment_session_id: Option<&str>,
        enrollment_expires_at: Option<DateTime<Utc>>,
        rotation_pending: bool,
    ) -> two_factor::ActiveModel {
        two_factor::ActiveModel {
            user_id: Set(user_id.to_owned()),
            secret: Set(secret.to_vec()),
            recovery_codes: Set(recovery_codes.map(<[u8]>::to_vec)),
            enrollment_auth_epoch: Set(enrollment_auth_epoch),
            enrollment_session_id: Set(enrollment_session_id.map(str::to_owned)),
            enrollment_expires_at: Set(enrollment_expires_at),
            rotation_pending: Set(rotation_pending),
            confirmed_at: Set(None),
            last_used_timestep: Set(None),
        }
    }

    async fn begin_enrollment_in(
        transaction: &mut AuthTransaction<'_>,
        user_id: &str,
        secret: &[u8],
        recovery_codes: Option<&[u8]>,
        enrollment_auth_epoch: i64,
        enrollment_session_id: Option<&str>,
        enrollment_expires_at: Option<DateTime<Utc>>,
    ) -> Result<bool> {
        let existing = two_factor::Entity::find_by_id(user_id.to_owned())
            .one(transaction.connection())
            .await
            .map_err(db_error)?;
        if existing
            .as_ref()
            .is_some_and(|row| row.confirmed_at.is_some() || row.rotation_pending)
        {
            return Ok(false);
        }
        let model = enrollment_model(
            user_id,
            secret,
            recovery_codes,
            enrollment_auth_epoch,
            enrollment_session_id,
            enrollment_expires_at,
            false,
        );
        if existing.is_some() {
            two_factor::Entity::update(model)
                .exec(transaction.connection())
                .await
                .map_err(db_error)?;
        } else {
            model
                .insert(transaction.connection())
                .await
                .map_err(db_error)?;
        }
        Ok(true)
    }

    async fn set_confirmed_in(
        transaction: &mut AuthTransaction<'_>,
        user_id: &str,
        enrollment_auth_epoch: i64,
        enrollment_session_id: Option<&str>,
        enrollment_expires_at: Option<DateTime<Utc>>,
        at: DateTime<Utc>,
    ) -> Result<bool> {
        let Some(enrollment) = two_factor::Entity::find_by_id(user_id.to_owned())
            .one(transaction.connection())
            .await
            .map_err(db_error)?
        else {
            return Ok(false);
        };
        if enrollment.enrollment_auth_epoch != enrollment_auth_epoch
            || enrollment.enrollment_session_id.as_deref() != enrollment_session_id
            || enrollment.enrollment_expires_at != enrollment_expires_at
        {
            return Err(stale_actor());
        }
        let update = two_factor::Entity::update_many()
            .col_expr(two_factor::Column::ConfirmedAt, Expr::value(at))
            .col_expr(two_factor::Column::RotationPending, Expr::value(false))
            .filter(two_factor::Column::UserId.eq(user_id.to_owned()))
            .exec(transaction.connection())
            .await
            .map_err(db_error)?;
        Ok(update.rows_affected == 1)
    }

    async fn claim_proof_in(
        transaction: &mut AuthTransaction<'_>,
        user_id: &str,
        claim: &TwoFactorProofClaim,
    ) -> Result<bool> {
        let update = match claim {
            TwoFactorProofClaim::Invalid => return Ok(false),
            TwoFactorProofClaim::Totp { matched_step } => two_factor::Entity::update_many()
                .col_expr(
                    two_factor::Column::LastUsedTimestep,
                    Expr::value(*matched_step),
                )
                .filter(two_factor::Column::UserId.eq(user_id.to_owned()))
                .filter(two_factor::Column::ConfirmedAt.is_not_null())
                .filter(
                    Condition::any()
                        .add(two_factor::Column::LastUsedTimestep.is_null())
                        .add(two_factor::Column::LastUsedTimestep.lt(*matched_step)),
                )
                .exec(transaction.connection())
                .await
                .map_err(db_error)?,
            TwoFactorProofClaim::Recovery {
                expected_ciphertext,
            } => two_factor::Entity::update_many()
                .col_expr(
                    two_factor::Column::RecoveryCodes,
                    Expr::value(None::<Vec<u8>>),
                )
                .filter(two_factor::Column::UserId.eq(user_id.to_owned()))
                .filter(two_factor::Column::ConfirmedAt.is_not_null())
                .filter(two_factor::Column::RecoveryCodes.eq(expected_ciphertext.clone()))
                .exec(transaction.connection())
                .await
                .map_err(db_error)?,
        };
        Ok(update.rows_affected == 1)
    }

    async fn rotate_enrollment_in(
        transaction: &mut AuthTransaction<'_>,
        user_id: &str,
        claim: &TwoFactorProofClaim,
        secret: &[u8],
        recovery_codes: Option<&[u8]>,
        snapshot: &EnrollmentActorSnapshot,
    ) -> Result<bool> {
        if !claim_proof_in(transaction, user_id, claim).await? {
            return Ok(false);
        }
        two_factor::Entity::update(enrollment_model(
            user_id,
            secret,
            recovery_codes,
            snapshot.auth_epoch,
            snapshot.session_id.as_deref(),
            snapshot.expires_at,
            true,
        ))
        .exec(transaction.connection())
        .await
        .map_err(db_error)?;
        Ok(true)
    }

    async fn regenerate_recovery_codes_in(
        transaction: &mut AuthTransaction<'_>,
        user_id: &str,
        claim: &TwoFactorProofClaim,
        next: &[u8],
    ) -> Result<bool> {
        match claim {
            TwoFactorProofClaim::Invalid => Ok(false),
            TwoFactorProofClaim::Totp { .. } => {
                if !claim_proof_in(transaction, user_id, claim).await? {
                    return Ok(false);
                }
                let update = two_factor::Entity::update_many()
                    .col_expr(
                        two_factor::Column::RecoveryCodes,
                        Expr::value(Some(next.to_vec())),
                    )
                    .filter(two_factor::Column::UserId.eq(user_id.to_owned()))
                    .exec(transaction.connection())
                    .await
                    .map_err(db_error)?;
                Ok(update.rows_affected == 1)
            }
            TwoFactorProofClaim::Recovery {
                expected_ciphertext,
            } => {
                let update = two_factor::Entity::update_many()
                    .col_expr(
                        two_factor::Column::RecoveryCodes,
                        Expr::value(Some(next.to_vec())),
                    )
                    .filter(two_factor::Column::UserId.eq(user_id.to_owned()))
                    .filter(two_factor::Column::ConfirmedAt.is_not_null())
                    .filter(two_factor::Column::RecoveryCodes.eq(expected_ciphertext.clone()))
                    .exec(transaction.connection())
                    .await
                    .map_err(db_error)?;
                Ok(update.rows_affected == 1)
            }
        }
    }

    async fn delete_enrollment_in(
        transaction: &mut AuthTransaction<'_>,
        user_id: &str,
    ) -> Result<bool> {
        let deleted = two_factor::Entity::delete_by_id(user_id.to_owned())
            .exec(transaction.connection())
            .await
            .map_err(db_error)?;
        Ok(deleted.rows_affected == 1)
    }

    /// Test-only fixture store mirroring the deployed `two_factor_credentials` row.
    #[derive(Clone)]
    pub struct SqlTwoFactorStore(pub DatabaseConnection);

    #[async_trait::async_trait]
    impl TwoFactorStore for SqlTwoFactorStore {
        async fn find_enrollment(&self, user_id: &str) -> Result<Option<TwoFactorRow>> {
            let Some(row) = two_factor::Entity::find_by_id(user_id.to_owned())
                .one(&self.0)
                .await
                .map_err(db_error)?
            else {
                return Ok(None);
            };
            let enrollment_auth_epoch = u64::try_from(row.enrollment_auth_epoch).map_err(|_| {
                magnetar::Error::Internal {
                    message: "stored two-factor enrollment_auth_epoch cannot be negative"
                        .to_owned(),
                }
            })?;
            Ok(Some(TwoFactorRow {
                user_id: row.user_id,
                secret: row.secret,
                recovery_codes: row.recovery_codes,
                enrollment_auth_epoch,
                enrollment_session_id: row.enrollment_session_id,
                enrollment_expires_at: row.enrollment_expires_at,
                rotation_pending: row.rotation_pending,
                confirmed_at: row.confirmed_at,
                last_used_timestep: row.last_used_timestep,
            }))
        }

        async fn begin_enrollment(
            &self,
            actor: &CredentialActor,
            secret: &[u8],
            recovery_codes: Option<&[u8]>,
        ) -> Result<bool> {
            let user_id = actor.user_id().to_owned();
            let secret = secret.to_vec();
            let recovery_codes = recovery_codes.map(<[u8]>::to_vec);
            let enrollment_auth_epoch = actor_epoch(actor)?;
            let enrollment_session_id = actor.opaque_session_id().map(str::to_owned);
            let enrollment_expires_at = actor.expires_at();
            let storage = SeaOrmStorage::<StorageSchema>::new(self.0.clone());
            fenced_credential_write(&storage, actor, move |transaction| {
                Box::pin(async move {
                    begin_enrollment_in(
                        transaction,
                        &user_id,
                        &secret,
                        recovery_codes.as_deref(),
                        enrollment_auth_epoch,
                        enrollment_session_id.as_deref(),
                        enrollment_expires_at,
                    )
                    .await
                })
            })
            .await
        }

        async fn set_confirmed(&self, actor: &CredentialActor, at: DateTime<Utc>) -> Result<bool> {
            let user_id = actor.user_id().to_owned();
            let enrollment_auth_epoch = actor_epoch(actor)?;
            let enrollment_session_id = actor.opaque_session_id().map(str::to_owned);
            let enrollment_expires_at = actor.expires_at();
            let storage = SeaOrmStorage::<StorageSchema>::new(self.0.clone());
            fenced_credential_write(&storage, actor, move |transaction| {
                Box::pin(async move {
                    set_confirmed_in(
                        transaction,
                        &user_id,
                        enrollment_auth_epoch,
                        enrollment_session_id.as_deref(),
                        enrollment_expires_at,
                        at,
                    )
                    .await
                })
            })
            .await
        }

        async fn claim_timestep(&self, user_id: &str, matched_step: i64) -> Result<bool> {
            let update = two_factor::Entity::update_many()
                .col_expr(
                    two_factor::Column::LastUsedTimestep,
                    Expr::value(matched_step),
                )
                .filter(two_factor::Column::UserId.eq(user_id.to_owned()))
                .filter(
                    Condition::any()
                        .add(two_factor::Column::LastUsedTimestep.is_null())
                        .add(two_factor::Column::LastUsedTimestep.lt(matched_step)),
                )
                .exec(&self.0)
                .await
                .map_err(db_error)?;
            Ok(update.rows_affected == 1)
        }

        async fn swap_recovery_codes(
            &self,
            user_id: &str,
            expected: &[u8],
            next: Option<&[u8]>,
        ) -> Result<bool> {
            let update = two_factor::Entity::update_many()
                .col_expr(
                    two_factor::Column::RecoveryCodes,
                    Expr::value(next.map(<[u8]>::to_vec)),
                )
                .filter(two_factor::Column::UserId.eq(user_id.to_owned()))
                .filter(two_factor::Column::RecoveryCodes.eq(expected.to_vec()))
                .exec(&self.0)
                .await
                .map_err(db_error)?;
            Ok(update.rows_affected == 1)
        }

        async fn rotate_enrollment(
            &self,
            actor: &CredentialActor,
            claim: TwoFactorProofClaim,
            secret: &[u8],
            recovery_codes: Option<&[u8]>,
        ) -> Result<bool> {
            let user_id = actor.user_id().to_owned();
            let secret = secret.to_vec();
            let recovery_codes = recovery_codes.map(<[u8]>::to_vec);
            let snapshot = enrollment_actor_snapshot(actor)?;
            let storage = SeaOrmStorage::<StorageSchema>::new(self.0.clone());
            fenced_credential_write(&storage, actor, move |transaction| {
                Box::pin(async move {
                    rotate_enrollment_in(
                        transaction,
                        &user_id,
                        &claim,
                        &secret,
                        recovery_codes.as_deref(),
                        &snapshot,
                    )
                    .await
                })
            })
            .await
        }

        async fn regenerate_recovery_codes(
            &self,
            actor: &CredentialActor,
            claim: TwoFactorProofClaim,
            next: &[u8],
        ) -> Result<bool> {
            let user_id = actor.user_id().to_owned();
            let next = next.to_vec();
            let storage = SeaOrmStorage::<StorageSchema>::new(self.0.clone());
            fenced_credential_write(&storage, actor, move |transaction| {
                Box::pin(async move {
                    regenerate_recovery_codes_in(transaction, &user_id, &claim, &next).await
                })
            })
            .await
        }

        async fn delete_enrollment(&self, actor: &CredentialActor) -> Result<bool> {
            let user_id = actor.user_id().to_owned();
            let storage = SeaOrmStorage::<StorageSchema>::new(self.0.clone());
            fenced_credential_write(&storage, actor, move |transaction| {
                Box::pin(async move { delete_enrollment_in(transaction, &user_id).await })
            })
            .await
        }
    }
}

pub async fn verified_opaque_session(
    database: &DatabaseConnection,
    user_id: &str,
    auth_epoch: u64,
    session_id: &str,
) -> magnetar::sessions::VerifiedSession {
    use magnetar::sessions::{
        OpaqueConfig, OpaqueSessionProvider, OpaqueSessionStore, SessionMetadata, SessionQueries,
        StoredSession,
    };

    let token = format!("fixture-token-{session_id}");
    let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    let store = std::sync::Arc::new(sql_stores::SqlSessionStore(database.clone()));
    store
        .insert_session_if_epoch_current(StoredSession {
            session_id: session_id.to_owned(),
            user_id: user_id.to_owned(),
            auth_epoch,
            token_hash: digest,
            token_digest: digest,
            expires_at: Utc::now() + chrono::Duration::hours(1),
            revoked_at: None,
            metadata: SessionMetadata::default(),
        })
        .await
        .expect("insert fixture session at the user's current epoch");
    OpaqueSessionProvider::new(store, OpaqueConfig::default())
        .verify_bearer(&token)
        .await
        .expect("official opaque provider verifies fixture session")
}

pub async fn credential_actor(
    database: &DatabaseConnection,
    user_id: &str,
    auth_epoch: u64,
    session_id: &str,
) -> magnetar::storage::CredentialActor {
    static NEXT_SESSION_SUFFIX: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let session_id = format!(
        "{session_id}-{}",
        NEXT_SESSION_SUFFIX.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    magnetar::storage::CredentialActor::from_session(
        &verified_opaque_session(database, user_id, auth_epoch, &session_id).await,
    )
}

pub async fn database() -> DatabaseConnection {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let schema = Schema::new(DbBackend::Sqlite);
    for sql in [
        schema
            .create_table_from_entity(users::Entity)
            .if_not_exists()
            .to_string(SqliteQueryBuilder),
        schema
            .create_table_from_entity(sessions::Entity)
            .if_not_exists()
            .to_string(SqliteQueryBuilder),
        schema
            .create_table_from_entity(methods::Entity)
            .if_not_exists()
            .to_string(SqliteQueryBuilder),
        schema
            .create_table_from_entity(accounts::Entity)
            .if_not_exists()
            .to_string(SqliteQueryBuilder),
        schema
            .create_table_from_entity(tokens::Entity)
            .if_not_exists()
            .to_string(SqliteQueryBuilder),
        schema
            .create_table_from_entity(ceremonies::Entity)
            .if_not_exists()
            .to_string(SqliteQueryBuilder),
        schema
            .create_table_from_entity(lockouts::Entity)
            .if_not_exists()
            .to_string(SqliteQueryBuilder),
        schema
            .create_table_from_entity(remembers::Entity)
            .if_not_exists()
            .to_string(SqliteQueryBuilder),
        schema
            .create_table_from_entity(two_factor::Entity)
            .if_not_exists()
            .to_string(SqliteQueryBuilder),
        schema
            .create_table_from_entity(provider_tokens::Entity)
            .if_not_exists()
            .to_string(SqliteQueryBuilder),
    ] {
        db.execute_raw(Statement::from_string(DbBackend::Sqlite, sql))
            .await
            .unwrap();
    }
    db.execute_raw(Statement::from_string(
        DbBackend::Sqlite,
        "CREATE UNIQUE INDEX IF NOT EXISTS storage_accounts_provider_subject \
     ON storage_accounts (provider, provider_account_id)"
            .to_owned(),
    ))
    .await
    .unwrap();
    users::ActiveModel {
        id: Set(1),
        email: Set("user@example.test".into()),
        password_hash: Set(Some("old".into())),
        remember_token: Set(Some("remember".into())),
        email_verified_at: Set(None),
        locked_at: Set(None),
        auth_epoch: Set(0),
    }
    .insert(&db)
    .await
    .unwrap();
    db
}
