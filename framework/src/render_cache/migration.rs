//! The RenderCache schema: [`Migration`] creates durable generation truth
//! (current generations, an append-only change log, and the authority
//! epoch), and [`TierMigration`] creates the four tables the Tier 1 and
//! Tier 2 providers need on top of it.
//!
//! The identity column holds the lowercase hex of a
//! [`DependencyIdentity`](super::DependencyIdentity)'s 32-byte digest, never
//! raw binary: a fixed 64-character hex string
//! compares and indexes identically on Postgres, MySQL, and SQLite, and it
//! is the exact wire form [`GenerationSet`](suprnova_live::render_cache::GenerationSet)
//! already uses. `string_len(64)` is sized explicitly rather than left at
//! SeaORM's default `.string()` (`VARCHAR(255)`): under `utf8mb4` that
//! default is 1020 bytes, which runs into MySQL's index key length limit on
//! a primary key column.
//!
//! Consumer apps include this migration in their `Migrator`'s
//! `migrations()` list - the framework owns the schema, the app owns when
//! to apply it.

use sea_orm_migration::prelude::*;

/// Creates the three `suprnova_render_` tables.
pub struct Migration;

impl MigrationName for Migration {
    // Explicit, file-stable name. `DeriveMigrationName` derives from the
    // file stem (`file!()`), which for this file is just "migration" - not
    // unique enough once a second framework-owned migration lands in
    // another module also named `migration.rs` (the two-factor migration
    // already is one; it explains the same collision risk for the same
    // reason). Matches that convention.
    fn name(&self) -> &str {
        "m20260903_000000_create_render_cache_tables"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Generations::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Generations::Identity)
                            .string_len(64)
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Generations::Generation)
                            .big_integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(Generations::Epoch).big_integer().not_null())
                    .col(
                        ColumnDef::new(Generations::UpdatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(GenerationLog::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(GenerationLog::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(GenerationLog::Identity)
                            .string_len(64)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(GenerationLog::Generation)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(GenerationLog::Epoch)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(GenerationLog::CommittedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;
        // `if_not_exists()` here too: an idempotency test that re-runs the
        // whole migration would otherwise fail on this index specifically,
        // not just on the epoch seed below - found by writing that test.
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("suprnova_render_generation_log_identity")
                    .table(GenerationLog::Table)
                    .col(GenerationLog::Identity)
                    .col(GenerationLog::Id)
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Epochs::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Epochs::Singleton)
                            .small_integer()
                            .not_null()
                            .primary_key()
                            .check(Expr::col(Epochs::Singleton).eq(1_i16)),
                    )
                    .col(ColumnDef::new(Epochs::Epoch).big_integer().not_null())
                    .to_owned(),
            )
            .await?;
        // `do_nothing_on` rather than the bare `do_nothing()`: sea-query's
        // own doc admits the target-less `DO NOTHING` "is not valid today"
        // for MySQL and renders `ON DUPLICATE KEY IGNORE`, which MySQL
        // rejects outright. Naming the conflict column here compiles to
        // `INSERT IGNORE` on MySQL and `ON CONFLICT (singleton) DO NOTHING`
        // on Postgres/SQLite - both real, and both make the seed a no-op on
        // a second run, which every `create_table(...).if_not_exists()`
        // above already implies is safe.
        let insert = Query::insert()
            .into_table(Epochs::Table)
            .columns([Epochs::Singleton, Epochs::Epoch])
            .values_panic([1_i16.into(), 1_i64.into()])
            .on_conflict(
                OnConflict::column(Epochs::Singleton)
                    .do_nothing_on([Epochs::Singleton])
                    .to_owned(),
            )
            .to_owned();
        manager.exec_stmt(insert).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Epochs::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(GenerationLog::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Generations::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Generations {
    #[sea_orm(iden = "suprnova_render_generations")]
    Table,
    Identity,
    Generation,
    Epoch,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum GenerationLog {
    #[sea_orm(iden = "suprnova_render_generation_log")]
    Table,
    Id,
    Identity,
    Generation,
    Epoch,
    CommittedAt,
}

#[derive(DeriveIden)]
enum Epochs {
    #[sea_orm(iden = "suprnova_render_epochs")]
    Table,
    Singleton,
    Epoch,
}

/// Creates the four tables the database-backed (Tier 1) providers use: the
/// L1 render store, the fenced rebuild leases, and the Live instance and
/// promotion records.
///
/// Registered next to [`Migration`] rather than folded into it: every
/// RenderCache profile needs the generation ledger, and only a profile whose
/// L1, rebuild coordinator, or Live instance ledger is database-backed needs
/// these four, so an application on the embedded profile carries none of
/// them. `RenderCache::install` refuses such a profile when they are absent
/// (see
/// [`ledger::tier_migration_present`](super::ledger::tier_migration_present)).
///
/// # Column types
///
/// Render keys are stored as
/// [`RenderKey::to_base64url`](suprnova_live::render_cache::key::RenderKey::to_base64url)
/// (`rk1.` plus 43 base64url characters, 47 in total), never a second hash
/// of the key, in a `VARCHAR(48)` sized explicitly for the same reason
/// [`Migration`]'s own `string_len(64)` is: SeaORM's default `.string()` is
/// `VARCHAR(255)`, which under `utf8mb4` runs into MySQL's index key length
/// limit on a primary key column. Scope, instance, idempotency, and
/// generation digests are lowercase hex of a fixed-width digest, so they are
/// `CHAR` of exactly that width. Every millisecond timestamp is a `BIGINT`
/// holding milliseconds since the Unix epoch as the *database* reports it
/// (see `render_cache::providers::sql_now_ms`), never a node clock.
///
/// Entry and record payloads are blobs, and the blob column type is the one
/// dialect difference this migration cannot express portably: sea-query's
/// `ColumnType::Blob` renders as `blob` on MySQL, which caps at 64 KiB - far
/// below a cached document - so MySQL gets an explicit `LONGBLOB` while
/// Postgres (`bytea`) and SQLite (`blob`) take the portable spelling.
///
/// # Indexes
///
/// Three of the four tables carry a non-unique index on `expires_at_ms`,
/// because three of them are reclaimed by a bounded sweep that reads
/// `ORDER BY expires_at_ms LIMIT n` - the L1 store's own `sweep`, and the
/// record store's per-operation reclamation of elapsed instances and
/// promotions. Without the index that ordered read is a full table scan on
/// every sweep, which is exactly the shape a shared table cannot afford.
/// `suprnova_render_leases` needs none: a lease is only ever read, taken
/// over, or released by its primary key, and nothing sweeps it.
pub struct TierMigration;

impl MigrationName for TierMigration {
    // Explicit and file-stable, for the same reason [`Migration`]'s name is:
    // `DeriveMigrationName` would derive "migration" from this file's stem
    // for both migrations in it.
    fn name(&self) -> &str {
        "m20260906_000000_create_render_cache_tier_tables"
    }
}

/// A blob column sized for the active backend. See [`TierMigration`]'s own
/// documentation for why MySQL cannot take the portable spelling.
fn blob_column<T: IntoIden>(backend: sea_orm::DbBackend, name: T) -> ColumnDef {
    let mut column = ColumnDef::new(name);
    if backend == sea_orm::DbBackend::MySql {
        column.custom(Alias::new("LONGBLOB"));
    } else {
        column.blob();
    }
    column.not_null();
    column.take()
}

#[async_trait::async_trait]
impl MigrationTrait for TierMigration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();

        manager
            .create_table(
                Table::create()
                    .table(RenderEntries::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(RenderEntries::RenderKey)
                            .string_len(48)
                            .not_null()
                            .primary_key(),
                    )
                    .col(blob_column(backend, RenderEntries::Bytes))
                    .col(
                        ColumnDef::new(RenderEntries::Epoch)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(RenderEntries::Token)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(RenderEntries::GenerationDigest)
                            .char_len(64)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(RenderEntries::PublishedAtMs)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(RenderEntries::ExpiresAtMs)
                            .big_integer()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_suprnova_render_entries_expires")
                    .table(RenderEntries::Table)
                    .col(RenderEntries::ExpiresAtMs)
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(RenderLeases::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(RenderLeases::RenderKey)
                            .string_len(48)
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(RenderLeases::Epoch).big_integer().not_null())
                    .col(
                        ColumnDef::new(RenderLeases::LeaseId)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(RenderLeases::ExpiresAtMs)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(RenderLeases::NextToken)
                            .big_integer()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(LiveInstances::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(LiveInstances::Scope).char_len(64).not_null())
                    .col(
                        ColumnDef::new(LiveInstances::Instance)
                            .char_len(32)
                            .not_null(),
                    )
                    .col(blob_column(backend, LiveInstances::Record))
                    .col(
                        ColumnDef::new(LiveInstances::Version)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(LiveInstances::ExpiresAtMs)
                            .big_integer()
                            .not_null(),
                    )
                    .primary_key(
                        Index::create()
                            .col(LiveInstances::Scope)
                            .col(LiveInstances::Instance),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_suprnova_live_instances_expires")
                    .table(LiveInstances::Table)
                    .col(LiveInstances::ExpiresAtMs)
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(LivePromotions::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(LivePromotions::Scope)
                            .char_len(64)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(LivePromotions::Idempotency)
                            .char_len(32)
                            .not_null(),
                    )
                    .col(blob_column(backend, LivePromotions::Record))
                    .col(
                        ColumnDef::new(LivePromotions::ExpiresAtMs)
                            .big_integer()
                            .not_null(),
                    )
                    .primary_key(
                        Index::create()
                            .col(LivePromotions::Scope)
                            .col(LivePromotions::Idempotency),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_suprnova_live_promotions_expires")
                    .table(LivePromotions::Table)
                    .col(LivePromotions::ExpiresAtMs)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Symmetric with `up`, indexes included, and each index is dropped
        // before its table so a backend that refuses to drop an index of a
        // table that is already gone never sees that order.
        manager
            .drop_index(
                Index::drop()
                    .if_exists()
                    .name("idx_suprnova_live_promotions_expires")
                    .table(LivePromotions::Table)
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(Table::drop().table(LivePromotions::Table).to_owned())
            .await?;
        manager
            .drop_index(
                Index::drop()
                    .if_exists()
                    .name("idx_suprnova_live_instances_expires")
                    .table(LiveInstances::Table)
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(Table::drop().table(LiveInstances::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(RenderLeases::Table).to_owned())
            .await?;
        manager
            .drop_index(
                Index::drop()
                    .if_exists()
                    .name("idx_suprnova_render_entries_expires")
                    .table(RenderEntries::Table)
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(Table::drop().table(RenderEntries::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum RenderEntries {
    #[sea_orm(iden = "suprnova_render_entries")]
    Table,
    RenderKey,
    Bytes,
    Epoch,
    Token,
    GenerationDigest,
    PublishedAtMs,
    ExpiresAtMs,
}

#[derive(DeriveIden)]
enum RenderLeases {
    #[sea_orm(iden = "suprnova_render_leases")]
    Table,
    RenderKey,
    Epoch,
    LeaseId,
    ExpiresAtMs,
    NextToken,
}

#[derive(DeriveIden)]
enum LiveInstances {
    #[sea_orm(iden = "suprnova_live_instances")]
    Table,
    Scope,
    Instance,
    Record,
    Version,
    ExpiresAtMs,
}

#[derive(DeriveIden)]
enum LivePromotions {
    #[sea_orm(iden = "suprnova_live_promotions")]
    Table,
    Scope,
    Idempotency,
    Record,
    ExpiresAtMs,
}
