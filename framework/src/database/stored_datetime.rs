//! Date-time columns whose SQL type differs by engine and by schema age.
//!
//! The framework's stores keep UTC wall-clock times, but the migrations
//! that create their tables do not agree on one column type. `.timestamp()`
//! is `TIMESTAMP` on MySQL and MariaDB and `timestamp` on Postgres,
//! `.date_time()` is `DATETIME` on MySQL, and `.timestamp_with_time_zone()`
//! is `timestamptz` on Postgres. The SQL drivers type-check every decode:
//! MySQL decodes a `NaiveDateTime` from `DATETIME` only, and Postgres from
//! `timestamp` only. A store that named one Rust type failed on the other
//! column type, which is how every session read failed on a scaffolded
//! MySQL app, whose sessions table is `TIMESTAMP`.
//!
//! Writes need no conversion: a bound `NaiveDateTime` is stored as the
//! same UTC wall clock in each of these column types, because sqlx runs
//! every MySQL session at `time_zone = '+00:00'` and every Postgres session
//! at `TimeZone = UTC`. The one exception is the range limit of MySQL's
//! `TIMESTAMP`, which [`storable_expiry`] handles.

use chrono::{DateTime, NaiveDateTime, Utc};
use sea_orm::sea_query::{ArrayType, ColumnType, Nullable, ValueType, ValueTypeErr};
use sea_orm::{ColIdx, DatabaseBackend, QueryResult, TryGetError, TryGetable, Value};

/// A UTC wall-clock time stored in a date-time column of any type: the
/// field type of the framework's own entities for the session,
/// remember-me, auth-flow token and ceremony tables.
///
/// The decode tries `NaiveDateTime` first, the type every store writes,
/// then `DateTime<Utc>`, which MySQL decodes from `TIMESTAMP` and Postgres
/// from `timestamptz`. Both sessions run in UTC (see the module docs), so
/// the second path reads the same wall clock the store wrote. Written back,
/// it binds as a `NaiveDateTime`, which every one of those column types
/// stores as the same UTC wall clock.
///
/// A `NaiveDateTime` field could not be read whole from a `TIMESTAMP` or
/// `timestamptz` column, the types older scaffolds created; this type
/// reads both shapes, so a whole-row read through the entity works on
/// every table those migrations made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StoredDateTime(pub(crate) NaiveDateTime);

impl StoredDateTime {
    /// The stored time as the UTC wall clock.
    pub fn naive_utc(self) -> NaiveDateTime {
        self.0
    }

    /// The stored time in UTC.
    pub fn and_utc(self) -> DateTime<Utc> {
        self.0.and_utc()
    }
}

impl From<NaiveDateTime> for StoredDateTime {
    fn from(value: NaiveDateTime) -> Self {
        Self(value)
    }
}

impl From<DateTime<Utc>> for StoredDateTime {
    fn from(value: DateTime<Utc>) -> Self {
        Self(value.naive_utc())
    }
}

impl From<StoredDateTime> for NaiveDateTime {
    fn from(value: StoredDateTime) -> Self {
        value.0
    }
}

impl From<StoredDateTime> for DateTime<Utc> {
    fn from(value: StoredDateTime) -> Self {
        value.and_utc()
    }
}

impl From<StoredDateTime> for Value {
    fn from(value: StoredDateTime) -> Self {
        Value::ChronoDateTime(Some(value.0))
    }
}

impl Nullable for StoredDateTime {
    fn null() -> Value {
        Value::ChronoDateTime(None)
    }
}

impl ValueType for StoredDateTime {
    fn try_from(value: Value) -> Result<Self, ValueTypeErr> {
        match value {
            Value::ChronoDateTime(Some(at)) => Ok(Self(at)),
            Value::ChronoDateTimeUtc(Some(at)) => Ok(Self(at.naive_utc())),
            _ => Err(ValueTypeErr),
        }
    }

    fn type_name() -> String {
        "StoredDateTime".to_owned()
    }

    fn array_type() -> ArrayType {
        ArrayType::ChronoDateTime
    }

    fn column_type() -> ColumnType {
        ColumnType::DateTime
    }
}

