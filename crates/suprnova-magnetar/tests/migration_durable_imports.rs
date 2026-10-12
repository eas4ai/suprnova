#![cfg(all(
    feature = "migration",
    feature = "seaorm-sqlite",
    feature = "two-factor"
))]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use magnetar::default_migration::DefaultMigrationBindings;
use magnetar::default_schema::{
    accounts, lockouts, methods, migration_state, tokens, two_factor, users,
};
use magnetar::migration::{
    DurableAuthRecord, ImportedSecureToken, MigrationBindings, MigrationEngine, MigrationRunner,
    ShapeConfirmation, SourceShape,
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, Database, DatabaseConnection, DbBackend, EntityTrait,
    QueryFilter, Statement,
};
use secrecy::SecretString;

static NEXT_DATABASE: AtomicU64 = AtomicU64::new(0);

#[tokio::test]
async fn torii_apply_imports_users_accounts_passkeys_tokens_and_lockout_history() {
    let (source, source_path) = open_fixture("torii").await;
    source
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "DELETE FROM users WHERE id = 'torii-user-collision-lower'",
        ))
        .await
        .unwrap();
    source.execute_raw(Statement::from_string(DbBackend::Sqlite,
    "INSERT INTO secure_tokens (user_id, token, purpose, used_at, expires_at, created_at, updated_at) VALUES ('torii-user-passwordless', 'fixture-secure-token', 'email_verification', NULL, '2030-01-01T00:00:00+00:00', '2024-01-02T03:04:05+00:00', '2024-01-02T03:04:05+00:00')",))
        .await
        .unwrap();
    source.execute_raw(Statement::from_string(DbBackend::Sqlite,
    "INSERT INTO failed_login_attempts (email, ip_address, attempted_at) VALUES ('fixture.passwordless@example.test', '203.0.113.7', '2024-01-02T03:04:05+00:00')",))
        .await
        .unwrap();
    source.execute_raw(Statement::from_string(DbBackend::Sqlite,
    "INSERT INTO failed_login_attempts (email, ip_address, attempted_at) VALUES ('fixture.passwordless@example.test', '203.0.113.7', '2024-01-02T03:04:05+00:00')",))
        .await
        .unwrap();
    let (app, app_path) = empty_app_database().await;
    let runner = MigrationEngine::new(source.clone(), DefaultMigrationBindings::new(app.clone()));
    let plan = runner
        .dry_run(ShapeConfirmation {
            detected: SourceShape::Torii,
            operator_selected: SourceShape::Torii,
        })
        .await
        .unwrap();

    runner.apply(&plan).await.unwrap();

    let user = users::Entity::find()
        .filter(users::Column::Email.eq("fixture.passwordless@example.test"))
        .one(&app)
        .await
        .unwrap()
        .unwrap();
    assert!(user.email_verified_at.is_some());
    assert_eq!(
        accounts::Entity::find()
            .filter(accounts::Column::UserId.eq(user.id))
            .all(&app)
            .await
            .unwrap()
            .len(),
        2
    );
    let linked = accounts::Entity::find()
        .filter(accounts::Column::Provider.eq("fixture-provider"))
        .one(&app)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        linked.created_at.unwrap().to_rfc3339(),
        "2024-01-02T03:04:05+00:00"
    );
    assert_eq!(
        linked.updated_at.unwrap().to_rfc3339(),
        "2024-01-02T03:04:05+00:00"
    );
    let passkey = methods::Entity::find()
        .filter(methods::Column::UserId.eq(user.id))
        .one(&app)
        .await
        .unwrap()
        .unwrap();
    assert!(passkey.public_key.unwrap().contains("Fixture passkey"));
    assert_eq!(
        tokens::Entity::find()
            .filter(tokens::Column::UserId.eq(user.id))
            .all(&app)
            .await
            .unwrap()
            .len(),
        1
    );
    let secure_token = tokens::Entity::find()
        .filter(tokens::Column::UserId.eq(user.id))
        .one(&app)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        secure_token.expires_at.to_rfc3339(),
        "2030-01-01T00:00:00+00:00"
    );
    assert!(secure_token.used_at.is_none());
    assert_eq!(
        secure_token.created_at.unwrap().to_rfc3339(),
        "2024-01-02T03:04:05+00:00"
    );
    assert_eq!(
        secure_token.updated_at.unwrap().to_rfc3339(),
        "2024-01-02T03:04:05+00:00"
    );
    let attempt = lockouts::Entity::find()
        .filter(lockouts::Column::Identity.eq("fixture.passwordless@example.test"))
        .one(&app)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(attempt.ip_address.as_deref(), Some("203.0.113.7"));
    assert_eq!(count(&source, "sessions").await, 0);
    assert_eq!(
        lockouts::Entity::find()
            .filter(lockouts::Column::Identity.eq("fixture.passwordless@example.test"))
            .all(&app)
            .await
            .unwrap()
            .len(),
        2
    );

    let bindings = DefaultMigrationBindings::new(app.clone());
    let mut transaction = bindings.begin_transaction(None).await.unwrap();
    let mismatch = transaction
        .import_durable_record(DurableAuthRecord::SecureToken(ImportedSecureToken {
            app_user_id: user.id,
            token: SecretString::from("fixture-secure-token".to_owned()),
            purpose: "email_verification".to_owned(),
            used_at: Some("2024-01-03T00:00:00+00:00".to_owned()),
            expires_at: "2031-01-01T00:00:00+00:00".to_owned(),
            created_at: "2024-01-02T03:04:05+00:00".to_owned(),
            updated_at: "2024-01-02T03:04:05+00:00".to_owned(),
        }))
        .await
        .unwrap_err();
    assert!(matches!(mismatch, magnetar::Error::Conflict { .. }));
    transaction.rollback().await.unwrap();

    drop(runner);
    drop(app);
    fs::remove_file(source_path).unwrap();
    fs::remove_file(app_path).unwrap();
}

