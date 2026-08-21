//! Framework-owned Magnetar `AuthSchema` binding contract.
//!
//! This test compiles a Suprnova model and uses its generated SeaORM entity as
//! Magnetar's application-owned user descriptor. It persists and reloads a
//! passwordless account through the real Magnetar SeaORM storage adapter.

use magnetar::schema::{AuthSchema, EntityBinding, SessionEpoch, UserFields, UserOptionalFields};
use magnetar::storage::{NewUser, UserStore};
use sea_orm::{ActiveValue::Set, ConnectionTrait, Database};
use suprnova::magnetar_integration::engine::{MagnetarAuthStore, MagnetarBinding};
use suprnova::{Model as EloquentModel, model};

/// An application model whose generated SeaORM entity is bound to Magnetar.
#[model(table = "framework_magnetar_users", timestamps = false)]
pub struct FrameworkMagnetarBindingRecord {
    pub id: i64,
    pub login_email: String,
    pub password_hash: String,
    pub email_verified_at: Option<suprnova::chrono::DateTime<suprnova::chrono::Utc>>,
    pub locked_at: Option<suprnova::chrono::DateTime<suprnova::chrono::Utc>>,
    pub session_version: i64,
}

impl EntityBinding for framework_magnetar_binding_record::Entity {
    type Entity = framework_magnetar_binding_record::Entity;
    type Column = framework_magnetar_binding_record::Column;
    type PrimaryKey = framework_magnetar_binding_record::PrimaryKey;
    type Model = framework_magnetar_binding_record::Model;
    type ActiveModel = framework_magnetar_binding_record::ActiveModel;
}

// AuthSchema roles remain distinct application entities even where this
// focused binding test only exercises the user store. The host-engine test
// supplies the role-specific storage capabilities it executes at runtime.
#[model(table = "framework_magnetar_binding_sessions", timestamps = false)]
pub struct FrameworkMagnetarBindingSession {
    pub id: i64,
}
#[model(table = "framework_magnetar_binding_accounts", timestamps = false)]
pub struct FrameworkMagnetarBindingAccount {
    pub id: i64,
}
#[model(table = "framework_magnetar_binding_passkeys", timestamps = false)]
pub struct FrameworkMagnetarBindingPasskey {
    pub id: i64,
}
#[model(table = "framework_magnetar_binding_tokens", timestamps = false)]
pub struct FrameworkMagnetarBindingToken {
    pub id: i64,
}
#[model(table = "framework_magnetar_binding_ceremonies", timestamps = false)]
pub struct FrameworkMagnetarBindingCeremony {
    pub id: i64,
}
#[model(table = "framework_magnetar_binding_lockouts", timestamps = false)]
pub struct FrameworkMagnetarBindingLockout {
    pub id: i64,
}
#[model(table = "framework_magnetar_binding_token_records", timestamps = false)]
pub struct FrameworkMagnetarBindingTokenRecord {
    pub id: i64,
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

bind_entity!(framework_magnetar_binding_session);
bind_entity!(framework_magnetar_binding_account);
bind_entity!(framework_magnetar_binding_passkey);
bind_entity!(framework_magnetar_binding_token);
bind_entity!(framework_magnetar_binding_ceremony);
bind_entity!(framework_magnetar_binding_lockout);
bind_entity!(framework_magnetar_binding_token_record);

impl UserFields for framework_magnetar_binding_record::Entity {
    fn read_user_id(model: &Self::Model) -> String {
        model.id.to_string()
    }
    fn user_id_column() -> Self::Column {
        framework_magnetar_binding_record::Column::Id
    }
    fn write_user_id(model: &mut Self::ActiveModel, value: &str) {
        model.id = Set(value
            .parse()
            .expect("Magnetar storage generates i64-compatible IDs"));
    }
    fn read_email(model: &Self::Model) -> String {
        model.login_email.clone()
    }
    fn email_column() -> Self::Column {
        framework_magnetar_binding_record::Column::LoginEmail
    }
    fn write_email(model: &mut Self::ActiveModel, value: &str) {
        model.login_email = Set(value.to_owned());
    }
    fn read_password_hash(model: &Self::Model) -> Option<String> {
        (!model.password_hash.is_empty()).then(|| model.password_hash.clone())
    }
    fn password_hash_column() -> Self::Column {
        framework_magnetar_binding_record::Column::PasswordHash
    }
    fn write_password_hash(model: &mut Self::ActiveModel, value: Option<&str>) {
        model.password_hash = Set(value.unwrap_or_default().to_owned());
    }
    fn read_locked_at(
        model: &Self::Model,
    ) -> Option<suprnova::chrono::DateTime<suprnova::chrono::Utc>> {
        model
            .locked_at
            .as_deref()
            .and_then(|value| suprnova::chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&suprnova::chrono::Utc))
    }
    fn write_locked_at(
        model: &mut Self::ActiveModel,
        value: Option<suprnova::chrono::DateTime<suprnova::chrono::Utc>>,
    ) {
        model.locked_at = Set(value.map(|value| value.to_rfc3339()));
    }
}

