//! Default application-owned SeaORM schema and SQL stores.

#![allow(dead_code)]
use crate::schema::{
    AuthSchema, BrokerSchema, CeremonyFields, EntityBinding, LinkedAccountFields, PasskeyFields,
    ProviderTokenFields, SessionEpoch, SessionFields, TokenFields, UserFields, UserOptionalFields,
};
use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::{ActiveModelBehavior, ActiveModelTrait, EntityTrait, IdenStatic};
use sea_orm::{
    ActiveValue::Set, ConnectionTrait, Database, DatabaseConnection, DbBackend, Iterable, Schema,
    Statement,
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
    #[sea_orm(column_type = "Text")] pub user_agent: Option<String>,
    pub ip_address: Option<String>,
    pub expires_at: ChronoDateTime<ChronoUtc>,
    pub revoked_at: Option<ChronoDateTime<ChronoUtc>>,
});
entity_common!(methods, "auth_methods", {
    #[sea_orm(primary_key)] pub id: i64,
    pub user_id: i64,
    #[sea_orm(column_type = "Text")] pub credential_id: Option<String>,
    #[sea_orm(column_type = "Text")] pub public_key: Option<String>,
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
    #[sea_orm(column_type = "Blob")] pub payload: Vec<u8>,
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
entity_common!(second_factor_lockouts, "auth_second_factor_lockouts", {
    #[sea_orm(primary_key)] pub id: i64,
    pub identity: String,
    pub attempted_at: ChronoDateTime<ChronoUtc>,
    pub locked_at: Option<ChronoDateTime<ChronoUtc>>,
    pub reason: Option<String>,
});
entity_common!(two_factor, "auth_two_factor", {
    #[sea_orm(primary_key, auto_increment = false)] pub user_id: String,
    #[sea_orm(column_type = "Blob")] pub secret: Vec<u8>,
    #[sea_orm(column_type = "Blob")] pub recovery_codes: Option<Vec<u8>>,
    pub enrollment_auth_epoch: i64,
    pub enrollment_session_id: Option<String>,
    pub enrollment_expires_at: Option<ChronoDateTime<ChronoUtc>>,
    pub rotation_pending: bool,
    #[sea_orm(column_type = "Blob")] pub pending_secret: Option<Vec<u8>>,
    #[sea_orm(column_type = "Blob")] pub pending_recovery_codes: Option<Vec<u8>>,
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
    #[sea_orm(column_type = "Blob")] pub access_ciphertext: Vec<u8>,
    #[sea_orm(column_type = "Blob")] pub refresh_ciphertext: Option<Vec<u8>>,
    #[sea_orm(column_type = "Blob")] pub raw_payload_ciphertext: Vec<u8>,
    pub token_type: String,
    #[sea_orm(column_type = "Text")] pub scopes: String,
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
    second_factor_lockouts,
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

/// The default schema with second-factor attempts counted in a table of
/// their own, `auth_second_factor_lockouts`.
///
/// Password sign-in counts its failures against whatever address it is
/// given, and an address is any string a visitor registers. Second-factor
/// attempts counted in the same table, under a key derived from the user
/// id, were reachable by password identities: registering the key as an
/// address and signing in to it cleared the second factor's failures, and
/// failing a sign-in as it locked the second factor. A separate table gives
/// the second factor a namespace no password identity can reach. Build the
/// second factor's [`crate::password::LockoutService`] over
/// `SeaOrmStorage<DefaultSecondFactorSchema>`; every other role is the
/// default schema's own.
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultSecondFactorSchema;
impl AuthSchema for DefaultSecondFactorSchema {
    type User = users::Entity;
    type Session = sessions::Entity;
    type LinkedAccount = accounts::Entity;
    type Passkey = methods::Entity;
    type Token = tokens::Entity;
    type Ceremony = ceremonies::Entity;
    type Lockout = second_factor_lockouts::Entity;
    type TokenRecord = tokens::Entity;
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
    fn account_id_value(value: &str) -> sea_orm::Value {
        // Ids come from the store, so they parse. Text that does not names
        // no row; it is bound as text and the database refuses it.
        value
            .parse::<i64>()
            .map_or_else(|_| value.to_owned().into(), Into::into)
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
    fn passkey_id_value(value: &str) -> sea_orm::Value {
        // Ids come from the store, so they parse. Text that does not names
        // no row; it is bound as text and the database refuses it.
        value
            .parse::<i64>()
            .map_or_else(|_| value.to_owned().into(), Into::into)
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
    fn ceremony_id_value(value: &str) -> sea_orm::Value {
        // Ids come from `read_ceremony_id`, so they parse. Text that does
        // not names no row; it is bound as text and the database refuses it.
        value
            .parse::<i64>()
            .map_or_else(|_| value.to_owned().into(), Into::into)
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

impl crate::schema::LockoutFields for second_factor_lockouts::Entity {
    // Second-factor identities are not addresses; see
    // `LockoutFields::IDENTITY_IS_EMAIL`.
    const IDENTITY_IS_EMAIL: bool = false;
    fn read_lockout_id(m: &Self::Model) -> String {
        m.id.to_string()
    }
    fn write_lockout_id(m: &mut Self::ActiveModel, v: &str) {
        // The lockout store mints these ids as integers in i64 range.
        if let Ok(id) = v.parse() {
            m.id = Set(id);
        }
    }
    fn read_user_id(m: &Self::Model) -> String {
        m.identity.clone()
    }
    fn user_id_column() -> Self::Column {
        second_factor_lockouts::Column::Identity
    }
    fn write_user_id(m: &mut Self::ActiveModel, v: &str) {
        m.identity = Set(v.to_owned());
    }
    fn read_attempted_at(m: &Self::Model) -> DateTime<Utc> {
        m.attempted_at
    }
    fn attempted_at_column() -> Self::Column {
        second_factor_lockouts::Column::AttemptedAt
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
        crate::storage::hex_lower(&bytes)
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
        // A host that calls the store directly gets the bound a grant gets.
        let metadata = session.metadata.bounded();
        sessions::ActiveModel {
            id: Set(session.session_id),
            user_id: Set(session.user_id.parse().expect("fixture ids are i64")),
            auth_epoch: Set(auth_epoch),
            token_digest: Set(hex(session.token_digest)),
            token_hash: Set(Some(hex(session.token_hash))),
            user_agent: Set(metadata.user_agent),
            ip_address: Set(metadata.ip_address),
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
            let mut rows = rows.into_iter().filter(|row| row.selector == selector);
            let row = rows.next();
            if rows.next().is_some() {
                return Err(crate::Error::Conflict {
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
                    return Err(crate::Error::Conflict {
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
                    Err(crate::Error::Conflict {
                        resource: "remember credential".to_owned(),
                        message: "owner and selector matched multiple rows".to_owned(),
                    })
                }
                Err(error) => match transaction.rollback().await {
                    Ok(()) => Err(error),
                    Err(rollback_error) => Err(crate::Error::Internal {
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

        async fn revoke_other_remember(
            &self,
            user_id: &str,
            current_selector: Option<&str>,
        ) -> Result<u64> {
            let mut delete = remembers::Entity::delete_many()
                .filter(remembers::Column::UserId.eq(user_id.to_owned()));
            if let Some(selector) = current_selector {
                delete = delete.filter(remembers::Column::Selector.ne(selector.to_owned()));
            }
            Ok(delete.exec(&self.0).await.map_err(db_error)?.rows_affected)
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
    ) -> two_factor::ActiveModel {
        two_factor::ActiveModel {
            user_id: Set(user_id.to_owned()),
            secret: Set(secret.to_vec()),
            recovery_codes: Set(recovery_codes.map(<[u8]>::to_vec)),
            enrollment_auth_epoch: Set(enrollment_auth_epoch),
            enrollment_session_id: Set(enrollment_session_id.map(str::to_owned)),
            enrollment_expires_at: Set(enrollment_expires_at),
            rotation_pending: Set(false),
            pending_secret: Set(None),
            pending_recovery_codes: Set(None),
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
        snapshot: &EnrollmentActorSnapshot,
        expected_secret: &[u8],
        matched_step: i64,
        at: DateTime<Utc>,
    ) -> Result<bool> {
        let Some(enrollment) = two_factor::Entity::find_by_id(user_id.to_owned())
            .one(transaction.connection())
            .await
            .map_err(db_error)?
        else {
            return Ok(false);
        };
        if enrollment.enrollment_auth_epoch != snapshot.auth_epoch
            || enrollment.enrollment_session_id != snapshot.session_id
            || enrollment.enrollment_expires_at != snapshot.expires_at
        {
            return Err(stale_actor());
        }
        let update = two_factor::Entity::update_many()
            .col_expr(two_factor::Column::ConfirmedAt, Expr::value(at))
            .col_expr(two_factor::Column::RotationPending, Expr::value(false))
            .col_expr(
                two_factor::Column::LastUsedTimestep,
                Expr::value(matched_step),
            )
            .filter(two_factor::Column::UserId.eq(user_id.to_owned()))
            .filter(two_factor::Column::Secret.eq(expected_secret.to_vec()))
            .filter(two_factor::Column::ConfirmedAt.is_null())
            .filter(
                Condition::any()
                    .add(two_factor::Column::LastUsedTimestep.is_null())
                    .add(two_factor::Column::LastUsedTimestep.lt(matched_step)),
            )
            .exec(transaction.connection())
            .await
            .map_err(db_error)?;
        Ok(update.rows_affected == 1)
    }

    async fn confirm_rotation_in(
        transaction: &mut AuthTransaction<'_>,
        user_id: &str,
        snapshot: &EnrollmentActorSnapshot,
        expected_pending_secret: &[u8],
        matched_step: i64,
        at: DateTime<Utc>,
    ) -> Result<bool> {
        let Some(enrollment) = two_factor::Entity::find_by_id(user_id.to_owned())
            .one(transaction.connection())
            .await
            .map_err(db_error)?
        else {
            return Ok(false);
        };
        if enrollment.enrollment_auth_epoch != snapshot.auth_epoch
            || enrollment.enrollment_session_id != snapshot.session_id
            || enrollment.enrollment_expires_at != snapshot.expires_at
        {
            return Err(stale_actor());
        }
        // The claimed step belongs to the new secret, which no code has
        // matched yet, so it replaces the old secret's step outright.
        let update = two_factor::Entity::update_many()
            .col_expr(
                two_factor::Column::Secret,
                Expr::col(two_factor::Column::PendingSecret),
            )
            .col_expr(
                two_factor::Column::RecoveryCodes,
                Expr::col(two_factor::Column::PendingRecoveryCodes),
            )
            .col_expr(
                two_factor::Column::PendingSecret,
                Expr::value(None::<Vec<u8>>),
            )
            .col_expr(
                two_factor::Column::PendingRecoveryCodes,
                Expr::value(None::<Vec<u8>>),
            )
            .col_expr(two_factor::Column::RotationPending, Expr::value(false))
            .col_expr(two_factor::Column::ConfirmedAt, Expr::value(at))
            .col_expr(
                two_factor::Column::LastUsedTimestep,
                Expr::value(matched_step),
            )
            .filter(two_factor::Column::UserId.eq(user_id.to_owned()))
            .filter(two_factor::Column::ConfirmedAt.is_not_null())
            .filter(two_factor::Column::PendingSecret.eq(expected_pending_secret.to_vec()))
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
        // The confirmed secret, its recovery codes and its confirmation stay:
        // they keep gating sign-in until the rotation is confirmed.
        let update = two_factor::Entity::update_many()
            .col_expr(
                two_factor::Column::PendingSecret,
                Expr::value(Some(secret.to_vec())),
            )
            .col_expr(
                two_factor::Column::PendingRecoveryCodes,
                Expr::value(recovery_codes.map(<[u8]>::to_vec)),
            )
            .col_expr(two_factor::Column::RotationPending, Expr::value(true))
            .col_expr(
                two_factor::Column::EnrollmentAuthEpoch,
                Expr::value(snapshot.auth_epoch),
            )
            .col_expr(
                two_factor::Column::EnrollmentSessionId,
                Expr::value(snapshot.session_id.clone()),
            )
            .col_expr(
                two_factor::Column::EnrollmentExpiresAt,
                Expr::value(snapshot.expires_at),
            )
            .filter(two_factor::Column::UserId.eq(user_id.to_owned()))
            .filter(two_factor::Column::ConfirmedAt.is_not_null())
            .exec(transaction.connection())
            .await
            .map_err(db_error)?;
        Ok(update.rows_affected == 1)
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
                pending_secret: row.pending_secret,
                pending_recovery_codes: row.pending_recovery_codes,
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

        async fn set_confirmed(
            &self,
            actor: &CredentialActor,
            expected_secret: &[u8],
            matched_step: i64,
            at: DateTime<Utc>,
        ) -> Result<bool> {
            let user_id = actor.user_id().to_owned();
            let snapshot = enrollment_actor_snapshot(actor)?;
            let expected_secret = expected_secret.to_vec();
            let storage = SeaOrmStorage::<DefaultAuthSchema>::new(self.0.clone());
            fenced_credential_write(&storage, actor, move |transaction| {
                Box::pin(async move {
                    set_confirmed_in(
                        transaction,
                        &user_id,
                        &snapshot,
                        &expected_secret,
                        matched_step,
                        at,
                    )
                    .await
                })
            })
            .await
        }

        async fn confirm_rotation(
            &self,
            actor: &CredentialActor,
            expected_pending_secret: &[u8],
            matched_step: i64,
            at: DateTime<Utc>,
        ) -> Result<bool> {
            let user_id = actor.user_id().to_owned();
            let snapshot = enrollment_actor_snapshot(actor)?;
            let expected_pending_secret = expected_pending_secret.to_vec();
            let storage = SeaOrmStorage::<DefaultAuthSchema>::new(self.0.clone());
            fenced_credential_write(&storage, actor, move |transaction| {
                Box::pin(async move {
                    confirm_rotation_in(
                        transaction,
                        &user_id,
                        &snapshot,
                        &expected_pending_secret,
                        matched_step,
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

        async fn withdraw_enrollment(&self, user_id: &str, expected_secret: &[u8]) -> Result<bool> {
            let withdrawn = two_factor::Entity::delete_many()
                .filter(two_factor::Column::UserId.eq(user_id.to_owned()))
                .filter(two_factor::Column::Secret.eq(expected_secret.to_vec()))
                .filter(two_factor::Column::ConfirmedAt.is_null())
                .filter(two_factor::Column::RotationPending.eq(false))
                .exec(&self.0)
                .await
                .map_err(db_error)?;
            Ok(withdrawn.rows_affected == 1)
        }
    }
}

const LOCKOUT_IDENTITY_INDEX: &str = "auth_lockouts_identity";
const SECOND_FACTOR_LOCKOUT_IDENTITY_INDEX: &str = "auth_second_factor_lockouts_identity";

fn second_factor_lockout_identity_index() -> sea_orm::sea_query::IndexCreateStatement {
    sea_orm::sea_query::Index::create()
        .name(SECOND_FACTOR_LOCKOUT_IDENTITY_INDEX)
        .table(second_factor_lockouts::Entity)
        .col(second_factor_lockouts::Column::Identity)
        .to_owned()
}

fn lockout_identity_index() -> sea_orm::sea_query::IndexCreateStatement {
    sea_orm::sea_query::Index::create()
        .name(LOCKOUT_IDENTITY_INDEX)
        .table(lockouts::Entity)
        .col(lockouts::Column::Identity)
        .to_owned()
}

const USER_EMAIL_INDEX: &str = "app_users_email_unique";

/// One account per email address. Registration, magic-link sign-up and
/// passkey sign-up each look the address up and then insert; two of them
/// racing for one new address both miss the lookup. Without this index both
/// inserts succeed, and every later sign-in, reset or verification by email
/// picks one of the two accounts at random.
fn user_email_index() -> sea_orm::sea_query::IndexCreateStatement {
    sea_orm::sea_query::Index::create()
        .name(USER_EMAIL_INDEX)
        .table(users::Entity)
        .col(users::Column::Email)
        .unique()
        .to_owned()
}

/// Refuse to continue when `app_users` already holds two accounts for one
/// email address: the unique index cannot be created over them, and which
/// account is the real one is the operator's decision, not the migration's.
/// The message names no address.
async fn ensure_unique_user_emails(db: &DatabaseConnection) -> crate::Result<()> {
    let backend = db.get_database_backend();
    let duplicate = db
        .query_one_raw(Statement::from_string(
            backend,
            "SELECT email FROM app_users GROUP BY email HAVING COUNT(*) > 1 LIMIT 1",
        ))
        .await
        .map_err(|error| crate::Error::Internal {
            message: format!("inspect app_users email uniqueness: {error}"),
        })?;
    if duplicate.is_some() {
        return Err(crate::Error::Internal {
            message: "app_users holds more than one account for one email address; merge or \
                      remove the duplicate accounts, then run the migration again"
                .to_owned(),
        });
    }
    Ok(())
}

/// Create every default auth table and required index.
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
    let app_users_is_legacy_source = app_users_present && !app_users_is_default;
    let legacy_source_present = users_source_present || app_users_is_legacy_source;
    let completed_marker = migration_state_value(db, "schema_version").await?;
    let pending_marker = migration_state_value(db, "source_pending").await?;
    let defer_completion_marker =
        completed_marker.is_none() && (legacy_source_present || pending_marker.is_some());
    let backend = db.get_database_backend();
    for (table, statement) in default_tables(backend) {
        create_table_if_missing(db, table, &statement).await?;
    }
    add_session_auth_epoch_column(db).await?;
    add_remember_auth_epoch_column(db).await?;
    add_two_factor_enrollment_snapshot_columns(db).await?;
    widen_mysql_columns(db).await?;
    let mut ip_address = sea_orm::sea_query::ColumnDef::new(lockouts::Column::IpAddress);
    ip_address.string().null();
    add_column_if_missing(
        db,
        "auth_lockouts",
        "ip_address",
        ip_address,
        "add lockout IP-address column",
    )
    .await?;
    let mut source_id = sea_orm::sea_query::ColumnDef::new(lockouts::Column::MigrationSourceId);
    source_id.string().null();
    add_column_if_missing(
        db,
        "auth_lockouts",
        "migration_source_id",
        source_id,
        "add lockout migration source column",
    )
    .await?;
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
        create_missing_index(
            db,
            "auth_linked_accounts",
            INDEX,
            &index,
            "linked-account uniqueness index",
        )
        .await?;
    }
    const LOCKOUT_SOURCE_INDEX: &str = "auth_lockouts_migration_source";
    if !index_exists(db, "auth_lockouts", LOCKOUT_SOURCE_INDEX).await? {
        let index = sea_orm::sea_query::Index::create()
            .name(LOCKOUT_SOURCE_INDEX)
            .table(lockouts::Entity)
            .col(lockouts::Column::MigrationSourceId)
            .unique()
            .to_owned();
        create_missing_index(
            db,
            "auth_lockouts",
            LOCKOUT_SOURCE_INDEX,
            &index,
            "lockout migration-source index",
        )
        .await?;
    }
    if !index_exists(db, "auth_lockouts", LOCKOUT_IDENTITY_INDEX).await? {
        create_missing_index(
            db,
            "auth_lockouts",
            LOCKOUT_IDENTITY_INDEX,
            &lockout_identity_index(),
            "lockout identity index",
        )
        .await?;
    }
    // A legacy `app_users` is a migration source, not the default table:
    // the migration engine rebuilds it, and a later run indexes the result.
    if !app_users_is_legacy_source && !index_exists(db, "app_users", USER_EMAIL_INDEX).await? {
        ensure_unique_user_emails(db).await?;
        create_missing_index(
            db,
            "app_users",
            USER_EMAIL_INDEX,
            &user_email_index(),
            "app_users email uniqueness index",
        )
        .await?;
    }
    if !index_exists(
        db,
        "auth_second_factor_lockouts",
        SECOND_FACTOR_LOCKOUT_IDENTITY_INDEX,
    )
    .await?
    {
        create_missing_index(
            db,
            "auth_second_factor_lockouts",
            SECOND_FACTOR_LOCKOUT_IDENTITY_INDEX,
            &second_factor_lockout_identity_index(),
            "second-factor lockout identity index",
        )
        .await?;
    }
    if !app_users_is_legacy_source {
        warn_about_over_long_emails(db).await?;
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

/// Log one warning, naming the count, when `app_users` holds addresses
/// longer than [`crate::password::MAX_EMAIL_LENGTH`] characters.
///
/// Sign-in by email answers such an address as an unknown one, so those
/// accounts can no longer sign in with it; the operator decides what
/// becomes of them. The warning names no address.
async fn warn_about_over_long_emails(db: &DatabaseConnection) -> crate::Result<()> {
    let backend = db.get_database_backend();
    let length = match backend {
        DbBackend::MySql => "CHAR_LENGTH(email)",
        _ => "LENGTH(email)",
    };
    let limit = crate::password::MAX_EMAIL_LENGTH;
    let row = db
        .query_one_raw(Statement::from_string(
            backend,
            format!("SELECT COUNT(*) AS over_long FROM app_users WHERE {length} > {limit}"),
        ))
        .await
        .map_err(|error| crate::Error::Internal {
            message: format!("inspect app_users email lengths: {error}"),
        })?;
    let over_long = row
        .map(|row| row.try_get::<i64>("", "over_long"))
        .transpose()
        .map_err(|error| crate::Error::Internal {
            message: format!("inspect app_users email lengths: {error}"),
        })?
        .unwrap_or(0);
    if over_long > 0 {
        tracing::warn!(
            accounts = over_long,
            "app_users holds {over_long} email addresses longer than {limit} characters; \
             those accounts can no longer sign in by email"
        );
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

/// The conflict for a marker row that holds a value this release does not
/// write.
fn unsupported_marker(key: &str) -> crate::Error {
    crate::Error::Conflict {
        resource: "Magnetar migration marker".to_owned(),
        message: format!("marker {key} has an unsupported value"),
    }
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
        Some(_) => Err(unsupported_marker(key)),
        None => {
            let inserted = migration_state::ActiveModel {
                key: Set(key.to_owned()),
                value: Set(value.to_owned()),
            }
            .insert(db)
            .await;
            let Err(error) = inserted else {
                return Ok(());
            };
            // A concurrent migrate writes the same marker between the read
            // and this insert, and the insert hits its key. The marker it
            // wrote is the one this run wanted. Another value is the conflict
            // the read reports, and no row leaves the insert's own error.
            let written = migration_state::Entity::find_by_id(key)
                .one(db)
                .await
                .map_err(|error| crate::Error::Internal {
                    message: format!("read Magnetar migration marker: {error}"),
                })?;
            match written {
                Some(existing) if existing.value == value => Ok(()),
                Some(_) => Err(unsupported_marker(key)),
                None => Err(crate::Error::Internal {
                    message: format!("write Magnetar migration marker: {error}"),
                }),
            }
        }
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

/// Run one default table's `CREATE TABLE IF NOT EXISTS`.
///
/// Postgres refuses one of two concurrent `CREATE TABLE IF NOT EXISTS` for
/// one table. The table the other run created is the table this run wanted;
/// a failure that leaves no table is an error.
async fn create_table_if_missing(
    db: &DatabaseConnection,
    table: &str,
    statement: &sea_orm::sea_query::TableCreateStatement,
) -> crate::Result<()> {
    let Err(error) = db.execute(statement).await else {
        return Ok(());
    };
    if table_exists(db, table).await? {
        return Ok(());
    }
    Err(crate::Error::Internal {
        message: format!("create default auth table: {error}"),
    })
}

/// Add one column to `table` unless it is there.
///
/// Two processes that upgrade one database both find the column missing and
/// both add it, and the second add fails: MySQL and MariaDB report a
/// duplicate column, Postgres and SQLite an existing one. A failure after
/// which the column exists is that race, and the column this run wanted is
/// in place. A failure that leaves no column is an error.
async fn add_column_if_missing(
    db: &DatabaseConnection,
    table: &str,
    column_name: &str,
    definition: sea_orm::sea_query::ColumnDef,
    description: &str,
) -> crate::Result<()> {
    if column_exists(db, table, column_name).await? {
        return Ok(());
    }
    let alter = sea_orm::sea_query::Table::alter()
        .table(sea_orm::sea_query::Alias::new(table))
        .add_column(definition)
        .to_owned();
    let Err(error) = db.execute(&alter).await else {
        return Ok(());
    };
    if column_exists(db, table, column_name).await? {
        return Ok(());
    }
    Err(crate::Error::Internal {
        message: format!("{description}: {error}"),
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
    let mut definition = sea_orm::sea_query::ColumnDef::new(column);
    definition.timestamp_with_time_zone().null();
    add_column_if_missing(
        db,
        table,
        column_name,
        definition,
        &format!("add {table}.{column_name}"),
    )
    .await
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
    let mut definition = sea_orm::sea_query::ColumnDef::new(column);
    definition.string().null();
    add_column_if_missing(
        db,
        table,
        column_name,
        definition,
        &format!("add {table}.{column_name}"),
    )
    .await
}

async fn add_two_factor_enrollment_snapshot_columns(db: &DatabaseConnection) -> crate::Result<()> {
    let mut enrollment_epoch =
        sea_orm::sea_query::ColumnDef::new(two_factor::Column::EnrollmentAuthEpoch);
    enrollment_epoch.big_integer().not_null().default(0);
    add_column_if_missing(
        db,
        "auth_two_factor",
        "enrollment_auth_epoch",
        enrollment_epoch,
        "add auth_two_factor.enrollment_auth_epoch",
    )
    .await?;
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
    let mut rotation_pending =
        sea_orm::sea_query::ColumnDef::new(two_factor::Column::RotationPending);
    rotation_pending.boolean().not_null().default(false);
    add_column_if_missing(
        db,
        "auth_two_factor",
        "rotation_pending",
        rotation_pending,
        "add auth_two_factor.rotation_pending",
    )
    .await?;
    // A rotation waits beside the confirmed secret in these two columns.
    // Before they existed a rotation overwrote the confirmed secret.
    for (name, column) in [
        ("pending_secret", two_factor::Column::PendingSecret),
        (
            "pending_recovery_codes",
            two_factor::Column::PendingRecoveryCodes,
        ),
    ] {
        let definition =
            Schema::new(db.get_database_backend()).get_column_def::<two_factor::Entity>(column);
        add_column_if_missing(
            db,
            "auth_two_factor",
            name,
            definition,
            &format!("add auth_two_factor.{name}"),
        )
        .await?;
    }
    Ok(())
}

async fn add_auth_epoch_column(db: &DatabaseConnection) -> crate::Result<()> {
    let mut definition = sea_orm::sea_query::ColumnDef::new(users::Column::AuthEpoch);
    definition.big_integer().not_null().default(0);
    add_column_if_missing(
        db,
        "app_users",
        "auth_epoch",
        definition,
        "add app_users.auth_epoch",
    )
    .await
}

async fn add_session_auth_epoch_column(db: &DatabaseConnection) -> crate::Result<()> {
    let mut definition = sea_orm::sea_query::ColumnDef::new(sessions::Column::AuthEpoch);
    definition.big_integer().not_null().default(-1);
    add_column_if_missing(
        db,
        "auth_sessions",
        "auth_epoch",
        definition,
        "add auth_sessions.auth_epoch",
    )
    .await
}

async fn add_remember_auth_epoch_column(db: &DatabaseConnection) -> crate::Result<()> {
    let mut definition = sea_orm::sea_query::ColumnDef::new(remembers::Column::AuthEpoch);
    definition.big_integer().not_null().default(-1);
    add_column_if_missing(
        db,
        "auth_remember_tokens",
        "auth_epoch",
        definition,
        "add auth_remember_tokens.auth_epoch",
    )
    .await
}

/// Run the `CREATE INDEX` for an index [`index_exists`] found missing.
///
/// Two processes that migrate one fresh database both find the index
/// missing and both create it, and the second create fails: MySQL and
/// MariaDB report a duplicate key name, Postgres and SQLite an existing
/// relation. A failure after which the named index exists is that race, and
/// the index this run wanted is in place. A failure that leaves no index is
/// an error.
async fn create_missing_index(
    db: &DatabaseConnection,
    table: &str,
    name: &str,
    index: &sea_orm::sea_query::IndexCreateStatement,
    description: &str,
) -> crate::Result<()> {
    let Err(error) = db.execute(index).await else {
        return Ok(());
    };
    if index_exists(db, table, name).await? {
        return Ok(());
    }
    Err(crate::Error::Internal {
        message: format!("create {description}: {error}"),
    })
}

/// The MySQL type of every byte and long-text column of the default schema.
///
/// MySQL and MariaDB create an entity's `Vec<u8>` field that has no column
/// type as `varbinary(255)` and its `String` field as `varchar(255)`, and
/// strict mode refuses a longer write. `migrate` created these columns that
/// way before the entities named a type for them, so
/// [`widen_mysql_columns`] upgrades a database it created then. Postgres
/// (`bytea`, `varchar`) and SQLite hold any length on either layout.
///
/// The entities name `Blob` and `Text`, which MySQL holds up to 64 KiB. The
/// provider-token ciphertexts are `longblob` instead: they hold the
/// provider's token response, whole in the raw payload and in parts in the
/// other two, an OAuth transport accepts up to a megabyte of it (the
/// framework's default limit, which a host may raise), and encryption grows
/// it by about four fifths in the framework. A token response whose
/// id_token and access token carry long group or role claims can pass 64 KiB
/// once encrypted, and a refused commit after a rotating exchange loses the
/// refresh token the provider just issued.
const MYSQL_COLUMN_TYPES: [(&str, &str, &str); 12] = [
    ("auth_sessions", "user_agent", "text"),
    ("auth_methods", "credential_id", "text"),
    ("auth_methods", "public_key", "text"),
    ("auth_ceremonies", "payload", "blob"),
    ("auth_two_factor", "secret", "blob"),
    ("auth_two_factor", "recovery_codes", "blob"),
    ("auth_two_factor", "pending_secret", "blob"),
    ("auth_two_factor", "pending_recovery_codes", "blob"),
    ("auth_provider_tokens", "access_ciphertext", "longblob"),
    ("auth_provider_tokens", "refresh_ciphertext", "longblob"),
    ("auth_provider_tokens", "raw_payload_ciphertext", "longblob"),
    ("auth_provider_tokens", "scopes", "text"),
];

/// The MySQL type [`MYSQL_COLUMN_TYPES`] names for one column, if any.
fn mysql_column_type(table: &str, column: &str) -> Option<&'static str> {
    MYSQL_COLUMN_TYPES
        .iter()
        .find(|(listed_table, listed_column, _)| *listed_table == table && *listed_column == column)
        .map(|(_, _, mysql_type)| *mysql_type)
}

/// Every default table with its `CREATE TABLE IF NOT EXISTS` on `backend`.
fn default_tables(
    backend: DbBackend,
) -> [(&'static str, sea_orm::sea_query::TableCreateStatement); 15] {
    [
        default_table(backend, users::Entity),
        default_table(backend, sessions::Entity),
        default_table(backend, methods::Entity),
        default_table(backend, accounts::Entity),
        default_table(backend, tokens::Entity),
        default_table(backend, ceremonies::Entity),
        default_table(backend, lockouts::Entity),
        default_table(backend, second_factor_lockouts::Entity),
        default_table(backend, remembers::Entity),
        default_table(backend, two_factor::Entity),
        default_table(backend, lifecycle_deliveries::Entity),
        default_table(backend, migration_runs::Entity),
        default_table(backend, migration_identities::Entity),
        default_table(backend, migration_state::Entity),
        default_table(backend, provider_tokens::Entity),
    ]
}

/// One default table's `CREATE TABLE IF NOT EXISTS`, built column by column
/// from the entity, as `create_table_from_entity` builds it, so that a
/// MySQL column can take the type [`MYSQL_COLUMN_TYPES`] names where the
/// entity's portable column type cannot say it. No default entity has a
/// composite key, a relation or a comment, the parts of
/// `create_table_from_entity` this leaves out.
fn default_table<E>(
    backend: DbBackend,
    entity: E,
) -> (&'static str, sea_orm::sea_query::TableCreateStatement)
where
    E: EntityTrait,
{
    let table = entity.table_name();
    let schema = Schema::new(backend);
    let mut statement = sea_orm::sea_query::Table::create();
    statement.table(entity).if_not_exists();
    for column in E::Column::iter() {
        let mut definition = schema.get_column_def::<E>(column);
        if backend == DbBackend::MySql
            && let Some(mysql_type) = mysql_column_type(table, column.as_str())
        {
            definition.custom(sea_orm::sea_query::Alias::new(mysql_type));
        }
        statement.col(definition);
    }
    (table, statement)
}

/// Whether MySQL data type `current` holds less than `target`.
///
/// Both are compared within one family, byte strings or text. A type of
/// another family, such as a `json` column an application chose, is never
/// changed, and a wider type is never narrowed.
fn mysql_type_is_narrower(current: &str, target: &str) -> bool {
    const FAMILIES: [&[&str]; 2] = [
        &[
            "binary",
            "varbinary",
            "tinyblob",
            "blob",
            "mediumblob",
            "longblob",
        ],
        &[
            "char",
            "varchar",
            "tinytext",
            "text",
            "mediumtext",
            "longtext",
        ],
    ];
    let current = current.to_ascii_lowercase();
    FAMILIES.iter().any(|family| {
        let position = |name: &str| family.iter().position(|listed| *listed == name);
        matches!(
            (position(&current), position(target)),
            (Some(current), Some(target)) if current < target
        )
    })
}

/// One MySQL column as `information_schema.COLUMNS` describes it.
struct MysqlColumn {
    data_type: String,
    nullable: bool,
    /// Character set and collation of a text column; `None` for bytes.
    character_set: Option<String>,
    collation: Option<String>,
}

/// One MySQL column, or `None` when the table has no such column.
async fn mysql_column_definition(
    db: &DatabaseConnection,
    table: &str,
    column: &str,
) -> crate::Result<Option<MysqlColumn>> {
    let inspect_error = |error: sea_orm::DbErr| crate::Error::Internal {
        message: format!("inspect {table}.{column} type: {error}"),
    };
    let row = db
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::MySql,
            "SELECT CAST(DATA_TYPE AS CHAR) AS data_type, CAST(IS_NULLABLE AS CHAR) AS is_nullable, \
                    CAST(CHARACTER_SET_NAME AS CHAR) AS character_set, \
                    CAST(COLLATION_NAME AS CHAR) AS collation \
             FROM information_schema.COLUMNS \
             WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ? AND COLUMN_NAME = ? LIMIT 1",
            [table.to_owned().into(), column.to_owned().into()],
        ))
        .await
        .map_err(inspect_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    Ok(Some(MysqlColumn {
        data_type: row
            .try_get::<String>("", "data_type")
            .map_err(inspect_error)?,
        nullable: row
            .try_get::<String>("", "is_nullable")
            .map_err(inspect_error)?
            .eq_ignore_ascii_case("YES"),
        character_set: row
            .try_get::<Option<String>>("", "character_set")
            .map_err(inspect_error)?,
        collation: row
            .try_get::<Option<String>>("", "collation")
            .map_err(inspect_error)?,
    }))
}

/// The indexes that cover one MySQL column.
async fn mysql_column_indexes(
    db: &DatabaseConnection,
    table: &str,
    column: &str,
) -> crate::Result<Vec<String>> {
    let inspect_error = |error: sea_orm::DbErr| crate::Error::Internal {
        message: format!("inspect {table}.{column} indexes: {error}"),
    };
    db.query_all_raw(Statement::from_sql_and_values(
        DbBackend::MySql,
        "SELECT DISTINCT CAST(INDEX_NAME AS CHAR) AS index_name \
         FROM information_schema.STATISTICS \
         WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ? AND COLUMN_NAME = ? \
         ORDER BY index_name",
        [table.to_owned().into(), column.to_owned().into()],
    ))
    .await
    .map_err(inspect_error)?
    .iter()
    .map(|row| {
        row.try_get::<String>("", "index_name")
            .map_err(inspect_error)
    })
    .collect()
}

/// The MySQL type a widened column takes: `target`, and for a text column
/// the character set and collation it already has. Left out, `MODIFY
/// COLUMN` would take the table's defaults and drop an operator's choice.
fn mysql_widened_type(target: &str, current: &MysqlColumn) -> crate::Result<String> {
    let (Some(character_set), Some(collation)) = (&current.character_set, &current.collation)
    else {
        return Ok(target.to_owned());
    };
    // The names come from the server's catalog; anything but a plain name
    // is refused rather than written into the statement.
    let plain = |name: &str| {
        !name.is_empty()
            && name
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
    };
    if !plain(character_set) || !plain(collation) {
        return Err(crate::Error::Internal {
            message: format!(
                "unexpected character set {character_set:?} or collation {collation:?}"
            ),
        });
    }
    Ok(format!(
        "{target} CHARACTER SET {character_set} COLLATE {collation}"
    ))
}

/// On MySQL and MariaDB, widen every column [`MYSQL_COLUMN_TYPES`] names
/// that still has a narrower type, as a database `migrate` created before
/// those columns carried a type holds them.
///
/// Each table changes in one `ALTER TABLE`, so a table with several narrow
/// columns is rebuilt once, and each column keeps its nullability, its
/// character set and collation, and its rows. A column that is already as
/// wide is left alone, so a second run changes nothing. Two processes that
/// widen one table at once run the same `MODIFY COLUMN` one after the
/// other, and both succeed. A column an index covers stays as it is, with a
/// warning: MySQL refuses a `text` or `blob` column in an index without a
/// key length, and that refusal must not stop boot. Postgres and SQLite are
/// never touched: their columns hold any length.
async fn widen_mysql_columns(db: &DatabaseConnection) -> crate::Result<()> {
    if db.get_database_backend() != DbBackend::MySql {
        return Ok(());
    }
    let mut tables: Vec<&str> = Vec::new();
    for (table, _, _) in MYSQL_COLUMN_TYPES {
        if !tables.contains(&table) {
            tables.push(table);
        }
    }
    for table in tables {
        let mut alter = sea_orm::sea_query::Table::alter();
        alter.table(sea_orm::sea_query::Alias::new(table));
        let mut widened = Vec::new();
        for (_, column, target) in MYSQL_COLUMN_TYPES
            .iter()
            .filter(|(listed_table, _, _)| *listed_table == table)
        {
            let Some(current) = mysql_column_definition(db, table, column).await? else {
                continue;
            };
            if !mysql_type_is_narrower(&current.data_type, target) {
                continue;
            }
            let indexes = mysql_column_indexes(db, table, column).await?;
            if !indexes.is_empty() {
                let indexes = indexes.join(", ");
                tracing::warn!(
                    table = %table,
                    column = %column,
                    indexes = %indexes,
                    "{table}.{column} stays {}: index {indexes} covers it, and MySQL cannot \
                     index a {target} column without a key length; drop or change the index, \
                     then run the migration again to widen it",
                    current.data_type
                );
                continue;
            }
            let mut definition =
                sea_orm::sea_query::ColumnDef::new(sea_orm::sea_query::Alias::new(*column));
            definition.custom(sea_orm::sea_query::Alias::new(mysql_widened_type(
                target, &current,
            )?));
            if current.nullable {
                definition.null();
            } else {
                definition.not_null();
            }
            alter.modify_column(definition);
            widened.push(*column);
        }
        if widened.is_empty() {
            continue;
        }
        db.execute(&alter)
            .await
            .map_err(|error| crate::Error::Internal {
                message: format!("widen {table} columns {}: {error}", widened.join(", ")),
            })?;
    }
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

#[cfg(all(test, feature = "seaorm-sqlite"))]
mod tests {
    use sea_orm::sea_query::{MysqlQueryBuilder, PostgresQueryBuilder, SqliteQueryBuilder};
    use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};

    #[test]
    fn lockout_identity_index_builds_for_every_supported_backend() {
        for sql in [
            super::lockout_identity_index().to_string(SqliteQueryBuilder),
            super::lockout_identity_index().to_string(PostgresQueryBuilder),
            super::lockout_identity_index().to_string(MysqlQueryBuilder),
        ] {
            assert!(sql.contains(super::LOCKOUT_IDENTITY_INDEX));
            assert!(sql.contains("auth_lockouts"));
            assert!(sql.contains("identity"));
            assert!(!sql.contains("UNIQUE"));
        }
    }

    /// The default tables, as `create_table_from_entity` builds them.
    fn tables_from_entities(backend: DbBackend) -> Vec<String> {
        use super::*;
        let schema = Schema::new(backend);
        macro_rules! from_entities {
            ($($entity:expr),*) => {
                vec![$(schema.create_table_from_entity($entity).if_not_exists().to_owned()),*]
            };
        }
        let statements: Vec<sea_orm::sea_query::TableCreateStatement> = from_entities!(
            users::Entity,
            sessions::Entity,
            methods::Entity,
            accounts::Entity,
            tokens::Entity,
            ceremonies::Entity,
            lockouts::Entity,
            second_factor_lockouts::Entity,
            remembers::Entity,
            two_factor::Entity,
            lifecycle_deliveries::Entity,
            migration_runs::Entity,
            migration_identities::Entity,
            migration_state::Entity,
            provider_tokens::Entity
        );
        statements
            .iter()
            .map(|statement| render(backend, statement))
            .collect()
    }

    fn render(backend: DbBackend, statement: &sea_orm::sea_query::TableCreateStatement) -> String {
        match backend {
            DbBackend::MySql => statement.to_string(MysqlQueryBuilder),
            DbBackend::Postgres => statement.to_string(PostgresQueryBuilder),
            _ => statement.to_string(SqliteQueryBuilder),
        }
    }

    #[test]
    fn default_tables_match_the_entities_except_for_named_mysql_types() {
        for backend in [DbBackend::Sqlite, DbBackend::Postgres, DbBackend::MySql] {
            let built = super::default_tables(backend)
                .iter()
                .map(|(table, statement)| (*table, render(backend, statement)))
                .collect::<Vec<_>>();
            let from_entities = tables_from_entities(backend);
            assert_eq!(
                built.len(),
                from_entities.len(),
                "every entity has its table and no more"
            );
            for ((table, built), from_entity) in built.iter().zip(from_entities) {
                if backend == DbBackend::MySql && *table == "auth_provider_tokens" {
                    assert_eq!(
                        built.replace("longblob", "blob"),
                        from_entity,
                        "only the named MySQL types differ"
                    );
                } else {
                    assert_eq!(*built, from_entity, "{backend:?} {table}");
                }
            }
        }
    }

    #[test]
    fn mysql_default_tables_hold_long_bytes_and_text() {
        let built = super::default_tables(DbBackend::MySql)
            .iter()
            .map(|(table, statement)| (*table, render(DbBackend::MySql, statement)))
            .collect::<Vec<_>>();
        for (table, sql) in &built {
            assert!(!sql.contains("varbinary"), "{table}: {sql}");
        }
        for (table, column, mysql_type) in super::MYSQL_COLUMN_TYPES {
            let (_, sql) = built
                .iter()
                .find(|(built_table, _)| *built_table == table)
                .expect("every named table is a default table");
            assert!(
                sql.contains(&format!("`{column}` {mysql_type}")),
                "{table}.{column} is {mysql_type}: {sql}"
            );
        }
        for backend in [DbBackend::Sqlite, DbBackend::Postgres] {
            for (table, statement) in super::default_tables(backend) {
                let sql = render(backend, &statement);
                assert!(!sql.contains("longblob"), "{backend:?} {table}: {sql}");
            }
        }
    }

    #[test]
    fn mysql_widening_only_widens_within_one_family() {
        assert!(super::mysql_type_is_narrower("varbinary", "blob"));
        assert!(super::mysql_type_is_narrower("VARBINARY", "longblob"));
        assert!(super::mysql_type_is_narrower("blob", "longblob"));
        assert!(super::mysql_type_is_narrower("varchar", "text"));
        assert!(!super::mysql_type_is_narrower("blob", "blob"));
        assert!(!super::mysql_type_is_narrower("longblob", "blob"));
        assert!(!super::mysql_type_is_narrower("mediumtext", "text"));
        assert!(!super::mysql_type_is_narrower("json", "text"));
        assert!(!super::mysql_type_is_narrower("varchar", "blob"));
    }

    #[tokio::test]
    async fn an_index_a_concurrent_migrate_created_counts_as_created() {
        let database = Database::connect("sqlite::memory:").await.unwrap();
        super::migrate(&database).await.unwrap();
        // The index exists, as a concurrent run leaves it between this
        // run's check and its create; the create fails and is the race.
        super::create_missing_index(
            &database,
            "auth_lockouts",
            super::LOCKOUT_IDENTITY_INDEX,
            &super::lockout_identity_index(),
            "lockout identity index",
        )
        .await
        .expect("an index that exists after a failed create is in place");
    }

    #[tokio::test]
    async fn an_index_create_that_leaves_no_index_stays_an_error() {
        let database = Database::connect("sqlite::memory:").await.unwrap();
        super::migrate(&database).await.unwrap();
        let index = sea_orm::sea_query::Index::create()
            .name("no_such_table_identity")
            .table(sea_orm::sea_query::Alias::new("no_such_table"))
            .col(sea_orm::sea_query::Alias::new("identity"))
            .to_owned();
        let error = super::create_missing_index(
            &database,
            "no_such_table",
            "no_such_table_identity",
            &index,
            "broken index",
        )
        .await
        .expect_err("a create that leaves no index is an error")
        .to_string();
        assert!(error.contains("create broken index"), "got: {error}");
    }

    #[tokio::test]
    async fn a_create_table_that_leaves_no_table_stays_an_error() {
        let database = Database::connect("sqlite::memory:").await.unwrap();
        // Two columns of one name: SQLite refuses the table.
        let statement = sea_orm::sea_query::Table::create()
            .table(sea_orm::sea_query::Alias::new("no_such_table"))
            .if_not_exists()
            .col(sea_orm::sea_query::ColumnDef::new(sea_orm::sea_query::Alias::new("id")).integer())
            .col(sea_orm::sea_query::ColumnDef::new(sea_orm::sea_query::Alias::new("id")).integer())
            .to_owned();
        let error = super::create_table_if_missing(&database, "no_such_table", &statement)
            .await
            .expect_err("a create that leaves no table is an error")
            .to_string();
        assert!(error.contains("create default auth table"), "got: {error}");
    }

    #[tokio::test]
    async fn a_foreign_marker_written_during_the_race_is_a_conflict() {
        let database = Database::connect("sqlite::memory:").await.unwrap();
        super::migrate(&database).await.unwrap();
        database
            .execute_unprepared("DELETE FROM magnetar_migration_state WHERE key = 'schema_version'")
            .await
            .unwrap();
        // Another writer stores a different value between this run's read
        // and its insert, and this run's insert writes no row. RAISE(IGNORE)
        // drops the insert and keeps the other writer's row, as a commit by
        // another process would.
        database
            .execute_unprepared(
                "CREATE TRIGGER foreign_marker BEFORE INSERT ON magnetar_migration_state \
                 WHEN NEW.key = 'schema_version' BEGIN \
                 INSERT INTO magnetar_migration_state (key, value) VALUES ('schema_version', '2'); \
                 SELECT RAISE(IGNORE); \
                 END",
            )
            .await
            .unwrap();
        let error = super::ensure_migration_state(&database, "schema_version", "1")
            .await
            .expect_err("a foreign marker is refused");
        assert!(
            matches!(&error, crate::Error::Conflict { resource, message }
                if resource == "Magnetar migration marker"
                    && message == "marker schema_version has an unsupported value"),
            "the race answers as the read does: {error:?}"
        );
    }

    #[tokio::test]
    async fn an_add_column_that_leaves_no_column_stays_an_error() {
        let database = Database::connect("sqlite::memory:").await.unwrap();
        let mut definition =
            sea_orm::sea_query::ColumnDef::new(sea_orm::sea_query::Alias::new("auth_epoch"));
        definition.big_integer().not_null().default(0);
        let error = super::add_column_if_missing(
            &database,
            "no_such_table",
            "auth_epoch",
            definition,
            "add no_such_table.auth_epoch",
        )
        .await
        .expect_err("an add that leaves no column is an error")
        .to_string();
        assert!(
            error.contains("add no_such_table.auth_epoch"),
            "got: {error}"
        );
    }

    #[tokio::test]
    async fn default_lockout_schema_installs_identity_index_replay_safely() {
        let database = Database::connect("sqlite::memory:").await.unwrap();
        super::migrate(&database).await.unwrap();
        super::migrate(&database).await.unwrap();

        let indexes = database
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                "PRAGMA index_list('auth_lockouts')".to_owned(),
            ))
            .await
            .unwrap();
        let names = indexes
            .iter()
            .map(|row| row.try_get::<String>("", "name").unwrap())
            .collect::<Vec<_>>();

        assert!(
            names.iter().any(|name| name == "auth_lockouts_identity"),
            "default lockout schema must index the identity serialization key; got {names:?}"
        );
    }
}
