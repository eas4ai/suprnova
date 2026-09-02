#![cfg(all(
    feature = "migration",
    any(
        feature = "seaorm-sqlite",
        feature = "seaorm-postgres",
        feature = "seaorm-mysql"
    )
))]

use chrono::{Duration, Timelike, Utc};
#[cfg(any(feature = "seaorm-postgres", feature = "seaorm-mysql"))]
use magnetar::default_migration::DefaultMigrationBindings;
#[cfg(any(feature = "seaorm-postgres", feature = "seaorm-mysql"))]
use magnetar::default_schema::DefaultAuthSchema;
use magnetar::default_schema::sql_stores::SqlRememberStore;
#[cfg(feature = "seaorm-sqlite")]
use magnetar::default_schema::sql_stores::SqlSessionStore;
#[cfg(feature = "seaorm-postgres")]
use magnetar::migration::ShapeConfirmation;
#[cfg(any(feature = "seaorm-postgres", feature = "seaorm-mysql"))]
use magnetar::migration::{MigrationEngine, MigrationRunner, SourceShape};
#[cfg(feature = "seaorm-sqlite")]
use magnetar::sessions::{
    OpaqueConfig, OpaqueSessionProvider, OpaqueSessionStore, SessionMetadata, SessionQueries,
    StoredSession,
};
use magnetar::sessions::{RememberRow, RememberStore};
#[cfg(any(feature = "seaorm-postgres", feature = "seaorm-mysql"))]
use magnetar::storage::{NewUser, SeaOrmStorage, UserStore};
#[cfg(feature = "seaorm-sqlite")]
use sea_orm::{ActiveModelTrait, Set};
use sea_orm::{ColumnTrait, Database, DatabaseConnection, EntityTrait, QueryFilter};
#[cfg(any(feature = "seaorm-sqlite", feature = "seaorm-postgres"))]
use sea_orm::{ConnectionTrait, DbBackend, Statement};
#[cfg(feature = "seaorm-sqlite")]
use sha2::{Digest, Sha256};

