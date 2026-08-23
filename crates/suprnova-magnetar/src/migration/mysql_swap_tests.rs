use std::collections::BTreeMap;
use std::sync::Arc;

#[cfg(all(feature = "seaorm-mysql", feature = "seaorm-sqlite"))]
use crate::default_migration::DefaultMigrationBindings;
#[cfg(all(feature = "seaorm-mysql", feature = "seaorm-sqlite"))]
use crate::default_schema::{methods, users};
use crate::{Error, Result};
use parking_lot::Mutex;

use super::MigrationRecovery;
use super::fingerprint::TableFingerprint;
use super::mysql_swap::{
    JournalPhase, MySqlShadowSwap, MySqlSwapBackend, MySqlSwapRecovery, RenameState, SwapJournal,
    SwapTable,
};
#[cfg(all(feature = "seaorm-mysql", feature = "seaorm-sqlite"))]
use super::{MigrationEngine, MigrationRunner, ShapeConfirmation, SourceShape};
#[cfg(all(feature = "seaorm-mysql", feature = "seaorm-sqlite"))]
use sea_orm::{
    ColumnTrait, ConnectionTrait, Database, DbBackend, EntityTrait, QueryFilter, Statement,
};

#[derive(Clone)]
struct FakeMySql {
    state: Arc<Mutex<FakeMySqlState>>,
}

struct FakeMySqlState {
    tables: BTreeMap<String, TableFingerprint>,
    schemas: BTreeMap<String, String>,
    rename_calls: usize,
    fail_rename_call: Option<usize>,
    removed: Vec<String>,
    persist_calls: usize,
    fail_persist_call: Option<usize>,
    durable_journal: Option<SwapJournal>,
    barrier_held: bool,
    mutate_active_after_copy: bool,
    fail_copy_after_create: bool,
    degrade_shadow_schema: bool,
    cleanup_calls: usize,
}

impl FakeMySql {
    fn with_tables(names: &[&str], fail_rename_call: Option<usize>) -> Self {
        let fingerprint = fingerprint("source");
        Self {
            state: Arc::new(Mutex::new(FakeMySqlState {
                tables: names
                    .iter()
                    .map(|name| ((*name).to_owned(), fingerprint.clone()))
                    .collect(),
                schemas: names
                    .iter()
                    .map(|name| ((*name).to_owned(), "schema".to_owned()))
                    .collect(),
                rename_calls: 0,
                fail_rename_call,
                removed: Vec::new(),
                persist_calls: 0,
                fail_persist_call: None,
                durable_journal: None,
                barrier_held: true,
                mutate_active_after_copy: false,
                fail_copy_after_create: false,
                degrade_shadow_schema: false,
                cleanup_calls: 0,
            })),
        }
    }

    #[cfg(all(feature = "seaorm-mysql", feature = "seaorm-sqlite"))]
    fn with_fingerprints(fingerprints: BTreeMap<String, (TableFingerprint, String)>) -> Self {
        let schemas = fingerprints
            .iter()
            .map(|(table, (_, schema))| (table.clone(), schema.clone()))
            .collect();
        let tables = fingerprints
            .into_iter()
            .map(|(table, (fingerprint, _))| (table, fingerprint))
            .collect();
        Self {
            state: Arc::new(Mutex::new(FakeMySqlState {
                tables,
                schemas,
                rename_calls: 0,
                fail_rename_call: None,
                removed: Vec::new(),
                persist_calls: 0,
                fail_persist_call: None,
                durable_journal: None,
                barrier_held: false,
                mutate_active_after_copy: false,
                fail_copy_after_create: false,
                degrade_shadow_schema: false,
                cleanup_calls: 0,
            })),
        }
    }

    fn barrier_held(&self) -> bool {
        self.state.lock().barrier_held
    }

    fn cleanup_calls(&self) -> usize {
        self.state.lock().cleanup_calls
    }

    fn contains(&self, name: &str) -> bool {
        self.state.lock().tables.contains_key(name)
    }

    fn removed(&self) -> Vec<String> {
        self.state.lock().removed.clone()
    }

