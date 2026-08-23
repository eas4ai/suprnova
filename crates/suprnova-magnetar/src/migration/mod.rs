//! Source-shape-aware authentication migration primitives.
//!
//! This module plans and executes the source-owned part of an upgrade while an
//! application supplies the narrow [`MigrationBindings`] seam for its own
//! users, external identities, and passkey storage. Magnetar never assumes an
//! application's table names or public identifier format.

pub mod fingerprint;
mod identity_map;
pub mod mysql_swap;
#[cfg(test)]
mod mysql_swap_tests;
mod plan;
mod preflight;
mod records;
pub mod schema_guards;
mod shape;
mod source_records;
pub mod upgrade_guide;

use async_trait::async_trait;
use std::sync::Arc;

use crate::{Error, Result};
use sea_orm::{
    AccessMode, ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend,
    IsolationLevel, Statement, TransactionTrait,
};

pub use identity_map::{
    AppUser, ExternalIdentity, IdentityMapEntry, IdentityMapPlan, ImportedPasskey,
};
pub use mysql_swap::{RestoreReport, SwapJournal, SwapTable};
pub use plan::{
    BackendStrategy, FieldMapping, MigrationBackend, MigrationPlan, MigrationReport,
    SacrificeableCleanup, SourceRowCount, TableOperation, TableOperationKind,
};
pub use preflight::{CollisionGroup, CollisionOwner};
pub use records::{
    DurableAuthRecord, ImportedFailedLoginAttempt, ImportedLinkedAccount, ImportedSecureToken,
    ImportedTwoFactorCredential, ImportedUser,
};
pub use shape::{ShapeConfirmation, SourceShape};

/// One application-database transaction used by a migration apply.
///
/// Every method must stage its write in the same transaction. Implementations
/// must make imports idempotent so a completed import phase can be retried when
/// source cleanup fails. Dropping the transaction without [`Self::commit`]
/// must roll back all staged writes.
#[async_trait]
pub trait MigrationTransaction: Send {
    /// Reads application users through the same snapshot used for imports.
    async fn app_users(&mut self) -> Result<Vec<AppUser>>;

    /// Creates or updates one application-owned user from durable source data.
    ///
    /// When `preferred_app_user_id` is present, the returned user must retain
    /// that exact application identifier. The operation must be idempotent.
    async fn import_user(&mut self, user: &ImportedUser) -> Result<AppUser>;

    /// Persists an idempotent external identity binding.
    async fn bind_external_identity(
        &mut self,
        provider: &str,
        external_user_id: &str,
        app_user_id: i64,
    ) -> Result<()>;

    /// Persists a passkey envelope without decoding or reserializing it.
    ///
    /// `data_json` must be stored byte-for-byte as supplied.
    async fn import_passkey(
        &mut self,
        app_user_id: i64,
        credential_id: &str,
        data_json: &str,
    ) -> Result<()>;

    /// Persists one idempotent durable non-user authentication record.
    async fn import_durable_record(&mut self, record: DurableAuthRecord) -> Result<()>;

    /// Returns whether this exact plan's import phase committed previously.
    async fn imports_committed(&mut self, plan_id: &str) -> Result<bool>;

    /// Reads a durable source-to-application identity resolution.
    async fn resolved_app_user_id(
        &mut self,
        plan_id: &str,
        source_user_id: &str,
    ) -> Result<Option<i64>>;

    /// Stages one durable source-to-application identity resolution.
    async fn record_identity_resolution(
        &mut self,
        plan_id: &str,
        source_user_id: &str,
        app_user_id: i64,
    ) -> Result<()>;

    /// Stages the import-phase completion marker in this same transaction.
    async fn mark_imports_committed(&mut self, plan_id: &str) -> Result<()>;

    /// Atomically commits every staged application write.
    async fn commit(self: Box<Self>) -> Result<()>;

    /// Rolls back every staged application write.
    async fn rollback(self: Box<Self>) -> Result<()>;
}

/// Application-owned reads and transaction construction required by migration.
///
/// Implementations own application schema access. The engine never derives or
/// replaces an application's public identifiers, and it never accepts granular
/// writes outside a host transaction.
#[async_trait]
pub trait MigrationBindings: Send + Sync {
    /// Returns the current application users considered for normalized-email
    /// matching.
    async fn app_users(&self) -> Result<Vec<AppUser>>;

    /// Returns whether application imports use the same physical database.
    async fn shares_source_database(&self, _source: &DatabaseConnection) -> Result<bool> {
        Ok(false)
    }

    /// Reads application users through a shared source transaction.
    async fn app_users_in_source(
        &self,
        _source_transaction: &DatabaseTransaction,
    ) -> Result<Vec<AppUser>> {
        self.app_users().await
    }

