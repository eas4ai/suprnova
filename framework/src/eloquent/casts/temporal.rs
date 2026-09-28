//! Temporal casts - dates, datetimes, immutable variants, and
//! Unix-epoch timestamps.
//!
//! All non-timestamp temporals store as `TEXT` so the round-trip is
//! backend-agnostic - SQLite stores datetimes as strings natively
//! and Postgres / MySQL accept ISO-8601 / RFC-3339 strings transparently
//! through SeaORM's `Value::String` boundary.
//!
//! ## Immutable variants
//!
//! `AsImmutableDate` / `AsImmutableDateTime` are identical to their
//! mutable counterparts on the storage side; they exist for parity
//! with Laravel's `immutable_date` / `immutable_datetime` casts where
//! the runtime side returns a non-mutating wrapper. Rust's
//! borrow-checker already enforces immutability through `&` references,
//! so the two variants share underlying `chrono` types - the cast
//! names are documentation about user intent.
//!
//! ## AsTimestamp
//!
//! Stores as `INTEGER` (Unix epoch seconds). Distinct from
//! `AsDateTime` (TEXT, RFC-3339) - pick `AsTimestamp` when the column
//! is queried as a numeric range or used in arithmetic.

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};

use super::{Cast, DynCast, IntoDynCast};
use crate::error::FrameworkError;

// ---- AsDate ---------------------------------------------------------------

/// Cast `chrono::NaiveDate` ↔ `TEXT` (`YYYY-MM-DD`).
pub struct AsDate;

impl Cast for AsDate {
    type Runtime = NaiveDate;
    type Storage = String;

    fn to_storage(v: &NaiveDate) -> Result<String, FrameworkError> {
        Ok(v.to_string())
    }

    fn from_storage(s: &String) -> Result<NaiveDate, FrameworkError> {
        s.parse::<NaiveDate>()
            .map_err(|e| FrameworkError::validation("AsDate", format!("{e}")))
    }
}

struct AsDateDyn;

impl DynCast for AsDateDyn {
    fn from_storage_json(
        &self,
        v: &serde_json::Value,
    ) -> Result<serde_json::Value, FrameworkError> {
        // Domain 7 audit D7-A - was `v.as_str().unwrap_or("")` which
        // silently coerced non-strings to "" and produced a cryptic
        // chrono parse-error instead of an explicit "expected JSON
        // string, got <actual>" diagnostic.
        let s = v
            .as_str()
            .ok_or_else(|| {
                FrameworkError::validation(
                    "AsDate",
                    format!("dyn from_storage: expected JSON string, got {v:?}"),
                )
            })?
            .to_string();
        let d = AsDate::from_storage(&s)?;
        serde_json::to_value(d)
            .map_err(|e| FrameworkError::internal(format!("AsDate: re-serialize failed: {e}")))
    }

    fn to_storage_json(&self, v: &serde_json::Value) -> Result<serde_json::Value, FrameworkError> {
        Ok(v.clone())
    }
}

impl IntoDynCast for AsDate {
    fn into_dyn() -> Box<dyn DynCast> {
        Box::new(AsDateDyn)
    }
}

// ---- AsDateTime -----------------------------------------------------------

/// Cast `chrono::DateTime<Utc>` ↔ `TEXT` (RFC-3339 / ISO-8601).
pub struct AsDateTime;

impl Cast for AsDateTime {
    type Runtime = DateTime<Utc>;
    type Storage = String;

    fn to_storage(v: &DateTime<Utc>) -> Result<String, FrameworkError> {
        Ok(v.to_rfc3339())
    }

    fn from_storage(s: &String) -> Result<DateTime<Utc>, FrameworkError> {
        parse_database_datetime(s)
            .map_err(|e| FrameworkError::validation("AsDateTime", format!("{e}")))
    }
}

