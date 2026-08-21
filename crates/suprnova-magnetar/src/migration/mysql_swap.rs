//! Crash-recoverable MySQL shadow-copy cutover primitives.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use async_trait::async_trait;

use crate::{Error, Result};

use super::fingerprint::TableFingerprint;
use super::plan::SacrificeableCleanup;

/// One active, shadow, and retained-backup table set for a MySQL cutover.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SwapTable {
    /// The currently active source table.
    pub active: String,
    /// The staging shadow table populated before any rename.
    pub shadow: String,
    /// The retained backup table name after cutover.
    pub backup: String,
}

impl SwapTable {
    /// Defines one shadow-copy table set.
    pub fn new(active: &str, shadow: &str, backup: &str) -> Self {
        Self {
            active: active.to_owned(),
            shadow: shadow.to_owned(),
            backup: backup.to_owned(),
        }
    }
}

/// Durable progress for one forward rename and its reverse restore.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenameState {
    /// The rename is planned but has not reached its durable prepare boundary.
    #[default]
    Pending,
    /// Intent is durable; catalog reconciliation decides whether rename ran.
    Prepared,
    /// The forward rename is durably complete.
    Completed,
    /// Reverse intent is durable; catalog reconciliation decides progress.
    RestorePrepared,
    /// The reverse rename is durably complete or was never needed.
    Restored,
}

/// Overall durable cutover state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum JournalPhase {
    /// Shadows and fingerprints are still being prepared.
    #[default]
    Preparing,
    /// Every shadow and the complete rename plan are durable.
    Prepared,
    /// Forward renames are being reconciled.
    CuttingOver,
    /// Every forward rename completed.
    Complete,
    /// Reverse renames are being reconciled.
    Restoring,
    /// Every original active table was restored and verified.
    Restored,
    /// No cutover occurred and unused shadows were removed.
    Aborted,
}

/// One planned rename recorded in the durable host journal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenameJournalEntry {
    /// The table name before the rename.
    pub from: String,
    /// The table name after the rename.
    pub to: String,
    /// Durable forward or reverse progress.
    pub state: RenameState,
}

/// Retained state needed to resume, abort, or restore a MySQL cutover safely.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SwapJournal {
    /// The table sets included in this cutover.
    pub tables: Vec<SwapTable>,
    /// Pre-copy source fingerprints keyed by active table name.
    pub source_fingerprints: BTreeMap<String, TableFingerprint>,
    /// Pre-copy source schema digests keyed by active table name.
    pub source_schema_digests: BTreeMap<String, String>,
    /// Shadows durably verified during the preparing phase.
    pub prepared_shadows: BTreeSet<String>,
    /// Complete forward rename plan in execution order.
    pub renames: Vec<RenameJournalEntry>,
    /// Shape-owned transient cleanup reapplied after any restore.
    pub cleanup: SacrificeableCleanup,
    /// Durable overall cutover phase.
    pub phase: JournalPhase,
}

/// A report produced after fingerprints verify a restored source.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RestoreReport {
    /// Number of active source tables restored and fingerprint-verified.
    pub restored_tables: usize,
}

/// A cutover failure retaining its latest in-memory journal for recovery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwapFailure {
    /// Latest journal state. The backend's durable copy remains authoritative
    /// after a process crash or journal-persistence failure.
    pub journal: SwapJournal,
    /// The boundary error that stopped the cutover.
    pub error: Error,
}

