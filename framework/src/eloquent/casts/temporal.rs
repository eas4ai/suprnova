//! Temporal casts - dates, datetimes, immutable variants, and
//! Unix-epoch timestamps.
//!
//! The default temporals store as `TEXT` so the round-trip is
//! backend-agnostic: every backend stores and returns the RFC 3339
//! string unchanged. That text belongs in a text column. A text
//! parameter reaches a native date-time column only on MySQL; Postgres
//! refuses to bind text to `timestamp` or `timestamp with time zone`.
//!
//! ## Native columns
//!
//! [`AsNativeDateTime`] and [`AsNaiveDateTime`] (and their `Optional`
//! forms) store a `DateTime<Utc>` in a native column instead, so the
//! database's own date functions work on it:
//!
//! - `AsNativeDateTime` keeps the zone: `timestamp with time zone` on
//!   Postgres, `TIMESTAMP` or `DATETIME` on MySQL, text on SQLite. The
//!   schema builder's `timestamp_tz`, `native_timestamps` and
//!   `native_soft_deletes` create such columns.
//! - `AsNaiveDateTime` stores the UTC wall clock in a column without a
//!   zone: `timestamp` on Postgres, `DATETIME` or `TIMESTAMP` on MySQL,
//!   text on SQLite. It is the cast for a table Laravel's `timestamps()`
//!   created on Postgres, which the zone-aware cast cannot read.
//!
//! A cast names its storage type, and the database driver checks it
//! against the column, so pick the one that matches the column.
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

// ---- AsNativeDateTime -------------------------------------------------------

/// Cast `chrono::DateTime<Utc>` ↔ a native date-time column that keeps
/// the zone: `timestamp with time zone` on Postgres, `TIMESTAMP` or
/// `DATETIME` on MySQL, text on SQLite.
///
/// A `DateTime<Utc>` field defaults to the text [`AsDateTime`], so
/// declare this one per field:
/// `#[model(casts = { created_at = AsNativeDateTime, updated_at = AsNativeDateTime })]`.
/// The model's automatic timestamps, `touch`, soft deletes and the
/// touch of an owner all store through the declared cast.
pub struct AsNativeDateTime;

impl Cast for AsNativeDateTime {
    type Runtime = DateTime<Utc>;
    type Storage = DateTime<Utc>;

    fn to_storage(v: &DateTime<Utc>) -> Result<DateTime<Utc>, FrameworkError> {
        Ok(*v)
    }

    fn from_storage(s: &DateTime<Utc>) -> Result<DateTime<Utc>, FrameworkError> {
        Ok(*s)
    }
}

impl IntoDynCast for AsNativeDateTime {
    fn into_dyn() -> Box<dyn DynCast> {
        Box::new(NativeDateTimeDyn("AsNativeDateTime"))
    }
}

// ---- AsOptionalNativeDateTime -------------------------------------------------

/// The nullable form of [`AsNativeDateTime`], for `deleted_at` and any
/// other `Option<DateTime<Utc>>` field in a native column.
pub struct AsOptionalNativeDateTime;

impl Cast for AsOptionalNativeDateTime {
    type Runtime = Option<DateTime<Utc>>;
    type Storage = Option<DateTime<Utc>>;

    fn to_storage(v: &Option<DateTime<Utc>>) -> Result<Option<DateTime<Utc>>, FrameworkError> {
        Ok(*v)
    }

    fn from_storage(s: &Option<DateTime<Utc>>) -> Result<Option<DateTime<Utc>>, FrameworkError> {
        Ok(*s)
    }
}

impl IntoDynCast for AsOptionalNativeDateTime {
    fn into_dyn() -> Box<dyn DynCast> {
        Box::new(OptionalNativeDateTimeDyn("AsOptionalNativeDateTime"))
    }
}

// ---- AsNaiveDateTime ----------------------------------------------------------

/// Cast `chrono::DateTime<Utc>` ↔ a native date-time column without a
/// zone, holding the UTC wall clock: `timestamp` on Postgres, `DATETIME`
/// or `TIMESTAMP` on MySQL, text on SQLite.
///
/// This is the shape Laravel's `timestamps()` creates on Postgres. The
/// Postgres driver will not read such a column as a zone-aware value,
/// so [`AsNativeDateTime`] cannot serve it. Declare it per field, like
/// [`AsNativeDateTime`].
pub struct AsNaiveDateTime;

impl Cast for AsNaiveDateTime {
    type Runtime = DateTime<Utc>;
    type Storage = NaiveDateTime;

    fn to_storage(v: &DateTime<Utc>) -> Result<NaiveDateTime, FrameworkError> {
        Ok(v.naive_utc())
    }

    fn from_storage(s: &NaiveDateTime) -> Result<DateTime<Utc>, FrameworkError> {
        Ok(s.and_utc())
    }
}

impl IntoDynCast for AsNaiveDateTime {
    fn into_dyn() -> Box<dyn DynCast> {
        Box::new(NativeDateTimeDyn("AsNaiveDateTime"))
    }
}

// ---- AsOptionalNaiveDateTime --------------------------------------------------

/// The nullable form of [`AsNaiveDateTime`].
pub struct AsOptionalNaiveDateTime;

impl Cast for AsOptionalNaiveDateTime {
    type Runtime = Option<DateTime<Utc>>;
    type Storage = Option<NaiveDateTime>;