impl UserOptionalFields for framework_magnetar_binding_record::Entity {
    fn read_name(_: &Self::Model) -> Option<String> {
        None
    }
    fn read_email_verified_at(
        model: &Self::Model,
    ) -> Option<suprnova::chrono::DateTime<suprnova::chrono::Utc>> {
        model
            .email_verified_at
            .as_deref()
            .and_then(|value| suprnova::chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&suprnova::chrono::Utc))
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

impl SessionEpoch for framework_magnetar_binding_record::Entity {
    fn auth_epoch(model: &Self::Model) -> u64 {
        model.session_version as u64
    }
    fn auth_epoch_column() -> Self::Column {
        framework_magnetar_binding_record::Column::SessionVersion
    }
    fn write_auth_epoch(model: &mut Self::ActiveModel, value: u64) {
        model.session_version = Set(value as i64);
    }
}

struct FrameworkAuthSchema;

impl AuthSchema for FrameworkAuthSchema {
    type User = framework_magnetar_binding_record::Entity;
    type Session = framework_magnetar_binding_session::Entity;
    type LinkedAccount = framework_magnetar_binding_account::Entity;
    type Passkey = framework_magnetar_binding_passkey::Entity;
    type Token = framework_magnetar_binding_token::Entity;
    type Ceremony = framework_magnetar_binding_ceremony::Entity;
    type Lockout = framework_magnetar_binding_lockout::Entity;
    type TokenRecord = framework_magnetar_binding_token_record::Entity;
}

#[tokio::test]
async fn framework_model_binding_persists_passwordless_user_through_magnetar_storage() {
    let framework_model = FrameworkMagnetarBindingRecord {
        id: 41,
        login_email: "eloquent-binding@example.test".to_owned(),
        password_hash: String::new(),
        email_verified_at: None,
        locked_at: None,
        session_version: 0,
        ..Default::default()
    };
    assert_eq!(
        framework_model.to_array()["login_email"],
        "eloquent-binding@example.test"
    );

    let connection = Database::connect("sqlite::memory:")
        .await
        .expect("connect application-owned SQLite database");
    connection
        .execute_unprepared(
            "CREATE TABLE framework_magnetar_users (
                id INTEGER PRIMARY KEY NOT NULL,
                login_email TEXT NOT NULL,
                password_hash TEXT NOT NULL,
                email_verified_at TEXT,
                locked_at TEXT,
                session_version INTEGER NOT NULL
            )",
        )
        .await
        .expect("create application-owned auth users table");

    let binding = MagnetarBinding::<FrameworkAuthSchema>::new(connection);
    let created = binding
        .storage()
        .create_user(NewUser {
            email: "passwordless@example.test".to_owned(),
            password_hash: None,
        })
        .await
        .expect("persist passwordless user through Magnetar's framework binding");
    assert_eq!(created.email, "passwordless@example.test");
    assert_eq!(created.password_hash, None);
    assert_eq!(created.auth_epoch, 0);

    let reloaded = binding
        .storage()
        .find_by_email("passwordless@example.test")
        .await
        .expect("query generated renamed email column")
        .expect("created user must be readable through the framework binding");
    assert_eq!(reloaded, created);
}
