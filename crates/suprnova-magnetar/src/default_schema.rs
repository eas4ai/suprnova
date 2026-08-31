//! Default application-owned SeaORM schema and SQL stores.

#![allow(dead_code)]
use crate::schema::{
    AuthSchema, BrokerSchema, CeremonyFields, EntityBinding, LinkedAccountFields, PasskeyFields,
    ProviderTokenFields, SessionEpoch, SessionFields, TokenFields, UserFields, UserOptionalFields,
};
use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::{ActiveModelBehavior, ActiveModelTrait, EntityTrait};
use sea_orm::{
    ActiveValue::Set, ConnectionTrait, Database, DatabaseConnection, DbBackend, Schema, Statement,
};
use sea_orm::{DeriveEntityModel, DeriveRelation, EnumIter};

macro_rules! entity_common {
    ($module:ident, $table:literal, { $($field:tt)* }) => {
        #[doc = concat!("SeaORM entity for the `", $table, "` table.")]
        #[allow(missing_docs)]
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

entity_common!(users, "app_users", {
    #[sea_orm(primary_key)] pub id: i64,
    pub email: String,
    pub name: Option<String>,
    pub password_hash: Option<String>,
    pub remember_token: Option<String>,
    pub email_verified_at: Option<ChronoDateTime<ChronoUtc>>,
    pub locked_at: Option<ChronoDateTime<ChronoUtc>>,
    pub auth_epoch: i64,
    pub created_at: Option<ChronoDateTime<ChronoUtc>>,
    pub updated_at: Option<ChronoDateTime<ChronoUtc>>,
});
entity_common!(sessions, "auth_sessions", {
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
entity_common!(methods, "auth_methods", {
    #[sea_orm(primary_key)] pub id: i64,
    pub user_id: i64,
    pub credential_id: Option<String>,
    pub public_key: Option<String>,
    pub created_at: Option<ChronoDateTime<ChronoUtc>>,
});
entity_common!(accounts, "auth_linked_accounts", {
    #[sea_orm(primary_key)] pub id: i64,
    pub user_id: i64,
    pub provider: String,
    pub provider_account_id: String,
    pub created_at: Option<ChronoDateTime<ChronoUtc>>,
    pub updated_at: Option<ChronoDateTime<ChronoUtc>>,
});
entity_common!(tokens, "auth_tokens", {
    #[sea_orm(primary_key)] pub id: i64,
    pub user_id: Option<i64>,
    pub purpose: String,
    pub digest: String,
    pub expires_at: ChronoDateTime<ChronoUtc>,
    pub used_at: Option<ChronoDateTime<ChronoUtc>>,
    pub created_at: Option<ChronoDateTime<ChronoUtc>>,
    pub updated_at: Option<ChronoDateTime<ChronoUtc>>,
});
entity_common!(ceremonies, "auth_ceremonies", {
    #[sea_orm(primary_key)] pub id: i64,
    pub kind: String,
    pub selector: String,
    pub payload: Vec<u8>,
    pub state: String,
    pub expires_at: ChronoDateTime<ChronoUtc>,
    pub used_at: Option<ChronoDateTime<ChronoUtc>>,
});
entity_common!(lockouts, "auth_lockouts", {
    #[sea_orm(primary_key)] pub id: i64,
    pub identity: String,
    pub attempted_at: ChronoDateTime<ChronoUtc>,
    pub ip_address: Option<String>,
    pub migration_source_id: Option<String>,
    pub locked_at: Option<ChronoDateTime<ChronoUtc>>,
    pub reason: Option<String>,
});
entity_common!(two_factor, "auth_two_factor", {
    #[sea_orm(primary_key, auto_increment = false)] pub user_id: String,
    pub secret: Vec<u8>,
    pub recovery_codes: Option<Vec<u8>>,
    pub enrollment_auth_epoch: i64,
    pub enrollment_session_id: Option<String>,
    pub enrollment_expires_at: Option<ChronoDateTime<ChronoUtc>>,
    pub rotation_pending: bool,
    pub confirmed_at: Option<ChronoDateTime<ChronoUtc>>,
    pub last_used_timestep: Option<i64>,
    pub created_at: Option<ChronoDateTime<ChronoUtc>>,
    pub updated_at: Option<ChronoDateTime<ChronoUtc>>,
});
entity_common!(remembers, "auth_remember_tokens", {
    #[sea_orm(primary_key, auto_increment = false)] pub id: String,
    pub selector: String,
    pub user_id: String,
    pub auth_epoch: i64,
    pub verifier_hash: String,
    pub expires_at: ChronoDateTime<ChronoUtc>,
});
entity_common!(provider_tokens, "auth_provider_tokens", {
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
entity_common!(lifecycle_deliveries, "auth_lifecycle_deliveries", {
    #[sea_orm(primary_key, auto_increment = false)] pub mutation_id: String,
    pub lease_id: Option<String>,
    pub lease_until: Option<ChronoDateTime<ChronoUtc>>,
    pub delivered_at: Option<ChronoDateTime<ChronoUtc>>,
});
entity_common!(migration_runs, "auth_migration_runs", {
    #[sea_orm(primary_key, auto_increment = false)] pub plan_id: String,
    pub imports_committed: bool,
    pub completed_at: Option<ChronoDateTime<ChronoUtc>>,
});
entity_common!(migration_identities, "auth_migration_identities", {
    #[sea_orm(primary_key, auto_increment = false)] pub id: String,
    pub plan_id: String,
    pub source_user_id: String,
    pub app_user_id: i64,
});
entity_common!(migration_state, "magnetar_migration_state", {
    #[sea_orm(primary_key, auto_increment = false)] pub key: String,
    pub value: String,
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
    lifecycle_deliveries,
    migration_runs,
    migration_identities,
    migration_state,
    provider_tokens
);

/// Default auth schema using `app_users` and framework-owned auth tables.
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultAuthSchema;
impl AuthSchema for DefaultAuthSchema {
    type User = users::Entity;
    type Session = sessions::Entity;
    type LinkedAccount = accounts::Entity;
    type Passkey = methods::Entity;
    type Token = tokens::Entity;
    type Ceremony = ceremonies::Entity;
    type Lockout = lockouts::Entity;
    type TokenRecord = tokens::Entity;
}

impl BrokerSchema for DefaultAuthSchema {
    type ProviderToken = provider_tokens::Entity;
}

impl UserFields for users::Entity {
    fn read_user_id(m: &Self::Model) -> String {
        m.id.to_string()
    }
    fn user_id_column() -> Self::Column {
        users::Column::Id
    }
    fn user_id_value(value: &str) -> sea_orm::Value {
        value
            .parse::<i64>()
            .expect("DefaultAuthSchema user IDs are i64")
            .into()
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
    fn read_name(m: &Self::Model) -> Option<String> {
        m.name.clone()
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
    fn auth_epoch_value(value: u64) -> crate::Result<sea_orm::Value> {
        let value = i64::try_from(value).map_err(|_| crate::Error::InvalidInput {
            field: "auth_epoch".to_owned(),
            message: "exceeds the database integer range".to_owned(),
        })?;
        Ok(value.into())
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
    fn user_id_value(value: &str) -> sea_orm::Value {
        value
            .parse::<i64>()
            .expect("DefaultAuthSchema user IDs are i64")
            .into()
    }
    fn read_auth_epoch(m: &Self::Model) -> crate::Result<u64> {
        u64::try_from(m.auth_epoch).map_err(|_| crate::Error::Internal {
            message: "stored session auth_epoch cannot be negative".to_owned(),
        })
    }
    fn auth_epoch_column() -> Self::Column {
        sessions::Column::AuthEpoch
    }
    fn auth_epoch_value(value: u64) -> crate::Result<sea_orm::Value> {
        let value = i64::try_from(value).map_err(|_| crate::Error::InvalidInput {
            field: "auth_epoch".to_owned(),
            message: "exceeds the database integer range".to_owned(),
        })?;
        Ok(value.into())
    }
    fn write_auth_epoch(m: &mut Self::ActiveModel, v: u64) -> crate::Result<()> {
        let value = i64::try_from(v).map_err(|_| crate::Error::InvalidInput {
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
    fn user_id_value(value: &str) -> sea_orm::Value {
        value
            .parse::<i64>()
            .expect("DefaultAuthSchema user IDs are i64")
            .into()
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
    fn user_id_value(value: &str) -> sea_orm::Value {
        value
            .parse::<i64>()
            .expect("DefaultAuthSchema user IDs are i64")
            .into()
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
    fn user_id_value(value: &str) -> sea_orm::Value {
        value
            .parse::<i64>()
            .expect("DefaultAuthSchema user IDs are i64")
            .into()
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

impl crate::schema::LockoutFields for lockouts::Entity {
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
    use crate::Result;
    use crate::sessions::OpaqueSessionStore;
    use crate::sessions::{RememberRow, RememberStore, StoredSession, WebSessionBinding};
    use crate::storage::credential_writes::fenced_credential_write;
    use crate::storage::{AuthTransaction, CredentialActor, SeaOrmStorage};
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

    fn db_error(error: sea_orm::DbErr) -> crate::Error {
        crate::Error::Internal {
            message: error.to_string(),
        }
    }

    fn stored(row: &sessions::Model) -> Result<StoredSession> {
        let legacy = row.auth_epoch < 0;
        Ok(StoredSession {
            session_id: row.id.clone(),
            user_id: row.user_id.to_string(),
            auth_epoch: if legacy {
                0
            } else {
                <sessions::Entity as SessionFields>::read_auth_epoch(row)?
            },
            token_hash: unhex(row.token_hash.as_deref().unwrap_or_default()),
            token_digest: unhex(&row.token_digest),
            expires_at: if legacy {
                DateTime::<Utc>::UNIX_EPOCH
            } else {
                row.expires_at
            },
            revoked_at: if legacy {
                Some(DateTime::<Utc>::UNIX_EPOCH)
            } else {
                row.revoked_at
            },
            metadata: crate::sessions::SessionMetadata {
                user_agent: row.user_agent.clone(),
                ip_address: row.ip_address.clone(),
            },
        })
    }

    async fn insert_session_in(
        transaction: &mut AuthTransaction<'_>,
        session: StoredSession,
    ) -> Result<()> {
        let auth_epoch =
            i64::try_from(session.auth_epoch).map_err(|_| crate::Error::InvalidInput {
                field: "auth_epoch".to_owned(),
                message: "exceeds the database integer range".to_owned(),
            })?;
        sessions::ActiveModel {
            id: Set(session.session_id),
            user_id: Set(session.user_id.parse().expect("fixture ids are i64")),
            auth_epoch: Set(auth_epoch),
            token_digest: Set(hex(session.token_digest)),
            token_hash: Set(Some(hex(session.token_hash))),
            user_agent: Set(session.metadata.user_agent),
            ip_address: Set(session.metadata.ip_address),
            expires_at: Set(session.expires_at),
            revoked_at: Set(session.revoked_at),
        }
        .insert(transaction.connection())
        .await
        .map_err(db_error)?;
        Ok(())
    }

    /// Opaque-session store persisting into the fixture sessions table.
    #[derive(Clone)]
    pub struct SqlSessionStore(pub DatabaseConnection);

    #[async_trait::async_trait]
    impl OpaqueSessionStore for SqlSessionStore {
        async fn insert_session_if_epoch_current(&self, session: StoredSession) -> Result<()> {
            let actor = CredentialActor::verified_primary(&session.user_id, session.auth_epoch);
            let storage = SeaOrmStorage::<DefaultAuthSchema>::new(self.0.clone());
            fenced_credential_write(&storage, &actor, move |transaction| {
                Box::pin(async move { insert_session_in(transaction, session).await })
            })
            .await
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
            let auth_epoch =
                i64::try_from(row.auth_epoch).map_err(|_| crate::Error::InvalidInput {
                    field: "auth_epoch".to_owned(),
                    message: "exceeds the database integer range".to_owned(),
                })?;
            remembers::ActiveModel {
                id: Set(row.id),
                selector: Set(row.selector),
                user_id: Set(row.user_id),
                auth_epoch: Set(auth_epoch),
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
            rows.first()
                .map(|row| {
                    Ok(RememberRow {
                        id: row.id.clone(),
                        selector: row.selector.clone(),
                        user_id: row.user_id.clone(),
                        auth_epoch: u64::try_from(row.auth_epoch).map_err(|_| {
                            crate::Error::NotFound {
                                resource: "remember token".to_owned(),
                                identifier: "expired or revoked".to_owned(),
                            }
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
                i64::try_from(replacement.auth_epoch).map_err(|_| crate::Error::InvalidInput {
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
    use crate::Result;
    use crate::storage::credential_writes::fenced_credential_write;
    use crate::storage::{AuthTransaction, CredentialActor, SeaOrmStorage};
    use crate::two_factor::{TwoFactorProofClaim, TwoFactorRow, TwoFactorStore};
    use sea_orm::sea_query::Expr;
    use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter};

    fn db_error(error: sea_orm::DbErr) -> crate::Error {
        crate::Error::Internal {
            message: error.to_string(),
        }
    }

    fn stale_actor() -> crate::Error {
        crate::Error::NotFound {
            resource: "credential actor".to_owned(),
            identifier: "expired or revoked".to_owned(),
        }
    }

    fn actor_epoch(actor: &CredentialActor) -> Result<i64> {
        i64::try_from(actor.issuance_epoch()).map_err(|_| crate::Error::InvalidInput {
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
            ..Default::default()
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

    /// Fixture store mirroring the deployed `two_factor_credentials` row.
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
            let enrollment_auth_epoch =
                u64::try_from(row.enrollment_auth_epoch).map_err(|_| crate::Error::Internal {
                    message: "stored two-factor enrollment_auth_epoch cannot be negative"
                        .to_owned(),
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
            let storage = SeaOrmStorage::<DefaultAuthSchema>::new(self.0.clone());
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
            let storage = SeaOrmStorage::<DefaultAuthSchema>::new(self.0.clone());
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
            let storage = SeaOrmStorage::<DefaultAuthSchema>::new(self.0.clone());
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
            let storage = SeaOrmStorage::<DefaultAuthSchema>::new(self.0.clone());
            fenced_credential_write(&storage, actor, move |transaction| {
                Box::pin(async move {
                    regenerate_recovery_codes_in(transaction, &user_id, &claim, &next).await
                })
            })
            .await
        }

        async fn delete_enrollment(&self, actor: &CredentialActor) -> Result<bool> {
            let user_id = actor.user_id().to_owned();
            let storage = SeaOrmStorage::<DefaultAuthSchema>::new(self.0.clone());
            fenced_credential_write(&storage, actor, move |transaction| {
                Box::pin(async move { delete_enrollment_in(transaction, &user_id).await })
            })
            .await
        }
    }
}

/// Create every default auth table and required uniqueness index.
///
/// # Errors
///
/// Returns an error when the database rejects a schema operation.
pub async fn migrate(db: &DatabaseConnection) -> crate::Result<()> {
    ensure_supported_backend(db.get_database_backend())?;
    let users_source_present = table_exists(db, "users").await?;
    let app_users_present = table_exists(db, "app_users").await?;
    let app_users_is_default = app_users_present
        && column_exists(db, "app_users", "name").await?
        && column_exists(db, "app_users", "password_hash").await?
        && column_exists(db, "app_users", "auth_epoch").await?;
    let legacy_source_present =
        users_source_present || (app_users_present && !app_users_is_default);
    let completed_marker = migration_state_value(db, "schema_version").await?;
    let pending_marker = migration_state_value(db, "source_pending").await?;
    let defer_completion_marker =
        completed_marker.is_none() && (legacy_source_present || pending_marker.is_some());
    let backend = db.get_database_backend();
    let schema = Schema::new(backend);
    for statement in [
        schema
            .create_table_from_entity(users::Entity)
            .if_not_exists()
            .to_owned(),
        schema
            .create_table_from_entity(sessions::Entity)
            .if_not_exists()
            .to_owned(),
        schema
            .create_table_from_entity(methods::Entity)
            .if_not_exists()
            .to_owned(),
        schema
            .create_table_from_entity(accounts::Entity)
            .if_not_exists()
            .to_owned(),
        schema
            .create_table_from_entity(tokens::Entity)
            .if_not_exists()
            .to_owned(),
        schema
            .create_table_from_entity(ceremonies::Entity)
            .if_not_exists()
            .to_owned(),
        schema
            .create_table_from_entity(lockouts::Entity)
            .if_not_exists()
            .to_owned(),
        schema
            .create_table_from_entity(remembers::Entity)
            .if_not_exists()
            .to_owned(),
        schema
            .create_table_from_entity(two_factor::Entity)
            .if_not_exists()
            .to_owned(),
        schema
            .create_table_from_entity(lifecycle_deliveries::Entity)
            .if_not_exists()
            .to_owned(),
        schema
            .create_table_from_entity(migration_runs::Entity)
            .if_not_exists()
            .to_owned(),
        schema
            .create_table_from_entity(migration_identities::Entity)
            .if_not_exists()
            .to_owned(),
        schema
            .create_table_from_entity(migration_state::Entity)
            .if_not_exists()
            .to_owned(),
        schema
            .create_table_from_entity(provider_tokens::Entity)
            .if_not_exists()
            .to_owned(),
    ] {
        db.execute(&statement)
            .await
            .map_err(|error| crate::Error::Internal {
                message: format!("create default auth table: {error}"),
            })?;
    }
    add_session_auth_epoch_column(db).await?;
    add_remember_auth_epoch_column(db).await?;
    add_two_factor_enrollment_snapshot_columns(db).await?;
    if !column_exists(db, "auth_lockouts", "ip_address").await? {
        let mut ip_address = sea_orm::sea_query::ColumnDef::new(lockouts::Column::IpAddress);
        ip_address.string().null();
        let alter = sea_orm::sea_query::Table::alter()
            .table(lockouts::Entity)
            .add_column(ip_address)
            .to_owned();
        db.execute(&alter)
            .await
            .map_err(|error| crate::Error::Internal {
                message: format!("add lockout IP-address column: {error}"),
            })?;
    }
    if !column_exists(db, "auth_lockouts", "migration_source_id").await? {
        let mut source_id = sea_orm::sea_query::ColumnDef::new(lockouts::Column::MigrationSourceId);
        source_id.string().null();
        let alter = sea_orm::sea_query::Table::alter()
            .table(lockouts::Entity)
            .add_column(source_id)
            .to_owned();
        db.execute(&alter)
            .await
            .map_err(|error| crate::Error::Internal {
                message: format!("add lockout migration source column: {error}"),
            })?;
    }
    add_optional_string_column(db, "app_users", "name", users::Column::Name).await?;
    add_optional_string_column(
        db,
        "app_users",
        "password_hash",
        users::Column::PasswordHash,
    )
    .await?;
    add_optional_string_column(
        db,
        "app_users",
        "remember_token",
        users::Column::RememberToken,
    )
    .await?;
    add_optional_timestamp_column(
        db,
        "app_users",
        "email_verified_at",
        users::Column::EmailVerifiedAt,
    )
    .await?;
    add_optional_timestamp_column(db, "app_users", "locked_at", users::Column::LockedAt).await?;
    add_optional_timestamp_column(db, "app_users", "created_at", users::Column::CreatedAt).await?;
    add_optional_timestamp_column(db, "app_users", "updated_at", users::Column::UpdatedAt).await?;
    add_auth_epoch_column(db).await?;
    const INDEX: &str = "auth_linked_accounts_provider_subject";
    add_optional_timestamp_column(
        db,
        "auth_linked_accounts",
        "created_at",
        accounts::Column::CreatedAt,
    )
    .await?;
    add_optional_timestamp_column(
        db,
        "auth_linked_accounts",
        "updated_at",
        accounts::Column::UpdatedAt,
    )
    .await?;
    add_optional_timestamp_column(db, "auth_tokens", "created_at", tokens::Column::CreatedAt)
        .await?;
    add_optional_timestamp_column(db, "auth_tokens", "updated_at", tokens::Column::UpdatedAt)
        .await?;
    add_optional_timestamp_column(
        db,
        "auth_two_factor",
        "created_at",
        two_factor::Column::CreatedAt,
    )
    .await?;
    add_optional_timestamp_column(
        db,
        "auth_two_factor",
        "updated_at",
        two_factor::Column::UpdatedAt,
    )
    .await?;
    if !index_exists(db, "auth_linked_accounts", INDEX).await? {
        let index = sea_orm::sea_query::Index::create()
            .name(INDEX)
            .table(accounts::Entity)
            .col(accounts::Column::Provider)
            .col(accounts::Column::ProviderAccountId)
            .unique()
            .to_owned();
        db.execute(&index)
            .await
            .map_err(|error| crate::Error::Internal {
                message: format!("create linked-account uniqueness index: {error}"),
            })?;
    }
    const LOCKOUT_SOURCE_INDEX: &str = "auth_lockouts_migration_source";
    if !index_exists(db, "auth_lockouts", LOCKOUT_SOURCE_INDEX).await? {
        let index = sea_orm::sea_query::Index::create()
            .name(LOCKOUT_SOURCE_INDEX)
            .table(lockouts::Entity)
            .col(lockouts::Column::MigrationSourceId)
            .unique()
            .to_owned();
        db.execute(&index)
            .await
            .map_err(|error| crate::Error::Internal {
                message: format!("create lockout migration-source index: {error}"),
            })?;
    }
    if completed_marker.as_deref() != Some("1") {
        if defer_completion_marker {
            ensure_migration_state(db, "source_pending", "1").await?;
        } else {
            ensure_migration_state(db, "schema_version", "1").await?;
        }
    }
    Ok(())
}

async fn table_exists(db: &DatabaseConnection, table: &str) -> crate::Result<bool> {
    let backend = db.get_database_backend();
    let (query, values) = match backend {
        DbBackend::Sqlite => (
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ? LIMIT 1",
            vec![table.to_owned().into()],
        ),
        DbBackend::Postgres => (
            "SELECT 1 FROM information_schema.tables WHERE table_schema = current_schema() AND table_name = $1 AND table_type = 'BASE TABLE' LIMIT 1",
            vec![table.to_owned().into()],
        ),
        DbBackend::MySql => (
            "SELECT 1 FROM information_schema.tables WHERE table_schema = DATABASE() AND table_name = ? AND table_type = 'BASE TABLE' LIMIT 1",
            vec![table.to_owned().into()],
        ),
        _ => return Err(unsupported_backend(backend)),
    };
    db.query_one_raw(Statement::from_sql_and_values(backend, query, values))
        .await
        .map(|row| row.is_some())
        .map_err(|error| crate::Error::Internal {
            message: format!("inspect default auth table {table}: {error}"),
        })
}
async fn migration_state_value(
    db: &DatabaseConnection,
    key: &str,
) -> crate::Result<Option<String>> {
    if !table_exists(db, "magnetar_migration_state").await? {
        return Ok(None);
    }
    if !column_exists(db, "magnetar_migration_state", "key").await?
        || !column_exists(db, "magnetar_migration_state", "value").await?
    {
        return Err(crate::Error::Conflict {
            resource: "Magnetar migration marker".to_owned(),
            message: "marker table is missing key or value columns".to_owned(),
        });
    }
    migration_state::Entity::find_by_id(key)
        .one(db)
        .await
        .map(|row| row.map(|row| row.value))
        .map_err(|error| crate::Error::Internal {
            message: format!("read Magnetar migration marker: {error}"),
        })
}

async fn ensure_migration_state(
    db: &DatabaseConnection,
    key: &str,
    value: &str,
) -> crate::Result<()> {
    match migration_state::Entity::find_by_id(key)
        .one(db)
        .await
        .map_err(|error| crate::Error::Internal {
            message: format!("read Magnetar migration marker: {error}"),
        })? {
        Some(existing) if existing.value == value => Ok(()),
        Some(_) => Err(crate::Error::Conflict {
            resource: "Magnetar migration marker".to_owned(),
            message: format!("marker {key} has an unsupported value"),
        }),
        None => migration_state::ActiveModel {
            key: Set(key.to_owned()),
            value: Set(value.to_owned()),
        }
        .insert(db)
        .await
        .map(|_| ())
        .map_err(|error| crate::Error::Internal {
            message: format!("write Magnetar migration marker: {error}"),
        }),
    }
}

async fn column_exists(db: &DatabaseConnection, table: &str, column: &str) -> crate::Result<bool> {
    let backend = db.get_database_backend();
    let (query, values) = match backend {
        DbBackend::Sqlite => (
            "SELECT 1 FROM pragma_table_info(?) WHERE name = ? LIMIT 1",
            vec![table.to_owned().into(), column.to_owned().into()],
        ),
        DbBackend::Postgres => (
            "SELECT 1 FROM information_schema.columns WHERE table_schema = current_schema() AND table_name = $1 AND column_name = $2 LIMIT 1",
            vec![table.to_owned().into(), column.to_owned().into()],
        ),
        DbBackend::MySql => (
            "SELECT 1 FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name = ? AND column_name = ? LIMIT 1",
            vec![table.to_owned().into(), column.to_owned().into()],
        ),
        _ => return Err(unsupported_backend(backend)),
    };
    db.query_one_raw(Statement::from_sql_and_values(backend, query, values))
        .await
        .map(|row| row.is_some())
        .map_err(|error| crate::Error::Internal {
            message: format!("inspect default auth column: {error}"),
        })
}

async fn add_optional_timestamp_column<I>(
    db: &DatabaseConnection,
    table: &str,
    column_name: &str,
    column: I,
) -> crate::Result<()>
where
    I: sea_orm::sea_query::IntoIden,
{
    if column_exists(db, table, column_name).await? {
        return Ok(());
    }
    let mut definition = sea_orm::sea_query::ColumnDef::new(column);
    definition.timestamp_with_time_zone().null();
    let alter = sea_orm::sea_query::Table::alter()
        .table(sea_orm::sea_query::Alias::new(table))
        .add_column(definition)
        .to_owned();
    db.execute(&alter)
        .await
        .map_err(|error| crate::Error::Internal {
            message: format!("add {table}.{column_name}: {error}"),
        })?;
    Ok(())
}

async fn add_optional_string_column<I>(
    db: &DatabaseConnection,
    table: &str,
    column_name: &str,
    column: I,
) -> crate::Result<()>
where
    I: sea_orm::sea_query::IntoIden,
{
    if column_exists(db, table, column_name).await? {
        return Ok(());
    }
    let mut definition = sea_orm::sea_query::ColumnDef::new(column);
    definition.string().null();
    let alter = sea_orm::sea_query::Table::alter()
        .table(sea_orm::sea_query::Alias::new(table))
        .add_column(definition)
        .to_owned();
    db.execute(&alter)
        .await
        .map_err(|error| crate::Error::Internal {
            message: format!("add {table}.{column_name}: {error}"),
        })?;
    Ok(())
}

async fn add_two_factor_enrollment_snapshot_columns(db: &DatabaseConnection) -> crate::Result<()> {
    if !column_exists(db, "auth_two_factor", "enrollment_auth_epoch").await? {
        let mut definition =
            sea_orm::sea_query::ColumnDef::new(two_factor::Column::EnrollmentAuthEpoch);
        definition.big_integer().not_null().default(0);
        let alter = sea_orm::sea_query::Table::alter()
            .table(two_factor::Entity)
            .add_column(definition)
            .to_owned();
        db.execute(&alter)
            .await
            .map_err(|error| crate::Error::Internal {
                message: format!("add auth_two_factor.enrollment_auth_epoch: {error}"),
            })?;
    }
    add_optional_string_column(
        db,
        "auth_two_factor",
        "enrollment_session_id",
        two_factor::Column::EnrollmentSessionId,
    )
    .await?;
    add_optional_timestamp_column(
        db,
        "auth_two_factor",
        "enrollment_expires_at",
        two_factor::Column::EnrollmentExpiresAt,
    )
    .await?;
    if !column_exists(db, "auth_two_factor", "rotation_pending").await? {
        let mut definition =
            sea_orm::sea_query::ColumnDef::new(two_factor::Column::RotationPending);
        definition.boolean().not_null().default(false);
        let alter = sea_orm::sea_query::Table::alter()
            .table(two_factor::Entity)
            .add_column(definition)
            .to_owned();
        db.execute(&alter)
            .await
            .map_err(|error| crate::Error::Internal {
                message: format!("add auth_two_factor.rotation_pending: {error}"),
            })?;
    }
    Ok(())
}

async fn add_auth_epoch_column(db: &DatabaseConnection) -> crate::Result<()> {
    if column_exists(db, "app_users", "auth_epoch").await? {
        return Ok(());
    }
    let mut definition = sea_orm::sea_query::ColumnDef::new(users::Column::AuthEpoch);
    definition.big_integer().not_null().default(0);
    let alter = sea_orm::sea_query::Table::alter()
        .table(users::Entity)
        .add_column(definition)
        .to_owned();
    db.execute(&alter)
        .await
        .map_err(|error| crate::Error::Internal {
            message: format!("add app_users.auth_epoch: {error}"),
        })?;
    Ok(())
}

async fn add_session_auth_epoch_column(db: &DatabaseConnection) -> crate::Result<()> {
    if column_exists(db, "auth_sessions", "auth_epoch").await? {
        return Ok(());
    }
    let mut definition = sea_orm::sea_query::ColumnDef::new(sessions::Column::AuthEpoch);
    definition.big_integer().not_null().default(-1);
    let alter = sea_orm::sea_query::Table::alter()
        .table(sessions::Entity)
        .add_column(definition)
        .to_owned();
    db.execute(&alter)
        .await
        .map_err(|error| crate::Error::Internal {
            message: format!("add auth_sessions.auth_epoch: {error}"),
        })?;
    Ok(())
}

async fn add_remember_auth_epoch_column(db: &DatabaseConnection) -> crate::Result<()> {
    if column_exists(db, "auth_remember_tokens", "auth_epoch").await? {
        return Ok(());
    }
    let mut definition = sea_orm::sea_query::ColumnDef::new(remembers::Column::AuthEpoch);
    definition.big_integer().not_null().default(-1);
    let alter = sea_orm::sea_query::Table::alter()
        .table(remembers::Entity)
        .add_column(definition)
        .to_owned();
    db.execute(&alter)
        .await
        .map_err(|error| crate::Error::Internal {
            message: format!("add auth_remember_tokens.auth_epoch: {error}"),
        })?;
    Ok(())
}

async fn index_exists(db: &DatabaseConnection, table: &str, name: &str) -> crate::Result<bool> {
    let backend = db.get_database_backend();
    let (query, values) = match backend {
        DbBackend::Sqlite => (
            "SELECT 1 FROM sqlite_master WHERE type = 'index' AND tbl_name = ? AND name = ? LIMIT 1",
            vec![table.to_owned().into(), name.to_owned().into()],
        ),
        DbBackend::Postgres => (
            "SELECT 1 FROM pg_indexes WHERE schemaname = current_schema() AND tablename = $1 AND indexname = $2 LIMIT 1",
            vec![table.to_owned().into(), name.to_owned().into()],
        ),
        DbBackend::MySql => (
            "SELECT 1 FROM information_schema.statistics WHERE table_schema = DATABASE() AND table_name = ? AND index_name = ? LIMIT 1",
            vec![table.to_owned().into(), name.to_owned().into()],
        ),
        _ => return Err(unsupported_backend(backend)),
    };
    db.query_one_raw(Statement::from_sql_and_values(backend, query, values))
        .await
        .map(|row| row.is_some())
        .map_err(|error| crate::Error::Internal {
            message: format!("inspect {table} index {name}: {error}"),
        })
}

fn ensure_supported_backend(backend: DbBackend) -> crate::Result<()> {
    match backend {
        DbBackend::Sqlite | DbBackend::Postgres | DbBackend::MySql => Ok(()),
        _ => Err(unsupported_backend(backend)),
    }
}

fn unsupported_backend(backend: DbBackend) -> crate::Error {
    crate::Error::DependencyUnavailable {
        dependency: "database backend".to_owned(),
        message: format!("unsupported SeaORM database backend: {backend:?}"),
    }
}

/// Create and seed an in-memory SQLite database with every default auth table.
pub async fn database() -> DatabaseConnection {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    migrate(&db).await.unwrap();
    users::ActiveModel {
        id: Set(1),
        email: Set("user@example.test".into()),
        password_hash: Set(Some("old".into())),
        remember_token: Set(Some("remember".into())),
        email_verified_at: Set(None),
        locked_at: Set(None),
        auth_epoch: Set(0),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();
    db
}
