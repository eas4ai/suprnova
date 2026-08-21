//! Stable dry-run plans and source cleanup declarations.

use sea_orm::DbBackend;
use sha2::{Digest, Sha256};

use super::fingerprint::SourceTableFingerprint;
use super::{ExternalIdentity, IdentityMapPlan, ShapeConfirmation, SourceShape};

/// A table-level migration operation presented during dry run.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TableOperation {
    /// The source table or column targeted by this operation.
    pub target: String,
    /// The action taken after explicit apply.
    pub kind: TableOperationKind,
}

/// The category of a planned table operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TableOperationKind {
    /// Source data is read and sent through an application binding.
    Import,
    /// Existing durable authentication data remains untouched.
    Preserve,
    /// A bearer, ceremony, or migration marker is invalidated.
    Invalidate,
}

/// Explicitly loss-tolerant state invalidated by an upgrade.
///
/// This list intentionally excludes users, credentials, passkeys, linked
/// accounts, verification state, and two-factor enrollments.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SacrificeableCleanup {
    /// Stable sorted source targets invalidated during apply when they exist.
    pub targets: Vec<String>,
}

impl SacrificeableCleanup {
    pub(crate) fn for_source(source: SourceShape) -> Self {
        let targets = match source {
            SourceShape::Torii => vec![
                "passkey_challenges",
                "pkce_verifiers",
                "sessions",
                "torii_migrations",
            ],
            SourceShape::SuprnovaWeb => vec![
                "auth_ceremony_tokens",
                "auth_flow_tokens",
                "passkey_challenges",
                "pkce_verifiers",
                "remember_tokens",
                "sessions",
                "users.remember_token",
            ],
            SourceShape::SuprnovaApi | SourceShape::Magnetar => Vec::new(),
        };
        Self {
            targets: targets.into_iter().map(str::to_owned).collect(),
        }
    }

    /// Returns whether the cleanup contract invalidates one source target.
    pub fn invalidates(&self, target: &str) -> bool {
        self.targets
            .binary_search_by(|item| item.as_str().cmp(target))
            .is_ok()
    }
}

/// A stable source table row count captured during dry run.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourceRowCount {
    /// The source table name.
    pub table: String,
    /// The exact number of rows observed during preflight.
    pub rows: u64,
}

/// One explicit source field to host-binding destination rule.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FieldMapping {
    /// The source table and field.
    pub source: String,
    /// The host-binding destination, not a presumed application table.
    pub destination: String,
    /// The preservation or normalization rule applied to this field.
    pub semantics: String,
}

/// The database family that determines the migration write posture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MigrationBackend {
    /// SQLite supports the standard transactional posture.
    Sqlite,
    /// PostgreSQL supports the standard transactional posture.
    Postgres,
}

/// Explicit database write and recovery posture emitted in every dry run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackendStrategy {
    /// Use a host transaction for normal source and binding writes.
    Transactional {
        /// The transactional database family.
        backend: MigrationBackend,
    },
    /// Use shadow copies, fingerprints, and a retained rename journal.
    MySqlShadowSwap {
        /// Copy mechanism used before cutover.
        copy: &'static str,
        /// Data-integrity verification used before and after recovery.
        verification: &'static str,
        /// Cutover mechanism retained in the host journal.
        cutover: &'static str,
        /// Recovery mechanism after a partial cutover.
        recovery: &'static str,
    },
}

impl BackendStrategy {
    /// Returns the stable write posture for a SeaORM database backend.
    pub const fn for_backend(backend: DbBackend) -> Self {
        match backend {
            DbBackend::Sqlite => Self::Transactional {
                backend: MigrationBackend::Sqlite,
            },
            DbBackend::Postgres => Self::Transactional {
                backend: MigrationBackend::Postgres,
            },
            DbBackend::MySql => Self::MySqlShadowSwap {
                copy: "shadow-copy",
                verification: "table-fingerprint",
                cutover: "rename-journal",
                recovery: "reverse-rename-restore",
            },
        }
    }
}