/// Host-provided MySQL DDL, catalog, fingerprint, and durable-journal boundary.
#[async_trait]
pub trait MySqlSwapBackend: Send + Sync {
    /// Idempotently acquires the host's source-write barrier.
    async fn acquire_write_barrier(&self) -> Result<()>;
    /// Returns whether the source-write barrier is currently held.
    async fn write_barrier_held(&self) -> Result<bool>;
    /// Releases the source-write barrier after cleanup or explicit recovery.
    async fn release_write_barrier(&self) -> Result<()>;
    /// Copies an active table into an unused shadow table.
    async fn copy_to_shadow(&self, active: &str, shadow: &str) -> Result<()>;
    /// Computes the current table fingerprint using the agreed field list.
    async fn fingerprint(&self, table: &str) -> Result<TableFingerprint>;
    /// Computes a canonical table schema/index/constraint digest.
    async fn schema_digest(&self, table: &str) -> Result<String>;
    /// Returns whether one table currently exists.
    async fn table_exists(&self, table: &str) -> Result<bool>;
    /// Renames one table.
    async fn rename(&self, from: &str, to: &str) -> Result<()>;
    /// Removes an unused shadow table during an abort.
    async fn remove_shadow(&self, table: &str) -> Result<()>;
    /// Applies the journal's shape-owned cleanup idempotently.
    async fn apply_cleanup(&self, cleanup: &SacrificeableCleanup) -> Result<usize>;
    /// Atomically persists the complete journal before returning.
    async fn persist_journal(&self, journal: &SwapJournal) -> Result<()>;
}

/// Bridges a retained MySQL swap journal into a [`super::MigrationRunner`].
pub struct MySqlSwapRecovery<B> {
    backend: B,
    journal: Mutex<SwapJournal>,
}

impl<B> MySqlSwapRecovery<B> {
    /// Retains a completed or interrupted journal with its host backend.
    pub fn new(backend: B, journal: SwapJournal) -> Self {
        Self {
            backend,
            journal: Mutex::new(journal),
        }
    }