fn parse_database_datetime(raw: &str) -> Result<DateTime<Utc>, chrono::ParseError> {
    if let Ok(datetime) = DateTime::parse_from_rfc3339(raw) {
        return Ok(datetime.with_timezone(&Utc));
    }
    if let Ok(datetime) = DateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S%.f%#z") {
        return Ok(datetime.with_timezone(&Utc));
    }

    NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S%.f").map(|datetime| datetime.and_utc())
}

struct AsDateTimeDyn;

impl DynCast for AsDateTimeDyn {
    fn from_storage_json(
        &self,
        v: &serde_json::Value,
    ) -> Result<serde_json::Value, FrameworkError> {
        // Domain 7 audit D7-A - strict-validate the input shape.
        let s = v
            .as_str()
            .ok_or_else(|| {
                FrameworkError::validation(
                    "AsDateTime",
                    format!("dyn from_storage: expected JSON string, got {v:?}"),
                )
            })?
            .to_string();
        let dt = AsDateTime::from_storage(&s)?;
        serde_json::to_value(dt)
            .map_err(|e| FrameworkError::internal(format!("AsDateTime: re-serialize failed: {e}")))
    }

    fn to_storage_json(&self, v: &serde_json::Value) -> Result<serde_json::Value, FrameworkError> {
        Ok(v.clone())
    }
}

impl IntoDynCast for AsDateTime {
    fn into_dyn() -> Box<dyn DynCast> {
        Box::new(AsDateTimeDyn)
    }
}

// ---- AsImmutableDate ------------------------------------------------------

/// Same storage shape as [`AsDate`]; the name documents user intent
/// that the field should not be mutated in place. Rust's borrow
/// checker enforces immutability through references at compile time,
/// so the cast types are identical.
pub struct AsImmutableDate;

impl Cast for AsImmutableDate {
    type Runtime = NaiveDate;
    type Storage = String;

    fn to_storage(v: &NaiveDate) -> Result<String, FrameworkError> {
        AsDate::to_storage(v)
    }

    fn from_storage(s: &String) -> Result<NaiveDate, FrameworkError> {
        AsDate::from_storage(s)
    }
}

impl IntoDynCast for AsImmutableDate {
    fn into_dyn() -> Box<dyn DynCast> {
        // Re-uses `AsDateDyn` rather than spinning a new unit type - the
        // erased shape is identical.
        AsDate::into_dyn()
    }
}

// ---- AsImmutableDateTime --------------------------------------------------

/// Same storage shape as [`AsDateTime`]; see [`AsImmutableDate`] for
/// why this is a distinct named cast.
pub struct AsImmutableDateTime;

impl Cast for AsImmutableDateTime {
    type Runtime = DateTime<Utc>;
    type Storage = String;

    fn to_storage(v: &DateTime<Utc>) -> Result<String, FrameworkError> {
        AsDateTime::to_storage(v)
    }

    fn from_storage(s: &String) -> Result<DateTime<Utc>, FrameworkError> {
        AsDateTime::from_storage(s)
    }
}

impl IntoDynCast for AsImmutableDateTime {
    fn into_dyn() -> Box<dyn DynCast> {
        AsDateTime::into_dyn()
    }
}

// ---- AsOptionalDateTime ---------------------------------------------------

/// Cast `Option<DateTime<Utc>>` ↔ `Option<String>` (RFC-3339 / ISO-8601).
///
/// Auto-injected by the `#[suprnova::model(soft_deletes)]` flag for the
/// nullable tombstone column (`deleted_at` by default). The wrapped
/// option keeps the storage column nullable - soft-deleted vs alive
/// rows discriminate on `IS NULL` / `IS NOT NULL` without forcing a
/// sentinel value.
///
/// Hand-declare via `#[model(casts = { col = AsOptionalDateTime })]`
/// for any other nullable datetime column that should round-trip as
/// RFC-3339 text.
pub struct AsOptionalDateTime;

impl Cast for AsOptionalDateTime {
    type Runtime = Option<DateTime<Utc>>;
    type Storage = Option<String>;

    fn to_storage(v: &Option<DateTime<Utc>>) -> Result<Option<String>, FrameworkError> {
        Ok(v.as_ref().map(|dt| dt.to_rfc3339()))
    }

