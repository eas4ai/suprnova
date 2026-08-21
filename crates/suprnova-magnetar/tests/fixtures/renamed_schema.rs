//! Standalone SeaORM bindings used to prove column-renaming isolation.
//!
//! This is intentionally a generated-entity-shaped fixture. The application
//! column changes from `email` to `login_email`; the generic operation in
//! `schema_binding.rs` remains unchanged.

use chrono::{DateTime, Utc};
use magnetar::schema::{
    AuthSchema, CeremonyFields, EntityBinding, LinkedAccountFields, LockoutFields,
    NOT_NULL_PASSWORD_EMPTY_SENTINEL, PasskeyFields, SessionEpoch, SessionFields, TokenFields,
    TokenRecordFields, UserFields, UserOptionalFields,
};
use sea_orm::ActiveValue::Set;
use sea_orm::entity::prelude::*;
#[cfg(feature = "seaorm-sqlite")]
use sea_orm::sea_query::SqliteQueryBuilder;
#[cfg(feature = "seaorm-sqlite")]
use sea_orm::{ConnectionTrait, Database, DatabaseConnection, DbBackend, Schema, Statement};

/// Original generated application entity.
pub mod original_users {
    use super::*;

    /// Original user row model.
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_users")]
    pub struct Model {
        /// Application-owned user identifier.
        #[sea_orm(primary_key)]
        pub id: i64,
        /// Original generated email column.
        pub email: String,
        /// Optional display name.
        pub name: Option<String>,
        /// Nullable email verification timestamp.
        pub email_verified_at: Option<DateTime<Utc>>,
        /// Optional remember-me token.
        pub remember_token: Option<String>,
        /// Nullable password hash.
        pub password_hash: Option<String>,
        /// Nullable user lock timestamp.
        pub locked_at: Option<DateTime<Utc>>,
        /// Monotonic global authentication epoch.
        pub auth_epoch: i64,
    }

    /// No relations are needed by this focused binding fixture.
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    /// Default active-model behavior for the generated entity.
    impl ActiveModelBehavior for ActiveModel {}
}

/// Renamed generated application entity.
pub mod renamed_users {
    use super::*;

    /// Renamed user row model.
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_users")]
    pub struct Model {
        /// Application-owned user identifier.
        #[sea_orm(primary_key)]
        pub id: i64,
        /// Nullable user lock timestamp.
        pub locked_at: Option<DateTime<Utc>>,
        /// Renamed generated email column. This is the only schema change.
        pub login_email: String,
        /// Optional display name.
        pub name: Option<String>,
        /// Nullable email verification timestamp.
        pub email_verified_at: Option<DateTime<Utc>>,
        /// Optional remember-me token.
        pub remember_token: Option<String>,
        /// Renamed generated session epoch column.
        pub session_version: i64,
        /// NOT-NULL password storage uses the documented empty sentinel.
        pub password_hash: String,
    }

    /// No relations are needed by this focused binding fixture.
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    /// Default active-model behavior for the generated entity.
    impl ActiveModelBehavior for ActiveModel {}
}

macro_rules! simple_entity {
    ($module:ident, $table:literal) => {
        /// Generated fixture entity for a non-user role.
        pub mod $module {
            use super::*;

            /// Generated fixture row model.
            #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
            #[sea_orm(table_name = $table)]
            pub struct Model {
                /// Application-owned row identifier.
                #[sea_orm(primary_key)]
                pub id: i64,
                /// Owning user identifier.
                pub user_id: i64,
                /// Provider name.
                pub provider: String,
                /// Provider account identifier.
                pub provider_account_id: String,
                /// Access token.
                pub access_token: Option<String>,
                /// Refresh token.
                pub refresh_token: Option<String>,
                /// Credential identifier.
                pub credential_id: String,
                /// Serialized public key.
                pub public_key: String,
                /// Authenticator sign counter.
                pub sign_count: i64,
                /// Serialized transports.
                pub transports: Option<String>,
                /// Generic purpose namespace.
                pub purpose: String,
                /// Generic digest.
                pub digest: String,
                /// Generic expiry timestamp.
                pub expires_at: DateTime<Utc>,
                /// Generic creation timestamp.
                pub created_at: DateTime<Utc>,
                /// Generic attempted timestamp.
                pub attempted_at: DateTime<Utc>,
                /// Generic optional consume timestamp.
                pub used_at: Option<DateTime<Utc>>,
                /// Generic optional revocation timestamp.
                pub revoked_at: Option<DateTime<Utc>>,
                /// Generic ceremony kind.
                pub kind: String,
                /// Generic ceremony selector.
                pub selector: String,
                /// Generic ceremony payload.
                pub payload: String,
                /// Generic ceremony state.
                pub state: String,
                /// Generic lock reason.
                pub reason: Option<String>,
            }

            /// This fixture does not model relations.
            #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
            pub enum Relation {}

            /// Default active-model behavior.
            impl ActiveModelBehavior for ActiveModel {}
        }
    };
}