#[tokio::test]
async fn suprnova_web_apply_maps_host_identity_and_preserves_fields_and_two_factor_ciphertext() {
    let (source, source_path) = open_fixture("suprnova-web").await;
    let (app, app_path) = empty_app_database().await;
    let runner = MigrationEngine::new(source, DefaultMigrationBindings::new(app.clone()));
    let plan = runner
        .dry_run(ShapeConfirmation {
            detected: SourceShape::SuprnovaWeb,
            operator_selected: SourceShape::SuprnovaWeb,
        })
        .await
        .unwrap();

    runner.apply(&plan).await.unwrap();

    let user = users::Entity::find()
        .filter(users::Column::Email.eq("fixture.web.passwordless@example.test"))
        .one(&app)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(user.name.as_deref(), Some("Fixture passwordless user"));
    assert_eq!(user.password_hash.as_deref(), Some(""));
    assert!(user.email_verified_at.is_some());
    let enrollment = two_factor::Entity::find_by_id(user.id.to_string())
        .one(&app)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        String::from_utf8(enrollment.secret).unwrap(),
        "K0A7aRihcbZ98aBhIAsJVCK9FDkKJEuw_B5XVMaUiyukQDe-Sd7kDHrYt8U"
    );
    assert!(enrollment.confirmed_at.is_some());
    assert_eq!(
        enrollment.created_at.unwrap().to_rfc3339(),
        "2024-01-02T03:04:05+00:00"
    );
    assert_eq!(
        enrollment.updated_at.unwrap().to_rfc3339(),
        "2024-01-02T03:04:05+00:00"
    );

    drop(runner);
    drop(app);
    fs::remove_file(source_path).unwrap();
    fs::remove_file(app_path).unwrap();
}

#[tokio::test]
async fn existing_application_password_cannot_be_replaced_by_legacy_hash() {
    let (source, source_path) = open_fixture("torii").await;
    source
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "DELETE FROM users WHERE id = 'torii-user-collision-lower'",
        ))
        .await
        .unwrap();
    source.execute_raw(Statement::from_string(DbBackend::Sqlite,
    "UPDATE users SET password_hash = 'legacy-source-hash' WHERE id = 'torii-user-passwordless'",))
        .await
        .unwrap();
    let (app, app_path) = empty_app_database().await;
    app.execute_raw(Statement::from_string(DbBackend::Sqlite,
    "INSERT INTO app_users (id, email, password_hash, auth_epoch) VALUES (4242, 'fixture.passwordless@example.test', 'app-hash-at-plan', 0)",))
    .await
    .unwrap();
    let runner = MigrationEngine::new(source.clone(), DefaultMigrationBindings::new(app.clone()));
    let plan = runner
        .dry_run(ShapeConfirmation {
            detected: SourceShape::Torii,
            operator_selected: SourceShape::Torii,
        })
        .await
        .unwrap();
    app.execute_raw(Statement::from_string(
        DbBackend::Sqlite,
        "UPDATE app_users SET password_hash = 'newer-application-hash' WHERE id = 4242",
    ))
    .await
    .unwrap();

    let error = runner.apply(&plan).await.unwrap_err();
    assert!(matches!(error, magnetar::Error::Conflict { .. }));
    let user = users::Entity::find_by_id(4242)
        .one(&app)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        user.password_hash.as_deref(),
        Some("newer-application-hash")
    );
    assert_eq!(count(&source, "sessions").await, 1);

    drop(runner);
    drop(app);
    fs::remove_file(source_path).unwrap();
    fs::remove_file(app_path).unwrap();
}