/// A complete no-write migration plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationPlan {
    /// Stable digest identifying the exact reviewed source and identity plan.
    pub plan_id: String,
    /// The unambiguous source schema shape.
    pub source: SourceShape,
    /// The operator confirmation recorded with the plan.
    pub confirmation: ShapeConfirmation,
    /// Normalized-email collision groups found during preflight.
    ///
    /// A runnable plan always has an empty list because collisions abort before
    /// a plan is returned. The field makes the stable report shape explicit.
    pub normalized_collisions: Vec<super::CollisionGroup>,
    /// Warnings about source tables that already occupy host target names.
    pub warnings: Vec<String>,
    /// Stable counts for every source table observed by this dry run.
    pub source_row_counts: Vec<SourceRowCount>,
    /// Stable content fingerprints for every source table.
    pub source_fingerprints: Vec<SourceTableFingerprint>,
    /// Explicit field-level source-to-binding rules.
    pub field_mappings: Vec<FieldMapping>,
    /// The backend write, cutover, and recovery posture.
    pub backend_strategy: BackendStrategy,
    /// Stable table-level operations shown to the operator.
    pub table_operations: Vec<TableOperation>,
    /// Torii-to-application mapping decisions.
    pub identity_map: IdentityMapPlan,
    /// State intentionally invalidated only after imports complete.
    pub sacrificeable_cleanup: SacrificeableCleanup,
}

impl MigrationPlan {
    pub(crate) fn new(
        source: SourceShape,
        confirmation: ShapeConfirmation,
        identity_map: IdentityMapPlan,
        warnings: Vec<String>,
        source_row_counts: Vec<SourceRowCount>,
        source_fingerprints: Vec<SourceTableFingerprint>,
        backend_strategy: BackendStrategy,
    ) -> Self {
        let cleanup = SacrificeableCleanup::for_source(source);
        let mut table_operations = vec![
            TableOperation {
                target: "credentials".to_owned(),
                kind: TableOperationKind::Preserve,
            },
            TableOperation {
                target: "linked_accounts".to_owned(),
                kind: TableOperationKind::Preserve,
            },
            TableOperation {
                target: "passkeys".to_owned(),
                kind: TableOperationKind::Import,
            },
            TableOperation {
                target: "users".to_owned(),
                kind: TableOperationKind::Import,
            },
            TableOperation {
                target: "verification".to_owned(),
                kind: TableOperationKind::Preserve,
            },
        ];
        table_operations.extend(
            cleanup
                .targets
                .iter()
                .cloned()
                .map(|target| TableOperation {
                    target,
                    kind: TableOperationKind::Invalidate,
                }),
        );
        table_operations.sort();
        let normalized_collisions = Vec::new();
        let field_mappings = field_mappings(source);
        let canonical = format!(
            "{source:?}\n{confirmation:?}\n{normalized_collisions:?}\n{warnings:?}\n{source_row_counts:?}\n{source_fingerprints:?}\n{field_mappings:?}\n{backend_strategy:?}\n{table_operations:?}\n{identity_map:?}\n{cleanup:?}"
        );
        let plan_id = format!("{:x}", Sha256::digest(canonical.as_bytes()));
        Self {
            plan_id,
            source,
            confirmation,
            normalized_collisions,
            warnings,
            source_row_counts,
            source_fingerprints,
            field_mappings,
            backend_strategy,
            table_operations,
            identity_map,
            sacrificeable_cleanup: cleanup,
        }
    }
}

