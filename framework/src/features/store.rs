//! Reads and writes of the `features` table in laravel/pennant's layout.
//!
//! The framework keys a flag by `(name, scope_key)`: `""` for the global
//! flag, `user:{id}` for a user, `team:{name}` for a team, or any string an
//! application chooses. Pennant keys it by `(name, scope)`: `__laravel_null`
//! for the global flag and `{class}|{key}` for a model. This module turns
//! one into the other at the table, so the evaluator, the admin facade and
//! the sync fan-out keep the framework's keys while the rows stay the ones
//! Pennant writes and reads:
//!
//! | framework `scope_key` | stored `scope` |
//! |---|---|
//! | `""` | `__laravel_null` |
//! | `user:42` | `App\Models\User\|42` (the user scope, see [`user_scope`]) |
//! | anything else | itself |
//!
//! `value` is JSON, as Pennant stores it. A flag is enabled for any value
//! other than `false`, as Pennant's `active()` reads it, so a rich value
//! such as `"blue"` reads as enabled. The framework writes `true` or
//! `false`.
//!
//! The admin facade's description and actor have no column in Pennant's
//! table; they live in `suprnova_feature_details`, keyed by the stored
//! `(name, scope)`.

use sea_orm::sea_query::{Alias, Expr, ExprTrait, OnConflict, Query};
use sea_orm::{ConnectionTrait, DbErr, QueryResult};

use crate::database::stored_datetime::StoredDateTime;
use crate::error::FrameworkError;

/// Pennant's table.
pub(crate) const FEATURES_TABLE: &str = "features";

/// The framework's table for what Pennant's has no column for.
pub(crate) const DETAILS_TABLE: &str = "suprnova_feature_details";

/// Pennant's scope for the global flag (`Feature::serializeScope(null)`).
pub(crate) const GLOBAL_SCOPE: &str = "__laravel_null";

/// The environment variable naming the user scope.
pub const USER_SCOPE_ENV: &str = "FEATURES_USER_SCOPE";

/// The default user scope: the class Laravel's skeleton gives its user.
pub const DEFAULT_USER_SCOPE: &str = "App\\Models\\User";

const USER_PREFIX: &str = "user:";

/// What a user's flags are stored under, before `|{id}`: Pennant stores a
/// model scope as `{class}|{key}` (or `{morph alias}|{key}` with Pennant's
/// `useMorphMap`). `FEATURES_USER_SCOPE` names it; it defaults to
/// [`DEFAULT_USER_SCOPE`], so a user's flags are the ones Pennant stores for
/// `App\Models\User`. An application whose user model has another class or
/// alias in Laravel sets it to that.
pub fn user_scope() -> String {
    std::env::var(USER_SCOPE_ENV)
        .ok()
        .filter(|scope| !scope.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_USER_SCOPE.to_owned())
}

/// The stored `scope` for a framework `scope_key`.
pub(crate) fn scope_to_stored(scope_key: &str) -> String {
    if scope_key.is_empty() {
        return GLOBAL_SCOPE.to_owned();
    }
    if let Some(id) = scope_key.strip_prefix(USER_PREFIX) {
        return format!("{}|{id}", user_scope());
    }
    scope_key.to_owned()
}

/// The framework `scope_key` for a stored `scope`.
pub(crate) fn scope_from_stored(scope: &str) -> String {
    if scope == GLOBAL_SCOPE {
        return String::new();
    }
    let user = user_scope();
    if let Some(id) = scope
        .strip_prefix(user.as_str())
        .and_then(|rest| rest.strip_prefix('|'))
    {
        return format!("{USER_PREFIX}{id}");
    }
    scope.to_owned()
}

/// Whether a stored `value` reads as enabled: anything but JSON `false`,
/// as Pennant's `active()` decides.
pub(crate) fn value_enabled(value: &str) -> bool {
    !matches!(
        serde_json::from_str::<serde_json::Value>(value),
        Ok(serde_json::Value::Bool(false))
    )
}

/// The `value` the framework stores: `true` or `false`, as Pennant does
/// for an activated or deactivated flag.
pub(crate) fn stored_value(enabled: bool) -> &'static str {
    if enabled { "true" } else { "false" }
}