#[tokio::test]
async fn committed_import_ledger_allows_same_plan_cleanup_retry() {
    let (source, source_path) = open_fixture("torii").await;
    source
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "DELETE FROM users WHERE id = 'torii-user-collision-lower'",
        ))
        .await
        .unwrap();
    let (app, app_path) = empty_app_database().await;
    let runner = MigrationEngine::new(source.clone(), DefaultMigrationBindings::new(app.clone()));
    let plan = runner
        .dry_run(ShapeConfirmation {
            detected: SourceShape::Torii,
            operator_selected: SourceShape::Torii,
        })
        .await
        .unwrap();
    source.execute_raw(Statement::from_string(DbBackend::Sqlite,
    "CREATE TRIGGER fail_session_cleanup BEFORE DELETE ON sessions BEGIN SELECT RAISE(ABORT, 'injected cleanup failure'); END",))
        .await
        .unwrap();

    assert!(runner.apply(&plan).await.is_err());
    assert!(
        users::Entity::find()
            .filter(users::Column::Email.eq("fixture.passwordless@example.test"))
            .one(&app)
            .await
            .unwrap()
            .is_some()
    );
    source
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "DROP TRIGGER fail_session_cleanup",
        ))
        .await
        .unwrap();

    runner.apply(&plan).await.unwrap();
    assert_eq!(count(&source, "sessions").await, 0);
    runner.apply(&plan).await.unwrap();

    drop(runner);
    drop(app);
    fs::remove_file(source_path).unwrap();
    fs::remove_file(app_path).unwrap();
}

#[tokio::test]
async fn same_database_migration_uses_one_transaction_without_self_blocking() {
    let (database, path) = open_fixture("torii").await;
    database
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "DELETE FROM users WHERE id = 'torii-user-collision-lower'",
        ))
        .await
        .unwrap();
    magnetar::default_schema::migrate(&database).await.unwrap();
    let runner = MigrationEngine::new(
        database.clone(),
        DefaultMigrationBindings::new(database.clone()).sharing_source_database(),
    );
    let plan = runner
        .dry_run(ShapeConfirmation {
            detected: SourceShape::Torii,
            operator_selected: SourceShape::Torii,
        })
        .await
        .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(2), runner.apply(&plan))
        .await
        .expect("same-database apply must not block on its own source lock")
        .unwrap();
    assert!(
        users::Entity::find()
            .filter(users::Column::Email.eq("fixture.passwordless@example.test"))
            .one(&database)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(count(&database, "sessions").await, 0);
    assert_eq!(runner.detect_shape().await.unwrap(), SourceShape::Magnetar);

    drop(runner);
    drop(database);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn default_binding_declares_every_application_owned_auth_table() {
    let database = Database::connect("sqlite::memory:").await.unwrap();
    let bindings = DefaultMigrationBindings::new(database);
    let targets = bindings.migration_target_tables();

    for required in [
        "app_users",
        "auth_sessions",
        "auth_linked_accounts",
        "auth_methods",
        "auth_tokens",
        "auth_ceremonies",
        "auth_lockouts",
        "auth_remember_tokens",
        "auth_two_factor",
        "auth_lifecycle_deliveries",
        "auth_migration_runs",
        "auth_migration_identities",
        "auth_provider_tokens",
        "magnetar_migration_state",
    ] {
        assert!(targets.iter().any(|table| table == required), "{required}");
    }
}

#[tokio::test]
async fn completed_magnetar_marker_suppresses_destination_api_signature() {
    let database = Database::connect("sqlite::memory:").await.unwrap();
    magnetar::default_schema::migrate(&database).await.unwrap();
    let runner = MigrationEngine::new(
        database.clone(),
        DefaultMigrationBindings::new(database.clone()).sharing_source_database(),
    );
    assert_eq!(runner.detect_shape().await.unwrap(), SourceShape::Magnetar);
    database
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "UPDATE magnetar_migration_state SET value = '2' WHERE key = 'schema_version'",
        ))
        .await
        .unwrap();
    assert!(matches!(
        runner.detect_shape().await.unwrap_err(),
        magnetar::Error::Conflict { .. }
    ));
}