    fn to_storage(v: &Option<DateTime<Utc>>) -> Result<Option<NaiveDateTime>, FrameworkError> {
        Ok(v.map(|moment| moment.naive_utc()))
    }

    fn from_storage(s: &Option<NaiveDateTime>) -> Result<Option<DateTime<Utc>>, FrameworkError> {
        Ok(s.map(|moment| moment.and_utc()))
    }
}

impl IntoDynCast for AsOptionalNaiveDateTime {
    fn into_dyn() -> Box<dyn DynCast> {
        Box::new(OptionalNativeDateTimeDyn("AsOptionalNaiveDateTime"))
    }
}

/// Read a native column's JSON as a UTC moment. The row arrives as the
/// driver renders it: RFC 3339 for a zone-aware column, and a bare
/// `YYYY-MM-DD HH:MM:SS` or `YYYY-MM-DDTHH:MM:SS` for one without a zone,
/// which the naive casts store in UTC.
fn native_json_moment(cast: &'static str, raw: &str) -> Result<DateTime<Utc>, FrameworkError> {
    parse_database_datetime(raw)
        .or_else(|_| {
            NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S%.f")
                .map(|moment| moment.and_utc())
        })
        .map_err(|e| FrameworkError::validation(cast, format!("{e}")))
}

/// The erased form of the native casts, named for its errors.
struct NativeDateTimeDyn(&'static str);

impl DynCast for NativeDateTimeDyn {
    fn from_storage_json(
        &self,
        v: &serde_json::Value,
    ) -> Result<serde_json::Value, FrameworkError> {
        let raw = v.as_str().ok_or_else(|| {
            FrameworkError::validation(
                self.0,
                format!("dyn from_storage: expected JSON string, got {v:?}"),
            )
        })?;
        serde_json::to_value(native_json_moment(self.0, raw)?)
            .map_err(|e| FrameworkError::internal(format!("{}: re-serialize failed: {e}", self.0)))
    }

    fn to_storage_json(&self, v: &serde_json::Value) -> Result<serde_json::Value, FrameworkError> {
        Ok(v.clone())
    }
}

/// The erased form of the nullable native casts.
struct OptionalNativeDateTimeDyn(&'static str);

impl DynCast for OptionalNativeDateTimeDyn {
    fn from_storage_json(
        &self,
        v: &serde_json::Value,
    ) -> Result<serde_json::Value, FrameworkError> {
        match v {
            serde_json::Value::Null => Ok(serde_json::Value::Null),
            serde_json::Value::String(raw) => {
                serde_json::to_value(native_json_moment(self.0, raw)?).map_err(|e| {
                    FrameworkError::internal(format!("{}: re-serialize failed: {e}", self.0))
                })
            }
            other => Err(FrameworkError::validation(
                self.0,
                format!("expected null or string, got {other:?}"),
            )),
        }
    }

    fn to_storage_json(&self, v: &serde_json::Value) -> Result<serde_json::Value, FrameworkError> {
        Ok(v.clone())
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
    use super::{
        AsDateTime, AsNaiveDateTime, AsNativeDateTime, AsOptionalDateTime, AsOptionalNaiveDateTime,
        AsOptionalNativeDateTime, Cast, IntoDynCast,
    };
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    #[test]
    fn the_naive_casts_store_the_utc_wall_clock() {
        let moment = Utc.with_ymd_and_hms(2031, 3, 14, 9, 30, 0).unwrap();
        let stored = AsNaiveDateTime::to_storage(&moment).unwrap();
        assert_eq!(stored.to_string(), "2031-03-14 09:30:00");
        assert_eq!(AsNaiveDateTime::from_storage(&stored).unwrap(), moment);
        assert_eq!(
            AsOptionalNaiveDateTime::to_storage(&Some(moment)).unwrap(),
            Some(stored)
        );
        assert_eq!(AsOptionalNaiveDateTime::to_storage(&None).unwrap(), None);
        assert_eq!(AsNativeDateTime::to_storage(&moment).unwrap(), moment);
        assert_eq!(AsOptionalNativeDateTime::from_storage(&None).unwrap(), None);
    }

    /// `with_casts` reads a row as JSON, where a zone-aware column arrives as
    /// RFC 3339 and one without a zone as a bare date and time.
    #[test]
    fn the_native_dyn_casts_read_both_column_shapes_as_utc() {
        let want = json!("2031-03-14T09:30:00Z");
        for cast in [AsNativeDateTime::into_dyn(), AsNaiveDateTime::into_dyn()] {
            for stored in [
                json!("2031-03-14T09:30:00+00:00"),
                json!("2031-03-14T11:30:00+02:00"),
                json!("2031-03-14T09:30:00"),
                json!("2031-03-14 09:30:00"),
            ] {
                assert_eq!(cast.from_storage_json(&stored).unwrap(), want, "{stored}");
            }
            assert!(cast.from_storage_json(&json!(1)).is_err());
            assert!(cast.from_storage_json(&json!("soon")).is_err());
        }
        for cast in [
            AsOptionalNativeDateTime::into_dyn(),
            AsOptionalNaiveDateTime::into_dyn(),
        ] {
            assert_eq!(cast.from_storage_json(&json!(null)).unwrap(), json!(null));
            assert_eq!(
                cast.from_storage_json(&json!("2031-03-14 09:30:00"))
                    .unwrap(),
                want
            );
            assert!(cast.from_storage_json(&json!(true)).is_err());
        }
    }

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