/// One stored flag, in the framework's terms.
pub(crate) struct StoredFlag {
    pub(crate) id: i64,
    pub(crate) name: String,
    pub(crate) scope_key: String,
    pub(crate) enabled: bool,
    pub(crate) created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub(crate) updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn read_id(row: &QueryResult) -> Result<i64, DbErr> {
    if let Ok(id) = row.try_get::<i64>("", "id") {
        return Ok(id);
    }
    let id = row.try_get::<u64>("", "id")?;
    i64::try_from(id).map_err(|_| DbErr::Custom(format!("feature id {id} is out of range")))
}

fn decode(row: &QueryResult) -> Result<StoredFlag, DbErr> {
    let scope: String = row.try_get("", "scope")?;
    let value: String = row.try_get("", "value")?;
    Ok(StoredFlag {
        id: read_id(row)?,
        name: row.try_get("", "name")?,
        scope_key: scope_from_stored(&scope),
        enabled: value_enabled(&value),
        created_at: row
            .try_get::<Option<StoredDateTime>>("", "created_at")?
            .map(StoredDateTime::and_utc),
        updated_at: row
            .try_get::<Option<StoredDateTime>>("", "updated_at")?
            .map(StoredDateTime::and_utc),
    })
}

fn database(context: &str) -> impl Fn(DbErr) -> FrameworkError + '_ {
    move |e| FrameworkError::database(format!("{context}: {e}"))
}

/// Every flag, ordered by name then stored scope.
pub(crate) async fn all<C: ConnectionTrait>(conn: &C) -> Result<Vec<StoredFlag>, FrameworkError> {
    let select = Query::select()
        .columns(["id", "name", "scope", "value", "created_at", "updated_at"].map(Alias::new))
        .from(Alias::new(FEATURES_TABLE))
        .order_by(Alias::new("name"), sea_orm::Order::Asc)
        .order_by(Alias::new("scope"), sea_orm::Order::Asc)
        .to_owned();
    conn.query_all(&select)
        .await
        .map_err(database("features select"))?
        .iter()
        .map(|row| decode(row).map_err(database("features decode")))
        .collect()
}

/// The flag `(name, scope_key)`, if one is stored.
pub(crate) async fn find<C: ConnectionTrait>(
    conn: &C,
    name: &str,
    scope_key: &str,
) -> Result<Option<StoredFlag>, FrameworkError> {
    let select = Query::select()
        .columns(["id", "name", "scope", "value", "created_at", "updated_at"].map(Alias::new))
        .from(Alias::new(FEATURES_TABLE))
        .and_where(Expr::col(Alias::new("name")).eq(name))
        .and_where(Expr::col(Alias::new("scope")).eq(scope_to_stored(scope_key)))
        .to_owned();
    conn.query_one(&select)
        .await
        .map_err(database("features get"))?
        .map(|row| decode(&row).map_err(database("features decode")))
        .transpose()
}

/// The current time at whole seconds, the precision of Pennant's
/// timestamp columns.
fn now() -> chrono::NaiveDateTime {
    let now = crate::clock::now();
    chrono::DateTime::<chrono::Utc>::from_timestamp(now.timestamp(), 0)
        .unwrap_or(now)
        .naive_utc()
}

/// Store `enabled` for `(name, scope_key)`: insert the row Pennant would
/// insert, or update its `value` and `updated_at`.
pub(crate) async fn upsert<C: ConnectionTrait>(
    conn: &C,
    name: &str,
    scope_key: &str,
    enabled: bool,
) -> Result<(), FrameworkError> {
    let now = now();
    let mut insert = Query::insert();
    insert
        .into_table(Alias::new(FEATURES_TABLE))
        .columns(["name", "scope", "value", "created_at", "updated_at"].map(Alias::new))
        .values([
            Expr::value(name.to_owned()),
            Expr::value(scope_to_stored(scope_key)),
            Expr::value(stored_value(enabled).to_owned()),
            Expr::value(now),
            Expr::value(now),
        ])
        .map_err(|e| FrameworkError::internal(format!("features upsert statement: {e}")))?
        .on_conflict(
            OnConflict::columns([Alias::new("name"), Alias::new("scope")])
                .update_columns([Alias::new("value"), Alias::new("updated_at")])
                .to_owned(),
        );
    conn.execute(&insert)
        .await
        .map_err(database("features upsert"))?;
    Ok(())
}