    /// Starts one transaction covering every application-owned import.
    ///
    /// Same-database bindings must borrow `source_transaction`; separate
    /// database bindings begin and own their own serializable transaction.
    async fn begin_transaction<'a>(
        &'a self,
        source_transaction: Option<&'a DatabaseTransaction>,
    ) -> Result<Box<dyn MigrationTransaction + 'a>>;

    /// Marks a fully cleaned migration as complete and persists the Magnetar
    /// source marker. This runs only after source cleanup commits.
    async fn mark_migration_completed(&self, _plan_id: &str) -> Result<()> {
        Ok(())
    }

    /// Returns application-owned target table names for collision warnings.
    ///
    /// The runner never writes these tables directly. It only warns during
    /// dry run when a legacy source already owns one of the names.
    fn migration_target_tables(&self) -> Vec<String> {
        Vec::new()
    }
}

/// The source-shape-aware runner contract exposed to host integrations.
#[async_trait]
pub trait MigrationRunner: Send + Sync {
    /// Detects one unambiguous source shape.
    async fn detect_shape(&self) -> Result<SourceShape>;
    /// Builds a no-write migration plan after validating an explicit operator
    /// confirmation.
    async fn dry_run(&self, confirmation: ShapeConfirmation) -> Result<MigrationPlan>;
    /// Applies a previously generated plan after a fresh source check.
    async fn apply(&self, plan: &MigrationPlan) -> Result<MigrationReport>;
    /// Removes only uncut-over shadow artifacts, if a host registered them.
    async fn abort(&self) -> Result<()>;
    /// Restores a completed MySQL shadow swap through a retained host journal.
    async fn restore(&self) -> Result<RestoreReport>;
}

/// A retained recovery operation supplied by a host after a MySQL cutover.
#[async_trait]
pub trait MigrationRecovery: Send + Sync {
    /// Removes only unpromoted shadow tables from the retained journal.
    async fn abort(&self) -> Result<()>;
    /// Reverses retained renames and verifies source fingerprints.
    async fn restore(&self) -> Result<RestoreReport>;
}

/// Successful plan-bound MySQL migration and retained recovery journal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MySqlMigrationReport {
    /// Application imports and source cleanup report.
    pub migration: MigrationReport,
    /// Durable completed swap journal retained through health checks.
    pub journal: SwapJournal,
}

/// Failure from the plan-bound MySQL migration coordinator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MySqlMigrationFailure {
    /// Latest swap journal when shadow preparation had started.
    pub journal: Option<SwapJournal>,
    /// Boundary error.
    pub error: Error,
    /// Whether the host must keep source writes quiesced for recovery.
    pub write_barrier_held: bool,
}

