#![cfg(all(feature = "migration", feature = "seaorm-sqlite"))]

use parking_lot::Mutex;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use magnetar::migration::{
    AppUser, DurableAuthRecord, ExternalIdentity, ImportedPasskey, ImportedUser, MigrationBindings,
    MigrationEngine, MigrationRunner, MigrationTransaction, ShapeConfirmation, SourceShape,
};
use magnetar::{Error, Result};
use sea_orm::{
    ConnectionTrait, Database, DatabaseConnection, DatabaseTransaction, DbBackend, Statement,
};
use secrecy::ExposeSecret;

static NEXT_FIXTURE_COPY: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
struct RecordingBindings {
    state: Arc<Mutex<BindingState>>,
}

#[derive(Clone, Default)]
struct BindingState {
    users: Vec<AppUser>,
    created: Vec<AppUser>,
    external_identities: Vec<ExternalIdentity>,
    passkeys: Vec<ImportedPasskey>,
    imported_users: Vec<ImportedUser>,
    durable_records: Vec<DurableAuthRecord>,
    committed_plans: BTreeSet<String>,
    identity_resolutions: BTreeMap<(String, String), i64>,
    fail_on_passkey: bool,
}

impl RecordingBindings {
    fn with_user(user: AppUser) -> Self {
        Self {
            state: Arc::new(Mutex::new(BindingState {
                users: vec![user],
                ..BindingState::default()
            })),
        }
    }
    fn failing_on_passkey(user: AppUser) -> Self {
        let bindings = Self::with_user(user);
        bindings.state.lock().fail_on_passkey = true;
        bindings
    }

    fn snapshot(&self) -> BindingState {
        let state = self.state.lock();
        BindingState {
            users: state.users.clone(),
            created: state.created.clone(),
            external_identities: state.external_identities.clone(),
            passkeys: state.passkeys.clone(),
            imported_users: state.imported_users.clone(),
            durable_records: state.durable_records.clone(),
            committed_plans: state.committed_plans.clone(),
            identity_resolutions: state.identity_resolutions.clone(),
            fail_on_passkey: state.fail_on_passkey,
        }
    }
}

#[async_trait::async_trait]
impl MigrationBindings for RecordingBindings {
    async fn app_users(&self) -> Result<Vec<AppUser>> {
        Ok(self.state.lock().users.clone())
    }

    async fn begin_transaction<'a>(
        &'a self,
        _source_transaction: Option<&'a DatabaseTransaction>,
    ) -> Result<Box<dyn MigrationTransaction + 'a>> {
        Ok(Box::new(RecordingTransaction {
            target: self.state.clone(),
            staged: self.state.lock().clone(),
        }))
    }
}

struct RecordingTransaction {
    target: Arc<Mutex<BindingState>>,
    staged: BindingState,
}

#[async_trait::async_trait]
impl MigrationTransaction for RecordingTransaction {
    async fn app_users(&mut self) -> Result<Vec<AppUser>> {
        Ok(self.staged.users.clone())
    }

    async fn import_user(&mut self, imported: &ImportedUser) -> Result<AppUser> {
        let id = imported.preferred_app_user_id.unwrap_or_else(|| {
            self.staged
                .users
                .iter()
                .map(|user| user.id)
                .max()
                .unwrap_or(0)
                + 1
        });
        let user = if let Some(existing) = self.staged.users.iter().find(|user| user.id == id) {
            existing.clone()
        } else {
            let user = AppUser {
                id,
                email: imported.email.clone(),
                auth_epoch: imported.auth_epoch.unwrap_or(0),
                session_version: imported.session_version.unwrap_or(0),
            };
            self.staged.users.push(user.clone());
            self.staged.created.push(user.clone());
            user
        };
        self.staged.imported_users.push(imported.clone());
        Ok(user)
    }

    async fn bind_external_identity(
        &mut self,
        provider: &str,
        external_user_id: &str,
        app_user_id: i64,
    ) -> Result<()> {
        self.staged.external_identities.push(ExternalIdentity {
            provider: provider.to_owned(),
            external_user_id: external_user_id.to_owned(),
            app_user_id,
        });
        Ok(())
    }