impl TryGetable for StoredDateTime {
    fn try_get_by<I: ColIdx>(res: &QueryResult, index: I) -> Result<Self, TryGetError> {
        match <NaiveDateTime as TryGetable>::try_get_by(res, index) {
            Ok(value) => Ok(Self(value)),
            // NULL is NULL in every column type; `Option<StoredDateTime>`
            // turns it into `None`.
            Err(TryGetError::Null(column)) => Err(TryGetError::Null(column)),
            // When neither type decodes, the first error names the type the
            // store writes, which is the more useful report.
            Err(naive_error) => <DateTime<Utc> as TryGetable>::try_get_by(res, index)
                .map(|value| Self(value.naive_utc()))
                .map_err(|_| naive_error),
        }
    }
}

/// `at`, moved back to the latest moment a MySQL `TIMESTAMP` column can
/// store when `backend` is MySQL.
///
/// MySQL refuses to store a `TIMESTAMP` after 2038-01-19 03:14:07 UTC,
/// while `DATETIME` takes it. A store cannot tell which one its table has,
/// so an expiry past that moment is written as that moment. Expiring early
/// is the safe direction for a credential. MariaDB 11.5 and later store
/// later moments too, but the backend does not tell the two servers apart.
/// Postgres and SQLite need no limit.
pub(crate) fn storable_expiry(backend: DatabaseBackend, at: NaiveDateTime) -> NaiveDateTime {
    if backend != DatabaseBackend::MySql {
        return at;
    }
    // A `TIMESTAMP` is a signed 32-bit count of seconds since the epoch.
    match DateTime::from_timestamp(i64::from(i32::MAX), 0) {
        Some(latest) => Ord::min(at, latest.naive_utc()),
        None => at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use sea_orm::{ConnectionTrait, Database, Statement};

    fn at(year: i32, month: u32, day: u32, h: u32, m: u32, s: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(year, month, day)
            .and_then(|date| date.and_hms_opt(h, m, s))
            .expect("valid test date")
    }

    #[test]
    fn a_mysql_expiry_past_the_timestamp_range_is_written_at_its_end() {
        assert_eq!(
            storable_expiry(DatabaseBackend::MySql, at(2046, 5, 1, 0, 0, 0)),
            at(2038, 1, 19, 3, 14, 7)
        );
    }

    #[test]
    fn an_expiry_inside_the_range_or_on_another_engine_is_kept() {
        let near = at(2030, 1, 1, 12, 0, 0);
        assert_eq!(storable_expiry(DatabaseBackend::MySql, near), near);
        let far = at(2046, 5, 1, 0, 0, 0);
        assert_eq!(storable_expiry(DatabaseBackend::Postgres, far), far);
        assert_eq!(storable_expiry(DatabaseBackend::Sqlite, far), far);
    }

    #[test]
    fn it_binds_as_the_utc_wall_clock_and_converts_back() {
        let wall = at(2026, 10, 4, 12, 30, 5);
        let stored = StoredDateTime::from(wall);
        assert_eq!(Value::from(stored), Value::ChronoDateTime(Some(wall)));
        assert_eq!(
            <StoredDateTime as Nullable>::null(),
            Value::ChronoDateTime(None)
        );
        assert_eq!(
            <StoredDateTime as ValueType>::try_from(Value::ChronoDateTime(Some(wall))).ok(),
            Some(stored)
        );
        assert_eq!(
            <StoredDateTime as ValueType>::try_from(Value::ChronoDateTimeUtc(Some(wall.and_utc())))
                .ok(),
            Some(stored)
        );
        assert!(
            <StoredDateTime as ValueType>::try_from(Value::String(Some("2026".into()))).is_err()
        );
        assert_eq!(stored.naive_utc(), wall);
        assert_eq!(DateTime::<Utc>::from(stored), wall.and_utc());
        assert_eq!(StoredDateTime::from(wall.and_utc()), stored);
    }

    #[tokio::test]
    async fn a_text_time_reads_and_null_reads_as_none() {
        let db = Database::connect("sqlite::memory:").await.expect("sqlite");
        let row = db
            .query_one_raw(Statement::from_string(
                DatabaseBackend::Sqlite,
                "SELECT '2026-10-04 12:30:05' AS at, NULL AS never",
            ))
            .await
            .expect("query")
            .expect("row");
        let read: StoredDateTime = row.try_get("", "at").expect("text time decodes");
        assert_eq!(read.0, at(2026, 10, 4, 12, 30, 5));
        let never: Option<StoredDateTime> = row.try_get("", "never").expect("NULL decodes");
        assert_eq!(never, None);
    }
}
