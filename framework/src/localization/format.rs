//! ICU4X-backed locale-aware formatting: numbers, currency, dates, times,
//! lists, and relative time.
//!
//! This module is pure formatting — it never consults a `Translator` or a
//! Fluent catalog (see `functions.rs` for the `DATETIME()` Fluent function
//! that calls back into it). Every entry point takes the caller's data
//! plus a resolved [`Locale`] and hands back `Result<String,
//! FrameworkError>`; the `Lang` facade (`mod.rs`) wraps each of these in
//! an infallible sibling that resolves `Lang::locale()` itself and falls
//! back to a plain, non-localized rendering on any ICU failure.
//!
//! ICU4X 2.x's `icu_experimental` crate (currency, relative time) is used
//! only in this file — nothing outside `localization/` needs to know
//! about it.

use super::locale::Locale;
use crate::error::FrameworkError;
use chrono::{Datelike, NaiveDateTime, Timelike};
use fixed_decimal::{Decimal, FloatPrecision};
use icu_calendar::{Date, Iso};
use icu_datetime::fieldsets::{T, YMD, YMDE};
use icu_datetime::input::{DateTime, Time};
use icu_datetime::{DateTimeFormatter, DateTimeFormatterPreferences, NoCalendarFormatter};
use icu_decimal::{DecimalFormatter, DecimalFormatterPreferences};
use icu_experimental::dimension::currency::CurrencyCode;
use icu_experimental::dimension::currency::formatter::{
    CurrencyFormatter, CurrencyFormatterPreferences,
};
use icu_experimental::relativetime::{
    RelativeTimeFormatter, RelativeTimeFormatterOptions, RelativeTimeFormatterPreferences,
};
use icu_list::{ListFormatter, ListFormatterPreferences};
use tinystr::TinyAsciiStr;
use writeable::Writeable;

/// How much of a date to spell out.
///
/// Mirrors the four CLDR date lengths, though ICU4X 2.x's own `Length`
/// type collapses "full" into "long" (it has no fourth variant).
/// [`DateStyle::Full`] is rendered here as a long-length date with the
/// weekday prepended — the same distinction Java's `DateFormat.FULL` vs
/// `.LONG` and JS's `Intl.DateTimeFormat` make.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateStyle {
    /// Weekday + long date, e.g. "Saturday, August 1, 2026".
    Full,
    /// Spelled-out date, e.g. "August 1, 2026".
    Long,
    /// Abbreviated date, e.g. "Aug 1, 2026".
    Medium,
    /// Numeric date, e.g. "8/1/26".
    Short,
}

/// How much of a time-of-day to spell out.
///
/// Only `Medium`/`Short` are offered: without a time zone attached to a
/// plain `chrono::NaiveDateTime`, CLDR's `Long`/`Full` time lengths (which
/// add zone info) have nothing to add over `Medium`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeStyle {
    /// Hours, minutes, and seconds, e.g. "3:47:50 PM".
    Medium,
    /// Hours and minutes, e.g. "3:47 PM".
    Short,
}

/// The conjunction used to join a formatted list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListStyle {
    /// "a, b, and c".
    And,
    /// "a, b, or c".
    Or,
    /// A conjunction-free unit list, e.g. "3 ft, 2 in".
    Unit,
}

/// The unit a relative time offset is expressed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelativeUnit {
    /// Seconds.
    Second,
    /// Minutes.
    Minute,
    /// Hours.
    Hour,
    /// Days.
    Day,
    /// Weeks.
    Week,
    /// Months.
    Month,
    /// Years.
    Year,
}

/// Convert the framework's [`Locale`] to the `icu_locale_core::Locale`
/// every ICU4X formatter constructor here wants.
///
/// This should essentially never fail — every [`Locale`] was already
/// validated as a BCP-47 identifier by `unic-langid` when it was parsed —
/// but a divergence between the two parsers is surfaced as a normal `Err`
/// rather than a panic.
fn icu_locale(locale: &Locale) -> Result<icu_locale_core::Locale, FrameworkError> {
    icu_locale_core::Locale::try_from_str(&locale.as_str()).map_err(|e| {
        FrameworkError::internal(format!(
            "locale `{locale}` is not a valid ICU locale identifier: {e}"
        ))
    })
}

/// The date half of `dt`, converted to ICU4X's ISO calendar.
fn iso_date(dt: &NaiveDateTime) -> Result<Date<Iso>, FrameworkError> {
    Date::try_new_iso(dt.year(), dt.month() as u8, dt.day() as u8)
        .map_err(|e| FrameworkError::internal(format!("`{dt}` has an invalid date component: {e}")))
}

