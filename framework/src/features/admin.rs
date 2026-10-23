//! Admin CRUD for the `features` table, which is laravel/pennant's.
//!
//! Intended to back Phase 8's admin panel + any custom admin UI a
//! consumer builds. All mutations fire the appropriate event
//! ([`FeatureUpdated`] / [`FeatureDeleted`]) for audit listeners and
//! also call [`crate::features::sync::notify`] so live in-process
//! evaluators (DB snapshot + caches) reflect the new state before the
//! mutation returns. Bind a
//! [`FeatureSync`](crate::features::FeatureSync) into the App
//! container (the default is the composite produced by
//! [`crate::features::bootstrap_database_cached`]) to enable
//! sub-second propagation; without it `notify` is a no-op and the
//! row reaches readers only after a manual reload or TTL.

use crate::database::DB;
use crate::error::FrameworkError;
use crate::features::events::{FeatureDeleted, FeatureUpdated};
use crate::features::store;
use serde::Serialize;

/// One flag, projected for admin consumers: the row in Pennant's
/// `features` table, in the framework's terms, with the description and
/// actor the framework keeps beside it.
#[derive(Debug, Clone, Serialize)]
pub struct FeatureRow {
    /// Primary key from the `features` table.
    pub id: i64,
    /// Flag identifier (e.g. `"checkout.v2"`).
    pub name: String,
    /// Scope discriminator; empty string for the global default, otherwise `kind:identifier`.
    pub scope_key: String,
    /// Whether the flag resolves to enabled for this `(name, scope_key)`
    /// pair: any stored value other than `false`, as Pennant reads it.
    pub enabled: bool,
    /// Operator-facing description shown in the admin UI.
    pub description: Option<String>,
    /// Opaque identifier of the user who last toggled the flag, or `None` for system changes.
    pub updated_by: Option<String>,
    /// Timestamp at which the row was first inserted. Pennant's table
    /// allows `NULL` here.
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Timestamp at which the row was last mutated. Pennant's table
    /// allows `NULL` here.
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

async fn row_of(
    conn: &sea_orm::DatabaseConnection,
    flag: store::StoredFlag,
) -> Result<FeatureRow, FrameworkError> {
    let (description, updated_by) = store::details(conn, &flag.name, &flag.scope_key).await?;
    Ok(FeatureRow {
        id: flag.id,
        name: flag.name,
        scope_key: flag.scope_key,
        enabled: flag.enabled,
        description,
        updated_by,
        created_at: flag.created_at,
        updated_at: flag.updated_at,
    })
}

/// List every flag row, sorted by name, then by scope key, the global
/// `""` first. Use this to populate the admin panel's "all flags" table.
///
/// The sort runs here rather than in SQL: the stored scopes are Pennant's
/// (`__laravel_null`, `App\Models\User|42`), whose order differs from the
/// scope keys' and from one database collation to another.
pub async fn list() -> Result<Vec<FeatureRow>, FrameworkError> {
    let db = DB::connection()?;
    let mut rows = Vec::new();
    for flag in store::all(db.inner()).await? {
        rows.push(row_of(db.inner(), flag).await?);
    }
    rows.sort_by(|a, b| (&a.name, &a.scope_key).cmp(&(&b.name, &b.scope_key)));
    Ok(rows)
}

/// Fetch one flag row by name + scope_key. Returns `None` when the
/// row isn't present.
pub async fn get(name: &str, scope_key: &str) -> Result<Option<FeatureRow>, FrameworkError> {
    let db = DB::connection()?;
    match store::find(db.inner(), name, scope_key).await? {
        Some(flag) => Ok(Some(row_of(db.inner(), flag).await?)),
        None => Ok(None),
    }
}

/// Create or update a flag. `scope_key = ""` means a global flag;
/// non-empty values like `"user:42"` or `"team:staff"` create a
/// scoped override. Fires [`FeatureUpdated`] after the row is
/// persisted.
///
/// `actor_id` is the user id of the operator performing the change.
/// `None` denotes a system-initiated change (CLI bootstrap, seed,
/// migration).
///
/// Returns the resulting row so callers can re-render the admin
/// list without a follow-up `get`.
pub async fn upsert(
    name: &str,
    scope_key: &str,
    enabled: bool,
    description: Option<String>,
    actor_id: Option<String>,
) -> Result<FeatureRow, FrameworkError> {
    let db = DB::connection()?;
    store::upsert(db.inner(), name, scope_key, enabled).await?;
    store::upsert_details(
        db.inner(),
        name,
        scope_key,
        description.clone(),
        actor_id.clone(),
    )
    .await?;

    // Re-fetch to return the canonical row (especially the id +
    // created_at, which the insert above doesn't surface).
    let row = get(name, scope_key)
        .await?
        .ok_or_else(|| FrameworkError::internal("features upsert: row missing after insert"))?;

    // Propagate to live evaluators *before* returning - kill-switch
    // semantics require the in-memory snapshot + any cache to reflect
    // the new row by the time the admin call comes back. No-op when
    // no `FeatureSync` is bound.
    crate::features::sync::notify(name, scope_key).await;

    // Fire-and-forget audit event. A listener panic does not roll
    // back the committed insert.
    let _ = crate::events::EventFacade::dispatch(FeatureUpdated {
        name: name.to_string(),
        scope_key: scope_key.to_string(),
        enabled,
        actor_id,
    })
    .await;

    Ok(row)
}

/// Delete a flag row by name + scope_key. Returns `true` when a row
/// was actually removed, `false` when none matched (so admin UIs can
/// distinguish "deleted X" from "nothing to delete"). Fires
/// [`FeatureDeleted`] only on a real deletion.
pub async fn delete(
    name: &str,
    scope_key: &str,
    actor_id: Option<String>,
) -> Result<bool, FrameworkError> {
    let db = DB::connection()?;
    let deleted = store::delete(db.inner(), name, scope_key).await?;
    if deleted {
        // Propagate the deletion: DB snapshots drop the row, caches
        // drop matching entries. After this, `is_enabled!` falls back
        // to the compile-time default for the flag.
        crate::features::sync::notify(name, scope_key).await;

        let _ = crate::events::EventFacade::dispatch(FeatureDeleted {
            name: name.to_string(),
            scope_key: scope_key.to_string(),
            actor_id,
        })
        .await;
    }
    Ok(deleted)
}
