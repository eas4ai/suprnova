#![cfg(all(
    feature = "migration",
    any(
        feature = "seaorm-sqlite",
        feature = "seaorm-postgres",
        feature = "seaorm-mysql"
    )
))]

#[cfg(feature = "seaorm-sqlite")]
use chrono::{Duration, Utc};
use magnetar::default_migration::DefaultMigrationBindings;
use magnetar::default_schema::DefaultAuthSchema;
#[cfg(feature = "seaorm-sqlite")]
use magnetar::default_schema::sql_stores::{SqlRememberStore, SqlSessionStore};
use magnetar::migration::{MigrationEngine, MigrationRunner, ShapeConfirmation, SourceShape};
#[cfg(feature = "seaorm-sqlite")]
use magnetar::sessions::{
    OpaqueConfig, OpaqueSessionProvider, OpaqueSessionStore, RememberStore, SessionMetadata,
    SessionQueries, StoredSession,
};
use magnetar::storage::{NewUser, SeaOrmStorage, UserStore};
use sea_orm::Database;
#[cfg(feature = "seaorm-sqlite")]
use sea_orm::{ActiveModelTrait, ConnectionTrait, DbBackend, Set, Statement};
#[cfg(feature = "seaorm-sqlite")]
use sha2::{Digest, Sha256};

async fn verify(url: &str) {
    let database = Database::connect(url).await.expect("connect live backend");
    magnetar::default_schema::migrate(&database)
        .await
        .expect("create default auth tables");
    magnetar::default_schema::migrate(&database)
        .await
        .expect("default migration is replay-safe");
    let runner = MigrationEngine::new(
        database.clone(),
        DefaultMigrationBindings::new(database.clone()).sharing_source_database(),
    );
    assert_eq!(
        runner.detect_shape().await.expect("detect default schema"),
        SourceShape::Magnetar
    );
    let store = SeaOrmStorage::<DefaultAuthSchema>::new(database);
    let email = format!("default-schema-{}@example.test", rand::random::<u64>());
    let created = store
        .create_user(NewUser {
            email: email.clone(),
            password_hash: Some("fixture-hash".to_owned()),
        })
        .await
        .expect("create canonical i64 app user");
    assert_eq!(created.email, email);
    assert!(created.user_id.parse::<i64>().is_ok());
}

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn sqlite_legacy_sessions_are_invalidated_before_any_follow_up_migration_step() {
    let database = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory SQLite");
    database
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "CREATE TABLE app_users (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        email TEXT NOT NULL UNIQUE,
        name TEXT NULL,
        password_hash TEXT NULL,
        remember_token TEXT NULL,
        email_verified_at TEXT NULL,
        locked_at TEXT NULL,
        auth_epoch BIGINT NOT NULL DEFAULT 0,
        created_at TEXT NULL,
        updated_at TEXT NULL
    )"
            .to_owned(),
        ))
        .await
        .expect("create current user table");
    database
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "CREATE TABLE auth_sessions (
        id TEXT PRIMARY KEY NOT NULL,
        user_id BIGINT NOT NULL,
        token_digest TEXT NOT NULL,
        token_hash TEXT NULL,
        user_agent TEXT NULL,
        ip_address TEXT NULL,
        expires_at TEXT NOT NULL,
        revoked_at TEXT NULL
    )"
            .to_owned(),
        ))
        .await
        .expect("create legacy session table");
    database
        .execute_raw(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "INSERT INTO app_users (id, email, auth_epoch) VALUES (?, ?, ?)",
            [1_i64.into(), "legacy@example.test".into(), 0_i64.into()],
        ))
        .await
        .expect("insert epoch-zero user");

    let legacy_token = "legacy-opaque-token";
    let legacy_digest: [u8; 32] = Sha256::digest(legacy_token.as_bytes()).into();
    let legacy_digest_hex = legacy_digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    database
        .execute_raw(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "INSERT INTO auth_sessions (
        id, user_id, token_digest, token_hash, expires_at
     ) VALUES (?, ?, ?, ?, ?)",
            [
                "legacy-session".into(),
                1_i64.into(),
                legacy_digest_hex.clone().into(),
                legacy_digest_hex.into(),
                (Utc::now() + Duration::days(1)).into(),
            ],
        ))
        .await
        .expect("insert live legacy session");

    magnetar::default_schema::migrate(&database)
        .await
        .expect("upgrade legacy default schema");

    let migrated = database
        .query_one_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT auth_epoch FROM auth_sessions WHERE id = 'legacy-session'".to_owned(),
        ))
        .await
        .expect("read migrated session")
        .expect("legacy session remains present");
    assert_eq!(
        migrated.try_get::<i64>("", "auth_epoch").unwrap(),
        -1,
        "the ALTER itself must atomically invalidate every pre-epoch row"
    );

    let store = std::sync::Arc::new(SqlSessionStore(database.clone()));
    let provider = OpaqueSessionProvider::new(store.clone(), OpaqueConfig::default());
    assert!(
        provider.verify_bearer(legacy_token).await.is_err(),
        "a live pre-epoch row must not authenticate an epoch-zero user"
    );

    let fresh_token = "fresh-opaque-token";
    let fresh_digest: [u8; 32] = Sha256::digest(fresh_token.as_bytes()).into();
    store
        .insert_session_if_epoch_current(StoredSession {
            session_id: "fresh-session".to_owned(),
            user_id: "1".to_owned(),
            auth_epoch: 0,
            token_hash: fresh_digest,
            token_digest: fresh_digest,
            expires_at: Utc::now() + Duration::days(1),
            revoked_at: None,
            metadata: SessionMetadata::default(),
        })
        .await
        .expect("insert a new epoch-bound session");
    let verified = provider
        .verify_bearer(fresh_token)
        .await
        .expect("new epoch-bound session verifies");
    assert_eq!(verified.user_id(), "1");
    assert_eq!(verified.auth_epoch(), 0);
}

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn negative_legacy_remember_epoch_is_neutral_not_found() {
    let database = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory SQLite");
    magnetar::default_schema::migrate(&database)
        .await
        .expect("create default auth tables");
    magnetar::default_schema::remembers::ActiveModel {
        id: Set("legacy-negative-epoch".to_owned()),
        selector: Set("legacy-selector".to_owned()),
        user_id: Set("1".to_owned()),
        auth_epoch: Set(-1),
        verifier_hash: Set("sha256:irrelevant".to_owned()),
        expires_at: Set(Utc::now() + Duration::days(1)),
    }
    .insert(&database)
    .await
    .expect("seed migration-invalidated remember row");

    let error = SqlRememberStore(database)
        .find_for_rotation("legacy-selector", Utc::now())
        .await
        .expect_err("negative legacy epochs resolve as an expired credential");
    assert!(matches!(
        error,
        magnetar::Error::NotFound {
            resource,
            identifier
        } if resource == "remember token" && identifier == "expired or revoked"
    ));
}

