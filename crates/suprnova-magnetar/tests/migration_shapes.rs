#![cfg(all(feature = "migration", feature = "seaorm-sqlite"))]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use magnetar::migration::{
    AppUser, BackendStrategy, DurableAuthRecord, ImportedUser, MigrationBackend, MigrationBindings,
    MigrationEngine, MigrationRunner, MigrationTransaction, ShapeConfirmation, SourceShape,
};
use magnetar::{Error, Result};
use sea_orm::{
    ConnectionTrait, Database, DatabaseConnection, DatabaseTransaction, DbBackend, Statement,
};

static NEXT_FIXTURE_COPY: AtomicU64 = AtomicU64::new(0);

#[derive(Default)]
struct NoopBindings;

#[async_trait::async_trait]
impl MigrationBindings for NoopBindings {
    async fn app_users(&self) -> Result<Vec<AppUser>> {
        Ok(Vec::new())
    }

    async fn begin_transaction<'a>(
        &'a self,
        _source_transaction: Option<&'a DatabaseTransaction>,
    ) -> Result<Box<dyn MigrationTransaction + 'a>> {
        Ok(Box::new(NoopTransaction))
    }
}

struct NoopTransaction;

#[async_trait::async_trait]
impl MigrationTransaction for NoopTransaction {
    async fn app_users(&mut self) -> Result<Vec<AppUser>> {
        Ok(Vec::new())
    }

    async fn import_user(&mut self, _user: &ImportedUser) -> Result<AppUser> {
        Err(Error::Internal {
            message: "shape tests must not import app users".to_owned(),
        })
    }

    async fn bind_external_identity(
        &mut self,
        _provider: &str,
        _external_user_id: &str,
        _app_user_id: i64,
    ) -> Result<()> {
        Ok(())
    }

    async fn import_passkey(
        &mut self,
        _app_user_id: i64,
        _credential_id: &str,
        _data_json: &str,
    ) -> Result<()> {
        Ok(())
    }

    async fn import_durable_record(&mut self, _record: DurableAuthRecord) -> Result<()> {
        Ok(())
    }

    async fn imports_committed(&mut self, _plan_id: &str) -> Result<bool> {
        Ok(false)
    }

    async fn resolved_app_user_id(
        &mut self,
        _plan_id: &str,
        _source_user_id: &str,
    ) -> Result<Option<i64>> {
        Ok(None)
    }

    async fn record_identity_resolution(
        &mut self,
        _plan_id: &str,
        _source_user_id: &str,
        _app_user_id: i64,
    ) -> Result<()> {
        Ok(())
    }

    async fn mark_imports_committed(&mut self, _plan_id: &str) -> Result<()> {
        Ok(())
    }

    async fn commit(self: Box<Self>) -> Result<()> {
        Ok(())
    }

    async fn rollback(self: Box<Self>) -> Result<()> {
        Ok(())
    }
}

