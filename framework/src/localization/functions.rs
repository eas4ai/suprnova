//! The `DATETIME()` Fluent function: ICU4X-backed date/time formatting
//! callable from `.ftl` catalogs, e.g.
//! `published = Published { DATETIME($when, dateStyle: "medium") }`.
//!
//! `NUMBER()` ships with Fluent itself (`FluentBundle::add_builtins`,
//! called just before [`register`] in `fluent.rs`); `DATETIME()` is the
//! framework's own addition — upstream `fluent-bundle` has a
//! `// TODO: DATETIME()` where it would go.

use super::Lang;
use super::fluent::ConcurrentBundle;
use super::format::{DateStyle, TimeStyle};
use crate::error::FrameworkError;
use fluent_bundle::{FluentArgs, FluentValue};
use std::borrow::Cow;

/// Register `DATETIME()` on `bundle`.
///
/// Only the registration itself can fail (id already taken); the
/// function's own runtime behavior never returns `Err` because Fluent
/// functions can't propagate one — see [`datetime_function`].
pub(crate) fn register(bundle: &mut ConcurrentBundle) -> Result<(), FrameworkError> {
    bundle
        .add_function("DATETIME", datetime_function)
        .map_err(|e| {
            FrameworkError::internal(format!(
                "failed to register the DATETIME() Fluent function: {e}"
            ))
        })
}

/// The `DATETIME()` implementation.
///
/// `$value` (the first positional argument) accepts an ISO-8601 string
/// (`"2026-08-01T14:30:00"`, with or without a UTC offset, or a bare
/// date) or an epoch-milliseconds number. Named arguments `dateStyle`
/// and/or `timeStyle` take `"full"`/`"long"`/`"medium"`/`"short"` (only
/// `"medium"`/`"short"` are meaningful for `timeStyle` — see
/// [`TimeStyle`]); supplying both formats a combined date+time, either
/// alone formats just that part, and neither defaults to
/// [`DateStyle::Medium`].
///
/// Fluent function signatures can't return `Result` — a malformed or
/// unparseable `$value`, or an ICU formatting failure, can only be
/// signaled by logging (`tracing::warn!`) and returning *something*.
/// This returns `$value`'s own text verbatim in both cases, which is
/// documented framework behavior: a broken `DATETIME()` call degrades to
/// showing the raw value rather than blanking the message or panicking.
/// An unrecognized `dateStyle`/`timeStyle` keyword gets the same
/// treatment (warn, then fall back to the default) rather than silently
/// being ignored — see [`parse_named_date_style`]/[`parse_named_time_style`].
fn datetime_function<'a>(positional: &[FluentValue<'a>], named: &FluentArgs) -> FluentValue<'a> {
    let Some(value) = positional.first() else {
        tracing::warn!("DATETIME(): missing the required $value positional argument");
        return FluentValue::Error;
    };

    let Some(dt) = parse_input(value) else {
        let text = value_text(value);
        tracing::warn!(
            input = %text,
            "DATETIME(): could not parse $value as an ISO-8601 string or epoch-millisecond number; returning it verbatim"
        );
        return FluentValue::String(Cow::Owned(text));
    };

    let date_style = parse_named_date_style(named);
    let time_style = parse_named_time_style(named);

    let rendered = match (date_style, time_style) {
        (Some(d), Some(t)) => Lang::try_datetime(&dt, d, t),
        (Some(d), None) => Lang::try_date(&dt, d),
        (None, Some(t)) => Lang::try_time(&dt, t),
        (None, None) => Lang::try_date(&dt, DateStyle::Medium),
    };

    match rendered {
        Ok(s) => FluentValue::String(Cow::Owned(s)),
        Err(e) => {
            let text = value_text(value);
            tracing::warn!(
                error = %e,
                input = %text,
                "DATETIME(): ICU formatting failed; returning the input verbatim"
            );
            FluentValue::String(Cow::Owned(text))
        }
    }
}

/// Parse `$value` as an ISO-8601 string (with or without a UTC offset, or
/// a bare date) or an epoch-milliseconds number. `None` on anything else,
/// including a malformed string or a value of the wrong `FluentValue`
/// variant.
fn parse_input(value: &FluentValue<'_>) -> Option<chrono::NaiveDateTime> {
    match value {
        FluentValue::String(s) => parse_iso8601(s),
        FluentValue::Number(n) => {
            chrono::DateTime::from_timestamp_millis(n.value as i64).map(|dt| dt.naive_utc())
        }
        FluentValue::Custom(_) | FluentValue::None | FluentValue::Error => None,
    }
}