#[cfg(feature = "seaorm-postgres")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn postgres_default_schema_is_replay_safe() {
    let url = std::env::var("MAGNETAR_POSTGRES_TEST_URL")
        .expect("MAGNETAR_POSTGRES_TEST_URL is required");
    verify(&url).await;
}

#[cfg(feature = "seaorm-postgres")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn postgres_api_import_advances_the_default_user_sequence() {
    let server_url = std::env::var("MAGNETAR_POSTGRES_TEST_URL")
        .expect("MAGNETAR_POSTGRES_TEST_URL is required");
    let admin = Database::connect(&server_url)
        .await
        .expect("connect PostgreSQL admin database");
    let database_name = format!("magnetar_sequence_{}", rand::random::<u64>());
    admin
        .execute_raw(Statement::from_string(
            DbBackend::Postgres,
            format!("CREATE DATABASE \"{database_name}\""),
        ))
        .await
        .expect("create isolated PostgreSQL database");
    let prefix = server_url
        .rsplit_once('/')
        .expect("PostgreSQL URL contains a database path")
        .0;
    let database_url = format!("{prefix}/{database_name}");
    let database = Database::connect(&database_url)
        .await
        .expect("connect isolated PostgreSQL database");
    database
        .execute_raw(Statement::from_string(
            DbBackend::Postgres,
            "CREATE TABLE app_users (
        id BIGSERIAL PRIMARY KEY,
        email TEXT NOT NULL UNIQUE
    )"
            .to_owned(),
        ))
        .await
        .expect("create API source users table");
    database
        .execute_raw(Statement::from_string(
            DbBackend::Postgres,
            "INSERT INTO app_users (id, email)
     VALUES (4242, 'imported@example.test')"
                .to_owned(),
        ))
        .await
        .expect("insert API source user");
    magnetar::default_schema::migrate(&database)
        .await
        .expect("create default destination schema");
    let runner = MigrationEngine::new(
        database.clone(),
        DefaultMigrationBindings::new(database.clone()).sharing_source_database(),
    );
    let plan = runner
        .dry_run(ShapeConfirmation {
            detected: SourceShape::SuprnovaApi,
            operator_selected: SourceShape::SuprnovaApi,
        })
        .await
        .expect("plan API migration");
    runner.apply(&plan).await.expect("apply API migration");
    let imported_max = database
        .query_one_raw(Statement::from_string(
            DbBackend::Postgres,
            "SELECT MAX(id) AS max_id FROM app_users".to_owned(),
        ))
        .await
        .expect("read imported user IDs")
        .expect("maximum row");
    assert_eq!(
        imported_max.try_get::<Option<i64>>("", "max_id").unwrap(),
        Some(4242)
    );

    let created = database
        .query_one_raw(Statement::from_string(
            DbBackend::Postgres,
            "INSERT INTO app_users (email)
     VALUES ('after-import@example.test')
     RETURNING id"
                .to_owned(),
        ))
        .await
        .expect("insert application user after migration")
        .expect("inserted user ID");
    assert_eq!(created.try_get::<i64>("", "id").unwrap(), 4243);

    drop(runner);
    drop(database);
    admin
        .execute_raw(Statement::from_string(
            DbBackend::Postgres,
            format!("DROP DATABASE \"{database_name}\""),
        ))
        .await
        .expect("drop isolated PostgreSQL database");
}
#[cfg(feature = "seaorm-mysql")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn mysql_default_schema_is_replay_safe() {
    let url =
        std::env::var("MAGNETAR_MYSQL_TEST_URL").expect("MAGNETAR_MYSQL_TEST_URL is required");
    verify(&url).await;
}