/// The time-of-day half of `dt`, converted to ICU4X's `Time`.
fn iso_time(dt: &NaiveDateTime) -> Result<Time, FrameworkError> {
    Time::try_new(
        dt.hour() as u8,
        dt.minute() as u8,
        dt.second() as u8,
        dt.nanosecond(),
    )
    .map_err(|e| FrameworkError::internal(format!("`{dt}` has an invalid time component: {e}")))
}

/// Locale-aware decimal formatting, e.g. `1234567.89` renders as
/// `1,234,567.89` in `en-US` and `1.234.567,89` in `de-DE`.
pub(crate) fn try_number(locale: &Locale, n: f64) -> Result<String, FrameworkError> {
    let prefs: DecimalFormatterPreferences = icu_locale(locale)?.into();
    let formatter = DecimalFormatter::try_new(prefs, Default::default())
        .map_err(|e| FrameworkError::internal(format!("DecimalFormatter: {e}")))?;
    let decimal = Decimal::try_from_f64(n, FloatPrecision::RoundTrip)
        .map_err(|e| FrameworkError::internal(format!("`{n}` is not a formattable number: {e}")))?;
    Ok(formatter.format(&decimal).write_to_string().into_owned())
}

/// Locale-aware currency formatting. `iso_code` is a 3-letter ISO 4217
/// code (`"USD"`, `"EUR"`, ...), case-insensitive.
///
/// Renders with the short currency symbol (`$19.99`, not the narrow or
/// spelled-out forms) and assumes 2 fraction digits — correct for the
/// large majority of currencies, but not for zero-decimal currencies like
/// JPY. ICU4X 2.x's `CurrencyFormatter::format_fixed_decimal` takes
/// whatever `Decimal` it's handed; it does not resolve per-currency
/// fraction-digit counts on the caller's behalf.
pub(crate) fn try_currency(
    locale: &Locale,
    amount: f64,
    iso_code: &str,
) -> Result<String, FrameworkError> {
    let prefs: CurrencyFormatterPreferences = icu_locale(locale)?.into();
    let formatter = CurrencyFormatter::try_new(prefs, Default::default())
        .map_err(|e| FrameworkError::internal(format!("CurrencyFormatter: {e}")))?;
    let code = TinyAsciiStr::<3>::try_from_str(&iso_code.to_ascii_uppercase()).map_err(|e| {
        FrameworkError::param(format!(
            "`{iso_code}` is not a 3-letter ISO 4217 currency code: {e}"
        ))
    })?;
    let value = Decimal::try_from_f64(amount, FloatPrecision::Magnitude(-2)).map_err(|e| {
        FrameworkError::internal(format!("`{amount}` is not a formattable amount: {e}"))
    })?;
    Ok(formatter
        .format_fixed_decimal(&value, &CurrencyCode(code))
        .to_string())
}

/// Locale-aware date formatting. See [`DateStyle`] for how
/// [`DateStyle::Full`] differs from [`DateStyle::Long`].
pub(crate) fn try_date(
    locale: &Locale,
    dt: &NaiveDateTime,
    style: DateStyle,
) -> Result<String, FrameworkError> {
    let prefs: DateTimeFormatterPreferences = icu_locale(locale)?.into();
    let date = iso_date(dt)?;
    match style {
        DateStyle::Full => {
            let formatter = DateTimeFormatter::try_new(prefs, YMDE::long())
                .map_err(|e| FrameworkError::internal(format!("DateTimeFormatter: {e}")))?;
            Ok(formatter.format(&date).write_to_string().into_owned())
        }
        DateStyle::Long => {
            let formatter = DateTimeFormatter::try_new(prefs, YMD::long())
                .map_err(|e| FrameworkError::internal(format!("DateTimeFormatter: {e}")))?;
            Ok(formatter.format(&date).write_to_string().into_owned())
        }
        DateStyle::Medium => {
            let formatter = DateTimeFormatter::try_new(prefs, YMD::medium())
                .map_err(|e| FrameworkError::internal(format!("DateTimeFormatter: {e}")))?;
            Ok(formatter.format(&date).write_to_string().into_owned())
        }
        DateStyle::Short => {
            let formatter = DateTimeFormatter::try_new(prefs, YMD::short())
                .map_err(|e| FrameworkError::internal(format!("DateTimeFormatter: {e}")))?;
            Ok(formatter.format(&date).write_to_string().into_owned())
        }
    }
}

/// Locale-aware time-of-day formatting. See [`TimeStyle`].
pub(crate) fn try_time(
    locale: &Locale,
    dt: &NaiveDateTime,
    style: TimeStyle,
) -> Result<String, FrameworkError> {
    let prefs: DateTimeFormatterPreferences = icu_locale(locale)?.into();
    let time = iso_time(dt)?;
    let field_set = match style {
        TimeStyle::Medium => T::medium(),
        TimeStyle::Short => T::short(),
    };
    let formatter = NoCalendarFormatter::try_new(prefs, field_set)
        .map_err(|e| FrameworkError::internal(format!("NoCalendarFormatter: {e}")))?;
    Ok(formatter.format(&time).write_to_string().into_owned())
}