    async fn import_passkey(
        &mut self,
        app_user_id: i64,
        credential_id: &str,
        data_json: &str,
    ) -> Result<()> {
        if self.staged.fail_on_passkey {
            return Err(Error::Internal {
                message: "injected passkey import failure".to_owned(),
            });
        }
        self.staged.passkeys.push(ImportedPasskey {
            app_user_id,
            credential_id: credential_id.to_owned(),
            data_json: data_json.to_owned(),
        });
        Ok(())
    }

    async fn import_durable_record(&mut self, record: DurableAuthRecord) -> Result<()> {
        self.staged.durable_records.push(record);
        Ok(())
    }

    async fn imports_committed(&mut self, plan_id: &str) -> Result<bool> {
        Ok(self.staged.committed_plans.contains(plan_id))
    }

    async fn resolved_app_user_id(
        &mut self,
        plan_id: &str,
        source_user_id: &str,
    ) -> Result<Option<i64>> {
        Ok(self
            .staged
            .identity_resolutions
            .get(&(plan_id.to_owned(), source_user_id.to_owned()))
            .copied())
    }

    async fn record_identity_resolution(
        &mut self,
        plan_id: &str,
        source_user_id: &str,
        app_user_id: i64,
    ) -> Result<()> {
        let key = (plan_id.to_owned(), source_user_id.to_owned());
        if let Some(existing) = self.staged.identity_resolutions.insert(key, app_user_id)
            && existing != app_user_id
        {
            return Err(Error::Conflict {
                resource: "migration identity ledger".to_owned(),
                message: "source identity already resolved differently".to_owned(),
            });
        }
        Ok(())
    }

    async fn mark_imports_committed(&mut self, plan_id: &str) -> Result<()> {
        self.staged.committed_plans.insert(plan_id.to_owned());
        Ok(())
    }