    fn durable_journal(&self) -> SwapJournal {
        self.state
            .lock()
            .durable_journal
            .clone()
            .expect("journal must have been persisted")
    }

    fn fail_persist_on(&self, call: Option<usize>) {
        self.state.lock().fail_persist_call = call;
    }

    fn fail_rename_on(&self, call: Option<usize>) {
        self.state.lock().fail_rename_call = call;
    }

    fn mutate_active_after_copy(&self) {
        self.state.lock().mutate_active_after_copy = true;
    }

    fn fail_copy_after_create(&self, fail: bool) {
        self.state.lock().fail_copy_after_create = fail;
    }

    fn replace_fingerprint(&self, table: &str, replacement: TableFingerprint) {
        self.state
            .lock()
            .tables
            .insert(table.to_owned(), replacement);
    }
    fn degrade_shadow_schema(&self) {
        self.state.lock().degrade_shadow_schema = true;
    }
}

#[async_trait::async_trait]
impl MySqlSwapBackend for FakeMySql {
    async fn acquire_write_barrier(&self) -> Result<()> {
        self.state.lock().barrier_held = true;
        Ok(())
    }

    async fn write_barrier_held(&self) -> Result<bool> {
        Ok(self.state.lock().barrier_held)
    }

    async fn release_write_barrier(&self) -> Result<()> {
        self.state.lock().barrier_held = false;
        Ok(())
    }

    async fn copy_to_shadow(&self, active: &str, shadow: &str) -> Result<()> {
        let mut state = self.state.lock();
        let source_fingerprint =
            state
                .tables
                .get(active)
                .cloned()
                .ok_or_else(|| Error::NotFound {
                    resource: "active table".to_owned(),
                    identifier: active.to_owned(),
                })?;
        let source_schema = state
            .schemas
            .get(active)
            .cloned()
            .ok_or_else(|| Error::NotFound {
                resource: "active table schema".to_owned(),
                identifier: active.to_owned(),
            })?;
        let shadow_schema = if state.degrade_shadow_schema {
            "degraded-schema".to_owned()
        } else {
            source_schema
        };
        state.schemas.insert(shadow.to_owned(), shadow_schema);
        state.tables.insert(shadow.to_owned(), source_fingerprint);
        if state.fail_copy_after_create {
            return Err(Error::Internal {
                message: "fault after shadow creation".to_owned(),
            });
        }
        if state.mutate_active_after_copy {
            state
                .tables
                .insert(active.to_owned(), fingerprint("changed-after-copy"));
        }
        Ok(())
    }

    async fn fingerprint(&self, table: &str) -> Result<TableFingerprint> {
        self.state
            .lock()
            .tables
            .get(table)
            .cloned()
            .ok_or_else(|| Error::NotFound {
                resource: "table".to_owned(),
                identifier: table.to_owned(),
            })
    }

    async fn schema_digest(&self, table: &str) -> Result<String> {
        self.state
            .lock()
            .schemas
            .get(table)
            .cloned()
            .ok_or_else(|| Error::NotFound {
                resource: "table schema".to_owned(),
                identifier: table.to_owned(),
            })
    }

    async fn table_exists(&self, table: &str) -> Result<bool> {
        Ok(self.state.lock().tables.contains_key(table))
    }

    async fn rename(&self, from: &str, to: &str) -> Result<()> {
        let mut state = self.state.lock();
        state.rename_calls += 1;
        if state.fail_rename_call == Some(state.rename_calls) {
            return Err(Error::Internal {
                message: format!("fault at rename {}", state.rename_calls),
            });
        }
        let fingerprint = state.tables.remove(from).ok_or_else(|| Error::NotFound {
            resource: "table".to_owned(),
            identifier: from.to_owned(),
        })?;
        state.tables.insert(to.to_owned(), fingerprint);
        let schema = state.schemas.remove(from).ok_or_else(|| Error::NotFound {
            resource: "table schema".to_owned(),
            identifier: from.to_owned(),
        })?;
        state.schemas.insert(to.to_owned(), schema);
        Ok(())
    }