simple_entity!(original_sessions, "auth_sessions");
simple_entity!(original_accounts, "auth_accounts");
simple_entity!(original_passkeys, "auth_passkeys");
simple_entity!(original_tokens, "auth_tokens");
simple_entity!(original_ceremonies, "auth_ceremonies");
simple_entity!(original_lockouts, "auth_lockouts");
simple_entity!(original_token_records, "auth_token_records");
simple_entity!(renamed_sessions, "auth_sessions");
simple_entity!(renamed_accounts, "auth_accounts");
simple_entity!(renamed_passkeys, "auth_passkeys");
simple_entity!(renamed_tokens, "auth_tokens");
simple_entity!(renamed_ceremonies, "auth_ceremonies");
simple_entity!(renamed_lockouts, "auth_lockouts");
simple_entity!(renamed_token_records, "auth_token_records");

impl EntityBinding for original_users::Entity {
    type Entity = original_users::Entity;
    type Column = original_users::Column;
    type PrimaryKey = original_users::PrimaryKey;
    type Model = original_users::Model;
    type ActiveModel = original_users::ActiveModel;
}

impl EntityBinding for renamed_users::Entity {
    type Entity = renamed_users::Entity;
    type Column = renamed_users::Column;
    type PrimaryKey = renamed_users::PrimaryKey;
    type Model = renamed_users::Model;
    type ActiveModel = renamed_users::ActiveModel;
}

