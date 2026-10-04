//! Models and migrations an application ported from Laravel would have,
//! compiled under this package's `[package.metadata.suprnova]` settings.
//!
//! `datetime_cast = "native"` gives every `DateTime<Utc>` field without a
//! cast of its own `AsNativeDateTime`, the managed timestamps included;
//! `ProbePost::archived_at` names a text cast, which wins. The migrations
//! use `id()` and `foreign_id()`, which `unsigned_ids = true` makes
//! unsigned on MySQL once a binary installs it. The binaries of this
//! package run them; `tests/settings.rs` reads what they created.

use sea_orm_migration::prelude::*;
use suprnova::chrono::{DateTime, Utc};
use suprnova::model;
use suprnova::schema::Schema;

/// A users table with native timestamps and a key Laravel's `id()` makes.
#[model(table = "probe_users", fillable = ["name"], relations = {
    posts: HasMany<ProbePost>,
})]
pub struct ProbeUser {
    pub id: u64,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A soft-deleting post that touches its user. Every date-time field takes
/// the package's `datetime_cast` except `archived_at`, whose own cast wins:
/// its column is text.
#[model(
    table = "probe_posts",
    soft_deletes,
    fillable = ["probe_user_id", "title", "archived_at"],
    touches = ["user"],
    relations = {
        user: BelongsTo<ProbeUser> { fk = "probe_user_id" },
    },
    casts = {
        archived_at = suprnova::AsOptionalDateTime,
    },
)]
pub struct ProbePost {
    pub id: u64,
    pub probe_user_id: u64,
    pub title: String,
    pub archived_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// The tables the models above read, made with the schema builder.
#[derive(DeriveMigrationName)]
pub struct CreateProbeTables;

#[async_trait::async_trait]
impl MigrationTrait for CreateProbeTables {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        Schema::create(manager, "probe_users", |t| {
            t.id();
            t.string("name");
            t.timestamps_tz();
        })
        .await?;
        // MySQL refuses a foreign key whose sign differs from the key it
        // references, so this table also fails when the setting reaches
        // `id()` and not `foreign_id()`.
        Schema::create(manager, "probe_posts", |t| {
            t.id();
            t.foreign_id("probe_user_id").constrained("probe_users");
            t.string("title");
            t.string("archived_at").nullable();
            t.timestamps_tz();
            t.soft_deletes_tz();
        })
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        Schema::drop_if_exists(manager, "probe_posts").await?;
        Schema::drop_if_exists(manager, "probe_users").await
    }
}

/// This package's migrations.
pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(CreateProbeTables)]
    }
}

/// Runs every migration against the database at `url`, as an
/// application's `migrate` command would.
pub async fn migrate(url: &str) -> Result<(), DbErr> {
    let connection = suprnova::sea_orm::Database::connect(url).await?;
    Migrator::up(&connection, None).await
}

/// The URL the binaries migrate, from `PROBE_DATABASE_URL`.
pub fn database_url() -> Result<String, String> {
    std::env::var("PROBE_DATABASE_URL")
        .map_err(|_| "set PROBE_DATABASE_URL to the database to migrate".to_string())
}

/// Migrates the database `PROBE_DATABASE_URL` names; the binaries' body.
pub async fn run() -> std::process::ExitCode {
    let outcome = match database_url() {
        Ok(url) => migrate(&url).await.map_err(|error| error.to_string()),
        Err(problem) => Err(problem),
    };
    match outcome {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(problem) => {
            eprintln!("probe migration failed: {problem}");
            std::process::ExitCode::FAILURE
        }
    }
}
