//! The default schema's byte and long-text columns on every engine.
//!
//! On MySQL and MariaDB an entity `Vec<u8>` field without a column type is
//! created as `varbinary(255)` and a `String` field as `varchar(255)`, and
//! strict mode refuses a longer write. These tests write values past those
//! sizes through every store that owns such a column, upgrade a database
//! that holds the narrow layout `migrate` created before, and run two
//! migrations of one fresh database at once.
//!
//! SQLite runs in the default suite. PostgreSQL and MySQL/MariaDB are
//! manual, ignored qualification tests against the live backends named by
//! `MAGNETAR_POSTGRES_TEST_URL` and `MAGNETAR_MYSQL_TEST_URL`. Each creates a
//! database of its own and drops it at the end, so repeated runs against
//! one server never collide.

#![cfg(all(
    feature = "two-factor",
    any(
        feature = "seaorm-sqlite",
        feature = "seaorm-postgres",
        feature = "seaorm-mysql"
    )
))]

use std::sync::Arc;

use chrono::{Duration, Timelike, Utc};
use magnetar::default_schema::sql_stores::SqlSessionStore;
use magnetar::default_schema::sql_two_factor::SqlTwoFactorStore;
use magnetar::default_schema::{
    DefaultAuthSchema, ceremonies, methods, provider_tokens, sessions, two_factor,
};
use magnetar::sessions::{
    OpaqueConfig, OpaqueSessionProvider, OpaqueSessionStore, SessionMetadata, SessionQueries,
    StoredSession,
};
use magnetar::storage::{
    CeremonyStore, CommitProviderToken, CredentialActor, NewCeremony, NewProviderToken, NewUser,
    PasskeyStore, ProviderTokenStore, SeaOrmStorage, UserStore,
};
use magnetar::two_factor::{TwoFactorProofClaim, TwoFactorStore};
use sea_orm::{
    ActiveModelTrait, ConnectionTrait, Database, DatabaseConnection, DbBackend, EntityTrait,
    Schema, Set, Statement,
};
use sha2::{Digest, Sha256};

#[cfg(any(feature = "seaorm-postgres", feature = "seaorm-mysql"))]
#[path = "fixtures/live_database.rs"]
mod live_database;

#[cfg(any(feature = "seaorm-postgres", feature = "seaorm-mysql"))]
use live_database::LiveDatabase;