/// Delete the flag `(name, scope_key)` and its details. Returns whether a
/// flag row went.
pub(crate) async fn delete<C: ConnectionTrait>(
    conn: &C,
    name: &str,
    scope_key: &str,
) -> Result<bool, FrameworkError> {
    let scope = scope_to_stored(scope_key);
    let removed = conn
        .execute(
            &Query::delete()
                .from_table(Alias::new(FEATURES_TABLE))
                .and_where(Expr::col(Alias::new("name")).eq(name))
                .and_where(Expr::col(Alias::new("scope")).eq(scope.clone()))
                .to_owned(),
        )
        .await
        .map_err(database("features delete"))?
        .rows_affected();
    conn.execute(
        &Query::delete()
            .from_table(Alias::new(DETAILS_TABLE))
            .and_where(Expr::col(Alias::new("name")).eq(name))
            .and_where(Expr::col(Alias::new("scope")).eq(scope))
            .to_owned(),
    )
    .await
    .map_err(database("feature details delete"))?;
    Ok(removed > 0)
}

/// The description and actor recorded for `(name, scope_key)`.
pub(crate) async fn details<C: ConnectionTrait>(
    conn: &C,
    name: &str,
    scope_key: &str,
) -> Result<(Option<String>, Option<String>), FrameworkError> {
    let select = Query::select()
        .columns(["description", "updated_by"].map(Alias::new))
        .from(Alias::new(DETAILS_TABLE))
        .and_where(Expr::col(Alias::new("name")).eq(name))
        .and_where(Expr::col(Alias::new("scope")).eq(scope_to_stored(scope_key)))
        .to_owned();
    let Some(row) = conn
        .query_one(&select)
        .await
        .map_err(database("feature details select"))?
    else {
        return Ok((None, None));
    };
    Ok((
        row.try_get("", "description")
            .map_err(database("feature details decode"))?,
        row.try_get("", "updated_by")
            .map_err(database("feature details decode"))?,
    ))
}

/// Record the description and actor for `(name, scope_key)`.
pub(crate) async fn upsert_details<C: ConnectionTrait>(
    conn: &C,
    name: &str,
    scope_key: &str,
    description: Option<String>,
    updated_by: Option<String>,
) -> Result<(), FrameworkError> {
    let mut insert = Query::insert();
    insert
        .into_table(Alias::new(DETAILS_TABLE))
        .columns(["name", "scope", "description", "updated_by"].map(Alias::new))
        .values([
            Expr::value(name.to_owned()),
            Expr::value(scope_to_stored(scope_key)),
            Expr::value(description),
            Expr::value(updated_by),
        ])
        .map_err(|e| FrameworkError::internal(format!("feature details statement: {e}")))?
        .on_conflict(
            OnConflict::columns([Alias::new("name"), Alias::new("scope")])
                .update_columns([Alias::new("description"), Alias::new("updated_by")])
                .to_owned(),
        );
    conn.execute(&insert)
        .await
        .map_err(database("feature details upsert"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_map_both_ways() {
        assert_eq!(scope_to_stored(""), GLOBAL_SCOPE);
        assert_eq!(scope_from_stored(GLOBAL_SCOPE), "");
        assert_eq!(
            scope_to_stored("user:42"),
            format!("{DEFAULT_USER_SCOPE}|42")
        );
        assert_eq!(scope_from_stored("App\\Models\\User|42"), "user:42");
        assert_eq!(scope_to_stored("team:staff"), "team:staff");
        assert_eq!(
            scope_from_stored("App\\Models\\Team|5"),
            "App\\Models\\Team|5"
        );
    }

    #[test]
    fn any_value_but_false_is_enabled() {
        assert!(value_enabled("true"));
        assert!(value_enabled("\"blue\""));
        assert!(value_enabled("1"));
        assert!(value_enabled("null"));
        assert!(!value_enabled("false"));
        assert!(!value_enabled(" false "));
    }
}