    /// Returns the latest process-local journal snapshot.
    ///
    /// The backend's persisted journal remains authoritative after a crash.
    pub fn journal(&self) -> SwapJournal {
        self.journal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl<B> super::MigrationRecovery for MySqlSwapRecovery<B>
where
    B: MySqlSwapBackend + 'static,
{
    async fn abort(&self) -> Result<()> {
        acquire_recovery_barrier(&self.backend).await?;
        let mut journal = self.journal();
        let result = MySqlShadowSwap.abort(&self.backend, &mut journal).await;
        *self
            .journal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = journal;
        result?;
        self.backend.release_write_barrier().await
    }

    async fn restore(&self) -> Result<RestoreReport> {
        acquire_recovery_barrier(&self.backend).await?;
        let mut journal = self.journal();
        let result = MySqlShadowSwap.restore(&self.backend, &mut journal).await;
        *self
            .journal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = journal;
        let report = result?;
        self.backend.release_write_barrier().await?;
        Ok(report)
    }
}

/// Executes MySQL shadow-copy cutovers without assuming application tables.
#[derive(Clone, Debug, Default)]
pub struct MySqlShadowSwap;

impl MySqlShadowSwap {
    /// Low-level copy/swap primitive used only by
    /// [`super::MigrationEngine::apply_mysql`] after plan verification.
    pub(crate) async fn execute<B: MySqlSwapBackend>(
        &self,
        backend: &B,
        tables: &[SwapTable],
        cleanup: &SacrificeableCleanup,
    ) -> core::result::Result<SwapJournal, SwapFailure> {
        let mut journal = SwapJournal {
            tables: tables.to_vec(),
            cleanup: cleanup.clone(),
            ..SwapJournal::default()
        };
        match backend.write_barrier_held().await {
            Ok(true) => {}
            Ok(false) => {
                return Err(SwapFailure {
                    journal,
                    error: Error::Conflict {
                        resource: "MySQL source write barrier".to_owned(),
                        message: "shadow copy requires an active source-write barrier".to_owned(),
                    },
                });
            }
            Err(error) => return Err(SwapFailure { journal, error }),
        }
        if let Err(error) = validate_swap_set(backend, tables).await {
            return Err(SwapFailure { journal, error });
        }
        journal.renames = tables
            .iter()
            .map(|table| RenameJournalEntry {
                from: table.active.clone(),
                to: table.backup.clone(),
                state: RenameState::Pending,
            })
            .chain(tables.iter().map(|table| RenameJournalEntry {
                from: table.shadow.clone(),
                to: table.active.clone(),
                state: RenameState::Pending,
            }))
            .collect();
        for table in tables {
            let fingerprint = match backend.fingerprint(&table.active).await {
                Ok(fingerprint) => fingerprint,
                Err(error) => return Err(SwapFailure { journal, error }),
            };
            let schema_digest = match backend.schema_digest(&table.active).await {
                Ok(schema_digest) => schema_digest,
                Err(error) => return Err(SwapFailure { journal, error }),
            };
            journal
                .source_fingerprints
                .insert(table.active.clone(), fingerprint);
            journal
                .source_schema_digests
                .insert(table.active.clone(), schema_digest);
        }
        journal.phase = JournalPhase::Preparing;
        if let Err(error) = backend.persist_journal(&journal).await {
            return Err(SwapFailure { journal, error });
        }
        if let Err(error) = self.resume_preparation(backend, &mut journal).await {
            return Err(SwapFailure { journal, error });
        }
        self.resume(backend, journal).await
    }

    async fn resume_preparation<B: MySqlSwapBackend>(
        &self,
        backend: &B,
        journal: &mut SwapJournal,
    ) -> Result<()> {
        for table in journal.tables.clone() {
            let expected = journal
                .source_fingerprints
                .get(&table.active)
                .cloned()
                .ok_or_else(|| Error::Conflict {
                    resource: "MySQL swap journal".to_owned(),
                    message: format!("missing retained fingerprint for {}", table.active),
                })?;
            let expected_schema = journal
                .source_schema_digests
                .get(&table.active)
                .cloned()
                .ok_or_else(|| Error::Conflict {
                    resource: "MySQL swap journal".to_owned(),
                    message: format!("missing retained schema digest for {}", table.active),
                })?;
            if !backend.table_exists(&table.shadow).await? {
                backend.copy_to_shadow(&table.active, &table.shadow).await?;
            }
            let shadow_fingerprint = backend.fingerprint(&table.shadow).await?;
            if expected != shadow_fingerprint {
                return Err(Error::Conflict {
                    resource: "MySQL shadow fingerprint".to_owned(),
                    message: format!("{} does not match {}", table.shadow, table.active),
                });
            }
            let shadow_schema = backend.schema_digest(&table.shadow).await?;
            if expected_schema != shadow_schema {
                return Err(Error::Conflict {
                    resource: "MySQL shadow schema".to_owned(),
                    message: format!("{} schema does not match {}", table.shadow, table.active),
                });
            }
            if journal.prepared_shadows.insert(table.shadow.clone()) {
                backend.persist_journal(journal).await?;
            }
        }
        for table in &journal.tables {
            let current = backend.fingerprint(&table.active).await?;
            if journal.source_fingerprints.get(&table.active) != Some(&current) {
                return Err(Error::Conflict {
                    resource: "MySQL active fingerprint".to_owned(),
                    message: format!(
                        "{} changed between shadow verification and cutover",
                        table.active
                    ),
                });
            }
            let current_schema = backend.schema_digest(&table.active).await?;
            if journal.source_schema_digests.get(&table.active) != Some(&current_schema) {
                return Err(Error::Conflict {
                    resource: "MySQL active schema".to_owned(),
                    message: format!("{} schema changed before cutover", table.active),
                });
            }
        }
        journal.phase = JournalPhase::Prepared;
        backend.persist_journal(journal).await
    }

    /// Resumes a prepared or interrupted cutover from a durable journal.
    pub async fn resume<B: MySqlSwapBackend>(
        &self,
        backend: &B,
        mut journal: SwapJournal,
    ) -> core::result::Result<SwapJournal, SwapFailure> {
        if let Err(error) = acquire_recovery_barrier(backend).await {
            return Err(SwapFailure { journal, error });
        }
        if let Err(error) = validate_journal_baselines(&journal) {
            return Err(SwapFailure { journal, error });
        }
        if let Err(error) = validate_resume_journal(&journal) {
            return Err(SwapFailure { journal, error });
        }
        if journal.phase == JournalPhase::Preparing
            && let Err(error) = self.resume_preparation(backend, &mut journal).await
        {
            return Err(SwapFailure { journal, error });
        }
        if let Err(error) = validate_resume_journal(&journal) {
            return Err(SwapFailure { journal, error });
        }
        if matches!(
            journal.phase,
            JournalPhase::Restoring | JournalPhase::Restored | JournalPhase::Aborted
        ) {
            return Err(SwapFailure {
                journal,
                error: Error::Conflict {
                    resource: "MySQL swap journal".to_owned(),
                    message: "journal is not resumable in its current phase".to_owned(),
                },
            });
        }
        if let Err(error) = verify_resumable_shadows(backend, &journal).await {
            return Err(SwapFailure { journal, error });
        }
        journal.phase = JournalPhase::CuttingOver;
        if let Err(error) = backend.persist_journal(&journal).await {
            return Err(SwapFailure { journal, error });
        }
        for index in 0..journal.renames.len() {
            if journal.renames[index].state == RenameState::Completed {
                continue;
            }
            if journal.renames[index].state != RenameState::Prepared {
                journal.renames[index].state = RenameState::Prepared;
                if let Err(error) = backend.persist_journal(&journal).await {
                    return Err(SwapFailure { journal, error });
                }
            }
            let from = journal.renames[index].from.clone();
            let to = journal.renames[index].to.clone();
            if index < journal.tables.len() {
                let table = &journal.tables[index];
                match (
                    backend.table_exists(&table.active).await,
                    backend.table_exists(&table.backup).await,
                ) {
                    (Ok(true), Ok(false)) => {
                        if let Err(error) =
                            verify_active_unchanged(backend, &journal, &table.active).await
                        {
                            return Err(SwapFailure { journal, error });
                        }
                    }
                    (Err(error), _) | (_, Err(error)) => {
                        return Err(SwapFailure { journal, error });
                    }
                    _ => {}
                }
            } else {
                let table = &journal.tables[index - journal.tables.len()];
                let shadow_exists = match backend.table_exists(&table.shadow).await {
                    Ok(exists) => exists,
                    Err(error) => return Err(SwapFailure { journal, error }),
                };
                let candidate = if shadow_exists {
                    table.shadow.as_str()
                } else {
                    match (
                        backend.table_exists(&table.active).await,
                        backend.table_exists(&table.backup).await,
                    ) {
                        (Ok(true), Ok(true)) => table.active.as_str(),
                        (Ok(_), Ok(_)) => {
                            let error = topology_error(&table.shadow, &table.active);
                            return Err(SwapFailure { journal, error });
                        }
                        (Err(error), _) | (_, Err(error)) => {
                            return Err(SwapFailure { journal, error });
                        }
                    }
                };
                if let Err(error) =
                    verify_table_baseline(backend, &journal, candidate, "MySQL promotion shadow")
                        .await
                {
                    return Err(SwapFailure { journal, error });
                }
            }
            if let Err(error) = reconcile_rename(backend, &from, &to).await {
                return Err(SwapFailure { journal, error });
            }
            journal.renames[index].state = RenameState::Completed;
            if let Err(error) = backend.persist_journal(&journal).await {
                return Err(SwapFailure { journal, error });
            }
        }
        journal.phase = JournalPhase::Complete;
        if let Err(error) = backend.persist_journal(&journal).await {
            return Err(SwapFailure { journal, error });
        }
        Ok(journal)
    }

    /// Removes only unused shadows when catalog reconciliation proves that no
    /// forward rename committed.
    pub async fn abort<B: MySqlSwapBackend>(
        &self,
        backend: &B,
        journal: &mut SwapJournal,
    ) -> Result<()> {
        require_write_barrier(backend).await?;
        validate_abort_journal(backend, journal).await?;
        if journal.phase == JournalPhase::Aborted {
            return Ok(());
        }
        for table in &journal.tables {
            if backend.table_exists(&table.shadow).await? {
                backend.remove_shadow(&table.shadow).await?;
            }
        }
        journal.phase = JournalPhase::Aborted;
        backend.persist_journal(journal).await
    }

    /// Resumes reverse renames and verifies every original fingerprint.
    pub async fn restore<B: MySqlSwapBackend>(
        &self,
        backend: &B,
        journal: &mut SwapJournal,
    ) -> Result<RestoreReport> {
        require_write_barrier(backend).await?;
        validate_restore_journal(backend, journal).await?;
        if journal.phase == JournalPhase::Restored {
            return Ok(RestoreReport {
                restored_tables: journal.tables.len(),
            });
        }
        journal.phase = JournalPhase::Restoring;
        backend.persist_journal(journal).await?;
        let table_count = journal.tables.len();
        for table_index in (0..table_count).rev() {
            let table = journal.tables[table_index].clone();
            let backup_index = table_index;
            let promotion_index = table_count + table_index;
            let backup_exists = backend.table_exists(&table.backup).await?;
            let active_exists = backend.table_exists(&table.active).await?;
            let shadow_exists = backend.table_exists(&table.shadow).await?;

            if backup_exists {
                match (active_exists, shadow_exists) {
                    (true, false) => {
                        set_rename_state(
                            backend,
                            journal,
                            promotion_index,
                            RenameState::RestorePrepared,
                        )
                        .await?;
                        reconcile_rename(backend, &table.active, &table.shadow).await?;
                    }
                    (false, true) => {}
                    _ => {
                        return Err(Error::Conflict {
                            resource: "MySQL restore topology".to_owned(),
                            message: format!(
                                "{} has an ambiguous active/shadow topology",
                                table.active
                            ),
                        });
                    }
                }
                set_rename_state(backend, journal, promotion_index, RenameState::Restored).await?;
                set_rename_state(backend, journal, backup_index, RenameState::RestorePrepared)
                    .await?;
                reconcile_rename(backend, &table.backup, &table.active).await?;
                set_rename_state(backend, journal, backup_index, RenameState::Restored).await?;
            } else {
                if !active_exists {
                    return Err(Error::Conflict {
                        resource: "MySQL restore topology".to_owned(),
                        message: format!(
                            "{} has neither its original active table nor backup",
                            table.active
                        ),
                    });
                }
                set_rename_state(backend, journal, promotion_index, RenameState::Restored).await?;
                set_rename_state(backend, journal, backup_index, RenameState::Restored).await?;
            }
            if backend.table_exists(&table.shadow).await? {
                backend.remove_shadow(&table.shadow).await?;
            }
        }
        backend.apply_cleanup(&journal.cleanup).await?;
        for table in &journal.tables {
            let expected = journal
                .source_fingerprints
                .get(&table.active)
                .expect("validated fingerprint coverage");
            let actual = backend.fingerprint(&table.active).await?;
            if actual != *expected {
                return Err(Error::Conflict {
                    resource: "MySQL restore fingerprint".to_owned(),
                    message: format!(
                        "{} does not match its retained pre-cutover fingerprint",
                        table.active
                    ),
                });
            }
            let actual_schema = backend.schema_digest(&table.active).await?;
            if journal.source_schema_digests.get(&table.active) != Some(&actual_schema) {
                return Err(Error::Conflict {
                    resource: "MySQL restore schema".to_owned(),
                    message: format!(
                        "{} schema does not match its retained pre-cutover digest",
                        table.active
                    ),
                });
            }
        }
        journal.phase = JournalPhase::Restored;
        backend.persist_journal(journal).await?;
        Ok(RestoreReport {
            restored_tables: journal.tables.len(),
        })
    }
}
fn validate_journal_structure(journal: &SwapJournal) -> Result<()> {
    let table_count = journal.tables.len();
    if table_count == 0 || journal.renames.len() != table_count * 2 {
        return Err(Error::Conflict {
            resource: "MySQL swap journal".to_owned(),
            message: "rename plan does not match the journal table set".to_owned(),
        });
    }
    let mut names = BTreeSet::new();
    for table in &journal.tables {
        for name in [&table.active, &table.shadow, &table.backup] {
            if !is_identifier(name) || !names.insert(name.as_str()) {
                return Err(Error::Conflict {
                    resource: "MySQL swap journal".to_owned(),
                    message: "journal table names must be valid and globally unique".to_owned(),
                });
            }
        }
    }
    for (index, table) in journal.tables.iter().enumerate() {
        let backup = &journal.renames[index];
        let promotion = &journal.renames[table_count + index];
        if backup.from != table.active
            || backup.to != table.backup
            || promotion.from != table.shadow
            || promotion.to != table.active
        {
            return Err(Error::Conflict {
                resource: "MySQL swap journal".to_owned(),
                message: "retained rename entries do not match the journal tables".to_owned(),
            });
        }
    }
    Ok(())
}

fn validate_resume_journal(journal: &SwapJournal) -> Result<()> {
    validate_journal_structure(journal)?;
    if journal.renames.iter().any(|rename| {
        !matches!(
            rename.state,
            RenameState::Pending | RenameState::Prepared | RenameState::Completed
        )
    }) {
        return Err(Error::Conflict {
            resource: "MySQL swap journal".to_owned(),
            message: "resume journal contains reverse-only rename state".to_owned(),
        });
    }
    if journal.phase == JournalPhase::Complete
        && journal
            .renames
            .iter()
            .any(|rename| rename.state != RenameState::Completed)
    {
        return Err(Error::Conflict {
            resource: "MySQL swap journal".to_owned(),
            message: "complete journal contains unfinished renames".to_owned(),
        });
    }
    if journal.phase != JournalPhase::Preparing {
        let expected = journal
            .tables
            .iter()
            .map(|table| table.shadow.as_str())
            .collect::<BTreeSet<_>>();
        let prepared = journal
            .prepared_shadows
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if expected != prepared {
            return Err(Error::Conflict {
                resource: "MySQL swap journal".to_owned(),
                message: "prepared-shadow coverage does not match the journal table set".to_owned(),
            });
        }
    }
    Ok(())
}

async fn validate_abort_journal<B: MySqlSwapBackend>(
    backend: &B,
    journal: &SwapJournal,
) -> Result<()> {
    validate_journal_baselines(journal)?;
    validate_journal_structure(journal)?;
    if !matches!(
        journal.phase,
        JournalPhase::Preparing
            | JournalPhase::Prepared
            | JournalPhase::CuttingOver
            | JournalPhase::Aborted
    ) || journal
        .renames
        .iter()
        .any(|rename| !matches!(rename.state, RenameState::Pending | RenameState::Prepared))
    {
        return Err(Error::Conflict {
            resource: "MySQL swap abort".to_owned(),
            message: "journal is not in a pre-cutover abortable state".to_owned(),
        });
    }
    for table in &journal.tables {
        if !backend.table_exists(&table.active).await?
            || backend.table_exists(&table.backup).await?
        {
            return Err(Error::Conflict {
                resource: "MySQL swap abort".to_owned(),
                message: "cutover changed table topology; restore is required".to_owned(),
            });
        }
        verify_table_baseline(backend, journal, &table.active, "MySQL abort active table").await?;
        if backend.table_exists(&table.shadow).await? {
            if journal.phase == JournalPhase::Aborted {
                return Err(Error::Conflict {
                    resource: "MySQL swap abort".to_owned(),
                    message: "aborted journal unexpectedly has a shadow table".to_owned(),
                });
            }
            verify_table_baseline(backend, journal, &table.shadow, "MySQL abort shadow").await?;
        }
    }
    Ok(())
}

async fn validate_restore_journal<B: MySqlSwapBackend>(
    backend: &B,
    journal: &SwapJournal,
) -> Result<()> {
    validate_journal_baselines(journal)?;
    validate_journal_structure(journal)?;
    match journal.phase {
        JournalPhase::CuttingOver => {
            if journal.renames.iter().any(|rename| {
                !matches!(
                    rename.state,
                    RenameState::Pending | RenameState::Prepared | RenameState::Completed
                )
            }) {
                return Err(Error::Conflict {
                    resource: "MySQL swap restore".to_owned(),
                    message: "cutover journal contains reverse-only rename state".to_owned(),
                });
            }
            let mut committed = false;
            for rename in &journal.renames {
                let from_exists = backend.table_exists(&rename.from).await?;
                let to_exists = backend.table_exists(&rename.to).await?;
                committed |= rename.state == RenameState::Completed || (!from_exists && to_exists);
            }
            if !committed {
                return Err(Error::Conflict {
                    resource: "MySQL swap restore".to_owned(),
                    message: "cutover journal has no committed rename to restore".to_owned(),
                });
            }
        }
        JournalPhase::Complete => {
            if journal
                .renames
                .iter()
                .any(|rename| rename.state != RenameState::Completed)
            {
                return Err(Error::Conflict {
                    resource: "MySQL swap restore".to_owned(),
                    message: "complete journal contains unfinished renames".to_owned(),
                });
            }
        }
        JournalPhase::Restoring => {}
        JournalPhase::Restored => {
            if journal
                .renames
                .iter()
                .any(|rename| rename.state != RenameState::Restored)
            {
                return Err(Error::Conflict {
                    resource: "MySQL swap restore".to_owned(),
                    message: "restored journal contains unfinished reverse renames".to_owned(),
                });
            }
            for table in &journal.tables {
                if !backend.table_exists(&table.active).await?
                    || backend.table_exists(&table.backup).await?
                    || backend.table_exists(&table.shadow).await?
                {
                    return Err(Error::Conflict {
                        resource: "MySQL swap restore".to_owned(),
                        message: "restored journal does not match the current table topology"
                            .to_owned(),
                    });
                }
                verify_table_baseline(
                    backend,
                    journal,
                    &table.active,
                    "MySQL restored active table",
                )
                .await?;
            }
        }
        JournalPhase::Preparing | JournalPhase::Prepared | JournalPhase::Aborted => {
            return Err(Error::Conflict {
                resource: "MySQL swap restore".to_owned(),
                message: "journal is not in a restorable cutover state".to_owned(),
            });
        }
    }
    Ok(())
}

async fn verify_table_baseline<B: MySqlSwapBackend>(
    backend: &B,
    journal: &SwapJournal,
    table: &str,
    resource: &str,
) -> Result<()> {
    let active = journal
        .tables
        .iter()
        .find(|candidate| {
            candidate.active == table || candidate.shadow == table || candidate.backup == table
        })
        .map(|candidate| candidate.active.as_str())
        .ok_or_else(|| Error::Conflict {
            resource: resource.to_owned(),
            message: "table is outside the retained swap set".to_owned(),
        })?;
    let expected_fingerprint =
        journal
            .source_fingerprints
            .get(active)
            .ok_or_else(|| Error::Conflict {
                resource: resource.to_owned(),
                message: "missing retained fingerprint".to_owned(),
            })?;
    let expected_schema =
        journal
            .source_schema_digests
            .get(active)
            .ok_or_else(|| Error::Conflict {
                resource: resource.to_owned(),
                message: "missing retained schema digest".to_owned(),
            })?;
    if backend.fingerprint(table).await? != *expected_fingerprint
        || backend.schema_digest(table).await? != *expected_schema
    {
        return Err(Error::Conflict {
            resource: resource.to_owned(),
            message: format!("{table} does not match its retained pre-cutover baseline"),
        });
    }
    Ok(())
}

fn validate_journal_baselines(journal: &SwapJournal) -> Result<()> {
    let active = journal
        .tables
        .iter()
        .map(|table| table.active.as_str())
        .collect::<BTreeSet<_>>();
    let fingerprints = journal
        .source_fingerprints
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let schemas = journal
        .source_schema_digests
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if active.len() != journal.tables.len() || fingerprints != active || schemas != active {
        return Err(Error::Conflict {
            resource: "MySQL swap journal".to_owned(),
            message: "journal baselines must exactly cover every active table".to_owned(),
        });
    }
    Ok(())
}

async fn set_rename_state<B: MySqlSwapBackend>(
    backend: &B,
    journal: &mut SwapJournal,
    index: usize,
    state: RenameState,
) -> Result<()> {
    if journal.renames[index].state != state {
        journal.renames[index].state = state;
        backend.persist_journal(journal).await?;
    }
    Ok(())
}

async fn verify_active_unchanged<B: MySqlSwapBackend>(
    backend: &B,
    journal: &SwapJournal,
    active: &str,
) -> Result<()> {
    let expected = journal
        .source_fingerprints
        .get(active)
        .ok_or_else(|| Error::Conflict {
            resource: "MySQL swap journal".to_owned(),
            message: format!("missing retained fingerprint for {active}"),
        })?;
    let actual = backend.fingerprint(active).await?;
    if &actual != expected {
        return Err(Error::Conflict {
            resource: "MySQL active fingerprint".to_owned(),
            message: format!("{active} changed before resumed cutover"),
        });
    }
    let expected_schema =
        journal
            .source_schema_digests
            .get(active)
            .ok_or_else(|| Error::Conflict {
                resource: "MySQL swap journal".to_owned(),
                message: format!("missing retained schema digest for {active}"),
            })?;
    let actual_schema = backend.schema_digest(active).await?;
    if &actual_schema != expected_schema {
        return Err(Error::Conflict {
            resource: "MySQL active schema".to_owned(),
            message: format!("{active} schema changed before resumed cutover"),
        });
    }
    Ok(())
}

async fn acquire_recovery_barrier<B: MySqlSwapBackend>(backend: &B) -> Result<()> {
    backend.acquire_write_barrier().await?;
    require_write_barrier(backend).await
}

async fn require_write_barrier<B: MySqlSwapBackend>(backend: &B) -> Result<()> {
    if backend.write_barrier_held().await? {
        Ok(())
    } else {
        Err(Error::Conflict {
            resource: "MySQL source write barrier".to_owned(),
            message: "recovery requires an active source-write barrier".to_owned(),
        })
    }
}
async fn verify_resumable_shadows<B: MySqlSwapBackend>(
    backend: &B,
    journal: &SwapJournal,
) -> Result<()> {
    let table_count = journal.tables.len();
    for (index, table) in journal.tables.iter().enumerate() {
        if journal.renames[table_count + index].state == RenameState::Completed {
            continue;
        }
        let candidate = if backend.table_exists(&table.shadow).await? {
            table.shadow.as_str()
        } else if backend.table_exists(&table.active).await?
            && backend.table_exists(&table.backup).await?
        {
            table.active.as_str()
        } else {
            return Err(topology_error(&table.shadow, &table.active));
        };
        verify_table_baseline(
            backend,
            journal,
            candidate,
            "MySQL resumable promotion shadow",
        )
        .await?;
    }
    Ok(())
}

async fn validate_swap_set<B: MySqlSwapBackend>(backend: &B, tables: &[SwapTable]) -> Result<()> {
    if tables.is_empty() {
        return Err(Error::InvalidInput {
            field: "MySQL swap tables".to_owned(),
            message: "at least one table is required".to_owned(),
        });
    }
    let mut names = BTreeSet::new();
    for table in tables {
        for name in [&table.active, &table.shadow, &table.backup] {
            if !is_identifier(name) {
                return Err(Error::InvalidInput {
                    field: "MySQL swap table".to_owned(),
                    message: "table names must be non-empty ASCII identifiers".to_owned(),
                });
            }
            if !names.insert(name.clone()) {
                return Err(Error::InvalidInput {
                    field: "MySQL swap table".to_owned(),
                    message: "every active, shadow, and backup name must be globally unique"
                        .to_owned(),
                });
            }
        }
    }
    for table in tables {
        if !backend.table_exists(&table.active).await? {
            return Err(Error::NotFound {
                resource: "active table".to_owned(),
                identifier: table.active.clone(),
            });
        }
        for unused in [&table.shadow, &table.backup] {
            if backend.table_exists(unused).await? {
                return Err(Error::Conflict {
                    resource: "MySQL swap destination".to_owned(),
                    message: format!("{unused} already exists"),
                });
            }
        }
    }
    Ok(())
}

async fn reconcile_rename<B: MySqlSwapBackend>(backend: &B, from: &str, to: &str) -> Result<()> {
    match (
        backend.table_exists(from).await?,
        backend.table_exists(to).await?,
    ) {
        (true, false) => backend.rename(from, to).await,
        (false, true) => Ok(()),
        _ => Err(topology_error(from, to)),
    }
}

fn topology_error(from: &str, to: &str) -> Error {
    Error::Conflict {
        resource: "MySQL rename topology".to_owned(),
        message: format!("expected exactly one of {from} and {to} to exist"),
    }
}

fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}