macro_rules! bind_simple_entity {
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

bind_simple_entity!(original_sessions);
bind_simple_entity!(original_accounts);
bind_simple_entity!(original_passkeys);
bind_simple_entity!(original_tokens);
bind_simple_entity!(original_ceremonies);
bind_simple_entity!(original_lockouts);
bind_simple_entity!(original_token_records);
bind_simple_entity!(renamed_sessions);
bind_simple_entity!(renamed_accounts);
bind_simple_entity!(renamed_passkeys);
bind_simple_entity!(renamed_tokens);
bind_simple_entity!(renamed_ceremonies);
bind_simple_entity!(renamed_lockouts);
bind_simple_entity!(renamed_token_records);

impl UserFields for original_users::Entity {
    fn read_user_id(model: &Self::Model) -> String {
        model.id.to_string()
    }
    fn user_id_column() -> Self::Column {
        original_users::Column::Id
    }
    fn write_user_id(model: &mut Self::ActiveModel, value: &str) {
        model.id = Set(value.parse().expect("fixture user ids are i64"));
    }
    fn read_email(model: &Self::Model) -> String {
        model.email.clone()
    }
    fn email_column() -> Self::Column {
        original_users::Column::Email
    }
    fn write_email(model: &mut Self::ActiveModel, value: &str) {
        model.email = Set(value.to_owned());
    }
    fn read_password_hash(model: &Self::Model) -> Option<String> {
        model.password_hash.clone()
    }
    fn password_hash_column() -> Self::Column {
        original_users::Column::PasswordHash
    }
    fn write_password_hash(model: &mut Self::ActiveModel, value: Option<&str>) {
        model.password_hash = Set(value.map(str::to_owned));
    }
    fn read_locked_at(model: &Self::Model) -> Option<DateTime<Utc>> {
        model.locked_at
    }
    fn write_locked_at(model: &mut Self::ActiveModel, value: Option<DateTime<Utc>>) {
        model.locked_at = Set(value);
    }
}
impl UserFields for renamed_users::Entity {
    fn read_user_id(model: &Self::Model) -> String {
        model.id.to_string()
    }
    fn user_id_column() -> Self::Column {
        renamed_users::Column::Id
    }
    fn write_user_id(model: &mut Self::ActiveModel, value: &str) {
        model.id = Set(value.parse().expect("fixture user ids are i64"));
    }
    fn read_email(model: &Self::Model) -> String {
        model.login_email.clone()
    }
    fn email_column() -> Self::Column {
        renamed_users::Column::LoginEmail
    }
    fn write_email(model: &mut Self::ActiveModel, value: &str) {
        model.login_email = Set(value.to_owned());
    }
    fn read_password_hash(model: &Self::Model) -> Option<String> {
        match model.password_hash.as_str() {
            "" => None,
            hash => Some(hash.to_owned()),
        }
    }
    fn password_hash_column() -> Self::Column {
        renamed_users::Column::PasswordHash
    }
    fn write_password_hash(model: &mut Self::ActiveModel, value: Option<&str>) {
        model.password_hash = Set(value.unwrap_or("").to_owned());
    }
    fn read_locked_at(model: &Self::Model) -> Option<DateTime<Utc>> {
        model.locked_at
    }
    fn write_locked_at(model: &mut Self::ActiveModel, value: Option<DateTime<Utc>>) {
        model.locked_at = Set(value);
    }
}
impl UserOptionalFields for original_users::Entity {
    fn read_name(model: &Self::Model) -> Option<String> {
        model.name.clone()
    }
    fn read_email_verified_at(model: &Self::Model) -> Option<DateTime<Utc>> {
        model.email_verified_at
    }
    fn write_email_verified_at(model: &mut Self::ActiveModel, value: Option<DateTime<Utc>>) {
        model.email_verified_at = Set(value);
    }
    fn read_remember_token(model: &Self::Model) -> Option<String> {
        model.remember_token.clone()
    }
    fn write_remember_token(model: &mut Self::ActiveModel, value: Option<&str>) {
        model.remember_token = Set(value.map(str::to_owned));
    }
}

impl UserOptionalFields for renamed_users::Entity {
    fn read_name(model: &Self::Model) -> Option<String> {
        model.name.clone()
    }
    fn read_email_verified_at(model: &Self::Model) -> Option<DateTime<Utc>> {
        model.email_verified_at
    }
    fn write_email_verified_at(model: &mut Self::ActiveModel, value: Option<DateTime<Utc>>) {
        model.email_verified_at = Set(value);
    }
    fn read_remember_token(model: &Self::Model) -> Option<String> {
        model.remember_token.clone()
    }
    fn write_remember_token(model: &mut Self::ActiveModel, value: Option<&str>) {
        model.remember_token = Set(value.map(str::to_owned));
    }
}
impl SessionEpoch for original_users::Entity {
    fn auth_epoch(model: &Self::Model) -> u64 {
        model.auth_epoch as u64
    }
    fn auth_epoch_column() -> Self::Column {
        original_users::Column::AuthEpoch
    }
    fn write_auth_epoch(model: &mut Self::ActiveModel, value: u64) {
        model.auth_epoch = Set(value as i64);
    }
}
impl SessionEpoch for renamed_users::Entity {
    fn auth_epoch(model: &Self::Model) -> u64 {
        model.session_version as u64
    }
    fn auth_epoch_column() -> Self::Column {
        renamed_users::Column::SessionVersion
    }
    fn write_auth_epoch(model: &mut Self::ActiveModel, value: u64) {
        model.session_version = Set(value as i64);
    }
}
macro_rules! impl_role_fields {
    ($module:ident) => {
        impl SessionFields for $module::Entity {
            fn read_session_id(model: &Self::Model) -> String {
                model.id.to_string()
            }
            fn session_id_column() -> Self::Column {
                $module::Column::Id
            }
            fn read_user_id(model: &Self::Model) -> String {
                model.user_id.to_string()
            }
            fn user_id_column() -> Self::Column {
                $module::Column::UserId
            }
            fn read_token_digest(model: &Self::Model) -> String {
                model.digest.clone()
            }
            fn read_expires_at(model: &Self::Model) -> DateTime<Utc> {
                model.expires_at
            }
            fn read_revoked_at(model: &Self::Model) -> Option<DateTime<Utc>> {
                model.revoked_at
            }
            fn revoked_at_column() -> Self::Column {
                $module::Column::RevokedAt
            }
            fn write_revoked_at(model: &mut Self::ActiveModel, value: Option<DateTime<Utc>>) {
                model.revoked_at = Set(value);
            }
        }

        impl LinkedAccountFields for $module::Entity {
            fn read_account_id(model: &Self::Model) -> String {
                model.id.to_string()
            }
            fn account_id_column() -> Self::Column {
                $module::Column::Id
            }
            fn write_account_id(model: &mut Self::ActiveModel, value: &str) {
                model.id = Set(value.parse().expect("fixture account ids are i64"));
            }
            fn read_user_id(model: &Self::Model) -> String {
                model.user_id.to_string()
            }
            fn user_id_column() -> Self::Column {
                $module::Column::UserId
            }
            fn write_user_id(model: &mut Self::ActiveModel, value: &str) {
                model.user_id = Set(value.parse().expect("fixture user ids are i64"));
            }
            fn read_provider(model: &Self::Model) -> String {
                model.provider.clone()
            }
            fn provider_column() -> Self::Column {
                $module::Column::Provider
            }
            fn write_provider(model: &mut Self::ActiveModel, value: &str) {
                model.provider = Set(value.to_owned());
            }
            fn read_provider_account_id(model: &Self::Model) -> String {
                model.provider_account_id.clone()
            }
            fn provider_account_id_column() -> Self::Column {
                $module::Column::ProviderAccountId
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
            fn read_expires_at(model: &Self::Model) -> Option<DateTime<Utc>> {
                Some(model.expires_at)
            }
        }

        impl PasskeyFields for $module::Entity {
            fn read_passkey_id(model: &Self::Model) -> String {
                model.id.to_string()
            }
            fn passkey_id_column() -> Self::Column {
                $module::Column::Id
            }
            fn write_passkey_id(model: &mut Self::ActiveModel, value: &str) {
                model.id = Set(value.parse().expect("fixture passkey ids are i64"));
            }
            fn read_user_id(model: &Self::Model) -> String {
                model.user_id.to_string()
            }
            fn user_id_column() -> Self::Column {
                $module::Column::UserId
            }
            fn write_user_id(model: &mut Self::ActiveModel, value: &str) {
                model.user_id = Set(value.parse().unwrap_or_default());
            }
            fn read_credential_id(model: &Self::Model) -> String {
                model.credential_id.clone()
            }
            fn credential_id_column() -> Self::Column {
                $module::Column::CredentialId
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
            fn read_created_at(model: &Self::Model) -> DateTime<Utc> {
                model.created_at
            }
        }

        impl TokenFields for $module::Entity {
            fn read_token_id(model: &Self::Model) -> String {
                model.id.to_string()
            }
            fn token_id_column() -> Self::Column {
                $module::Column::Id
            }
            fn read_user_id(model: &Self::Model) -> Option<String> {
                Some(model.user_id.to_string())
            }
            fn user_id_column() -> Self::Column {
                $module::Column::UserId
            }
            fn read_purpose(model: &Self::Model) -> String {
                model.purpose.clone()
            }
            fn purpose_column() -> Self::Column {
                $module::Column::Purpose
            }
            fn purpose_column_name() -> &'static str {
                "purpose"
            }
            fn read_digest(model: &Self::Model) -> String {
                model.digest.clone()
            }
            fn digest_column() -> Self::Column {
                $module::Column::Digest
            }
            fn digest_column_name() -> &'static str {
                "digest"
            }
            fn read_expires_at(model: &Self::Model) -> DateTime<Utc> {
                model.expires_at
            }
            fn expires_at_column() -> Self::Column {
                $module::Column::ExpiresAt
            }
            fn read_used_at(model: &Self::Model) -> Option<DateTime<Utc>> {
                model.used_at
            }
            fn used_at_column() -> Self::Column {
                $module::Column::UsedAt
            }
            fn used_at_column_name() -> &'static str {
                "used_at"
            }
            fn write_used_at(model: &mut Self::ActiveModel, value: Option<DateTime<Utc>>) {
                model.used_at = Set(value);
            }
            fn write_token_id(model: &mut Self::ActiveModel, value: &str) {
                model.id = Set(value.parse().expect("fixture token ids are i64"));
            }
            fn write_user_id(model: &mut Self::ActiveModel, value: Option<&str>) {
                model.user_id = Set(value
                    .unwrap_or("0")
                    .parse()
                    .expect("fixture user ids are i64"));
            }
            fn write_purpose(model: &mut Self::ActiveModel, value: &str) {
                model.purpose = Set(value.to_owned());
            }
            fn write_digest(model: &mut Self::ActiveModel, value: &str) {
                model.digest = Set(value.to_owned());
            }
            fn write_expires_at(model: &mut Self::ActiveModel, value: DateTime<Utc>) {
                model.expires_at = Set(value);
            }
        }

        impl CeremonyFields for $module::Entity {
            fn read_ceremony_id(model: &Self::Model) -> String {
                model.id.to_string()
            }
            fn ceremony_id_column() -> Self::Column {
                $module::Column::Id
            }
            fn read_kind(model: &Self::Model) -> String {
                model.kind.clone()
            }
            fn kind_column() -> Self::Column {
                $module::Column::Kind
            }
            fn kind_column_name() -> &'static str {
                "kind"
            }
            fn read_selector(model: &Self::Model) -> String {
                model.selector.clone()
            }
            fn selector_column() -> Self::Column {
                $module::Column::Selector
            }
            fn selector_column_name() -> &'static str {
                "selector"
            }
            fn read_payload(model: &Self::Model) -> Vec<u8> {
                model.payload.as_bytes().to_vec()
            }
            fn read_state(model: &Self::Model) -> String {
                model.state.clone()
            }
            fn state_column() -> Self::Column {
                $module::Column::State
            }
            fn state_column_name() -> &'static str {
                "state"
            }
            fn read_expires_at(model: &Self::Model) -> DateTime<Utc> {
                model.expires_at
            }
            fn expires_at_column() -> Self::Column {
                $module::Column::ExpiresAt
            }
            fn read_used_at(model: &Self::Model) -> Option<DateTime<Utc>> {
                model.used_at
            }
            fn used_at_column() -> Self::Column {
                $module::Column::UsedAt
            }
            fn write_state(model: &mut Self::ActiveModel, state: &str) {
                model.state = Set(state.to_owned());
            }
            fn write_used_at(model: &mut Self::ActiveModel, value: Option<DateTime<Utc>>) {
                model.used_at = Set(value);
            }
            fn write_ceremony_id(model: &mut Self::ActiveModel, value: &str) {
                model.id = Set(value.parse().expect("fixture ceremony ids are i64"));
            }
            fn write_kind(model: &mut Self::ActiveModel, value: &str) {
                model.kind = Set(value.to_owned());
            }
            fn write_selector(model: &mut Self::ActiveModel, value: &str) {
                model.selector = Set(value.to_owned());
            }
            fn write_payload(model: &mut Self::ActiveModel, value: &[u8]) {
                model.payload =
                    Set(String::from_utf8(value.to_vec()).expect("fixture payload must be UTF-8"));
            }
            fn write_expires_at(model: &mut Self::ActiveModel, value: DateTime<Utc>) {
                model.expires_at = Set(value);
            }
        }

        impl LockoutFields for $module::Entity {
            fn read_lockout_id(model: &Self::Model) -> String {
                model.id.to_string()
            }
            fn write_lockout_id(model: &mut Self::ActiveModel, value: &str) {
                model.id = Set(value.parse().expect("fixture lockout ids are i64"));
            }
            fn read_user_id(model: &Self::Model) -> String {
                model.user_id.to_string()
            }
            fn user_id_column() -> Self::Column {
                $module::Column::UserId
            }
            fn write_user_id(model: &mut Self::ActiveModel, value: &str) {
                model.user_id = Set(value.parse().unwrap_or_default());
            }
            fn read_attempted_at(model: &Self::Model) -> DateTime<Utc> {
                model.attempted_at
            }
            fn attempted_at_column() -> Self::Column {
                $module::Column::AttemptedAt
            }
            fn write_attempted_at(model: &mut Self::ActiveModel, value: DateTime<Utc>) {
                model.attempted_at = Set(value);
            }
            fn write_reason(model: &mut Self::ActiveModel, value: Option<&str>) {
                model.reason = Set(value.map(str::to_owned));
            }
            fn read_locked_at(model: &Self::Model) -> Option<DateTime<Utc>> {
                model.revoked_at
            }
            fn read_reason(model: &Self::Model) -> Option<String> {
                model.reason.clone()
            }
            fn write_locked_at(model: &mut Self::ActiveModel, value: Option<DateTime<Utc>>) {
                model.revoked_at = Set(value);
            }
        }

        impl TokenRecordFields for $module::Entity {
            fn read_record_id(model: &Self::Model) -> String {
                model.id.to_string()
            }
            fn read_token_id(model: &Self::Model) -> String {
                model.id.to_string()
            }
            fn read_user_id(model: &Self::Model) -> String {
                model.user_id.to_string()
            }
            fn read_purpose(model: &Self::Model) -> String {
                model.purpose.clone()
            }
            fn read_digest(model: &Self::Model) -> String {
                model.digest.clone()
            }
        }
    };
}