    async fn remove_shadow(&self, table: &str) -> Result<()> {
        let mut state = self.state.lock();
        state.tables.remove(table);
        state.schemas.remove(table);
        state.removed.push(table.to_owned());
        Ok(())
    }

    async fn apply_cleanup(&self, _cleanup: &super::plan::SacrificeableCleanup) -> Result<usize> {
        self.state.lock().cleanup_calls += 1;
        Ok(1)
    }

    async fn persist_journal(&self, journal: &SwapJournal) -> Result<()> {
        let mut state = self.state.lock();
        state.persist_calls += 1;
        if state.fail_persist_call == Some(state.persist_calls) {
            return Err(Error::Internal {
                message: format!("fault at journal persistence {}", state.persist_calls),
            });
        }
        state.durable_journal = Some(journal.clone());
        Ok(())
    }
}

#[tokio::test]
async fn restore_recovers_when_only_first_active_table_reached_backup() {
    let backend = FakeMySql::with_tables(&["users", "credentials"], Some(2));
    let swap = MySqlShadowSwap;
    let failure = swap
        .execute(
            &backend,
            &[
                SwapTable::new("users", "users_shadow", "users_backup"),
                SwapTable::new("credentials", "credentials_shadow", "credentials_backup"),
            ],
            &Default::default(),
        )
        .await
        .unwrap_err();
    assert!(backend.contains("users_backup"));
    assert!(!backend.contains("users"));
    assert!(backend.contains("credentials"));
    assert!(backend.contains("credentials_shadow"));
    assert!(!backend.contains("credentials_backup"));
    let mut journal = failure.journal;

    let report = swap.restore(&backend, &mut journal).await.unwrap();
    assert_eq!(report.restored_tables, 2);
    assert!(backend.contains("users"));
    assert!(backend.contains("credentials"));
    assert!(!backend.contains("users_backup"));
    assert!(!backend.contains("credentials_backup"));
    assert!(!backend.contains("users_shadow"));
    assert!(!backend.contains("credentials_shadow"));
}

#[tokio::test]
async fn fault_before_swap_keeps_active_tables_and_abort_removes_only_shadows() {
    let backend = FakeMySql::with_tables(&["users"], Some(1));
    let swap = MySqlShadowSwap;
    let failure = swap
        .execute(
            &backend,
            &[SwapTable::new("users", "users_shadow", "users_backup")],
            &Default::default(),
        )
        .await
        .unwrap_err();

    assert!(backend.contains("users"));
    assert!(backend.contains("users_shadow"));
    assert!(!backend.contains("users_backup"));
    let mut journal = failure.journal;
    swap.abort(&backend, &mut journal).await.unwrap();
    assert!(backend.contains("users"));
    assert!(!backend.contains("users_shadow"));
    assert!(!backend.contains("users_backup"));
    assert_eq!(backend.removed(), vec!["users_shadow"]);
}

#[tokio::test]
async fn fault_after_partial_cutover_restores_completed_renames_in_reverse_order() {
    let backend = FakeMySql::with_tables(&["users", "credentials"], Some(4));
    let swap = MySqlShadowSwap;
    let failure = swap
        .execute(
            &backend,
            &[
                SwapTable::new("users", "users_shadow", "users_backup"),
                SwapTable::new("credentials", "credentials_shadow", "credentials_backup"),
            ],
            &Default::default(),
        )
        .await
        .unwrap_err();

    assert!(backend.contains("users"));
    assert!(backend.contains("credentials_backup"));
    assert!(backend.contains("credentials_shadow"));
    let mut journal = failure.journal;
    let report = swap.restore(&backend, &mut journal).await.unwrap();
    assert_eq!(report.restored_tables, 2);
    assert!(backend.contains("users"));
    assert!(backend.contains("credentials"));
    assert!(!backend.contains("users_backup"));
    assert!(!backend.contains("credentials_backup"));
}