/// Try, in order: full RFC 3339 (offset or `Z`), a bare `NaiveDateTime`
/// (no offset), then a bare `NaiveDate` (midnight is assumed).
fn parse_iso8601(s: &str) -> Option<chrono::NaiveDateTime> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.naive_utc());
    }
    if let Ok(dt) = s.parse::<chrono::NaiveDateTime>() {
        return Some(dt);
    }
    s.parse::<chrono::NaiveDate>()
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
}

/// The string opt at `key` in `named`, if present and actually a string.
fn named_style<'a>(named: &'a FluentArgs<'_>, key: &'a str) -> Option<&'a str> {
    match named.get(key) {
        Some(FluentValue::String(s)) => Some(s.as_ref()),
        _ => None,
    }
}

/// The `dateStyle` named option, parsed. `None` when the option is
/// absent (the normal "not asking for a date length" case, no log).
/// When it *is* present but isn't `"full"`/`"long"`/`"medium"`/`"short"`,
/// logs a `tracing::warn!` naming both the option and the bad value, then
/// still returns `None` — the caller falls back the same way an absent
/// option would, but the mistake isn't silent.
fn parse_named_date_style(named: &FluentArgs<'_>) -> Option<DateStyle> {
    let raw = named_style(named, "dateStyle")?;
    let parsed = parse_date_style(raw);
    if parsed.is_none() {
        tracing::warn!(
            value = raw,
            "DATETIME(): dateStyle is not one of \"full\"/\"long\"/\"medium\"/\"short\"; ignoring it and falling back to the default"
        );
    }
    parsed
}

/// The `timeStyle` named option, parsed. Same contract as
/// [`parse_named_date_style`], for `"medium"`/`"short"`.
fn parse_named_time_style(named: &FluentArgs<'_>) -> Option<TimeStyle> {
    let raw = named_style(named, "timeStyle")?;
    let parsed = parse_time_style(raw);
    if parsed.is_none() {
        tracing::warn!(
            value = raw,
            "DATETIME(): timeStyle is not one of \"medium\"/\"short\"; ignoring it and falling back to the default"
        );
    }
    parsed
}

fn parse_date_style(s: &str) -> Option<DateStyle> {
    match s {
        "full" => Some(DateStyle::Full),
        "long" => Some(DateStyle::Long),
        "medium" => Some(DateStyle::Medium),
        "short" => Some(DateStyle::Short),
        _ => None,
    }
}

fn parse_time_style(s: &str) -> Option<TimeStyle> {
    match s {
        "medium" => Some(TimeStyle::Medium),
        "short" => Some(TimeStyle::Short),
        _ => None,
    }
}

/// A plain-text rendering of `value`, used for the verbatim-passthrough
/// fallback and for logging. Not locale-aware — this is the last-resort
/// path where ICU formatting has already failed or was never attempted.
fn value_text(value: &FluentValue<'_>) -> String {
    match value {
        FluentValue::String(s) => s.to_string(),
        FluentValue::Number(n) => n.as_string().into_owned(),
        FluentValue::Custom(c) => format!("{c:?}"),
        FluentValue::None | FluentValue::Error => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rfc3339_bare_datetime_and_bare_date() {
        assert!(parse_iso8601("2026-08-01T14:30:00Z").is_some());
        assert!(parse_iso8601("2026-08-01T14:30:00+02:00").is_some());
        assert!(parse_iso8601("2026-08-01T14:30:00").is_some());
        assert_eq!(
            parse_iso8601("2026-08-01"),
            chrono::NaiveDate::from_ymd_opt(2026, 8, 1).and_then(|d| d.and_hms_opt(0, 0, 0))
        );
        assert!(parse_iso8601("not a date").is_none());
    }

    #[test]
    fn date_and_time_style_keywords_round_trip() {
        assert_eq!(parse_date_style("full"), Some(DateStyle::Full));
        assert_eq!(parse_date_style("long"), Some(DateStyle::Long));
        assert_eq!(parse_date_style("medium"), Some(DateStyle::Medium));
        assert_eq!(parse_date_style("short"), Some(DateStyle::Short));
        assert_eq!(parse_date_style("nonsense"), None);

        assert_eq!(parse_time_style("medium"), Some(TimeStyle::Medium));
        assert_eq!(parse_time_style("short"), Some(TimeStyle::Short));
        assert_eq!(parse_time_style("full"), None);
    }

    #[test]
    fn value_text_covers_string_and_number() {
        assert_eq!(value_text(&FluentValue::String(Cow::Borrowed("hi"))), "hi");
        assert_eq!(value_text(&FluentValue::None), "");
        assert_eq!(value_text(&FluentValue::Error), "");
    }
}