impl_role_fields!(original_sessions);
impl_role_fields!(original_accounts);
impl_role_fields!(original_passkeys);
impl_role_fields!(original_tokens);
impl_role_fields!(original_ceremonies);
impl_role_fields!(original_lockouts);
impl_role_fields!(original_token_records);
impl_role_fields!(renamed_sessions);
impl_role_fields!(renamed_accounts);
impl_role_fields!(renamed_passkeys);
impl_role_fields!(renamed_tokens);
impl_role_fields!(renamed_ceremonies);
impl_role_fields!(renamed_lockouts);
impl_role_fields!(renamed_token_records);
/// Schema bound to the original generated user entity.
#[derive(Clone, Copy, Debug, Default)]
pub struct OriginalSchema;

/// Schema bound to the renamed generated user entity.
#[derive(Clone, Copy, Debug, Default)]
pub struct RenamedSchema;

impl AuthSchema for OriginalSchema {
    type User = original_users::Entity;
    type Session = original_sessions::Entity;
    type LinkedAccount = original_accounts::Entity;
    type Passkey = original_passkeys::Entity;
    type Token = original_tokens::Entity;
    type Ceremony = original_ceremonies::Entity;
    type Lockout = original_lockouts::Entity;
    type TokenRecord = original_token_records::Entity;
}