#[tokio::test]
async fn abort_rejects_an_unverified_shadow_before_removal() {
    let backend = FakeMySql::with_tables(&["users"], None);
    backend.fail_persist_on(Some(4));
    let swap = MySqlShadowSwap;
    swap.execute(
        &backend,
        &[SwapTable::new("users", "users_shadow", "users_backup")],
        &Default::default(),
    )
    .await
    .unwrap_err();
    let mut journal = backend.durable_journal();
    backend.fail_persist_on(None);
    backend.replace_fingerprint("users_shadow", fingerprint("impostor"));

    let error = swap.abort(&backend, &mut journal).await.unwrap_err();
    assert!(matches!(error, Error::Conflict { .. }));
    assert!(backend.contains("users"));
    assert!(backend.contains("users_shadow"));
    assert!(!backend.contains("users_backup"));
}

#[tokio::test]
async fn restore_rejects_a_pre_cutover_journal() {
    let backend = FakeMySql::with_tables(&["users"], None);
    backend.fail_persist_on(Some(4));
    let swap = MySqlShadowSwap;
    swap.execute(
        &backend,
        &[SwapTable::new("users", "users_shadow", "users_backup")],
        &Default::default(),
    )
    .await
    .unwrap_err();
    let journal = backend.durable_journal();
    assert_eq!(journal.phase, JournalPhase::Prepared);
    backend.fail_persist_on(None);

    let error = MySqlSwapRecovery::new(backend.clone(), journal)
        .restore()
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Conflict { .. }));
    assert!(backend.contains("users"));
    assert!(backend.contains("users_shadow"));
    assert!(!backend.contains("users_backup"));
}

#[tokio::test]
async fn resume_rejects_a_modified_prepared_shadow_before_promotion() {
    let backend = FakeMySql::with_tables(&["users"], None);
    backend.fail_persist_on(Some(4));
    let swap = MySqlShadowSwap;
    swap.execute(
        &backend,
        &[SwapTable::new("users", "users_shadow", "users_backup")],
        &Default::default(),
    )
    .await
    .unwrap_err();
    let journal = backend.durable_journal();
    assert_eq!(journal.phase, JournalPhase::Prepared);
    backend.fail_persist_on(None);
    backend.replace_fingerprint("users_shadow", fingerprint("stale-shadow"));

    let failure = swap.resume(&backend, journal).await.unwrap_err();
    assert!(matches!(failure.error, Error::Conflict { .. }));
    assert!(backend.contains("users"));
    assert!(backend.contains("users_shadow"));
    assert!(!backend.contains("users_backup"));
}

#[tokio::test]
async fn resume_rejects_a_truncated_rename_plan_before_catalog_changes() {
    let backend = FakeMySql::with_tables(&["users"], None);
    backend.fail_persist_on(Some(4));
    let swap = MySqlShadowSwap;
    swap.execute(
        &backend,
        &[SwapTable::new("users", "users_shadow", "users_backup")],
        &Default::default(),
    )
    .await
    .unwrap_err();
    let mut corrupted = backend.durable_journal();
    corrupted.renames.clear();
    backend.fail_persist_on(None);

    let failure = swap.resume(&backend, corrupted).await.unwrap_err();
    assert!(matches!(failure.error, Error::Conflict { .. }));
    assert!(backend.contains("users"));
    assert!(backend.contains("users_shadow"));
    assert!(!backend.contains("users_backup"));
}

#[tokio::test]
async fn resumed_cutover_rejects_active_writes_after_preparation() {
    let backend = FakeMySql::with_tables(&["users"], None);
    backend.fail_persist_on(Some(4));
    let swap = MySqlShadowSwap;
    swap.execute(
        &backend,
        &[SwapTable::new("users", "users_shadow", "users_backup")],
        &Default::default(),
    )
    .await
    .unwrap_err();
    let prepared = backend.durable_journal();
    assert_eq!(prepared.phase, JournalPhase::Prepared);
    backend.fail_persist_on(None);
    backend.replace_fingerprint("users", fingerprint("newer-active-state"));

    let failure = swap.resume(&backend, prepared).await.unwrap_err();
    assert!(matches!(failure.error, Error::Conflict { .. }));
    assert!(backend.contains("users"));
    assert!(backend.contains("users_shadow"));
    assert!(!backend.contains("users_backup"));
}