fn fixture_copy(name: &str) -> PathBuf {
    let copy_id = NEXT_FIXTURE_COPY.fetch_add(1, Ordering::Relaxed);
    let destination = std::env::temp_dir().join(format!(
        "magnetar-migration-{name}-{}-{copy_id}.sqlite",
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

#[tokio::test]
async fn detects_each_frozen_source_shape_and_magnetar_marker() {
    for (fixture, expected) in [
        ("torii", SourceShape::Torii),
        ("suprnova-web", SourceShape::SuprnovaWeb),
        ("suprnova-api", SourceShape::SuprnovaApi),
    ] {
        let (database, path) = open_fixture(fixture).await;
        let runner = MigrationEngine::new(database, NoopBindings);
        assert_eq!(runner.detect_shape().await.unwrap(), expected);
        drop(runner);
        fs::remove_file(path).unwrap();
    }

    let database = Database::connect("sqlite::memory:").await.unwrap();
    database
        .execute(Statement::from_string(
            DbBackend::Sqlite,
            "CREATE TABLE magnetar_migration_state (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
        ))
        .await
        .unwrap();
    database
        .execute(Statement::from_string(
            DbBackend::Sqlite,
            "INSERT INTO magnetar_migration_state (key, value) VALUES ('schema_version', '1')",
        ))
        .await
        .unwrap();
    let runner = MigrationEngine::new(database, NoopBindings);
    assert_eq!(runner.detect_shape().await.unwrap(), SourceShape::Magnetar);
}

#[tokio::test]
async fn rejects_missing_confirmation_and_mismatch_before_planning_or_writes() {
    let (database, path) = open_fixture("torii").await;
    let runner = MigrationEngine::new(database.clone(), NoopBindings);

    let missing = runner.dry_run_optional(None).await.unwrap_err();
    assert!(matches!(missing, Error::InvalidInput { .. }));

    let mismatch = runner
        .dry_run(ShapeConfirmation {
            detected: SourceShape::Torii,
            operator_selected: SourceShape::SuprnovaWeb,
        })
        .await
        .unwrap_err();
    assert!(matches!(mismatch, Error::Conflict { .. }));
    assert_eq!(count(&database, "users").await, 3);
    assert_eq!(count(&database, "sessions").await, 1);

    drop(runner);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn rejects_hybrid_and_half_transformed_databases_before_any_write() {
    let (database, path) = open_fixture("torii").await;
    database
        .execute(Statement::from_string(
            DbBackend::Sqlite,
            "CREATE TABLE app_users (id INTEGER PRIMARY KEY, email TEXT NOT NULL)",
        ))
        .await
        .unwrap();
    let runner = MigrationEngine::new(database.clone(), NoopBindings);

    let error = runner.detect_shape().await.unwrap_err();
    assert!(matches!(error, Error::Conflict { .. }));
    assert_eq!(count(&database, "users").await, 3);
    assert_eq!(count(&database, "sessions").await, 1);

    drop(runner);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn dry_run_is_stable_and_does_not_write_fixture_data() {
    let (database, path) = open_fixture("suprnova-web").await;
    let runner = MigrationEngine::new(database.clone(), NoopBindings);
    let confirmation = ShapeConfirmation {
        detected: SourceShape::SuprnovaWeb,
        operator_selected: SourceShape::SuprnovaWeb,
    };

    let first = runner.dry_run(confirmation.clone()).await.unwrap();
    let second = runner.dry_run(confirmation).await.unwrap();
    assert_eq!(first, second);
    assert!(first.normalized_collisions.is_empty());
    assert!(!first.source_row_counts.is_empty());
    assert!(!first.field_mappings.is_empty());
    assert_eq!(
        first.backend_strategy,
        BackendStrategy::Transactional {
            backend: MigrationBackend::Sqlite,
        }
    );
    assert!(
        first
            .source_row_counts
            .iter()
            .any(|count| count.table == "users" && count.rows == 1)
    );
    assert_eq!(
        first
            .field_mappings
            .iter()
            .map(|mapping| mapping.source.as_str())
            .collect::<Vec<_>>(),
        vec![
            "two_factor_credentials.confirmed_at",
            "two_factor_credentials.last_used_timestep",
            "two_factor_credentials.recovery_codes",
            "two_factor_credentials.secret",
            "two_factor_credentials.user_id",
            "users.email",
            "users.email_verified_at",
            "users.id",
            "users.name",
            "users.password",
        ]
    );
    assert_eq!(count(&database, "sessions").await, 1);
    assert_eq!(count(&database, "auth_flow_tokens").await, 0);
    drop(runner);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn dry_run_field_mapping_sets_cover_torii_and_api_fixture_contracts() {
    let (torii, torii_path) = open_fixture("torii").await;
    torii
        .execute(Statement::from_string(
            DbBackend::Sqlite,
            "DELETE FROM users WHERE id = 'torii-user-collision-lower'",
        ))
        .await
        .unwrap();
    let torii_runner = MigrationEngine::new(torii, NoopBindings);
    let torii_plan = torii_runner
        .dry_run(ShapeConfirmation {
            detected: SourceShape::Torii,
            operator_selected: SourceShape::Torii,
        })
        .await
        .unwrap();
    assert_eq!(
        torii_plan
            .field_mappings
            .iter()
            .map(|mapping| mapping.source.as_str())
            .collect::<Vec<_>>(),
        vec![
            "failed_login_attempts.attempted_at",
            "failed_login_attempts.email",
            "failed_login_attempts.ip_address",
            "oauth_accounts.provider",
            "oauth_accounts.subject",
            "oauth_accounts.user_id",
            "passkeys.credential_id",
            "passkeys.data_json",
            "passkeys.user_id",
            "secure_tokens.expires_at",
            "secure_tokens.purpose",
            "secure_tokens.used_at",
            "secure_tokens.user_id",
            "users.email",
            "users.email_verified_at",
            "users.id",
            "users.locked_at",
            "users.name",
            "users.password_hash",
        ]
    );
    drop(torii_runner);
    fs::remove_file(torii_path).unwrap();

    let (api, api_path) = open_fixture("suprnova-api").await;
    let api_runner = MigrationEngine::new(api, NoopBindings);
    let api_plan = api_runner
        .dry_run(ShapeConfirmation {
            detected: SourceShape::SuprnovaApi,
            operator_selected: SourceShape::SuprnovaApi,
        })
        .await
        .unwrap();
    assert_eq!(
        api_plan
            .field_mappings
            .iter()
            .map(|mapping| mapping.source.as_str())
            .collect::<Vec<_>>(),
        vec!["app_users.email", "app_users.id"]
    );
    drop(api_runner);
    fs::remove_file(api_path).unwrap();
}

async fn count(database: &DatabaseConnection, table: &str) -> i64 {
    let row = database
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            format!("SELECT COUNT(*) FROM {table}"),
        ))
        .await
        .unwrap()
        .unwrap();
    row.try_get_by_index(0).unwrap()
}