    async fn commit(self: Box<Self>) -> Result<()> {
        *self.target.lock() = self.staged;
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
async fn dry_run_rejects_missing_base_user_import_columns() {
    let (database, path) = open_fixture("torii").await;
    database
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "DELETE FROM users WHERE id = 'torii-user-collision-lower'",
        ))
        .await
        .unwrap();
    database
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "ALTER TABLE users DROP COLUMN locked_at",
        ))
        .await
        .unwrap();
    let runner = MigrationEngine::new(
        database,
        RecordingBindings::with_user(AppUser {
            id: 4242,
            email: "fixture.passwordless@example.test".to_owned(),
            auth_epoch: 19,
            session_version: 23,
        }),
    );

    let error = runner
        .dry_run(ShapeConfirmation {
            detected: SourceShape::Torii,
            operator_selected: SourceShape::Torii,
        })
        .await
        .unwrap_err();
    let Error::Conflict { message, .. } = error else {
        panic!("missing base user column must be a conflict");
    };
    assert!(message.contains("users"));
    assert!(message.contains("locked_at"));

    drop(runner);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn dry_run_rejects_present_but_incomplete_durable_source_tables() {
    let (database, path) = open_fixture("torii").await;
    database
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "DELETE FROM users WHERE id = 'torii-user-collision-lower'",
        ))
        .await
        .unwrap();
    database
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "DROP TABLE secure_tokens",
        ))
        .await
        .unwrap();
    database.execute_raw(Statement::from_string(DbBackend::Sqlite,
    "CREATE TABLE secure_tokens (id INTEGER PRIMARY KEY, user_id TEXT NOT NULL, token TEXT NOT NULL)",))
        .await
        .unwrap();
    let runner = MigrationEngine::new(
        database.clone(),
        RecordingBindings::with_user(AppUser {
            id: 4242,
            email: "fixture.passwordless@example.test".to_owned(),
            auth_epoch: 19,
            session_version: 23,
        }),
    );

    let error = runner
        .dry_run(ShapeConfirmation {
            detected: SourceShape::Torii,
            operator_selected: SourceShape::Torii,
        })
        .await
        .unwrap_err();
    let Error::Conflict { message, .. } = error else {
        panic!("incomplete durable table must be a conflict");
    };
    assert!(message.contains("secure_tokens"));
    drop(runner);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn normalized_email_collisions_enumerate_every_owner_and_abort_before_writes() {
    let (database, path) = open_fixture("torii").await;
    let runner = MigrationEngine::new(
        database.clone(),
        RecordingBindings::with_user(AppUser {
            id: 4242,
            email: "fixture.passwordless@example.test".to_owned(),
            auth_epoch: 19,
            session_version: 23,
        }),
    );

    let error = runner
        .dry_run(ShapeConfirmation {
            detected: SourceShape::Torii,
            operator_selected: SourceShape::Torii,
        })
        .await
        .unwrap_err();
    let Error::Conflict { message, .. } = error else {
        panic!("collision preflight must be a conflict");
    };
    assert!(message.contains("fixture.collision@example.test"));
    assert!(message.contains("users:torii-user-collision-upper"));
    assert!(message.contains("users:torii-user-collision-lower"));
    assert_eq!(count(&database, "users").await, 3);
    assert_eq!(count(&database, "sessions").await, 1);

    drop(runner);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn torii_mapping_preserves_app_i64_identity_and_passkey_bytes() {
    let (database, path) = open_fixture("torii").await;
    database
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "DELETE FROM users WHERE id = 'torii-user-collision-lower'",
        ))
        .await
        .unwrap();
    let expected_data_json = string_value(
        &database,
        "SELECT data_json FROM passkeys WHERE user_id = 'torii-user-passwordless'",
    )
    .await;
    let bindings = RecordingBindings::with_user(AppUser {
        id: 4242,
        email: "fixture.passwordless@example.test".to_owned(),
        auth_epoch: 19,
        session_version: 23,
    });
    let runner = MigrationEngine::new(database.clone(), bindings.clone());
    let plan = runner
        .dry_run(ShapeConfirmation {
            detected: SourceShape::Torii,
            operator_selected: SourceShape::Torii,
        })
        .await
        .unwrap();
    assert_eq!(plan.identity_map.existing_app_user_ids(), vec![4242]);
    assert_eq!(plan.identity_map.pending_creates(), 1);

    let report = runner.apply(&plan).await.unwrap();
    assert_eq!(report.identity_mappings.len(), 2);
    let state = bindings.snapshot();
    assert_eq!(state.created.len(), 1);
    assert_eq!(state.created[0].auth_epoch, 0);
    assert_eq!(state.created[0].session_version, 0);
    assert_eq!(
        state
            .users
            .iter()
            .find(|user| user.id == 4242)
            .unwrap()
            .auth_epoch,
        19
    );
    assert_eq!(
        state
            .users
            .iter()
            .find(|user| user.id == 4242)
            .unwrap()
            .session_version,
        23
    );
    assert!(state.external_identities.iter().any(|identity| {
        identity.provider == "torii"
            && identity.external_user_id == "torii-user-passwordless"
            && identity.app_user_id == 4242
    }));
    assert_eq!(state.passkeys.len(), 1);
    assert_eq!(state.passkeys[0].app_user_id, 4242);
    assert_eq!(state.passkeys[0].data_json, expected_data_json);
    assert_eq!(count(&database, "sessions").await, 0);
    assert_eq!(count(&database, "pkce_verifiers").await, 0);
    assert_eq!(count(&database, "passkey_challenges").await, 0);
    assert_eq!(count(&database, "torii_migrations").await, 0);

    drop(runner);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn torii_apply_imports_every_promised_durable_auth_record() {
    let (database, path) = open_fixture("torii").await;
    database
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "DELETE FROM users WHERE id = 'torii-user-collision-lower'",
        ))
        .await
        .unwrap();
    database.execute_raw(Statement::from_string(DbBackend::Sqlite,
    "INSERT INTO secure_tokens (user_id, token, purpose, used_at, expires_at, created_at, updated_at) VALUES ('torii-user-passwordless', 'fixture-secure-token', 'email_verification', NULL, '2030-01-01T00:00:00+00:00', '2024-01-02T03:04:05+00:00', '2024-01-02T03:04:05+00:00')",))
        .await
        .unwrap();
    database.execute_raw(Statement::from_string(DbBackend::Sqlite,
    "INSERT INTO failed_login_attempts (email, ip_address, attempted_at) VALUES ('fixture.passwordless@example.test', '203.0.113.7', '2024-01-02T03:04:05+00:00')",))
        .await
        .unwrap();
    let bindings = RecordingBindings::with_user(AppUser {
        id: 4242,
        email: "fixture.passwordless@example.test".to_owned(),
        auth_epoch: 19,
        session_version: 23,
    });
    let runner = MigrationEngine::new(database.clone(), bindings.clone());
    let plan = runner
        .dry_run(ShapeConfirmation {
            detected: SourceShape::Torii,
            operator_selected: SourceShape::Torii,
        })
        .await
        .unwrap();

    runner.apply(&plan).await.unwrap();
    let state = bindings.snapshot();
    let imported = state
        .imported_users
        .iter()
        .find(|user| user.source_user_id == "torii-user-passwordless")
        .unwrap();
    assert_eq!(imported.preferred_app_user_id, Some(4242));
    assert_eq!(imported.name, None);
    assert_eq!(
        imported.email_verified_at.as_deref(),
        Some("2024-01-02T03:04:05+00:00")
    );
    assert!(state.durable_records.iter().any(|record| matches!(
        record,
        DurableAuthRecord::LinkedAccount(account)
            if account.app_user_id == 4242
                && account.provider == "fixture-provider"
                && account.subject == "fixture-subject"
    )));
    assert!(state.durable_records.iter().any(|record| matches!(
        record,
        DurableAuthRecord::SecureToken(token)
            if token.app_user_id == 4242
                && token.token.expose_secret() == "fixture-secure-token"
                && token.purpose == "email_verification"
    )));
    assert!(state.durable_records.iter().any(|record| matches!(
        record,
        DurableAuthRecord::FailedLoginAttempt(attempt)
            if attempt.email == "fixture.passwordless@example.test"
                && attempt.ip_address.as_deref() == Some("203.0.113.7")
    )));

    drop(runner);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn failed_import_rolls_back_every_binding_write_and_source_cleanup() {
    let (database, path) = open_fixture("torii").await;
    database
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "DELETE FROM users WHERE id = 'torii-user-collision-lower'",
        ))
        .await
        .unwrap();
    let bindings = RecordingBindings::failing_on_passkey(AppUser {
        id: 4242,
        email: "fixture.passwordless@example.test".to_owned(),
        auth_epoch: 19,
        session_version: 23,
    });
    let runner = MigrationEngine::new(database.clone(), bindings.clone());
    let plan = runner
        .dry_run(ShapeConfirmation {
            detected: SourceShape::Torii,
            operator_selected: SourceShape::Torii,
        })
        .await
        .unwrap();

    let error = runner.apply(&plan).await.unwrap_err();
    assert!(matches!(error, Error::Internal { .. }));
    let state = bindings.snapshot();
    assert!(state.created.is_empty());
    assert!(state.external_identities.is_empty());
    assert!(state.passkeys.is_empty());
    assert_eq!(count(&database, "sessions").await, 1);

    drop(runner);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn dry_run_rejects_malformed_cleanup_table_before_deletion() {
    let (database, path) = open_fixture("suprnova-web").await;
    database
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "DROP TABLE sessions",
        ))
        .await
        .unwrap();
    database
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "CREATE TABLE sessions (id TEXT PRIMARY KEY, unrelated_payload TEXT NOT NULL)",
        ))
        .await
        .unwrap();
    let runner = MigrationEngine::new(
        database.clone(),
        RecordingBindings::with_user(AppUser {
            id: 7,
            email: "other@example.test".to_owned(),
            auth_epoch: 0,
            session_version: 0,
        }),
    );

    let error = runner
        .dry_run(ShapeConfirmation {
            detected: SourceShape::SuprnovaWeb,
            operator_selected: SourceShape::SuprnovaWeb,
        })
        .await
        .unwrap_err();
    let Error::Conflict { message, .. } = error else {
        panic!("malformed cleanup table must be a conflict");
    };
    assert!(message.contains("sessions"));

    drop(runner);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn web_source_id_maps_to_matching_host_user_with_different_id() {
    let (database, path) = open_fixture("suprnova-web").await;
    let bindings = RecordingBindings::with_user(AppUser {
        id: 42,
        email: "fixture.web.passwordless@example.test".to_owned(),
        auth_epoch: 9,
        session_version: 11,
    });
    let runner = MigrationEngine::new(database, bindings.clone());
    let plan = runner
        .dry_run(ShapeConfirmation {
            detected: SourceShape::SuprnovaWeb,
            operator_selected: SourceShape::SuprnovaWeb,
        })
        .await
        .unwrap();
    assert_eq!(plan.identity_map.existing_app_user_ids(), vec![42]);

    runner.apply(&plan).await.unwrap();
    let state = bindings.snapshot();
    assert!(state.created.is_empty());
    assert!(state.durable_records.iter().any(|record| matches!(
        record,
        DurableAuthRecord::TwoFactorCredential(two_factor)
            if two_factor.app_user_id == 42
    )));

    drop(runner);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn cleanup_kill_list_invalidates_web_sessions_remember_and_auth_flow_rows() {
    let (database, path) = open_fixture("suprnova-web").await;
    database.execute_raw(Statement::from_string(DbBackend::Sqlite,
    "INSERT INTO remember_tokens (user_id, selector, token_hash, expires_at) VALUES ('1001', 'fixture-selector', 'fixture-hash', '2030-01-01')",))
        .await
        .unwrap();
    database.execute_raw(Statement::from_string(DbBackend::Sqlite,
    "INSERT INTO auth_flow_tokens (user_id, token_hash, purpose, expires_at, created_at) VALUES ('1001', 'fixture-flow', 'login', '2030-01-01', '2024-01-02')",))
        .await
        .unwrap();
    database
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "UPDATE users SET remember_token = 'legacy-token' WHERE id = 1001",
        ))
        .await
        .unwrap();
    let bindings = RecordingBindings::with_user(AppUser {
        id: 4242,
        email: "existing@example.test".to_owned(),
        auth_epoch: 0,
        session_version: 0,
    });
    let runner = MigrationEngine::new(database.clone(), bindings.clone());
    let plan = runner
        .dry_run(ShapeConfirmation {
            detected: SourceShape::SuprnovaWeb,
            operator_selected: SourceShape::SuprnovaWeb,
        })
        .await
        .unwrap();
    assert!(plan.sacrificeable_cleanup.invalidates("sessions"));
    assert!(plan.sacrificeable_cleanup.invalidates("remember_tokens"));
    assert!(plan.sacrificeable_cleanup.invalidates("auth_flow_tokens"));
    assert!(
        plan.sacrificeable_cleanup
            .invalidates("auth_ceremony_tokens")
    );
    assert!(plan.sacrificeable_cleanup.invalidates("passkey_challenges"));
    assert!(plan.sacrificeable_cleanup.invalidates("pkce_verifiers"));
    assert!(
        plan.sacrificeable_cleanup
            .invalidates("users.remember_token")
    );
    assert!(!plan.sacrificeable_cleanup.invalidates("torii_migrations"));

    runner.apply(&plan).await.unwrap();
    assert_eq!(count(&database, "sessions").await, 0);
    assert_eq!(count(&database, "remember_tokens").await, 0);
    assert_eq!(count(&database, "auth_flow_tokens").await, 0);
    assert_eq!(count(&database, "users").await, 1);
    assert_eq!(
        optional_string(
            &database,
            "SELECT remember_token FROM users WHERE id = 1001"
        )
        .await,
        None
    );
    assert_eq!(count(&database, "two_factor_credentials").await, 1);
    let state = bindings.snapshot();
    assert_eq!(state.imported_users.len(), 1);
    assert_eq!(state.imported_users[0].preferred_app_user_id, None);
    assert_eq!(state.imported_users[0].password_hash.as_deref(), Some(""));
    assert!(state.durable_records.iter().any(|record| matches!(
        record,
        DurableAuthRecord::TwoFactorCredential(two_factor)
            if two_factor.app_user_id == 4243
                && two_factor.secret.expose_secret()
                    == "K0A7aRihcbZ98aBhIAsJVCK9FDkKJEuw_B5XVMaUiyukQDe-Sd7kDHrYt8U"
    )));

    drop(runner);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn suprnova_api_apply_preserves_the_existing_i64_app_identity() {
    let (database, path) = open_fixture("suprnova-api").await;
    let bindings = RecordingBindings::with_user(AppUser {
        id: 7,
        email: "other@example.test".to_owned(),
        auth_epoch: 0,
        session_version: 0,
    });
    let runner = MigrationEngine::new(database.clone(), bindings.clone());
    let plan = runner
        .dry_run(ShapeConfirmation {
            detected: SourceShape::SuprnovaApi,
            operator_selected: SourceShape::SuprnovaApi,
        })
        .await
        .unwrap();

    runner.apply(&plan).await.unwrap();
    let state = bindings.snapshot();
    assert_eq!(state.imported_users.len(), 1);
    assert_eq!(state.imported_users[0].preferred_app_user_id, Some(4242));
    assert_eq!(
        state.imported_users[0].email,
        "fixture.api.user@example.test"
    );

    drop(runner);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn dry_run_fingerprints_non_utf8_binary_columns() {
    let (database, path) = open_fixture("torii").await;
    database
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "DELETE FROM users WHERE id = 'torii-user-collision-lower'",
        ))
        .await
        .unwrap();
    database
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "CREATE TABLE binary_evidence (id INTEGER PRIMARY KEY, payload BLOB NOT NULL)",
        ))
        .await
        .unwrap();
    database
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "INSERT INTO binary_evidence (payload) VALUES (X'FF00FE')",
        ))
        .await
        .unwrap();
    let runner = MigrationEngine::new(
        database,
        RecordingBindings::with_user(AppUser {
            id: 4242,
            email: "fixture.passwordless@example.test".to_owned(),
            auth_epoch: 19,
            session_version: 23,
        }),
    );

    let plan = runner
        .dry_run(ShapeConfirmation {
            detected: SourceShape::Torii,
            operator_selected: SourceShape::Torii,
        })
        .await
        .unwrap();
    assert!(
        plan.source_fingerprints
            .iter()
            .any(|entry| entry.table == "binary_evidence")
    );

    drop(runner);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn apply_rejects_schema_only_change_before_writes() {
    let (database, path) = open_fixture("suprnova-web").await;
    let bindings = RecordingBindings::with_user(AppUser {
        id: 1,
        email: "existing@example.test".to_owned(),
        auth_epoch: 0,
        session_version: 0,
    });
    let runner = MigrationEngine::new(database.clone(), bindings.clone());
    let plan = runner
        .dry_run(ShapeConfirmation {
            detected: SourceShape::SuprnovaWeb,
            operator_selected: SourceShape::SuprnovaWeb,
        })
        .await
        .unwrap();
    database
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "CREATE INDEX users_name_after_review ON users(name)",
        ))
        .await
        .unwrap();

    let error = runner.apply(&plan).await.unwrap_err();
    assert!(matches!(error, Error::Conflict { .. }));
    assert!(bindings.snapshot().imported_users.is_empty());
    assert_eq!(count(&database, "sessions").await, 1);

    drop(runner);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn apply_rejects_text_change_after_embedded_nul() {
    let (database, path) = open_fixture("suprnova-web").await;
    database
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "UPDATE users SET password = 'hash' || char(0) || 'before' WHERE id = 1001",
        ))
        .await
        .unwrap();
    let bindings = RecordingBindings::with_user(AppUser {
        id: 1,
        email: "existing@example.test".to_owned(),
        auth_epoch: 0,
        session_version: 0,
    });
    let runner = MigrationEngine::new(database.clone(), bindings.clone());
    let plan = runner
        .dry_run(ShapeConfirmation {
            detected: SourceShape::SuprnovaWeb,
            operator_selected: SourceShape::SuprnovaWeb,
        })
        .await
        .unwrap();
    database
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "UPDATE users SET password = 'hash' || char(0) || 'after' WHERE id = 1001",
        ))
        .await
        .unwrap();

    let error = runner.apply(&plan).await.unwrap_err();
    assert!(matches!(error, Error::Conflict { .. }));
    assert!(bindings.snapshot().imported_users.is_empty());
    assert_eq!(count(&database, "sessions").await, 1);

    drop(runner);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn apply_rejects_same_shape_source_mutation_before_writes() {
    let (database, path) = open_fixture("suprnova-web").await;
    let bindings = RecordingBindings::with_user(AppUser {
        id: 1,
        email: "existing@example.test".to_owned(),
        auth_epoch: 0,
        session_version: 0,
    });
    let runner = MigrationEngine::new(database.clone(), bindings.clone());
    let plan = runner
        .dry_run(ShapeConfirmation {
            detected: SourceShape::SuprnovaWeb,
            operator_selected: SourceShape::SuprnovaWeb,
        })
        .await
        .unwrap();
    database
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "UPDATE users SET password = 'changed-after-review' WHERE id = 1001",
        ))
        .await
        .unwrap();

    let error = runner.apply(&plan).await.unwrap_err();
    assert!(matches!(error, Error::Conflict { .. }));
    assert!(bindings.snapshot().imported_users.is_empty());
    assert_eq!(count(&database, "sessions").await, 1);

    drop(runner);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn web_apply_rejects_destination_identity_change_before_writes() {
    let (database, path) = open_fixture("suprnova-web").await;
    let bindings = RecordingBindings::with_user(AppUser {
        id: 7,
        email: "other@example.test".to_owned(),
        auth_epoch: 0,
        session_version: 0,
    });
    let runner = MigrationEngine::new(database.clone(), bindings.clone());
    let plan = runner
        .dry_run(ShapeConfirmation {
            detected: SourceShape::SuprnovaWeb,
            operator_selected: SourceShape::SuprnovaWeb,
        })
        .await
        .unwrap();
    assert_eq!(plan.identity_map.pending_creates(), 1);
    bindings.state.lock().users.push(AppUser {
        id: 1001,
        email: "fixture.web.passwordless@example.test".to_owned(),
        auth_epoch: 0,
        session_version: 0,
    });

    let error = runner.apply(&plan).await.unwrap_err();
    assert!(matches!(error, Error::Conflict { .. }));
    assert!(bindings.snapshot().imported_users.is_empty());
    assert_eq!(count(&database, "sessions").await, 1);

    drop(runner);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn apply_rejects_destination_identity_change_before_writes() {
    let (database, path) = open_fixture("torii").await;
    database
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "DELETE FROM users WHERE id = 'torii-user-collision-lower'",
        ))
        .await
        .unwrap();
    let bindings = RecordingBindings::with_user(AppUser {
        id: 4242,
        email: "fixture.passwordless@example.test".to_owned(),
        auth_epoch: 19,
        session_version: 23,
    });
    let runner = MigrationEngine::new(database.clone(), bindings.clone());
    let plan = runner
        .dry_run(ShapeConfirmation {
            detected: SourceShape::Torii,
            operator_selected: SourceShape::Torii,
        })
        .await
        .unwrap();
    bindings.state.lock().users.push(AppUser {
        id: 9001,
        email: "Fixture.Collision@Example.test".to_owned(),
        auth_epoch: 0,
        session_version: 0,
    });

    let error = runner.apply(&plan).await.unwrap_err();
    assert!(matches!(error, Error::Conflict { .. }));
    assert!(bindings.snapshot().imported_users.is_empty());
    assert_eq!(count(&database, "sessions").await, 1);

    drop(runner);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn apply_rechecks_fresh_shape_before_writing() {
    let (database, path) = open_fixture("suprnova-web").await;
    let runner = MigrationEngine::new(
        database.clone(),
        RecordingBindings::with_user(AppUser {
            id: 1,
            email: "existing@example.test".to_owned(),
            auth_epoch: 0,
            session_version: 0,
        }),
    );
    let plan = runner
        .dry_run(ShapeConfirmation {
            detected: SourceShape::SuprnovaWeb,
            operator_selected: SourceShape::SuprnovaWeb,
        })
        .await
        .unwrap();
    database
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "CREATE TABLE magnetar_migration_state (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
        ))
        .await
        .unwrap();
    database
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "INSERT INTO magnetar_migration_state (key, value) VALUES ('schema_version', '1')",
        ))
        .await
        .unwrap();

    let error = runner.apply(&plan).await.unwrap_err();
    assert!(matches!(error, Error::Conflict { .. }));
    assert_eq!(count(&database, "sessions").await, 1);

    drop(runner);
    fs::remove_file(path).unwrap();
}

async fn count(database: &DatabaseConnection, table: &str) -> i64 {
    let row = database
        .query_one_raw(Statement::from_string(
            DbBackend::Sqlite,
            format!("SELECT COUNT(*) FROM {table}"),
        ))
        .await
        .unwrap()
        .unwrap();
    row.try_get_by_index(0).unwrap()
}

async fn string_value(database: &DatabaseConnection, query: &str) -> String {
    let row = database
        .query_one_raw(Statement::from_string(DbBackend::Sqlite, query))
        .await
        .unwrap()
        .unwrap();
    row.try_get_by_index(0).unwrap()
}

async fn optional_string(database: &DatabaseConnection, query: &str) -> Option<String> {
    let row = database
        .query_one_raw(Statement::from_string(DbBackend::Sqlite, query))
        .await
        .unwrap()
        .unwrap();
    row.try_get_by_index(0).unwrap()
}