async fn verify_remember_owner_selector_contract(database: &DatabaseConnection) {
    let store = SqlRememberStore(database.clone());
    let now = Utc::now()
        .with_nanosecond(0)
        .expect("whole-second contract timestamp");
    let suffix = rand::random::<u64>();

    let first = RememberRow {
        id: format!("selector-revoke-first-id-{suffix}"),
        selector: format!("selector-revoke-first-{suffix}"),
        user_id: format!("selector-revoke-user-{suffix}"),
        auth_epoch: 1,
        verifier_hash: "sha256:first".to_owned(),
        expires_at: now + Duration::days(1),
    };
    let second = RememberRow {
        id: format!("selector-revoke-second-id-{suffix}"),
        selector: format!("selector-revoke-second-{suffix}"),
        verifier_hash: "sha256:second".to_owned(),
        ..first.clone()
    };
    store
        .insert_remember(first.clone())
        .await
        .expect("seed first selector-revocation row");
    store
        .insert_remember(second.clone())
        .await
        .expect("seed second selector-revocation row");

    assert!(
        store
            .revoke_remember_selector(&first.user_id, &first.selector)
            .await
            .expect("revoke the matching owner and selector")
    );
    assert!(
        !store
            .revoke_remember_selector(&first.user_id, &first.selector)
            .await
            .expect("repeating an exact revocation is a no-op")
    );
    assert_eq!(
        store
            .find_for_rotation(&first.selector, now)
            .await
            .expect("inspect the revoked selector"),
        None
    );
    assert_eq!(
        store
            .find_for_rotation(&second.selector, now)
            .await
            .expect("inspect the sibling selector"),
        Some(second)
    );

    let ambiguous_revoke_first = RememberRow {
        id: format!("ambiguous-selector-first-id-{suffix}"),
        selector: format!("ambiguous-selector-{suffix}"),
        user_id: format!("ambiguous-selector-owner-{suffix}"),
        auth_epoch: 1,
        verifier_hash: "sha256:first".to_owned(),
        expires_at: now + Duration::days(1),
    };
    let ambiguous_revoke_second = RememberRow {
        id: format!("ambiguous-selector-second-id-{suffix}"),
        verifier_hash: "sha256:second".to_owned(),
        ..ambiguous_revoke_first.clone()
    };
    store
        .insert_remember(ambiguous_revoke_first.clone())
        .await
        .expect("seed first ambiguous revocation row");
    store
        .insert_remember(ambiguous_revoke_second.clone())
        .await
        .expect("seed second ambiguous revocation row");

    assert!(
        !store
            .revoke_remember_selector(
                &format!("different-owner-{suffix}"),
                &ambiguous_revoke_first.selector,
            )
            .await
            .expect("owner mismatch must fail closed")
    );
    let error = store
        .revoke_remember_selector(
            &ambiguous_revoke_first.user_id,
            &ambiguous_revoke_first.selector,
        )
        .await
        .expect_err("ambiguous exact revocation must return an error");
    assert!(matches!(
        error,
        magnetar::Error::Conflict { resource, message }
            if resource == "remember credential"
                && message == "owner and selector matched multiple rows"
    ));
    let remaining = magnetar::default_schema::remembers::Entity::find()
        .filter(
            magnetar::default_schema::remembers::Column::Selector
                .eq(&ambiguous_revoke_first.selector),
        )
        .all(database)
        .await
        .expect("query ambiguous selector rows after rejected revocation");
    assert_eq!(
        remaining.len(),
        2,
        "ambiguous selector revocation must roll back without mutating rows"
    );

    let ambiguous_rotation_first = RememberRow {
        id: format!("ambiguous-rotation-first-id-{suffix}"),
        selector: format!("ambiguous-rotation-selector-{suffix}"),
        user_id: format!("ambiguous-rotation-first-owner-{suffix}"),
        auth_epoch: 1,
        verifier_hash: "sha256:first".to_owned(),
        expires_at: now + Duration::days(1),
    };
    let ambiguous_rotation_second = RememberRow {
        id: format!("ambiguous-rotation-second-id-{suffix}"),
        user_id: format!("ambiguous-rotation-second-owner-{suffix}"),
        verifier_hash: "sha256:second".to_owned(),
        ..ambiguous_rotation_first.clone()
    };
    store
        .insert_remember(ambiguous_rotation_first.clone())
        .await
        .expect("seed first ambiguous rotation row");
    store
        .insert_remember(ambiguous_rotation_second.clone())
        .await
        .expect("seed second ambiguous rotation row");

    let error = store
        .find_for_rotation(&ambiguous_rotation_first.selector, now)
        .await
        .expect_err("an ambiguous live selector must fail before verifier comparison");
    assert!(matches!(
        error,
        magnetar::Error::Conflict { resource, message }
            if resource == "remember credential"
                && message == "selector matched multiple active rows"
    ));

    let remaining = magnetar::default_schema::remembers::Entity::find()
        .filter(
            magnetar::default_schema::remembers::Column::Selector
                .eq(&ambiguous_rotation_first.selector),
        )
        .all(database)
        .await
        .expect("query ambiguous selector rows after rejected rotation lookup");
    assert_eq!(remaining.len(), 2);
    assert!(remaining.iter().any(|row| {
        row.id == ambiguous_rotation_first.id
            && row.user_id == ambiguous_rotation_first.user_id
            && row.verifier_hash == ambiguous_rotation_first.verifier_hash
    }));
    assert!(remaining.iter().any(|row| {
        row.id == ambiguous_rotation_second.id
            && row.user_id == ambiguous_rotation_second.user_id
            && row.verifier_hash == ambiguous_rotation_second.verifier_hash
    }));

    let mixed_case = RememberRow {
        id: format!("mixed-case-id-{suffix}"),
        selector: format!("Selector-Mixed-{suffix}"),
        user_id: format!("Owner-Mixed-{suffix}"),
        auth_epoch: 1,
        verifier_hash: "sha256:mixed-case".to_owned(),
        expires_at: now + Duration::days(1),
    };
    let lower_selector = mixed_case.selector.to_ascii_lowercase();
    let lower_owner = mixed_case.user_id.to_ascii_lowercase();
    store
        .insert_remember(mixed_case.clone())
        .await
        .expect("seed mixed-case remember row");

    assert_eq!(
        store
            .find_for_rotation(&lower_selector, now)
            .await
            .expect("case-mismatched selector lookup must be neutral"),
        None
    );
    assert!(
        !store
            .revoke_remember_selector(&mixed_case.user_id, &lower_selector)
            .await
            .expect("case-mismatched selector revocation must be neutral")
    );
    assert!(
        !store
            .revoke_remember_selector(&lower_owner, &mixed_case.selector)
            .await
            .expect("case-mismatched owner revocation must be neutral")
    );
    assert_eq!(
        store
            .find_for_rotation(&mixed_case.selector, now)
            .await
            .expect("exact mixed-case selector remains available"),
        Some(mixed_case)
    );
}