#[tokio::test]
async fn minimal_api_app_users_shape_gains_every_required_default_column() {
    let database = Database::connect("sqlite::memory:").await.unwrap();
    database.execute_raw(Statement::from_string(DbBackend::Sqlite,
    "CREATE TABLE app_users (id INTEGER PRIMARY KEY AUTOINCREMENT, email TEXT NOT NULL UNIQUE)",))
        .await
        .unwrap();
    database
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "INSERT INTO app_users (id, email) VALUES (4242, 'minimal.api@example.test')",
        ))
        .await
        .unwrap();

    magnetar::default_schema::migrate(&database).await.unwrap();
    let user = users::Entity::find_by_id(4242)
        .one(&database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(user.email, "minimal.api@example.test");
    assert!(user.created_at.is_none());
    assert!(user.updated_at.is_none());
    assert!(
        migration_state::Entity::find_by_id("source_pending")
            .one(&database)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        migration_state::Entity::find_by_id("schema_version")
            .one(&database)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn same_database_api_plan_keeps_app_users_in_source_fingerprints() {
    let (database, path) = open_fixture("suprnova-api").await;
    magnetar::default_schema::migrate(&database).await.unwrap();
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
        .unwrap();
    assert!(
        plan.source_fingerprints
            .iter()
            .any(|entry| entry.table == "app_users")
    );

    runner.apply(&plan).await.unwrap();
    let user = users::Entity::find_by_id(4242)
        .one(&database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(user.email, "fixture.api.user@example.test");
    assert_eq!(runner.detect_shape().await.unwrap(), SourceShape::Magnetar);
    assert!(
        migration_state::Entity::find_by_id("source_pending")
            .one(&database)
            .await
            .unwrap()
            .is_none()
    );

    drop(runner);
    drop(database);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn suprnova_api_apply_preserves_existing_i64_identity() {
    let (source, source_path) = open_fixture("suprnova-api").await;
    let (app, app_path) = empty_app_database().await;
    let runner = MigrationEngine::new(source, DefaultMigrationBindings::new(app.clone()));
    let plan = runner
        .dry_run(ShapeConfirmation {
            detected: SourceShape::SuprnovaApi,
            operator_selected: SourceShape::SuprnovaApi,
        })
        .await
        .unwrap();

    runner.apply(&plan).await.unwrap();

    let user = users::Entity::find_by_id(4242)
        .one(&app)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(user.email, "fixture.api.user@example.test");

    drop(runner);
    drop(app);
    fs::remove_file(source_path).unwrap();
    fs::remove_file(app_path).unwrap();
}

fn fixture_copy(name: &str) -> PathBuf {
    let id = NEXT_DATABASE.fetch_add(1, Ordering::Relaxed);
    let destination = std::env::temp_dir().join(format!(
        "magnetar-durable-source-{name}-{}-{id}.sqlite",
        std::process::id()
    ));
    fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/databases")
            .join(format!("{name}.sqlite")),
        &destination,
    )
    .unwrap();
    destination
}

async fn open_fixture(name: &str) -> (DatabaseConnection, PathBuf) {
    let path = fixture_copy(name);
    let database = Database::connect(format!("sqlite://{}?mode=rwc", path.display()))
        .await
        .unwrap();
    (database, path)
}

async fn empty_app_database() -> (DatabaseConnection, PathBuf) {
    let id = NEXT_DATABASE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "magnetar-durable-app-{}-{id}.sqlite",
        std::process::id()
    ));
    let database = Database::connect(format!("sqlite://{}?mode=rwc", path.display()))
        .await
        .unwrap();
    magnetar::default_schema::migrate(&database).await.unwrap();
    (database, path)
}

async fn count(database: &DatabaseConnection, table: &str) -> i64 {
    database
        .query_one_raw(Statement::from_string(
            DbBackend::Sqlite,
            format!("SELECT COUNT(*) FROM {table}"),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get_by_index(0)
        .unwrap()
}