#[tokio::test]
async fn resume_reconciles_a_rename_committed_before_journal_completion() {
    let backend = FakeMySql::with_tables(&["users"], None);
    backend.fail_persist_on(Some(6));
    let swap = MySqlShadowSwap;
    let _failure = swap
        .execute(
            &backend,
            &[SwapTable::new("users", "users_shadow", "users_backup")],
            &Default::default(),
        )
        .await
        .unwrap_err();
    assert!(!backend.contains("users"));
    assert!(backend.contains("users_backup"));
    let durable = backend.durable_journal();
    assert_eq!(durable.phase, JournalPhase::CuttingOver);
    assert_eq!(durable.renames[0].state, RenameState::Prepared);

    backend.fail_persist_on(None);
    let completed = swap.resume(&backend, durable).await.unwrap();
    assert_eq!(completed.phase, JournalPhase::Complete);
    assert!(backend.contains("users"));
    assert!(backend.contains("users_backup"));
    assert!(!backend.contains("users_shadow"));
}

#[tokio::test]
async fn restore_retry_skips_durably_restored_renames() {
    let backend = FakeMySql::with_tables(&["users"], None);
    let swap = MySqlShadowSwap;
    let mut journal = swap
        .execute(
            &backend,
            &[SwapTable::new("users", "users_shadow", "users_backup")],
            &Default::default(),
        )
        .await
        .unwrap();
    backend.fail_rename_on(Some(4));
    let error = swap.restore(&backend, &mut journal).await.unwrap_err();
    assert!(matches!(error, Error::Internal { .. }));
    assert_eq!(journal.renames[1].state, RenameState::Restored);
    assert_eq!(journal.renames[0].state, RenameState::RestorePrepared);

    backend.fail_rename_on(None);
    let report = swap.restore(&backend, &mut journal).await.unwrap();
    assert_eq!(report.restored_tables, 1);
    assert_eq!(journal.phase, JournalPhase::Restored);
    assert!(backend.contains("users"));
    assert!(!backend.contains("users_backup"));
}
#[cfg(all(feature = "seaorm-mysql", feature = "seaorm-sqlite"))]
#[tokio::test]
async fn plan_bound_coordinator_revalidates_imports_swaps_cleans_and_releases_barrier() {
    let admin_url =
        std::env::var("MAGNETAR_MYSQL_TEST_URL").expect("MAGNETAR_MYSQL_TEST_URL is required");
    let (server_url, _) = admin_url.rsplit_once('/').unwrap();
    let database_name = format!("magnetar_coordinator_{}", rand::random::<u64>());
    let admin = Database::connect(&admin_url).await.unwrap();
    admin.execute_raw(Statement::from_string(DbBackend::MySql,
    format!("CREATE DATABASE `{database_name}`"),))
        .await
        .unwrap();
    let source = Database::connect(format!("{server_url}/{database_name}"))
        .await
        .unwrap();
    for statement in [
        "CREATE TABLE users (id BIGINT PRIMARY KEY, email VARCHAR(255) NOT NULL, name VARCHAR(255) NULL, password_hash VARCHAR(255) NULL, email_verified_at DATETIME(6) NULL, created_at DATETIME(6) NOT NULL, updated_at DATETIME(6) NOT NULL, locked_at DATETIME(6) NULL)",
        "CREATE TABLE torii_migrations (version VARCHAR(255) PRIMARY KEY, applied_at BIGINT NOT NULL)",
        "CREATE TABLE sessions (id BIGINT PRIMARY KEY AUTO_INCREMENT, user_id BIGINT NOT NULL, token VARCHAR(255) NOT NULL, expires_at DATETIME(6) NOT NULL, created_at DATETIME(6) NOT NULL, updated_at DATETIME(6) NOT NULL)",
        "CREATE TABLE passkeys (id BIGINT PRIMARY KEY AUTO_INCREMENT, user_id BIGINT NOT NULL, credential_id VARCHAR(255) NOT NULL, data_json TEXT NOT NULL)",
        "INSERT INTO users (id, email, name, password_hash, email_verified_at, created_at, updated_at, locked_at) VALUES (1001, 'mysql.user@example.test', 'MySQL User', NULL, '2024-01-02 03:04:05', '2024-01-02 03:04:05', '2024-01-02 03:04:05', NULL)",
        "INSERT INTO torii_migrations (version, applied_at) VALUES ('m1', 1)",
        "INSERT INTO sessions (user_id, token, expires_at, created_at, updated_at) VALUES (1001, 'session-token', '2030-01-01 00:00:00', '2024-01-02 03:04:05', '2024-01-02 03:04:05')",
        "INSERT INTO passkeys (user_id, credential_id, data_json) VALUES (1001, 'numeric-owner-credential', '{\"name\":\"numeric owner\"}')",
        "CREATE VIEW user_emails AS SELECT id, email FROM users",
    ] {
        source.execute_raw(Statement::from_string(DbBackend::MySql, statement))
            .await
            .unwrap();
    }
    let app = Database::connect("sqlite::memory:").await.unwrap();
    crate::default_schema::migrate(&app).await.unwrap();
    let runner = MigrationEngine::new(source.clone(), DefaultMigrationBindings::new(app.clone()));
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
            .all(|entry| entry.table != "user_emails")
    );
    let backend = FakeMySql::with_fingerprints(
        plan.source_fingerprints
            .iter()
            .map(|entry| {
                (
                    entry.table.clone(),
                    (entry.fingerprint.clone(), entry.schema_digest.clone()),
                )
            })
            .collect(),
    );
    let tables = plan
        .source_fingerprints
        .iter()
        .map(|entry| {
            SwapTable::new(
                &entry.table,
                &format!("{}_shadow", entry.table),
                &format!("{}_backup", entry.table),
            )
        })
        .collect::<Vec<_>>();

    let report = runner.apply_mysql(&plan, &backend, &tables).await.unwrap();
    assert_eq!(report.journal.phase, JournalPhase::Complete);
    assert_eq!(report.migration.cleanup_statements, 2);
    assert!(!backend.barrier_held());
    let user = users::Entity::find()
        .filter(users::Column::Email.eq("mysql.user@example.test"))
        .one(&app)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(user.name.as_deref(), Some("MySQL User"));
    let passkey = methods::Entity::find()
        .filter(methods::Column::CredentialId.eq("numeric-owner-credential"))
        .one(&app)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(passkey.user_id, user.id);
    assert_eq!(
        source
            .query_one_raw(Statement::from_string(
                DbBackend::MySql,
                "SELECT COUNT(*) FROM sessions",
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get_by_index::<i64>(0)
            .unwrap(),
        0
    );

    drop(runner);
    drop(source);
    admin.execute_raw(Statement::from_string(DbBackend::MySql,
    format!("DROP DATABASE `{database_name}`"),))
        .await
        .unwrap();
}

#[tokio::test]
async fn preparing_resume_rejects_missing_retained_baseline() {
    let backend = FakeMySql::with_tables(&["users"], None);
    backend.fail_copy_after_create(true);
    let swap = MySqlShadowSwap;
    swap.execute(
        &backend,
        &[SwapTable::new("users", "users_shadow", "users_backup")],
        &Default::default(),
    )
    .await
    .unwrap_err();
    let mut corrupted = backend.durable_journal();
    corrupted.source_fingerprints.remove("users");
    backend.fail_copy_after_create(false);

    let failure = swap.resume(&backend, corrupted).await.unwrap_err();
    assert!(matches!(failure.error, Error::Conflict { .. }));
    assert!(backend.contains("users"));
    assert!(!backend.contains("users_backup"));
}

#[tokio::test]
async fn preparing_journal_recovers_shadow_created_before_copy_acknowledgement() {
    let backend = FakeMySql::with_tables(&["users"], None);
    backend.fail_copy_after_create(true);
    let swap = MySqlShadowSwap;
    swap.execute(
        &backend,
        &[SwapTable::new("users", "users_shadow", "users_backup")],
        &Default::default(),
    )
    .await
    .unwrap_err();
    let durable = backend.durable_journal();
    assert_eq!(durable.phase, JournalPhase::Preparing);
    assert_eq!(durable.renames.len(), 2);
    assert!(backend.contains("users_shadow"));

    backend.fail_copy_after_create(false);
    let completed = swap.resume(&backend, durable).await.unwrap();
    assert_eq!(completed.phase, JournalPhase::Complete);
    assert!(backend.contains("users"));
    assert!(backend.contains("users_backup"));
}

#[tokio::test]
async fn restore_rejects_incomplete_baseline_coverage_before_mutation() {
    let backend = FakeMySql::with_tables(&["users"], None);
    let swap = MySqlShadowSwap;
    let mut journal = swap
        .execute(
            &backend,
            &[SwapTable::new("users", "users_shadow", "users_backup")],
            &Default::default(),
        )
        .await
        .unwrap();
    journal.source_schema_digests.remove("users");

    let error = swap.restore(&backend, &mut journal).await.unwrap_err();
    assert!(matches!(error, Error::Conflict { .. }));
    assert_eq!(journal.phase, JournalPhase::Complete);
    assert!(backend.contains("users_backup"));
}

#[tokio::test]
async fn recovery_reacquires_and_releases_the_source_write_barrier() {
    let backend = FakeMySql::with_tables(&["users"], None);
    let journal = MySqlShadowSwap
        .execute(
            &backend,
            &[SwapTable::new("users", "users_shadow", "users_backup")],
            &Default::default(),
        )
        .await
        .unwrap();
    backend.release_write_barrier().await.unwrap();
    assert!(!backend.barrier_held());

    let recovery = MySqlSwapRecovery::new(backend.clone(), journal);
    let report = recovery.restore().await.unwrap();
    assert_eq!(report.restored_tables, 1);
    assert!(!backend.barrier_held());
    assert!(backend.contains("users"));
    assert_eq!(backend.cleanup_calls(), 1);
    assert!(!backend.contains("users_backup"));
}

#[tokio::test]
async fn shadow_schema_must_match_before_promotion() {
    let backend = FakeMySql::with_tables(&["users"], None);
    backend.degrade_shadow_schema();
    let failure = MySqlShadowSwap
        .execute(
            &backend,
            &[SwapTable::new("users", "users_shadow", "users_backup")],
            &Default::default(),
        )
        .await
        .unwrap_err();

    assert!(matches!(failure.error, Error::Conflict { .. }));
    assert!(backend.contains("users"));
    assert!(backend.contains("users_shadow"));
    assert!(!backend.contains("users_backup"));
}

#[tokio::test]
async fn active_change_between_copy_and_cutover_is_rejected() {
    let backend = FakeMySql::with_tables(&["users"], None);
    backend.mutate_active_after_copy();
    let failure = MySqlShadowSwap
        .execute(
            &backend,
            &[SwapTable::new("users", "users_shadow", "users_backup")],
            &Default::default(),
        )
        .await
        .unwrap_err();

    assert!(matches!(failure.error, Error::Conflict { .. }));
    assert!(backend.contains("users"));
    assert!(backend.contains("users_shadow"));
    assert!(!backend.contains("users_backup"));
}

#[tokio::test]
async fn cross_table_name_collision_is_rejected_before_copy_or_rename() {
    let backend = FakeMySql::with_tables(&["users", "credentials"], None);
    let failure = MySqlShadowSwap
        .execute(
            &backend,
            &[
                SwapTable::new("users", "users_shadow", "credentials"),
                SwapTable::new("credentials", "credentials_shadow", "credentials_backup"),
            ],
            &Default::default(),
        )
        .await
        .unwrap_err();

    assert!(matches!(failure.error, Error::InvalidInput { .. }));
    assert!(backend.contains("users"));
    assert!(backend.contains("credentials"));
    assert!(!backend.contains("users_shadow"));
    assert!(!backend.contains("credentials_shadow"));
}

fn fingerprint(value: &str) -> TableFingerprint {
    TableFingerprint::from_rows(
        &["id", "value"],
        vec![BTreeMap::from([
            ("id".to_owned(), "1".to_owned()),
            ("value".to_owned(), value.to_owned()),
        ])],
    )
    .unwrap()
}