impl AuthSchema for RenamedSchema {
    type User = renamed_users::Entity;
    type Session = renamed_sessions::Entity;
    type LinkedAccount = renamed_accounts::Entity;
    type Passkey = renamed_passkeys::Entity;
    type Token = renamed_tokens::Entity;
    type Ceremony = renamed_ceremonies::Entity;
    type Lockout = renamed_lockouts::Entity;
    type TokenRecord = renamed_token_records::Entity;
}

/// Construct one original-column fixture row.
pub fn original_user(email: &str, password_hash: Option<&str>) -> original_users::Model {
    original_users::Model {
        id: 1,
        email: email.to_owned(),
        name: None,
        email_verified_at: None,
        remember_token: None,
        password_hash: password_hash.map(str::to_owned),
        locked_at: None,
        auth_epoch: 7,
    }
}
/// Construct one renamed-column fixture row.
pub fn renamed_user(email: &str, password_hash: Option<&str>) -> renamed_users::Model {
    renamed_users::Model {
        name: None,
        email_verified_at: None,
        remember_token: None,
        id: 1,
        login_email: email.to_owned(),
        session_version: 7,
        locked_at: None,
        password_hash: password_hash
            .unwrap_or(NOT_NULL_PASSWORD_EMPTY_SENTINEL)
            .to_owned(),
    }
}