fn field_mappings(source: SourceShape) -> Vec<FieldMapping> {
    let mut mappings = match source {
        SourceShape::Torii => vec![
            mapping(
                "users.id",
                "ExternalIdentity.external_user_id",
                "provider-owned Torii ID; never becomes an app ID",
            ),
            mapping(
                "users.email",
                "MigrationBindings::create_app_user(email)",
                "normalize for matching; preserve source email for creation",
            ),
            mapping(
                "users.name",
                "host application binding",
                "preserve profile display name when the host owns one",
            ),
            mapping(
                "users.password_hash",
                "host credential binding",
                "preserve the opaque password hash; never rehash during migration",
            ),
            mapping(
                "users.email_verified_at",
                "host verification binding",
                "preserve verification timestamp",
            ),
            mapping(
                "users.locked_at",
                "host lockout binding",
                "preserve lockout state",
            ),
            mapping(
                "passkeys.user_id",
                "ExternalIdentity.app_user_id",
                "resolve through the Torii external identity binding",
            ),
            mapping(
                "passkeys.credential_id",
                "MigrationBindings::import_passkey(credential_id)",
                "preserve credential identifier",
            ),
            mapping(
                "passkeys.data_json",
                "MigrationBindings::import_passkey(data_json)",
                "byte-preserve without decoding or reserialization",
            ),
            mapping(
                "oauth_accounts.user_id",
                "ExternalIdentity.app_user_id",
                "resolve linked-account owner through the Torii external identity binding",
            ),
            mapping(
                "oauth_accounts.provider",
                "host linked-account binding",
                "preserve provider name",
            ),
            mapping(
                "oauth_accounts.subject",
                "host linked-account binding",
                "preserve provider subject",
            ),
            mapping(
                "secure_tokens.user_id",
                "ExternalIdentity.app_user_id",
                "resolve durable verification owner through the Torii external identity binding",
            ),
            mapping(
                "secure_tokens.purpose",
                "host verification binding",
                "preserve verification or reset purpose",
            ),
            mapping(
                "secure_tokens.used_at",
                "host verification binding",
                "preserve consumed state",
            ),
            mapping(
                "secure_tokens.expires_at",
                "host verification binding",
                "preserve expiry",
            ),
            mapping(
                "failed_login_attempts.email",
                "host lockout binding",
                "preserve normalized lockout subject",
            ),
            mapping(
                "failed_login_attempts.ip_address",
                "host lockout binding",
                "preserve lockout network context",
            ),
            mapping(
                "failed_login_attempts.attempted_at",
                "host lockout binding",
                "preserve failed-login chronology",
            ),
        ],
        SourceShape::SuprnovaWeb => vec![
            mapping(
                "users.id",
                "host application binding",
                "host-owned identity mapping; never infer an app primary key",
            ),
            mapping(
                "users.name",
                "host application binding",
                "preserve profile display name when the host owns one",
            ),
            mapping(
                "users.email",
                "host application binding",
                "normalize only for collision and identity matching",
            ),
            mapping(
                "users.password",
                "host credential binding",
                "preserve the opaque password hash; never rehash during migration",
            ),
            mapping(
                "users.email_verified_at",
                "host verification binding",
                "preserve verification timestamp",
            ),
            mapping(
                "two_factor_credentials.user_id",
                "host two-factor binding",
                "resolve durable two-factor owner through the host identity map",
            ),
            mapping(
                "two_factor_credentials.secret",
                "host two-factor binding",
                "preserve encrypted TOTP secret ciphertext",
            ),
            mapping(
                "two_factor_credentials.confirmed_at",
                "host two-factor binding",
                "preserve enrollment confirmation",
            ),
            mapping(
                "two_factor_credentials.recovery_codes",
                "host two-factor binding",
                "preserve recovery-code ciphertext",
            ),
            mapping(
                "two_factor_credentials.last_used_timestep",
                "host two-factor binding",
                "preserve TOTP replay protection state",
            ),
        ],
        SourceShape::SuprnovaApi => vec![
            mapping(
                "app_users.id",
                "host application binding",
                "preserve existing app-owned i64 identity",
            ),
            mapping(
                "app_users.email",
                "host application binding",
                "normalize only for collision and identity matching",
            ),
        ],
        SourceShape::Magnetar => Vec::new(),
    };
    mappings.sort();
    mappings
}

fn mapping(source: &str, destination: &str, semantics: &str) -> FieldMapping {
    FieldMapping {
        source: source.to_owned(),
        destination: destination.to_owned(),
        semantics: semantics.to_owned(),
    }
}

/// The result of an applied source migration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationReport {
    /// The source schema shape that was applied.
    pub source: SourceShape,
    /// Bindings persisted by the application-owned identity seam.
    pub identity_mappings: Vec<ExternalIdentity>,
    /// The cleanup contract executed against existing source artifacts.
    pub cleanup: SacrificeableCleanup,
    /// The number of source cleanup statements submitted.
    pub cleanup_statements: usize,
}
