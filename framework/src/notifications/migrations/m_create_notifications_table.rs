//! Migration that creates the `notifications` table that
//! [`crate::notifications::channels::database::DatabaseChannel`] writes
//! and the read helpers in [`crate::notifications`] query - see
//! [`crate::notifications::migrations`] for the convention.
//!
//! The table takes the layout Laravel 13's `make:notifications-table`
//! migration creates, so a Suprnova application and a Laravel one read
//! each other's notifications:
//!
//! - `id` is a UUID: `uuid` on Postgres, `CHAR(36)` elsewhere.
//! - `type` and `notifiable_type` are `VARCHAR(255)`.
//! - `notifiable_id` follows `NOTIFICATIONS_MORPH_KEY`, as Laravel's
//!   `morphs` follows its default morph key type: `int` (the default) for
//!   a big integer (unsigned on MySQL), `uuid` for the column
//!   `uuidMorphs` creates, `ulid` for the one `ulidMorphs` creates. The
//!   pair has Laravel's index.
//! - `data` is `TEXT` holding the JSON payload.
//! - `read_at`, `created_at` and `updated_at` are nullable. They are
//!   `DATETIME` on MySQL, where Laravel's migration says `TIMESTAMP`, the
//!   framework's rule for a time column: `TIMESTAMP` refuses any time after
//!   2038-01-19.
//!
//! A `notifications` table that already exists is left exactly as it is:
//! one Laravel created stays as Laravel left it, and one an earlier
//! version of this migration created is reshaped by
//! [`NotificationsToLaravelLayout`](super::NotificationsToLaravelLayout).

use sea_orm_migration::prelude::*;

use crate::schema::Schema;

/// Migration that creates the `notifications` table.
pub struct Migration;

impl MigrationName for Migration {
    // Explicit, date-prefixed name, as the workflow and features migrations
    // use. `DeriveMigrationName` would derive it from the module path,
    // which a later framework migration could collide with.
    fn name(&self) -> &str {
        "m20260516_000001_create_notifications_table"
    }
}

/// The key type of the notifiable models, from `NOTIFICATIONS_MORPH_KEY`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MorphKey {
    /// `int`: Laravel's `morphs`.
    Int,
    /// `uuid`: Laravel's `uuidMorphs`.
    Uuid,
    /// `ulid`: Laravel's `ulidMorphs`.
    Ulid,
}

/// Read `NOTIFICATIONS_MORPH_KEY`: `int` when unset.
///
/// # Errors
///
/// Returns [`DbErr`] naming the variable when it holds anything else.
pub(crate) fn morph_key() -> Result<MorphKey, DbErr> {
    match std::env::var("NOTIFICATIONS_MORPH_KEY") {
        Err(_) => Ok(MorphKey::Int),
        Ok(raw) => match raw.trim().to_ascii_lowercase().as_str() {
            "" | "int" => Ok(MorphKey::Int),
            "uuid" => Ok(MorphKey::Uuid),
            "ulid" => Ok(MorphKey::Ulid),
            _ => Err(DbErr::Migration(format!(
                "NOTIFICATIONS_MORPH_KEY={raw:?} is not one of int, uuid or ulid"
            ))),
        },
    }
}

/// Create `notifications` in Laravel's layout, with `notifiable_id` for
/// `key`, or add the indexes a stopped upgrade left out of the one it
/// created (see `Schema::create_or_complete`). Callers reach it only when
/// the table is missing or the upgrade created it.
pub(crate) async fn create_table(manager: &SchemaManager<'_>, key: MorphKey) -> Result<(), DbErr> {
    Schema::create_or_complete(manager, "notifications", |t| {
        t.uuid("id").primary();
        t.string("type");
        t.string("notifiable_type");
        match key {
            MorphKey::Int => t.unsigned_big_integer("notifiable_id"),
            MorphKey::Uuid => t.uuid("notifiable_id"),
            MorphKey::Ulid => t.ulid("notifiable_id"),
        };
        t.index(&["notifiable_type", "notifiable_id"]);
        t.text("data");
        t.date_time("read_at").precision(0).nullable();
        t.date_time("created_at").precision(0).nullable();
        t.date_time("updated_at").precision(0).nullable();
    })
    .await
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if manager.has_table("notifications").await? {
            return Ok(());
        }
        create_table(manager, morph_key()?).await
    }

    /// Leaves the table. A table Laravel created looks the same as the one
    /// this migration creates, and rolling back must not drop Laravel's
    /// notifications with it.
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