/// One table as an earlier release of `migrate` declared it.
macro_rules! narrow_entity {
    ($module:ident, $table:literal, { $($field:tt)* }) => {
        pub mod $module {
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

/// The tables as `migrate` declared them before their byte and long-text
/// columns carried a column type. On MySQL and MariaDB they render the
/// narrow `varbinary(255)` and `varchar(255)` columns an existing
/// application database holds.
mod narrow_layout {
    narrow_entity!(sessions, "auth_sessions", {
        #[sea_orm(primary_key, auto_increment = false)] pub id: String,
        pub user_id: i64,
        pub auth_epoch: i64,
        pub token_digest: String,
        pub token_hash: Option<String>,
        pub user_agent: Option<String>,
        pub ip_address: Option<String>,
        pub expires_at: ChronoDateTimeUtc,
        pub revoked_at: Option<ChronoDateTimeUtc>,
    });
    narrow_entity!(methods, "auth_methods", {
        #[sea_orm(primary_key)] pub id: i64,
        pub user_id: i64,
        pub credential_id: Option<String>,
        pub public_key: Option<String>,
        pub created_at: Option<ChronoDateTimeUtc>,
    });
    narrow_entity!(ceremonies, "auth_ceremonies", {
        #[sea_orm(primary_key)] pub id: i64,
        pub kind: String,
        pub selector: String,
        pub payload: Vec<u8>,
        pub state: String,
        pub expires_at: ChronoDateTimeUtc,
        pub used_at: Option<ChronoDateTimeUtc>,
    });
    narrow_entity!(two_factor, "auth_two_factor", {
        #[sea_orm(primary_key, auto_increment = false)] pub user_id: String,
        pub secret: Vec<u8>,
        pub recovery_codes: Option<Vec<u8>>,
        pub enrollment_auth_epoch: i64,
        pub enrollment_session_id: Option<String>,
        pub enrollment_expires_at: Option<ChronoDateTimeUtc>,
        pub rotation_pending: bool,
        pub pending_secret: Option<Vec<u8>>,
        pub pending_recovery_codes: Option<Vec<u8>>,
        pub confirmed_at: Option<ChronoDateTimeUtc>,
        pub last_used_timestep: Option<i64>,
        pub created_at: Option<ChronoDateTimeUtc>,
        pub updated_at: Option<ChronoDateTimeUtc>,
    });
    narrow_entity!(provider_tokens, "auth_provider_tokens", {
        #[sea_orm(primary_key, auto_increment = false)] pub id: String,
        pub provider: String,
        pub access_ciphertext: Vec<u8>,
        pub refresh_ciphertext: Option<Vec<u8>>,
        pub raw_payload_ciphertext: Vec<u8>,
        pub token_type: String,
        pub scopes: String,
        pub access_expires_at: Option<ChronoDateTimeUtc>,
        pub generation: i64,
        pub claim_id: Option<String>,
        pub claim_deadline: Option<ChronoDateTimeUtc>,
        pub revoked_at: Option<ChronoDateTimeUtc>,
        pub revoked_reused: Option<bool>,
        pub created_at: ChronoDateTimeUtc,
    });
}

/// Six tables as an earlier release created them, before `migrate` added
/// the authentication-epoch, enrolment-snapshot, pending-rotation, lockout
/// audit and timestamp columns. Upgrading them runs every add-column step.
mod older_layout {
    narrow_entity!(sessions, "auth_sessions", {
        #[sea_orm(primary_key, auto_increment = false)] pub id: String,
        pub user_id: i64,
        pub token_digest: String,
        pub token_hash: Option<String>,
        pub user_agent: Option<String>,
        pub ip_address: Option<String>,
        pub expires_at: ChronoDateTimeUtc,
        pub revoked_at: Option<ChronoDateTimeUtc>,
    });
    narrow_entity!(remembers, "auth_remember_tokens", {
        #[sea_orm(primary_key, auto_increment = false)] pub id: String,
        pub selector: String,
        pub user_id: String,
        pub verifier_hash: String,
        pub expires_at: ChronoDateTimeUtc,
    });
    narrow_entity!(two_factor, "auth_two_factor", {
        #[sea_orm(primary_key, auto_increment = false)] pub user_id: String,
        pub secret: Vec<u8>,
        pub recovery_codes: Option<Vec<u8>>,
        pub confirmed_at: Option<ChronoDateTimeUtc>,
        pub last_used_timestep: Option<i64>,
    });
    narrow_entity!(lockouts, "auth_lockouts", {
        #[sea_orm(primary_key)] pub id: i64,
        pub identity: String,
        pub attempted_at: ChronoDateTimeUtc,
        pub locked_at: Option<ChronoDateTimeUtc>,
        pub reason: Option<String>,
    });
    narrow_entity!(tokens, "auth_tokens", {
        #[sea_orm(primary_key)] pub id: i64,
        pub user_id: Option<i64>,
        pub purpose: String,
        pub digest: String,
        pub expires_at: ChronoDateTimeUtc,
        pub used_at: Option<ChronoDateTimeUtc>,
    });
    narrow_entity!(accounts, "auth_linked_accounts", {
        #[sea_orm(primary_key)] pub id: i64,
        pub user_id: i64,
        pub provider: String,
        pub provider_account_id: String,
    });
}

/// The columns `migrate` adds to the older layout, which every upgrade
/// must leave in place.
const ADDED_COLUMNS: [(&str, &str); 14] = [
    ("auth_sessions", "auth_epoch"),
    ("auth_remember_tokens", "auth_epoch"),
    ("auth_two_factor", "enrollment_auth_epoch"),
    ("auth_two_factor", "enrollment_session_id"),
    ("auth_two_factor", "enrollment_expires_at"),
    ("auth_two_factor", "rotation_pending"),
    ("auth_two_factor", "pending_secret"),
    ("auth_two_factor", "pending_recovery_codes"),
    ("auth_lockouts", "ip_address"),
    ("auth_lockouts", "migration_source_id"),
    ("auth_tokens", "created_at"),
    ("auth_tokens", "updated_at"),
    ("auth_linked_accounts", "created_at"),
    ("auth_linked_accounts", "updated_at"),
];

/// The index names `migrate` creates; a concurrent run must leave each one.
const DEFAULT_INDEXES: [&str; 5] = [
    "auth_linked_accounts_provider_subject",
    "auth_lockouts_migration_source",
    "auth_lockouts_identity",
    "app_users_email_unique",
    "auth_second_factor_lockouts_identity",
];

fn unique_suffix() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock is after the epoch")
        .as_nanos();
    format!("{nanos}-{:016x}", rand::random::<u64>())
}

/// `len` bytes that cover every byte value, zero and 0xff included.
fn long_bytes(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|index| (index as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

fn long_text(len: usize, seed: char) -> String {
    std::iter::repeat_n(seed, len).collect()
}

fn whole_second(at: chrono::DateTime<Utc>) -> chrono::DateTime<Utc> {
    at.with_nanosecond(0).expect("whole-second timestamp")
}

async fn create_user(db: &DatabaseConnection, label: &str) -> String {
    SeaOrmStorage::<DefaultAuthSchema>::new(db.clone())
        .create_user(NewUser {
            email: format!("columns-{label}-{}@example.test", unique_suffix()),
            password_hash: Some("fixture-hash".to_owned()),
        })
        .await
        .expect("create an application user")
        .user_id
}

/// Write a value past the old narrow sizes through every store that owns a
/// byte or long-text column of the default schema, then read it back.
async fn assert_long_values_round_trip(db: &DatabaseConnection, label: &str) {
    let user_id = create_user(db, label).await;
    let now = whole_second(Utc::now());

    // A session whose user agent is 400 characters.
    let token = format!("columns-token-{}", unique_suffix());
    let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    let user_agent = format!("Mozilla/5.0 {}", long_text(388, 'u'));
    let session_store = Arc::new(SqlSessionStore(db.clone()));
    session_store
        .insert_session_if_epoch_current(StoredSession {
            session_id: rand::random::<u32>().to_string(),
            user_id: user_id.clone(),
            auth_epoch: 0,
            token_hash: digest,
            token_digest: digest,
            expires_at: now + Duration::days(1),
            revoked_at: None,
            metadata: SessionMetadata {
                user_agent: Some(user_agent.clone()),
                ip_address: Some("203.0.113.9".to_owned()),
            },
        })
        .await
        .expect("a session with a 400-character user agent is stored");
    let stored = session_store
        .find_by_token_hash(digest)
        .await
        .expect("read the session back")
        .expect("the session exists");
    assert_eq!(
        stored.metadata.user_agent.as_deref(),
        Some(user_agent.as_str())
    );
    let session = OpaqueSessionProvider::new(session_store, OpaqueConfig::default())
        .verify_bearer(&token)
        .await
        .expect("the stored session verifies");
    let actor = CredentialActor::from_session(&session);

    // A ceremony whose payload is 4 KiB, created and consumed.
    let storage = SeaOrmStorage::<DefaultAuthSchema>::new(db.clone());
    let selector = format!("columns-{label}-{}", unique_suffix());
    let payload = long_bytes(4096, 1);
    storage
        .create(NewCeremony {
            selector: selector.clone(),
            kind: "columns-ceremony".to_owned(),
            state: "pending".to_owned(),
            payload: payload.clone(),
            expires_at: now + Duration::minutes(5),
        })
        .await
        .expect("a ceremony with a 4 KiB payload is stored");
    let consumed = storage
        .consume(&selector, "columns-ceremony")
        .await
        .expect("consume the ceremony")
        .expect("the ceremony is live");
    assert_eq!(consumed.payload, payload);

    // A two-factor enrolment with recovery codes, confirmed, then a
    // rotation waiting in the pending columns.
    let two_factor_store = SqlTwoFactorStore(db.clone());
    let secret = long_bytes(300, 2);
    let recovery_codes = long_bytes(2048, 3);
    let pending_secret = long_bytes(320, 4);
    let pending_recovery_codes = long_bytes(2100, 5);
    assert!(
        two_factor_store
            .begin_enrollment(&actor, &secret, Some(&recovery_codes))
            .await
            .expect("an enrolment with long ciphertexts is stored")
    );
    assert!(
        two_factor_store
            .set_confirmed(&actor, &secret, 1, now)
            .await
            .expect("confirm the enrolment")
    );
    assert!(
        two_factor_store
            .rotate_enrollment(
                &actor,
                TwoFactorProofClaim::Totp { matched_step: 2 },
                &pending_secret,
                Some(&pending_recovery_codes),
            )
            .await
            .expect("a rotation with long ciphertexts is stored")
    );
    let enrolment = two_factor_store
        .find_enrollment(&user_id)
        .await
        .expect("read the enrolment back")
        .expect("the enrolment exists");
    assert_eq!(enrolment.secret, secret);
    assert_eq!(enrolment.recovery_codes, Some(recovery_codes));
    assert_eq!(enrolment.pending_secret, Some(pending_secret));
    assert_eq!(
        enrolment.pending_recovery_codes,
        Some(pending_recovery_codes)
    );

    // A passkey with a maximum-length credential id (1023 bytes, base64)
    // and a serialized envelope of several kilobytes.
    let credential_id = format!("{}{}", long_text(1360, 'c'), &unique_suffix()[..4]);
    let envelope = format!("{{\"cred\":\"{}\"}}", long_text(4000, 'e'));
    storage
        .insert_passkey(&actor, &credential_id, &envelope)
        .await
        .expect("a passkey with a long credential id and envelope is stored");
    let passkey = storage
        .find_user_by_credential(&credential_id)
        .await
        .expect("read the passkey back")
        .expect("the passkey exists");
    assert_eq!(passkey.credential_id, credential_id);
    assert_eq!(passkey.envelope_json, envelope);

    // A provider token whose raw payload passes 64 KiB, with a long scope
    // list, committed under a claim and read back.
    let record_id = format!("columns-{label}-{}", unique_suffix());
    let access = long_bytes(9000, 6);
    let refresh = long_bytes(3000, 7);
    let raw_payload = long_bytes(100 * 1024, 8);
    let scopes = (0..12)
        .map(|index| format!("https://www.googleapis.com/auth/columns.scope.{index}"))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(scopes.len() > 255);
    storage
        .create_if_missing(NewProviderToken {
            id: record_id.clone(),
            provider: "mock".to_owned(),
        })
        .await
        .expect("provision the provider token");
    assert!(
        storage
            .claim(
                &record_id,
                0,
                "columns-claim",
                now + Duration::seconds(30),
                now
            )
            .await
            .expect("claim the provider token")
    );
    assert!(
        storage
            .commit(
                &record_id,
                "columns-claim",
                0,
                CommitProviderToken {
                    access_ciphertext: access.clone(),
                    refresh_ciphertext: Some(refresh.clone()),
                    raw_payload_ciphertext: raw_payload.clone(),
                    token_type: "Bearer".to_owned(),
                    scopes: scopes.clone(),
                    access_expires_at: Some(now + Duration::hours(1)),
                    new_generation: 1,
                },
            )
            .await
            .expect("a provider token with a long raw payload is committed")
    );
    let token_row = storage
        .read(&record_id)
        .await
        .expect("read the provider token back")
        .expect("the provider token exists");
    assert_eq!(token_row.access_ciphertext, access);
    assert_eq!(token_row.refresh_ciphertext, Some(refresh));
    assert_eq!(token_row.raw_payload_ciphertext, raw_payload);
    assert_eq!(token_row.scopes, scopes);
}

/// The rows an application stored in the narrow layout before upgrading.
struct NarrowRows {
    session_id: String,
    passkey_id: i64,
    ceremony_id: i64,
    two_factor_user: String,
    provider_token_id: String,
}

/// Create the five tables that hold byte or long-text columns in the
/// narrow layout and store one row in each.
async fn create_narrow_layout(db: &DatabaseConnection) -> NarrowRows {
    let backend = db.get_database_backend();
    let schema = Schema::new(backend);
    for statement in [
        schema.create_table_from_entity(narrow_layout::sessions::Entity),
        schema.create_table_from_entity(narrow_layout::methods::Entity),
        schema.create_table_from_entity(narrow_layout::ceremonies::Entity),
        schema.create_table_from_entity(narrow_layout::two_factor::Entity),
        schema.create_table_from_entity(narrow_layout::provider_tokens::Entity),
    ] {
        db.execute(&statement)
            .await
            .expect("create a narrow-layout table");
    }
    let now = whole_second(Utc::now());
    narrow_layout::sessions::ActiveModel {
        id: Set("narrow-session".to_owned()),
        user_id: Set(1),
        auth_epoch: Set(0),
        token_digest: Set("ab".repeat(32)),
        token_hash: Set(Some("ab".repeat(32))),
        user_agent: Set(Some("narrow user agent".to_owned())),
        ip_address: Set(Some("198.51.100.4".to_owned())),
        expires_at: Set(now + Duration::days(1)),
        revoked_at: Set(None),
    }
    .insert(db)
    .await
    .expect("store a narrow-layout session");
    let passkey = narrow_layout::methods::ActiveModel {
        user_id: Set(1),
        credential_id: Set(Some("narrow-credential".to_owned())),
        public_key: Set(Some("{\"narrow\":true}".to_owned())),
        created_at: Set(Some(now)),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("store a narrow-layout passkey");
    let ceremony = narrow_layout::ceremonies::ActiveModel {
        kind: Set("narrow-kind".to_owned()),
        selector: Set("narrow-selector".to_owned()),
        payload: Set(long_bytes(200, 9)),
        state: Set("pending".to_owned()),
        expires_at: Set(now + Duration::minutes(5)),
        used_at: Set(None),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("store a narrow-layout ceremony");
    narrow_layout::two_factor::ActiveModel {
        user_id: Set("narrow-user".to_owned()),
        secret: Set(long_bytes(200, 10)),
        recovery_codes: Set(Some(long_bytes(250, 11))),
        enrollment_auth_epoch: Set(0),
        enrollment_session_id: Set(None),
        enrollment_expires_at: Set(None),
        rotation_pending: Set(false),
        pending_secret: Set(None),
        pending_recovery_codes: Set(None),
        confirmed_at: Set(Some(now)),
        last_used_timestep: Set(Some(7)),
        created_at: Set(Some(now)),
        updated_at: Set(Some(now)),
    }
    .insert(db)
    .await
    .expect("store a narrow-layout enrolment");
    narrow_layout::provider_tokens::ActiveModel {
        id: Set("narrow-provider-token".to_owned()),
        provider: Set("mock".to_owned()),
        access_ciphertext: Set(long_bytes(240, 12)),
        refresh_ciphertext: Set(None),
        raw_payload_ciphertext: Set(long_bytes(250, 13)),
        token_type: Set("Bearer".to_owned()),
        scopes: Set("openid email".to_owned()),
        access_expires_at: Set(None),
        generation: Set(3),
        claim_id: Set(None),
        claim_deadline: Set(None),
        revoked_at: Set(None),
        revoked_reused: Set(None),
        created_at: Set(now),
    }
    .insert(db)
    .await
    .expect("store a narrow-layout provider token");
    NarrowRows {
        session_id: "narrow-session".to_owned(),
        passkey_id: passkey.id,
        ceremony_id: ceremony.id,
        two_factor_user: "narrow-user".to_owned(),
        provider_token_id: "narrow-provider-token".to_owned(),
    }
}

/// The narrow-layout rows, read through the current entities, are the rows
/// that were stored.
async fn assert_narrow_rows_kept(db: &DatabaseConnection, rows: &NarrowRows) {
    let session = sessions::Entity::find_by_id(rows.session_id.clone())
        .one(db)
        .await
        .expect("read the session")
        .expect("the session survives the upgrade");
    assert_eq!(session.user_agent.as_deref(), Some("narrow user agent"));
    assert_eq!(session.ip_address.as_deref(), Some("198.51.100.4"));
    let passkey = methods::Entity::find_by_id(rows.passkey_id)
        .one(db)
        .await
        .expect("read the passkey")
        .expect("the passkey survives the upgrade");
    assert_eq!(passkey.credential_id.as_deref(), Some("narrow-credential"));
    assert_eq!(passkey.public_key.as_deref(), Some("{\"narrow\":true}"));
    let ceremony = ceremonies::Entity::find_by_id(rows.ceremony_id)
        .one(db)
        .await
        .expect("read the ceremony")
        .expect("the ceremony survives the upgrade");
    assert_eq!(ceremony.payload, long_bytes(200, 9));
    let enrolment = two_factor::Entity::find_by_id(rows.two_factor_user.clone())
        .one(db)
        .await
        .expect("read the enrolment")
        .expect("the enrolment survives the upgrade");
    assert_eq!(enrolment.secret, long_bytes(200, 10));
    assert_eq!(enrolment.recovery_codes, Some(long_bytes(250, 11)));
    assert_eq!(enrolment.pending_secret, None);
    assert_eq!(enrolment.last_used_timestep, Some(7));
    let token = provider_tokens::Entity::find_by_id(rows.provider_token_id.clone())
        .one(db)
        .await
        .expect("read the provider token")
        .expect("the provider token survives the upgrade");
    assert_eq!(token.access_ciphertext, long_bytes(240, 12));
    assert_eq!(token.refresh_ciphertext, None);
    assert_eq!(token.raw_payload_ciphertext, long_bytes(250, 13));
    assert_eq!(token.scopes, "openid email");
    assert_eq!(token.generation, 3);
}

/// Every column of the database, with its type and nullability, in a
/// stable order.
async fn column_layout(db: &DatabaseConnection) -> Vec<(String, String, String, String)> {
    let backend = db.get_database_backend();
    let query = match backend {
        DbBackend::MySql => {
            "SELECT CAST(TABLE_NAME AS CHAR) AS table_name, \
                    CAST(COLUMN_NAME AS CHAR) AS column_name, \
                    CAST(COLUMN_TYPE AS CHAR) AS column_type, \
                    CAST(IS_NULLABLE AS CHAR) AS nullable \
             FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = DATABASE() \
             ORDER BY TABLE_NAME, ORDINAL_POSITION"
        }
        DbBackend::Postgres => {
            "SELECT table_name::text AS table_name, column_name::text AS column_name, \
                    data_type::text AS column_type, is_nullable::text AS nullable \
             FROM information_schema.columns WHERE table_schema = current_schema() \
             ORDER BY table_name, ordinal_position"
        }
        _ => {
            "SELECT m.name AS table_name, p.name AS column_name, p.type AS column_type, \
                    CAST(p.\"notnull\" AS TEXT) AS nullable \
             FROM sqlite_master m JOIN pragma_table_info(m.name) p \
             WHERE m.type = 'table' ORDER BY m.name, p.cid"
        }
    };
    db.query_all_raw(Statement::from_string(backend, query))
        .await
        .expect("read the column layout")
        .iter()
        .map(|row| {
            (
                row.try_get::<String>("", "table_name").unwrap(),
                row.try_get::<String>("", "column_name").unwrap(),
                row.try_get::<String>("", "column_type").unwrap(),
                row.try_get::<String>("", "nullable").unwrap(),
            )
        })
        .collect()
}

async fn index_names(db: &DatabaseConnection) -> Vec<String> {
    let backend = db.get_database_backend();
    let query = match backend {
        DbBackend::MySql => {
            "SELECT DISTINCT CAST(INDEX_NAME AS CHAR) AS name \
             FROM information_schema.STATISTICS WHERE TABLE_SCHEMA = DATABASE()"
        }
        DbBackend::Postgres => {
            "SELECT indexname::text AS name FROM pg_indexes WHERE schemaname = current_schema()"
        }
        _ => "SELECT name FROM sqlite_master WHERE type = 'index'",
    };
    db.query_all_raw(Statement::from_string(backend, query))
        .await
        .expect("read the index names")
        .iter()
        .map(|row| row.try_get::<String>("", "name").unwrap())
        .collect()
}

/// A database that holds the narrow layout is widened by `migrate`, keeps
/// its rows, accepts the long values, and a second `migrate` changes
/// nothing.
async fn verify_narrow_layout_upgrade(db: &DatabaseConnection, label: &str) {
    let rows = create_narrow_layout(db).await;
    let narrow = column_layout(db).await;
    magnetar::default_schema::migrate(db)
        .await
        .expect("migrate upgrades the narrow layout");
    assert_narrow_rows_kept(db, &rows).await;
    let upgraded = column_layout(db).await;
    for (table, column, _, nullable) in &narrow {
        let (_, _, _, upgraded_nullable) = upgraded
            .iter()
            .find(|(upgraded_table, upgraded_column, _, _)| {
                upgraded_table == table && upgraded_column == column
            })
            .expect("every narrow-layout column survives the upgrade");
        assert_eq!(
            upgraded_nullable, nullable,
            "{table}.{column} keeps its nullability"
        );
    }
    assert_long_values_round_trip(db, label).await;

    magnetar::default_schema::migrate(db)
        .await
        .expect("a second migrate succeeds");
    assert_eq!(
        column_layout(db).await,
        upgraded,
        "a second migrate changes no column"
    );
    assert_narrow_rows_kept(db, &rows).await;
}

/// Two `migrate` calls on one fresh database, each over its own
/// connection pool as two processes would run them, both succeed and leave
/// every default index.
async fn verify_concurrent_fresh_migrations(first: DatabaseConnection, second: DatabaseConnection) {
    let (left, right) = tokio::join!(
        magnetar::default_schema::migrate(&first),
        magnetar::default_schema::migrate(&second)
    );
    assert!(left.is_ok(), "first concurrent migrate: {left:?}");
    assert!(right.is_ok(), "second concurrent migrate: {right:?}");
    let indexes = index_names(&first).await;
    for index in DEFAULT_INDEXES {
        assert!(
            indexes.iter().any(|name| name == index),
            "{index} exists after two concurrent migrations; got {indexes:?}"
        );
    }
    assert_long_values_round_trip(&first, "concurrent").await;
}

/// Two `migrate` calls that upgrade one database holding the older layout,
/// each over its own connection pool as two processes would run them, both
/// succeed. Every add-column step and every widening runs in both, and the
/// upgraded database keeps its row and accepts long values.
async fn verify_concurrent_older_layout_upgrades(
    first: DatabaseConnection,
    second: DatabaseConnection,
) {
    let backend = first.get_database_backend();
    let schema = Schema::new(backend);
    for statement in [
        schema.create_table_from_entity(older_layout::sessions::Entity),
        schema.create_table_from_entity(older_layout::remembers::Entity),
        schema.create_table_from_entity(older_layout::two_factor::Entity),
        schema.create_table_from_entity(older_layout::lockouts::Entity),
        schema.create_table_from_entity(older_layout::tokens::Entity),
        schema.create_table_from_entity(older_layout::accounts::Entity),
    ] {
        first
            .execute(&statement)
            .await
            .expect("create an older-layout table");
    }
    older_layout::sessions::ActiveModel {
        id: Set("older-session".to_owned()),
        user_id: Set(1),
        token_digest: Set("cd".repeat(32)),
        token_hash: Set(Some("cd".repeat(32))),
        user_agent: Set(None),
        ip_address: Set(None),
        expires_at: Set(whole_second(Utc::now()) + Duration::days(1)),
        revoked_at: Set(None),
    }
    .insert(&first)
    .await
    .expect("store an older-layout session");

    let (left, right) = tokio::join!(
        magnetar::default_schema::migrate(&first),
        magnetar::default_schema::migrate(&second)
    );
    assert!(left.is_ok(), "first concurrent upgrade: {left:?}");
    assert!(right.is_ok(), "second concurrent upgrade: {right:?}");

    let layout = column_layout(&first).await;
    for (table, column) in ADDED_COLUMNS {
        assert!(
            layout
                .iter()
                .any(|(listed_table, listed_column, _, _)| listed_table == table
                    && listed_column == column),
            "{table}.{column} exists after two concurrent upgrades"
        );
    }
    let session = sessions::Entity::find_by_id("older-session".to_owned())
        .one(&first)
        .await
        .expect("read the upgraded session")
        .expect("the session survives the upgrade");
    assert_eq!(
        session.auth_epoch, -1,
        "the added epoch invalidates the session that predates it"
    );
    assert_long_values_round_trip(&first, "older-concurrent").await;
}

#[cfg(feature = "seaorm-sqlite")]
async fn sqlite_memory() -> DatabaseConnection {
    Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory SQLite")
}

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn sqlite_long_values_round_trip_through_every_store() {
    let db = sqlite_memory().await;
    magnetar::default_schema::migrate(&db)
        .await
        .expect("create default auth tables");
    assert_long_values_round_trip(&db, "sqlite").await;
}

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn sqlite_narrow_layout_is_upgraded_in_place() {
    let db = sqlite_memory().await;
    verify_narrow_layout_upgrade(&db, "sqlite-upgrade").await;
}

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn sqlite_concurrent_fresh_migrations_both_succeed() {
    let path = std::env::temp_dir().join(format!(
        "magnetar-concurrent-migrate-{}.sqlite",
        unique_suffix()
    ));
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let first = Database::connect(&url).await.expect("connect SQLite file");
    let second = Database::connect(&url).await.expect("connect SQLite file");
    verify_concurrent_fresh_migrations(first, second).await;
    let _ = std::fs::remove_file(&path);
}

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn sqlite_concurrent_upgrades_of_an_older_layout_both_succeed() {
    let path = std::env::temp_dir().join(format!(
        "magnetar-concurrent-upgrade-{}.sqlite",
        unique_suffix()
    ));
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let first = Database::connect(&url).await.expect("connect SQLite file");
    let second = Database::connect(&url).await.expect("connect SQLite file");
    verify_concurrent_older_layout_upgrades(first, second).await;
    let _ = std::fs::remove_file(&path);
}

#[cfg(any(feature = "seaorm-postgres", feature = "seaorm-mysql"))]
async fn live_concurrent_older_layout_upgrades(server_url: &str, label: &str) {
    let live = LiveDatabase::create(server_url, label).await;
    let first = live.connect().await;
    let second = live.connect().await;
    verify_concurrent_older_layout_upgrades(first.clone(), second.clone()).await;
    first.close().await.expect("close the first pool");
    second.close().await.expect("close the second pool");
    live.drop_database().await;
}

#[cfg(any(feature = "seaorm-postgres", feature = "seaorm-mysql"))]
async fn live_long_values_round_trip(server_url: &str, label: &str) {
    let live = LiveDatabase::create(server_url, label).await;
    let db = live.connect().await;
    magnetar::default_schema::migrate(&db)
        .await
        .expect("create default auth tables");
    assert_long_values_round_trip(&db, label).await;
    db.close().await.expect("close the isolated database");
    live.drop_database().await;
}

#[cfg(any(feature = "seaorm-postgres", feature = "seaorm-mysql"))]
async fn live_narrow_layout_upgrade(server_url: &str, label: &str) {
    let live = LiveDatabase::create(server_url, label).await;
    let db = live.connect().await;
    verify_narrow_layout_upgrade(&db, label).await;
    db.close().await.expect("close the isolated database");
    live.drop_database().await;
}

#[cfg(any(feature = "seaorm-postgres", feature = "seaorm-mysql"))]
async fn live_concurrent_fresh_migrations(server_url: &str, label: &str) {
    let live = LiveDatabase::create(server_url, label).await;
    let first = live.connect().await;
    let second = live.connect().await;
    verify_concurrent_fresh_migrations(first.clone(), second.clone()).await;
    first.close().await.expect("close the first pool");
    second.close().await.expect("close the second pool");
    live.drop_database().await;
}

#[cfg(feature = "seaorm-postgres")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn postgres_long_values_round_trip_through_every_store() {
    let url = std::env::var("MAGNETAR_POSTGRES_TEST_URL")
        .expect("MAGNETAR_POSTGRES_TEST_URL is required");
    live_long_values_round_trip(&url, "pg_values").await;
}

#[cfg(feature = "seaorm-postgres")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn postgres_narrow_layout_is_upgraded_in_place() {
    let url = std::env::var("MAGNETAR_POSTGRES_TEST_URL")
        .expect("MAGNETAR_POSTGRES_TEST_URL is required");
    live_narrow_layout_upgrade(&url, "pg_upgrade").await;
}

#[cfg(feature = "seaorm-postgres")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn postgres_concurrent_fresh_migrations_both_succeed() {
    let url = std::env::var("MAGNETAR_POSTGRES_TEST_URL")
        .expect("MAGNETAR_POSTGRES_TEST_URL is required");
    live_concurrent_fresh_migrations(&url, "pg_concurrent").await;
}

#[cfg(feature = "seaorm-postgres")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn postgres_concurrent_upgrades_of_an_older_layout_both_succeed() {
    let url = std::env::var("MAGNETAR_POSTGRES_TEST_URL")
        .expect("MAGNETAR_POSTGRES_TEST_URL is required");
    live_concurrent_older_layout_upgrades(&url, "pg_older").await;
}

#[cfg(feature = "seaorm-mysql")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn mysql_long_values_round_trip_through_every_store() {
    let url =
        std::env::var("MAGNETAR_MYSQL_TEST_URL").expect("MAGNETAR_MYSQL_TEST_URL is required");
    live_long_values_round_trip(&url, "my_values").await;
}

#[cfg(feature = "seaorm-mysql")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn mysql_narrow_layout_is_upgraded_in_place() {
    let url =
        std::env::var("MAGNETAR_MYSQL_TEST_URL").expect("MAGNETAR_MYSQL_TEST_URL is required");
    live_narrow_layout_upgrade(&url, "my_upgrade").await;
}

#[cfg(feature = "seaorm-mysql")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn mysql_concurrent_fresh_migrations_both_succeed() {
    let url =
        std::env::var("MAGNETAR_MYSQL_TEST_URL").expect("MAGNETAR_MYSQL_TEST_URL is required");
    live_concurrent_fresh_migrations(&url, "my_concurrent").await;
}

#[cfg(feature = "seaorm-mysql")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn mysql_concurrent_upgrades_of_an_older_layout_both_succeed() {
    let url =
        std::env::var("MAGNETAR_MYSQL_TEST_URL").expect("MAGNETAR_MYSQL_TEST_URL is required");
    live_concurrent_older_layout_upgrades(&url, "my_older").await;
}