/// Build and populate the original-column SQLite fixture.
#[cfg(feature = "seaorm-sqlite")]
pub async fn original_fixture_db() -> DatabaseConnection {
    let db = Database::connect("sqlite::memory:").await.expect("sqlite");
    let schema = Schema::new(DbBackend::Sqlite);
    db.execute(Statement::from_string(
        DbBackend::Sqlite,
        schema
            .create_table_from_entity(original_users::Entity)
            .if_not_exists()
            .to_string(SqliteQueryBuilder),
    ))
    .await
    .expect("create original fixture");
    original_users::ActiveModel {
        id: Set(1),
        email: Set("original@example.test".to_owned()),
        name: Set(None),
        email_verified_at: Set(None),
        remember_token: Set(None),
        auth_epoch: Set(7),
        password_hash: Set(None),
        locked_at: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert original fixture");
    db
}

/// Build and populate the renamed-column SQLite fixture.
#[cfg(feature = "seaorm-sqlite")]
pub async fn renamed_fixture_db() -> DatabaseConnection {
    let db = Database::connect("sqlite::memory:").await.expect("sqlite");
    let schema = Schema::new(DbBackend::Sqlite);
    db.execute(Statement::from_string(
        DbBackend::Sqlite,
        schema
            .create_table_from_entity(renamed_users::Entity)
            .if_not_exists()
            .to_string(SqliteQueryBuilder),
    ))
    .await
    .expect("create renamed fixture");
    renamed_users::ActiveModel {
        id: Set(1),
        login_email: Set("renamed@example.test".to_owned()),
        name: Set(None),
        email_verified_at: Set(None),
        remember_token: Set(None),
        password_hash: Set(NOT_NULL_PASSWORD_EMPTY_SENTINEL.to_owned()),
        locked_at: Set(None),
        session_version: Set(7),
    }
    .insert(&db)
    .await
    .expect("insert renamed fixture");
    db
}