impl core::fmt::Display for MySqlMigrationFailure {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::error::Error for MySqlMigrationFailure {}

/// A database-backed migration runner with application-owned write bindings.
pub struct MigrationEngine<B> {
    database: DatabaseConnection,
    bindings: Arc<B>,
    recovery: Option<Arc<dyn MigrationRecovery>>,
}

impl<B> MigrationEngine<B>
where
    B: MigrationBindings + 'static,
{
    /// Creates a runner over one legacy source database and application binding.
    pub fn new(database: DatabaseConnection, bindings: B) -> Self {
        Self {
            database,
            bindings: Arc::new(bindings),
            recovery: None,
        }
    }

    /// Registers the host-retained recovery journal for a MySQL cutover.
    pub fn with_recovery(mut self, recovery: impl MigrationRecovery + 'static) -> Self {
        self.recovery = Some(Arc::new(recovery));
        self
    }

    /// Refuses a dry run when the CLI did not supply `--source-shape`.
    pub async fn dry_run_optional(
        &self,
        confirmation: Option<ShapeConfirmation>,
    ) -> Result<MigrationPlan> {
        let confirmation = confirmation.ok_or_else(|| Error::InvalidInput {
            field: "source-shape".to_owned(),
            message: "--source-shape torii|suprnova-web|suprnova-api|magnetar is required"
                .to_owned(),
        })?;
        self.dry_run(confirmation).await
    }

    async fn build_plan<C: ConnectionTrait + ?Sized>(
        &self,
        database: &C,
        confirmation: ShapeConfirmation,
        app_users: Vec<AppUser>,
        same_database: bool,
    ) -> Result<MigrationPlan> {
        ensure_supported_backend(database.get_database_backend())?;
        let target_tables = self.bindings.migration_target_tables();
        let detection_targets = if same_database {
            target_tables.as_slice()
        } else {
            &[]
        };
        let detected =
            preflight::detect_source_shape_for_targets(database, detection_targets).await?;
        preflight::validate_confirmation(&confirmation, detected)?;
        source_records::validate_schema(database, detected).await?;
        source_records::validate_cleanup_schema(database, detected).await?;
        let collisions = preflight::normalized_collisions(database, detected).await?;
        if !collisions.is_empty() {
            return Err(preflight::collision_error(&collisions));
        }
        let source_users = preflight::source_users(database, detected).await?;
        let identity_map = identity_map::plan_identity_map(source_users, app_users)?;
        let cleanup = SacrificeableCleanup::for_source(detected);
        let mut excluded_tables = if same_database {
            target_tables
                .iter()
                .filter(|table| {
                    !(detected == SourceShape::SuprnovaApi && table.as_str() == "app_users")
                })
                .cloned()
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let mut excluded_columns =
            std::collections::BTreeMap::<String, std::collections::BTreeSet<String>>::new();
        for target in &cleanup.targets {
            if let Some((table, column)) = target.split_once('.') {
                excluded_columns
                    .entry(table.to_owned())
                    .or_default()
                    .insert(column.to_owned());
            } else {
                excluded_tables.push(target.clone());
            }
        }
        excluded_tables.sort();
        excluded_tables.dedup();
        let source_fingerprints = fingerprint::source_database_fingerprints(
            database,
            &excluded_tables,
            &excluded_columns,
        )
        .await?;
        let mut source_row_counts = preflight::source_row_counts(database).await?;
        source_row_counts.retain(|entry| !excluded_tables.contains(&entry.table));
        let backend_strategy = BackendStrategy::for_backend(database.get_database_backend());
        let warnings = if same_database {
            Vec::new()
        } else {
            preflight::foreign_target_table_warnings(database, &target_tables).await?
        };
        Ok(MigrationPlan::new(
            detected,
            confirmation,
            identity_map,
            warnings,
            source_row_counts,
            source_fingerprints,
            backend_strategy,
        ))
    }

    async fn import_users(
        &self,
        transaction: &mut dyn MigrationTransaction,
        plan: &MigrationPlan,
        mut source_users: Vec<ImportedUser>,
    ) -> Result<(
        Vec<ExternalIdentity>,
        std::collections::BTreeMap<String, i64>,
    )> {
        let mut resolved_ids = std::collections::BTreeMap::new();
        let mut bindings = Vec::new();
        let imports_committed = transaction.imports_committed(&plan.plan_id).await?;

        for user in &mut source_users {
            let decision = plan
                .identity_map
                .entries
                .iter()
                .find(|entry| entry.source_user_id() == user.source_user_id)
                .ok_or_else(|| Error::Conflict {
                    resource: "migration identity map".to_owned(),
                    message: format!(
                        "source user {} is absent from the reviewed identity map",
                        user.source_user_id
                    ),
                })?;
            user.preferred_app_user_id = match decision {
                IdentityMapEntry::Existing { app_user_id, .. } => Some(*app_user_id),
                IdentityMapEntry::Create { app_user_id, .. } if imports_committed => {
                    let resolved = transaction
                        .resolved_app_user_id(&plan.plan_id, &user.source_user_id)
                        .await?
                        .ok_or_else(|| Error::Conflict {
                            resource: "migration identity ledger".to_owned(),
                            message: format!(
                                "committed plan has no resolution for {}",
                                user.source_user_id
                            ),
                        })?;
                    if app_user_id.is_some_and(|required| required != resolved) {
                        return Err(Error::Conflict {
                            resource: "migration identity ledger".to_owned(),
                            message: "resolved application ID violates the reviewed plan"
                                .to_owned(),
                        });
                    }
                    Some(resolved)
                }
                IdentityMapEntry::Create { app_user_id, .. } => *app_user_id,
            };

            let imported = transaction.import_user(user).await?;
            if let Some(expected) = user.preferred_app_user_id {
                if imported.id != expected {
                    return Err(Error::Conflict {
                        resource: "application user identity".to_owned(),
                        message: format!(
                            "source user {} required application id {expected}, binding returned {}",
                            user.source_user_id, imported.id
                        ),
                    });
                }
            } else if !imports_committed
                && (imported.auth_epoch != 0 || imported.session_version != 0)
            {
                return Err(Error::Conflict {
                    resource: "new app user".to_owned(),
                    message: "new users must start with auth_epoch and session_version at zero"
                        .to_owned(),
                });
            }
            transaction
                .record_identity_resolution(&plan.plan_id, &user.source_user_id, imported.id)
                .await?;

            resolved_ids.insert(user.source_user_id.clone(), imported.id);
            if plan.source == SourceShape::Torii {
                transaction
                    .bind_external_identity("torii", &user.source_user_id, imported.id)
                    .await?;
                bindings.push(ExternalIdentity {
                    provider: "torii".to_owned(),
                    external_user_id: user.source_user_id.clone(),
                    app_user_id: imported.id,
                });
            }
        }

        Ok((bindings, resolved_ids))
    }

    async fn import_auth_records(
        &self,
        transaction: &mut dyn MigrationTransaction,
        resolved_ids: &std::collections::BTreeMap<String, i64>,
        pending_records: Vec<records::PendingAuthRecord>,
    ) -> Result<()> {
        for pending in pending_records {
            let owner = pending
                .source_user_id()
                .and_then(|source_user_id| resolved_ids.get(source_user_id))
                .copied();
            transaction
                .import_durable_record(pending.resolve(owner)?)
                .await?;
        }
        Ok(())
    }

    async fn stage_imports<C: ConnectionTrait + ?Sized>(
        &self,
        source: &C,
        transaction: &mut dyn MigrationTransaction,
        plan: &MigrationPlan,
    ) -> Result<Vec<ExternalIdentity>> {
        let source_users = source_records::users(source, plan.source).await?;
        let pending_records = source_records::auth_records(source, plan.source).await?;
        let source_passkeys = if plan.source == SourceShape::Torii {
            preflight::source_passkeys(source).await?
        } else {
            Vec::new()
        };
        let (identity_mappings, resolved_ids) =
            self.import_users(transaction, plan, source_users).await?;
        for passkey in source_passkeys {
            let app_user_id = resolved_ids
                .get(&passkey.external_user_id)
                .copied()
                .ok_or_else(|| Error::Conflict {
                    resource: "passkey owner".to_owned(),
                    message: format!(
                        "source passkey {} has no resolved application identity",
                        passkey.credential_id
                    ),
                })?;
            transaction
                .import_passkey(app_user_id, &passkey.credential_id, &passkey.data_json)
                .await?;
        }
        self.import_auth_records(transaction, &resolved_ids, pending_records)
            .await?;
        transaction.mark_imports_committed(&plan.plan_id).await?;
        Ok(identity_mappings)
    }
    async fn reviewed_plan_matches(
        &self,
        transaction: &mut dyn MigrationTransaction,
        reviewed: &MigrationPlan,
        fresh: &MigrationPlan,
    ) -> Result<bool> {
        if fresh == reviewed {
            return Ok(true);
        }
        if !transaction.imports_committed(&reviewed.plan_id).await? {
            return Ok(false);
        }
        for reviewed_entry in &reviewed.identity_map.entries {
            let source_user_id = reviewed_entry.source_user_id();
            let Some(fresh_entry) = fresh
                .identity_map
                .entries
                .iter()
                .find(|entry| entry.source_user_id() == source_user_id)
            else {
                return Ok(false);
            };
            match reviewed_entry {
                IdentityMapEntry::Existing { app_user_id, .. } => {
                    if !matches!(
                        fresh_entry,
                        IdentityMapEntry::Existing {
                            app_user_id: fresh_id,
                            ..
                        } if fresh_id == app_user_id
                    ) {
                        return Ok(false);
                    }
                }
                IdentityMapEntry::Create {
                    app_user_id: required_id,
                    ..
                } => {
                    let Some(resolved_id) = transaction
                        .resolved_app_user_id(&reviewed.plan_id, source_user_id)
                        .await?
                    else {
                        return Ok(false);
                    };
                    if required_id.is_some_and(|required| required != resolved_id)
                        || !matches!(
                            fresh_entry,
                            IdentityMapEntry::Existing {
                                app_user_id: fresh_id,
                                ..
                            } if *fresh_id == resolved_id
                        )
                    {
                        return Ok(false);
                    }
                }
            }
        }
        let mut normalized = fresh.clone();
        normalized.plan_id = reviewed.plan_id.clone();
        normalized.identity_map = reviewed.identity_map.clone();
        Ok(&normalized == reviewed)
    }

    /// Applies one reviewed MySQL plan under a host-enforced source-write
    /// barrier, imports application state, performs the durable shadow swap,
    /// and clears only sacrificeable state before releasing the barrier.
    pub async fn apply_mysql<M: mysql_swap::MySqlSwapBackend>(
        &self,
        plan: &MigrationPlan,
        backend: &M,
        tables: &[SwapTable],
    ) -> core::result::Result<MySqlMigrationReport, MySqlMigrationFailure> {
        if !matches!(
            plan.backend_strategy,
            BackendStrategy::MySqlShadowSwap { .. }
        ) {
            return Err(MySqlMigrationFailure {
                journal: None,
                error: Error::InvalidInput {
                    field: "MySQL migration plan".to_owned(),
                    message: "plan does not declare the MySQL shadow-swap strategy".to_owned(),
                },
                write_barrier_held: false,
            });
        }
        if let Err(error) = backend.acquire_write_barrier().await {
            return Err(MySqlMigrationFailure {
                journal: None,
                error,
                write_barrier_held: false,
            });
        }
        match backend.write_barrier_held().await {
            Ok(true) => {}
            Ok(false) => {
                return Err(release_mysql_failure(
                    backend,
                    None,
                    Error::Conflict {
                        resource: "MySQL source write barrier".to_owned(),
                        message: "backend did not retain the acquired write barrier".to_owned(),
                    },
                )
                .await);
            }
            Err(error) => {
                return Err(release_mysql_failure(backend, None, error).await);
            }
        }

        let detected = match self.detect_shape().await {
            Ok(detected) => detected,
            Err(error) => return Err(release_mysql_failure(backend, None, error).await),
        };
        let shares_source = match self.bindings.shares_source_database(&self.database).await {
            Ok(shares_source) => shares_source,
            Err(error) => return Err(release_mysql_failure(backend, None, error).await),
        };
        let source_snapshot = match begin_source_snapshot(&self.database, detected).await {
            Ok(snapshot) => snapshot,
            Err(error) => return Err(release_mysql_failure(backend, None, error).await),
        };
        let mut transaction = match self
            .bindings
            .begin_transaction(shares_source.then_some(&source_snapshot))
            .await
        {
            Ok(transaction) => transaction,
            Err(error) => {
                return Err(release_mysql_failure(backend, None, error).await);
            }
        };
        let import_result = async {
            let app_users = transaction.app_users().await?;
            let fresh_plan = self
                .build_plan(
                    &source_snapshot,
                    plan.confirmation.clone(),
                    app_users,
                    shares_source,
                )
                .await?;
            if !self
                .reviewed_plan_matches(transaction.as_mut(), plan, &fresh_plan)
                .await?
            {
                return Err(Error::Conflict {
                    resource: "migration plan".to_owned(),
                    message:
                        "source or application identity state changed after dry run; generate and review a fresh plan"
                            .to_owned(),
                });
            }
            validate_mysql_swap_tables(plan, backend, tables).await?;
            self.stage_imports(&source_snapshot, transaction.as_mut(), plan)
                .await
        }
        .await;
        let identity_mappings = match import_result {
            Ok(identity_mappings) => identity_mappings,
            Err(error) => {
                let app_rollback = transaction.rollback().await;
                let source_rollback = source_snapshot.rollback().await;
                let error = app_rollback
                    .err()
                    .map(|rollback_error| Error::Internal {
                        message: format!(
                            "MySQL migration import failed: {error}; application rollback failed: {rollback_error}"
                        ),
                    })
                    .or_else(|| {
                        source_rollback.err().map(|rollback_error| {
                            database_error(
                                "rolling back MySQL source snapshot after import failure",
                                rollback_error,
                            )
                        })
                    })
                    .unwrap_or(error);
                return Err(release_mysql_failure(backend, None, error).await);
            }
        };
        if let Err(error) = transaction.commit().await {
            let error = match source_snapshot.rollback().await {
                Ok(()) => error,
                Err(rollback_error) => Error::Internal {
                    message: format!(
                        "application commit outcome is ambiguous: {error}; source snapshot rollback failed: {rollback_error}"
                    ),
                },
            };
            return Err(MySqlMigrationFailure {
                journal: None,
                error,
                write_barrier_held: true,
            });
        }
        if let Err(error) = source_snapshot.commit().await {
            return Err(MySqlMigrationFailure {
                journal: None,
                error: database_error("committing MySQL source snapshot", error),
                write_barrier_held: true,
            });
        }

        let journal = match mysql_swap::MySqlShadowSwap
            .execute(backend, tables, &plan.sacrificeable_cleanup)
            .await
        {
            Ok(journal) => journal,
            Err(failure) => {
                return Err(MySqlMigrationFailure {
                    journal: Some(failure.journal),
                    error: failure.error,
                    write_barrier_held: true,
                });
            }
        };
        let cleanup_transaction = match self.database.begin().await {
            Ok(transaction) => transaction,
            Err(error) => {
                return Err(MySqlMigrationFailure {
                    journal: Some(journal),
                    error: database_error("starting MySQL cleanup transaction", error),
                    write_barrier_held: true,
                });
            }
        };
        let cleanup_statements =
            match preflight::apply_cleanup(&cleanup_transaction, &plan.sacrificeable_cleanup).await
            {
                Ok(cleanup_statements) => cleanup_statements,
                Err(error) => {
                    let _ = cleanup_transaction.rollback().await;
                    return Err(MySqlMigrationFailure {
                        journal: Some(journal),
                        error,
                        write_barrier_held: true,
                    });
                }
            };
        if let Err(error) = cleanup_transaction.commit().await {
            return Err(MySqlMigrationFailure {
                journal: Some(journal),
                error: database_error("committing MySQL cleanup transaction", error),
                write_barrier_held: true,
            });
        }
        if let Err(error) = self.bindings.mark_migration_completed(&plan.plan_id).await {
            return Err(MySqlMigrationFailure {
                journal: Some(journal),
                error,
                write_barrier_held: true,
            });
        }
        if let Err(error) = backend.release_write_barrier().await {
            return Err(MySqlMigrationFailure {
                journal: Some(journal),
                error,
                write_barrier_held: true,
            });
        }
        Ok(MySqlMigrationReport {
            migration: MigrationReport {
                source: plan.source,
                identity_mappings,
                cleanup: plan.sacrificeable_cleanup.clone(),
                cleanup_statements,
            },
            journal,
        })
    }
}

async fn release_mysql_failure<M: mysql_swap::MySqlSwapBackend>(
    backend: &M,
    journal: Option<SwapJournal>,
    error: Error,
) -> MySqlMigrationFailure {
    match backend.release_write_barrier().await {
        Ok(()) => MySqlMigrationFailure {
            journal,
            error,
            write_barrier_held: false,
        },
        Err(release_error) => MySqlMigrationFailure {
            journal,
            error: Error::Internal {
                message: format!(
                    "MySQL migration failed: {error}; releasing source write barrier failed: {release_error}"
                ),
            },
            write_barrier_held: true,
        },
    }
}

async fn validate_mysql_swap_tables<M: mysql_swap::MySqlSwapBackend>(
    plan: &MigrationPlan,
    backend: &M,
    tables: &[SwapTable],
) -> Result<()> {
    let planned = plan
        .source_fingerprints
        .iter()
        .map(|entry| (entry.table.as_str(), entry))
        .collect::<std::collections::BTreeMap<_, _>>();
    let active = tables
        .iter()
        .map(|table| table.active.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    if planned
        .keys()
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
        != active
    {
        return Err(Error::Conflict {
            resource: "MySQL swap table set".to_owned(),
            message: "swap active tables must exactly match the reviewed source fingerprints"
                .to_owned(),
        });
    }
    for table in tables {
        let expected =
            planned
                .get(table.active.as_str())
                .copied()
                .ok_or_else(|| Error::Conflict {
                    resource: "MySQL swap table set".to_owned(),
                    message: format!("{} is absent from the reviewed plan", table.active),
                })?;
        let actual = backend.fingerprint(&table.active).await?;
        if expected.fingerprint != actual {
            return Err(Error::Conflict {
                resource: "MySQL reviewed source fingerprint".to_owned(),
                message: format!("{} no longer matches the reviewed plan", table.active),
            });
        }
        let actual_schema = backend.schema_digest(&table.active).await?;
        if expected.schema_digest != actual_schema {
            return Err(Error::Conflict {
                resource: "MySQL reviewed source schema".to_owned(),
                message: format!(
                    "{} schema no longer matches the reviewed plan",
                    table.active
                ),
            });
        }
    }
    Ok(())
}

#[async_trait]
impl<B> MigrationRunner for MigrationEngine<B>
where
    B: MigrationBindings + 'static,
{
    async fn detect_shape(&self) -> Result<SourceShape> {
        let shares_source = self.bindings.shares_source_database(&self.database).await?;
        let target_tables = self.bindings.migration_target_tables();
        let detection_targets = if shares_source {
            target_tables.as_slice()
        } else {
            &[]
        };
        preflight::detect_source_shape_for_targets(&self.database, detection_targets).await
    }

    async fn dry_run(&self, confirmation: ShapeConfirmation) -> Result<MigrationPlan> {
        ensure_supported_backend(self.database.get_database_backend())?;
        let detected = self.detect_shape().await?;
        let shares_source = self.bindings.shares_source_database(&self.database).await?;
        let snapshot = begin_source_snapshot(&self.database, detected).await?;
        let app_users = if shares_source {
            self.bindings.app_users_in_source(&snapshot).await?
        } else {
            self.bindings.app_users().await?
        };
        let plan = self
            .build_plan(&snapshot, confirmation, app_users, shares_source)
            .await;
        match plan {
            Ok(plan) => {
                snapshot
                    .commit()
                    .await
                    .map_err(|error| database_error("committing dry-run snapshot", error))?;
                Ok(plan)
            }
            Err(error) => {
                snapshot.rollback().await.map_err(|rollback_error| {
                    database_error("rolling back dry-run snapshot", rollback_error)
                })?;
                Err(error)
            }
        }
    }

    async fn apply(&self, plan: &MigrationPlan) -> Result<MigrationReport> {
        ensure_supported_backend(self.database.get_database_backend())?;
        let detected = self.detect_shape().await?;
        let shares_source = self.bindings.shares_source_database(&self.database).await?;
        let source_snapshot = begin_source_snapshot(&self.database, detected).await?;
        let mut transaction = self
            .bindings
            .begin_transaction(shares_source.then_some(&source_snapshot))
            .await?;
        let import_result = async {
            let app_users = transaction.app_users().await?;
            let fresh_plan = self
                .build_plan(
                    &source_snapshot,
                    plan.confirmation.clone(),
                    app_users,
                    shares_source,
                )
                .await?;
            if !self
                .reviewed_plan_matches(transaction.as_mut(), plan, &fresh_plan)
                .await?
            {
                return Err(Error::Conflict {
                    resource: "migration plan".to_owned(),
                    message:
                        "source or application identity state changed after dry run; generate and review a fresh plan"
                            .to_owned(),
                });
            }
            validate_direct_apply_strategy(
                self.database.get_database_backend(),
                &plan.backend_strategy,
            )?;

            self.stage_imports(&source_snapshot, transaction.as_mut(), plan)
                .await
        }
        .await;

        let identity_mappings = match import_result {
            Ok(identity_mappings) => identity_mappings,
            Err(error) => {
                let app_rollback = transaction.rollback().await;
                let source_rollback = source_snapshot.rollback().await;
                if let Err(rollback_error) = app_rollback {
                    return Err(Error::Internal {
                        message: format!(
                            "migration import failed: {error}; application rollback failed: {rollback_error}"
                        ),
                    });
                }
                if let Err(rollback_error) = source_rollback {
                    return Err(database_error(
                        "rolling back source snapshot after import failure",
                        rollback_error,
                    ));
                }
                return Err(error);
            }
        };
        if let Err(error) = transaction.commit().await {
            source_snapshot.rollback().await.map_err(|rollback_error| {
                database_error(
                    "rolling back source snapshot after application commit failure",
                    rollback_error,
                )
            })?;
            return Err(error);
        }
        let cleanup_statements =
            match preflight::apply_cleanup(&source_snapshot, &plan.sacrificeable_cleanup).await {
                Ok(cleanup_statements) => cleanup_statements,
                Err(error) => {
                    source_snapshot.rollback().await.map_err(|rollback_error| {
                        database_error(
                            "rolling back source snapshot after cleanup failure",
                            rollback_error,
                        )
                    })?;
                    return Err(error);
                }
            };
        source_snapshot
            .commit()
            .await
            .map_err(|error| database_error("committing source cleanup snapshot", error))?;
        self.bindings
            .mark_migration_completed(&plan.plan_id)
            .await?;

        Ok(MigrationReport {
            source: plan.source,
            identity_mappings,
            cleanup: plan.sacrificeable_cleanup.clone(),
            cleanup_statements,
        })
    }

    async fn abort(&self) -> Result<()> {
        match &self.recovery {
            Some(recovery) => recovery.abort().await,
            None => Ok(()),
        }
    }

    async fn restore(&self) -> Result<RestoreReport> {
        self.recovery
            .as_ref()
            .ok_or_else(|| Error::InvalidInput {
                field: "restore".to_owned(),
                message: "restore requires the host-retained MySQL swap journal".to_owned(),
            })?
            .restore()
            .await
    }
}

async fn begin_source_snapshot(
    database: &DatabaseConnection,
    shape: SourceShape,
) -> Result<DatabaseTransaction> {
    ensure_supported_backend(database.get_database_backend())?;
    let backend = database.get_database_backend();
    let postgres_tables = if backend == DbBackend::Postgres {
        regular_table_names(database).await?
    } else {
        Vec::new()
    };
    let transaction = match backend {
        DbBackend::Sqlite => database.begin().await,
        DbBackend::Postgres => {
            database
                .begin_with_config(
                    Some(IsolationLevel::Serializable),
                    Some(AccessMode::ReadWrite),
                )
                .await
        }
        DbBackend::MySql => {
            database
                .begin_with_config(
                    Some(IsolationLevel::RepeatableRead),
                    Some(AccessMode::ReadWrite),
                )
                .await
        }
        _ => return Err(unsupported_backend_error(backend)),
    }
    .map_err(|error| database_error("starting source migration snapshot", error))?;

    match backend {
        DbBackend::Sqlite if shape != SourceShape::Magnetar => {
            let table = match shape {
                SourceShape::Torii | SourceShape::SuprnovaWeb => "users",
                SourceShape::SuprnovaApi => "app_users",
                SourceShape::Magnetar => unreachable!("guarded above"),
            };
            transaction.execute_raw(Statement::from_string(backend,
            format!("UPDATE \"{table}\" SET \"email\" = \"email\" WHERE 0"),))
                .await
                .map_err(|error| database_error("locking SQLite migration source", error))?;
        }
        DbBackend::Postgres if !postgres_tables.is_empty() => {
            let tables = postgres_tables
                .iter()
                .map(|table| quote_source_identifier(table))
                .collect::<Result<Vec<_>>>()?
                .join(", ");
            transaction.execute_raw(Statement::from_string(backend,
            format!("LOCK TABLE {tables} IN SHARE MODE"),))
                .await
                .map_err(|error| database_error("locking PostgreSQL migration source", error))?;
        }
        DbBackend::Sqlite | DbBackend::Postgres | DbBackend::MySql => {}
        _ => return Err(unsupported_backend_error(backend)),
    }
    Ok(transaction)
}

async fn regular_table_names(database: &DatabaseConnection) -> Result<Vec<String>> {
    let backend = database.get_database_backend();
    let query = match backend {
        DbBackend::Sqlite => {
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
        }
        DbBackend::Postgres => {
            "SELECT table_name FROM information_schema.tables WHERE table_schema = current_schema() AND table_type = 'BASE TABLE' ORDER BY table_name"
        }
        DbBackend::MySql => {
            "SELECT table_name FROM information_schema.tables WHERE table_schema = DATABASE() AND table_type = 'BASE TABLE' ORDER BY table_name"
        }
        _ => return Err(unsupported_backend_error(backend)),
    };
    database.query_all_raw(Statement::from_string(backend, query))
        .await
        .map_err(|error| database_error("listing source migration tables", error))?
        .into_iter()
        .map(|row| {
            row.try_get_by_index(0)
                .map_err(|error| database_error("reading source migration table", error))
        })
        .collect()
}

fn quote_source_identifier(identifier: &str) -> Result<String> {
    if identifier.is_empty()
        || !identifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(Error::InvalidInput {
            field: "source table".to_owned(),
            message: format!("unsupported source table name {identifier:?}"),
        });
    }
    Ok(format!("\"{identifier}\""))
}

pub(crate) fn unsupported_backend_error(backend: DbBackend) -> Error {
    Error::DependencyUnavailable {
        dependency: "database backend".to_owned(),
        message: format!("unsupported SeaORM database backend: {backend:?}"),
    }
}

fn ensure_supported_backend(backend: DbBackend) -> Result<()> {
    match backend {
        DbBackend::Sqlite | DbBackend::Postgres | DbBackend::MySql => Ok(()),
        _ => Err(unsupported_backend_error(backend)),
    }
}

fn validate_direct_apply_strategy(backend: DbBackend, strategy: &BackendStrategy) -> Result<()> {
    match strategy {
        BackendStrategy::Transactional { .. } => Ok(()),
        BackendStrategy::MySqlShadowSwap { .. } => Err(Error::InvalidInput {
            field: "MySQL migration apply".to_owned(),
            message: "direct apply cannot provide crash-safe MySQL DDL; execute the reviewed plan through MySqlShadowSwap with a durable MySqlSwapBackend journal"
                .to_owned(),
        }),
        BackendStrategy::Unsupported => Err(Error::DependencyUnavailable {
            dependency: "database backend".to_owned(),
            message: format!("unsupported SeaORM database backend: {backend:?}"),
        }),
    }
}

pub(crate) fn database_error(context: &str, error: sea_orm::DbErr) -> Error {
    Error::Internal {
        message: format!("{context}: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "seaorm-sqlite")]
    use sea_orm::{ConnectionTrait, DbBackend, Statement};

    #[cfg(feature = "seaorm-sqlite")]
    #[tokio::test]
    async fn source_snapshot_keeps_all_record_reads_on_one_reviewed_view() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "magnetar-source-snapshot-{}-{unique}.sqlite",
            std::process::id()
        ));
        let url = format!("sqlite://{}?mode=rwc", path.display());
        let first = sea_orm::Database::connect(&url).await.unwrap();
        first.execute_raw(Statement::from_string(DbBackend::Sqlite,
        "PRAGMA journal_mode = WAL",))
            .await
            .unwrap();
        first.execute_raw(Statement::from_string(DbBackend::Sqlite,
        "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, email TEXT NOT NULL, password TEXT NOT NULL, remember_token TEXT, email_verified_at TEXT, created_at TEXT, updated_at TEXT)",))
            .await
            .unwrap();
        first.execute_raw(Statement::from_string(DbBackend::Sqlite,
        "INSERT INTO users (id, name, email, password) VALUES (1, 'Before', 'before@example.test', 'hash-before')",))
            .await
            .unwrap();
        let second = sea_orm::Database::connect(&url).await.unwrap();
        let snapshot = begin_source_snapshot(&first, SourceShape::SuprnovaWeb)
            .await
            .unwrap();
        let before = source_records::users(&snapshot, SourceShape::SuprnovaWeb)
            .await
            .unwrap();
        let writer = tokio::spawn(async move {
            second.execute_raw(Statement::from_string(DbBackend::Sqlite,
            "UPDATE users SET password = 'hash-after' WHERE id = 1",))
                .await
                .unwrap();
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            !writer.is_finished(),
            "source writer must wait for the migration snapshot barrier"
        );
        let after = source_records::users(&snapshot, SourceShape::SuprnovaWeb)
            .await
            .unwrap();
        assert_eq!(before[0].password_hash.as_deref(), Some("hash-before"));
        assert_eq!(after[0].password_hash.as_deref(), Some("hash-before"));
        snapshot.rollback().await.unwrap();
        writer.await.unwrap();
        drop(first);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn direct_apply_refuses_mysql_shadow_strategy() {
        let error = validate_direct_apply_strategy(
            DbBackend::MySql,
            &BackendStrategy::for_backend(DbBackend::MySql),
        )
        .unwrap_err();
        assert!(matches!(error, Error::InvalidInput { .. }));
    }
}