/// Locale-aware combined date + time formatting. See [`DateStyle`] and
/// [`TimeStyle`].
pub(crate) fn try_datetime(
    locale: &Locale,
    dt: &NaiveDateTime,
    date: DateStyle,
    time: TimeStyle,
) -> Result<String, FrameworkError> {
    let prefs: DateTimeFormatterPreferences = icu_locale(locale)?.into();
    let combined = DateTime {
        date: iso_date(dt)?,
        time: iso_time(dt)?,
    };
    // Each arm below instantiates `DateTimeFormatter` over a different
    // concrete field-set type (`YMDT`/`YMDET`, at a different `Length`) —
    // `icu_datetime`'s static field sets are types, not runtime values, so
    // there is no single generic path through this match. The macro just
    // removes the boilerplate that's identical across all eight arms.
    macro_rules! render {
        ($field_set:expr) => {{
            let formatter = DateTimeFormatter::try_new(prefs, $field_set)
                .map_err(|e| FrameworkError::internal(format!("DateTimeFormatter: {e}")))?;
            Ok(formatter.format(&combined).write_to_string().into_owned())
        }};
    }
    match (date, time) {
        (DateStyle::Full, TimeStyle::Medium) => render!(YMDE::long().with_time_hms()),
        (DateStyle::Full, TimeStyle::Short) => render!(YMDE::long().with_time_hm()),
        (DateStyle::Long, TimeStyle::Medium) => render!(YMD::long().with_time_hms()),
        (DateStyle::Long, TimeStyle::Short) => render!(YMD::long().with_time_hm()),
        (DateStyle::Medium, TimeStyle::Medium) => render!(YMD::medium().with_time_hms()),
        (DateStyle::Medium, TimeStyle::Short) => render!(YMD::medium().with_time_hm()),
        (DateStyle::Short, TimeStyle::Medium) => render!(YMD::short().with_time_hms()),
        (DateStyle::Short, TimeStyle::Short) => render!(YMD::short().with_time_hm()),
    }
}

/// Locale-aware list formatting. See [`ListStyle`].
pub(crate) fn try_list(
    locale: &Locale,
    items: &[&str],
    style: ListStyle,
) -> Result<String, FrameworkError> {
    let prefs: ListFormatterPreferences = icu_locale(locale)?.into();
    let formatter = match style {
        ListStyle::And => ListFormatter::try_new_and(prefs, Default::default()),
        ListStyle::Or => ListFormatter::try_new_or(prefs, Default::default()),
        ListStyle::Unit => ListFormatter::try_new_unit(prefs, Default::default()),
    }
    .map_err(|e| FrameworkError::internal(format!("ListFormatter: {e}")))?;
    Ok(formatter
        .format(items.iter().copied())
        .write_to_string()
        .into_owned())
}

/// Locale-aware relative time formatting, e.g. `-3` with
/// [`RelativeUnit::Day`] renders as `"3 days ago"` in `en`.
///
/// Always renders numerically — `RelativeTimeFormatterOptions`'s default
/// `Numeric::Always` — rather than substituting special forms like
/// "yesterday"/"today" for small offsets.
pub(crate) fn try_relative(
    locale: &Locale,
    amount: i64,
    unit: RelativeUnit,
) -> Result<String, FrameworkError> {
    let prefs: RelativeTimeFormatterPreferences = icu_locale(locale)?.into();
    let options = RelativeTimeFormatterOptions::default();
    let formatter = match unit {
        RelativeUnit::Second => RelativeTimeFormatter::try_new_long_second(prefs, options),
        RelativeUnit::Minute => RelativeTimeFormatter::try_new_long_minute(prefs, options),
        RelativeUnit::Hour => RelativeTimeFormatter::try_new_long_hour(prefs, options),
        RelativeUnit::Day => RelativeTimeFormatter::try_new_long_day(prefs, options),
        RelativeUnit::Week => RelativeTimeFormatter::try_new_long_week(prefs, options),
        RelativeUnit::Month => RelativeTimeFormatter::try_new_long_month(prefs, options),
        RelativeUnit::Year => RelativeTimeFormatter::try_new_long_year(prefs, options),
    }
    .map_err(|e| FrameworkError::internal(format!("RelativeTimeFormatter: {e}")))?;
    Ok(formatter
        .format(Decimal::from(amount))
        .write_to_string()
        .into_owned())
}
