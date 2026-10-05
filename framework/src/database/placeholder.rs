//! Backend-aware positional placeholder rendering for hand-written SQL.
//!
//! SeaORM's query builder picks the right dialect on its own, but the
//! subsystems that compose SQL by hand - the queue driver, the failed-jobs
//! store, the notification store, the pivot-table relation helpers - have to
//! choose the convention themselves. SQLite and MySQL take `?` in every
//! position; Postgres takes ordinals (`$1`, `$2`, …) and rejects a bare `?`
//! as a syntax error at parse time, before a single row is touched.
//!
//! Hard-coding `?` therefore isn't a portability wart, it's an outage on one
//! of the three first-class backends, so every hand-written statement routes
//! its placeholders through here rather than embedding a literal.

use crate::error::FrameworkError;
use sea_orm::DatabaseBackend;

/// Render the `n`-th (1-based) positional placeholder for `backend`.
///
/// `n` is ignored outside Postgres because `?` carries no ordinal - callers
/// still pass it so the same numbering logic reads identically at every site.
pub(crate) fn placeholder(backend: DatabaseBackend, n: usize) -> Result<String, FrameworkError> {
    match backend {
        DatabaseBackend::Postgres => Ok(format!("${n}")),
        DatabaseBackend::MySql | DatabaseBackend::Sqlite => Ok("?".to_string()),
        _ => Err(super::unsupported_database_backend(backend)),
    }
}

/// The placeholder for parameter `n` holding `value`.
///
/// On Postgres a `numeric` parameter is cast in the SQL text. sqlx keeps a
/// prepared statement per SQL text, with the parameter types of its first
/// use, and a comparison such as `"id" = $1` usually binds an integer
/// there; a `numeric` sent to that statement later fails with "incorrect
/// binary data format". The cast gives the `numeric` form a text of its
/// own. A `u64` above `i64::MAX` binds as `numeric` on Postgres when the
/// column's type is unknown, which is the case that needs it.
pub(crate) fn typed_placeholder(
    backend: DatabaseBackend,
    n: usize,
    value: &sea_orm::Value,
) -> Result<String, FrameworkError> {
    let ph = placeholder(backend, n)?;
    Ok(match (backend, value) {
        (DatabaseBackend::Postgres, sea_orm::Value::Decimal(_)) => format!("CAST({ph} AS NUMERIC)"),
        _ => ph,
    })
}

/// Render `count` consecutive placeholders starting at ordinal `start`,
/// comma-joined for direct interpolation into a `VALUES (…)` or `IN (…)`
/// list.
///
/// Ordinals are threaded rather than restarted: a clause that follows other
/// binds (the queue filter's `IN` list sits behind two timestamp binds) must
/// continue the statement's numbering or Postgres silently reads the wrong
/// parameter.
pub(crate) fn placeholder_list(
    backend: DatabaseBackend,
    start: usize,
    count: usize,
) -> Result<String, FrameworkError> {
    (start..start + count)
        .map(|n| placeholder(backend, n))
        .collect::<Result<Vec<_>, _>>()
        .map(|values| values.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postgres_numbers_ordinals_and_others_stay_positional() {
        assert_eq!(placeholder(DatabaseBackend::Postgres, 3).unwrap(), "$3");
        assert_eq!(placeholder(DatabaseBackend::Sqlite, 3).unwrap(), "?");
        assert_eq!(placeholder(DatabaseBackend::MySql, 3).unwrap(), "?");
    }

    #[test]
    fn lists_continue_the_statement_numbering() {
        assert_eq!(
            placeholder_list(DatabaseBackend::Postgres, 3, 3).unwrap(),
            "$3, $4, $5"
        );
        assert_eq!(
            placeholder_list(DatabaseBackend::Sqlite, 3, 3).unwrap(),
            "?, ?, ?"
        );
    }

    #[test]
    fn an_empty_list_renders_nothing() {
        assert_eq!(
            placeholder_list(DatabaseBackend::Postgres, 1, 0).unwrap(),
            ""
        );
        assert_eq!(placeholder_list(DatabaseBackend::Sqlite, 1, 0).unwrap(), "");
    }
}