    fn from_storage(s: &Option<String>) -> Result<Option<DateTime<Utc>>, FrameworkError> {
        match s.as_deref() {
            None => Ok(None),
            Some(raw) => parse_database_datetime(raw)
                .map(Some)
                .map_err(|e| FrameworkError::validation("AsOptionalDateTime", format!("{e}"))),
        }
    }
}

struct AsOptionalDateTimeDyn;

impl DynCast for AsOptionalDateTimeDyn {
    fn from_storage_json(
        &self,
        v: &serde_json::Value,
    ) -> Result<serde_json::Value, FrameworkError> {
        match v {
            serde_json::Value::Null => Ok(serde_json::Value::Null),
            serde_json::Value::String(s) => {
                let dt = AsDateTime::from_storage(s)?;
                serde_json::to_value(dt).map_err(|e| {
                    FrameworkError::internal(format!(
                        "AsOptionalDateTime: re-serialize failed: {e}"
                    ))
                })
            }
            other => Err(FrameworkError::validation(
                "AsOptionalDateTime",
                format!("expected null or string, got {other:?}"),
            )),
        }
    }

    fn to_storage_json(&self, v: &serde_json::Value) -> Result<serde_json::Value, FrameworkError> {
        Ok(v.clone())
    }
}

impl IntoDynCast for AsOptionalDateTime {
    fn into_dyn() -> Box<dyn DynCast> {
        Box::new(AsOptionalDateTimeDyn)
    }
}

// ---- AsTimestamp ----------------------------------------------------------

/// Cast Unix-epoch `i64` ↔ `INTEGER`. Use when you want numeric
/// queries / arithmetic over the time column; use `AsDateTime` when
/// you want RFC-3339 strings.
pub struct AsTimestamp;

impl Cast for AsTimestamp {
    type Runtime = i64;
    type Storage = i64;

    fn to_storage(v: &i64) -> Result<i64, FrameworkError> {
        Ok(*v)
    }

    fn from_storage(s: &i64) -> Result<i64, FrameworkError> {
        Ok(*s)
    }
}

struct AsTimestampDyn;

impl DynCast for AsTimestampDyn {
    fn from_storage_json(
        &self,
        v: &serde_json::Value,
    ) -> Result<serde_json::Value, FrameworkError> {
        Ok(v.clone())
    }

    fn to_storage_json(&self, v: &serde_json::Value) -> Result<serde_json::Value, FrameworkError> {
        Ok(v.clone())
    }
}

impl IntoDynCast for AsTimestamp {
    fn into_dyn() -> Box<dyn DynCast> {
        Box::new(AsTimestampDyn)
    }
}

#[cfg(test)]
mod tests {
    use super::{AsDateTime, AsOptionalDateTime, Cast};

    #[test]
    fn datetime_accepts_postgres_current_timestamp_text() {
        let parsed = AsDateTime::from_storage(&"2026-08-16 22:19:34.912606+00".to_owned())
            .expect("PostgreSQL CURRENT_TIMESTAMP text should parse");

        assert_eq!(parsed.to_rfc3339(), "2026-08-16T22:19:34.912606+00:00");
    }

    #[test]
    fn datetime_accepts_utc_naive_database_default_text() {
        let parsed = AsDateTime::from_storage(&"2026-08-16 22:19:34".to_owned())
            .expect("UTC-naive database CURRENT_TIMESTAMP text should parse");

        assert_eq!(parsed.to_rfc3339(), "2026-08-16T22:19:34+00:00");
    }

    #[test]
    fn optional_datetime_uses_the_same_database_default_parser() {
        let parsed =
            AsOptionalDateTime::from_storage(&Some("2026-08-16 22:19:34.912606+00".to_owned()))
                .expect("optional PostgreSQL CURRENT_TIMESTAMP text should parse")
                .expect("timestamp should remain present");

        assert_eq!(parsed.to_rfc3339(), "2026-08-16T22:19:34.912606+00:00");
    }
}