#[cfg(any(feature = "seaorm-postgres", feature = "seaorm-mysql"))]
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
    verify_remember_owner_selector_contract(&database).await;
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

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn sqlite_remember_replacement_rolls_back_and_has_one_winner() {
    let database = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory SQLite");
    magnetar::default_schema::migrate(&database)
        .await
        .expect("create default auth tables");
    let store = SqlRememberStore(database.clone());
    let now = Utc::now();
    let original = RememberRow {
        id: "original-id".to_owned(),
        selector: "original-selector".to_owned(),
        user_id: "rotation-user".to_owned(),
        auth_epoch: 9,
        verifier_hash: "sha256:original".to_owned(),
        expires_at: now + Duration::days(1),
    };
    let first_replacement = RememberRow {
        id: "first-replacement-id".to_owned(),
        selector: "first-replacement-selector".to_owned(),
        verifier_hash: "sha256:first".to_owned(),
        ..original.clone()
    };
    let second_replacement = RememberRow {
        id: "second-replacement-id".to_owned(),
        selector: "second-replacement-selector".to_owned(),
        verifier_hash: "sha256:second".to_owned(),
        ..original.clone()
    };
    store
        .insert_remember(original.clone())
        .await
        .expect("seed original remember row");

    database
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "CREATE TRIGGER fail_auth_remember_replacement
             BEFORE INSERT ON auth_remember_tokens
             WHEN NEW.user_id = 'rotation-user'
             BEGIN
                 SELECT RAISE(ABORT, 'injected auth remember replacement failure');
             END"
            .to_owned(),
        ))
        .await
        .expect("install scoped replacement failure trigger");

    let error = store
        .replace_for_rotation(
            &original.id,
            &original.selector,
            now,
            first_replacement.clone(),
        )
        .await
        .expect_err("replacement insertion failure must surface");
    assert!(matches!(error, magnetar::Error::Internal { .. }));
    assert_eq!(
        store
            .find_for_rotation(&original.selector, now)
            .await
            .expect("inspect original after failed replacement"),
        Some(original.clone()),
    );

    database
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "DROP TRIGGER fail_auth_remember_replacement".to_owned(),
        ))
        .await
        .expect("remove replacement failure trigger before retry");

    assert!(
        store
            .replace_for_rotation(
                &original.id,
                &original.selector,
                now,
                first_replacement.clone(),
            )
            .await
            .expect("retry replacement succeeds")
    );
    assert!(
        !store
            .replace_for_rotation(
                &original.id,
                &original.selector,
                now,
                second_replacement.clone(),
            )
            .await
            .expect("second replacement attempt loses the comparison")
    );
    assert!(
        store
            .find_for_rotation(&original.selector, now)
            .await
            .expect("original selector lookup succeeds")
            .is_none()
    );
    assert_eq!(
        store
            .find_for_rotation(&first_replacement.selector, now)
            .await
            .expect("winner selector lookup succeeds"),
        Some(first_replacement),
    );
    assert!(
        store
            .find_for_rotation(&second_replacement.selector, now)
            .await
            .expect("loser selector lookup succeeds")
            .is_none()
    );
}

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn sqlite_remember_owner_selector_contract() {
    let database = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory SQLite");
    magnetar::default_schema::migrate(&database)
        .await
        .expect("create default auth tables");
    verify_remember_owner_selector_contract(&database).await;
}

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn sqlite_nocase_remember_owner_selector_contract() {
    let database = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory SQLite");
    database
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "CREATE TABLE auth_remember_tokens (\
                id TEXT PRIMARY KEY NOT NULL, \
                selector TEXT COLLATE NOCASE NOT NULL, \
                user_id TEXT COLLATE NOCASE NOT NULL, \
                auth_epoch BIGINT NOT NULL, \
                verifier_hash TEXT NOT NULL, \
                expires_at TEXT NOT NULL\
            )",
        ))
        .await
        .expect("create case-insensitive remember table");
    verify_remember_owner_selector_contract(&database).await;
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
