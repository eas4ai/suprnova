//! `Date`: read a date or a time from text, as Laravel's `Date::parse`
//! reads free-form text through Carbon.
//!
//! Carbon reads PHP's whole relative-format grammar. This parser reads a
//! named subset of it, listed on [`Date::parse`], because that grammar is
//! large and a form read differently from PHP would be worse than a form
//! refused. Every relative form reads [`crate::clock::now`], so a test
//! that moves `TestClock` moves the parser too.

use chrono::{
    DateTime, Datelike, Days, Duration, NaiveDate, NaiveDateTime, NaiveTime, Offset, TimeZone, Utc,
    Weekday,
};
use chrono_tz::Tz;
use thiserror::Error;

use crate::error::FrameworkError;

/// Reads dates and times from text, in the shape of Laravel's `Date`
/// facade.
///
/// ```
/// use suprnova::Date;
///
/// let moment = Date::parse("2026-10-09 09:30").unwrap();
/// assert_eq!(moment.to_rfc3339(), "2026-10-09T09:30:00+00:00");
/// assert!(Date::parse("soonish").is_err());
/// ```
pub struct Date;

/// The text [`Date::parse`] could not read.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error(
    "`{text}` is not a date or time `Date::parse` reads; it reads RFC 3339, `YYYY-MM-DD`, `YYYY-MM-DD HH:MM[:SS]`, `@<seconds>`, `now`, `today`, `tomorrow`, `yesterday`, `midnight`, `noon`, `+N <unit>`, `N <unit> ago`, `[next|last] <weekday>` and `first|last day of this|next|last month`"
)]
pub struct DateParseError {
    text: String,
}

impl DateParseError {
    fn new(text: &str) -> Self {
        Self {
            text: text.to_owned(),
        }
    }

    /// The text that was not read, as it was given.
    pub fn text(&self) -> &str {
        &self.text
    }
}

impl From<DateParseError> for FrameworkError {
    fn from(error: DateParseError) -> Self {
        FrameworkError::internal(error.to_string())
    }
}

impl Date {
    /// Read `text` as a moment in UTC.
    ///
    /// It reads, ignoring case and extra spaces:
    ///
    /// - RFC 3339, such as `2026-10-09T12:00:00+02:00`.
    /// - `YYYY-MM-DD`, which is that day's midnight.
    /// - `YYYY-MM-DD HH:MM`, `YYYY-MM-DD HH:MM:SS` or
    ///   `YYYY-MM-DD HH:MM:SS.fraction`, with a space or a `T`.
    /// - `@<unix seconds>`, such as `@1791547200` or `@-60`.
    /// - `now`, `today`, `tomorrow`, `yesterday`, `midnight` and `noon`.
    /// - `[+|-]N <unit>` and `N <unit> ago`, where the unit is `second`,
    ///   `minute`, `hour`, `day`, `week`, `fortnight`, `month` or `year`,
    ///   singular or plural.
    /// - `next <weekday>`, `last <weekday>` and `<weekday>`, with the
    ///   weekday spelled in full in English.
    /// - `first day of` or `last day of`, then `this`, `next` or `last`,
    ///   then `month`.
    /// - Any of the word forms above (not the `N <unit>` forms) followed
    ///   by `HH:MM`.
    ///
    /// The relative forms read [`crate::clock::now`]. As in PHP, `today`,
    /// `tomorrow`, `yesterday`, `midnight` and the weekday forms set the
    /// time to midnight, `noon` sets it to 12:00, a following `HH:MM` sets
    /// it to that time, and the other forms keep the time of day. A
    /// weekday alone is today when today is that day, `next` is strictly
    /// after today and `last` strictly before. Days, weeks, fortnights,
    /// months and years move the calendar date and keep the wall-clock
    /// time; seconds, minutes and hours move the moment. A month that has
    /// no such day overflows into the next, as PHP's does: one month after
    /// January 31 is March 3 in a common year.
    ///
    /// # Errors
    ///
    /// [`DateParseError`], naming the text, for text it does not read.
    pub fn parse(text: &str) -> Result<DateTime<Utc>, DateParseError> {
        Self::parse_in(text, Tz::UTC).map(|moment| moment.with_timezone(&Utc))
    }

    /// Read `text` as [`parse`](Self::parse) does, in the time zone `tz`:
    /// a day, a time without an offset and every relative form are taken
    /// in that zone. A local time that a daylight-saving change skips is
    /// read with the offset in force before the change, which moves it
    /// forward by the length of the skip, as PHP does; a local time that a
    /// change repeats is the earlier of the two.
    ///
    /// # Errors
    ///
    /// [`DateParseError`], naming the text, for text it does not read.
    pub fn parse_in(text: &str, tz: Tz) -> Result<DateTime<Tz>, DateParseError> {
        let now = crate::clock::now().with_timezone(&tz);
        read(text, now).ok_or_else(|| DateParseError::new(text))
    }

    /// The same as [`parse`](Self::parse). Laravel's `rawParse` skips
    /// Carbon's parse hook; Suprnova has no such hook, so there is
    /// nothing to skip.
    ///
    /// # Errors
    ///
    /// [`DateParseError`], naming the text, for text it does not read.
    pub fn raw_parse(text: &str) -> Result<DateTime<Utc>, DateParseError> {
        Self::parse(text)
    }
}

/// Read `text` against `now`, in `now`'s zone.
fn read(text: &str, now: DateTime<Tz>) -> Option<DateTime<Tz>> {
    let tz = now.timezone();
    let spaced = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some(moment) = absolute(text.trim(), &spaced, tz) {
        return Some(moment);
    }
    let words: Vec<String> = text.split_whitespace().map(str::to_lowercase).collect();
    if words == ["now"] {
        return Some(now);
    }
    if let Some(moment) = relative(&words, now) {
        return Some(moment);
    }
    let (form, time) = match words.split_last() {
        Some((last, rest)) if !rest.is_empty() && last.contains(':') => {
            (rest, Some(clock_time(last)?))
        }
        _ => (words.as_slice(), None),
    };
    let moment = word_form(form, now.naive_local())?;
    let moment = match time {
        Some(time) => moment.date().and_time(time),
        None => moment,
    };
    resolve_local(tz, moment)
}

/// RFC 3339, a date, a date with a time, or `@<seconds>`. `raw` is the
/// trimmed text and `spaced` the text with each run of whitespace made
/// one space.
fn absolute(raw: &str, spaced: &str, tz: Tz) -> Option<DateTime<Tz>> {
    if let Some(seconds) = spaced.strip_prefix('@') {
        let seconds: i64 = seconds.parse().ok()?;
        return DateTime::from_timestamp(seconds, 0).map(|moment| moment.with_timezone(&tz));
    }
    if let Ok(moment) = DateTime::parse_from_rfc3339(raw) {
        return Some(moment.with_timezone(&tz));
    }
    const LOCAL: [&str; 4] = [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M",
    ];
    for format in LOCAL {
        if let Ok(moment) = NaiveDateTime::parse_from_str(spaced, format) {
            return resolve_local(tz, moment);
        }
    }
    let day = NaiveDate::parse_from_str(spaced, "%Y-%m-%d").ok()?;
    resolve_local(tz, day.and_time(NaiveTime::MIN))
}

/// `[+|-]N <unit>` or `N <unit> ago`.
fn relative(words: &[String], now: DateTime<Tz>) -> Option<DateTime<Tz>> {
    let (count, unit, ago) = match words {
        [count, unit] => (count.as_str(), unit.as_str(), false),
        [count, unit, ago] if ago == "ago" => {
            if count.starts_with(['+', '-']) {
                return None;
            }
            (count.as_str(), unit.as_str(), true)
        }
        _ => return None,
    };
    let (sign, digits) = if let Some(digits) = count.strip_prefix('+') {
        (1, digits)
    } else if let Some(digits) = count.strip_prefix('-') {
        (-1, digits)
    } else {
        (1, count)
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let count = digits.parse::<i64>().ok()?.checked_mul(sign)?;
    let count = if ago { count.checked_neg()? } else { count };
    let unit = unit.strip_suffix('s').unwrap_or(unit);
    let elapsed = |seconds_per: i64| -> Option<DateTime<Tz>> {
        let seconds = count.checked_mul(seconds_per)?;
        now.checked_add_signed(Duration::try_seconds(seconds)?)
    };
    let local = now.naive_local();
    let moved = match unit {
        "second" => return elapsed(1),
        "minute" => return elapsed(60),
        "hour" => return elapsed(3_600),
        "day" => add_days(local, count)?,
        "week" => add_days(local, count.checked_mul(7)?)?,
        "fortnight" => add_days(local, count.checked_mul(14)?)?,
        "month" => add_months(local, count)?,
        "year" => add_months(local, count.checked_mul(12)?)?,
        _ => return None,
    };
    resolve_local(now.timezone(), moved)
}

/// The word forms, as a local date and time.
fn word_form(words: &[String], now: NaiveDateTime) -> Option<NaiveDateTime> {
    let today = now.date();
    let midnight = |day: NaiveDate| Some(day.and_time(NaiveTime::MIN));
    match words {
        [word] => match word.as_str() {
            "now" => Some(now),
            "today" | "midnight" => midnight(today),
            "tomorrow" => midnight(today.succ_opt()?),
            "yesterday" => midnight(today.pred_opt()?),
            "noon" => Some(today.and_time(NaiveTime::from_hms_opt(12, 0, 0)?)),
            day => {
                let weekday = weekday(day)?;
                let ahead = days_until(today.weekday(), weekday);
                midnight(today.checked_add_days(Days::new(ahead))?)
            }
        },
        [direction, day] => {
            let weekday = weekday(day)?;
            match direction.as_str() {
                "next" => {
                    let ahead = match days_until(today.weekday(), weekday) {
                        0 => 7,
                        ahead => ahead,
                    };
                    midnight(today.checked_add_days(Days::new(ahead))?)
                }
                "last" => {
                    let behind = match days_until(weekday, today.weekday()) {
                        0 => 7,
                        behind => behind,
                    };
                    midnight(today.checked_sub_days(Days::new(behind))?)
                }
                _ => None,
            }
        }
        [edge, day, of, which, month] if day == "day" && of == "of" && month == "month" => {
            let offset = match which.as_str() {
                "this" => 0,
                "next" => 1,
                "last" => -1,
                _ => return None,
            };
            let first = first_of_month(today, offset)?;
            let day = match edge.as_str() {
                "first" => first,
                "last" => first_of_month(first, 1)?.pred_opt()?,
                _ => return None,
            };
            Some(day.and_time(now.time()))
        }
        _ => None,
    }
}

/// An English weekday spelled in full.
fn weekday(word: &str) -> Option<Weekday> {
    match word {
        "monday" => Some(Weekday::Mon),
        "tuesday" => Some(Weekday::Tue),
        "wednesday" => Some(Weekday::Wed),
        "thursday" => Some(Weekday::Thu),
        "friday" => Some(Weekday::Fri),
        "saturday" => Some(Weekday::Sat),
        "sunday" => Some(Weekday::Sun),
        _ => None,
    }
}

/// Days from `from` forward to the next `to`, zero when they are the
/// same day.
fn days_until(from: Weekday, to: Weekday) -> u64 {
    u64::from((to.num_days_from_monday() + 7 - from.num_days_from_monday()) % 7)
}

/// `HH:MM` with an hour from 0 to 23 and a minute from 0 to 59.
fn clock_time(word: &str) -> Option<NaiveTime> {
    let (hour, minute) = word.split_once(':')?;
    let digits = |part: &str, most: usize| {
        !part.is_empty() && part.len() <= most && part.bytes().all(|b| b.is_ascii_digit())
    };
    if !digits(hour, 2) || minute.len() != 2 || !digits(minute, 2) {
        return None;
    }
    NaiveTime::from_hms_opt(hour.parse().ok()?, minute.parse().ok()?, 0)
}

/// `local` moved by `days` calendar days, keeping its time.
fn add_days(local: NaiveDateTime, days: i64) -> Option<NaiveDateTime> {
    let magnitude = Days::new(days.unsigned_abs());
    if days < 0 {
        local.checked_sub_days(magnitude)
    } else {
        local.checked_add_days(magnitude)
    }
}

/// `local` moved by `months`, keeping its day of the month and time. A
/// day past the end of the target month overflows into the next, as
/// PHP's date arithmetic does.
fn add_months(local: NaiveDateTime, months: i64) -> Option<NaiveDateTime> {
    let first = first_of_month(local.date(), months)?;
    let day = first.checked_add_days(Days::new(u64::from(local.day()) - 1))?;
    Some(day.and_time(local.time()))
}

/// The first day of the month `offset` months from `day`'s.
fn first_of_month(day: NaiveDate, offset: i64) -> Option<NaiveDate> {
    let index = i64::from(day.year())
        .checked_mul(12)?
        .checked_add(i64::from(day.month0()))?
        .checked_add(offset)?;
    let year = i32::try_from(index.div_euclid(12)).ok()?;
    let month = u32::try_from(index.rem_euclid(12)).ok()? + 1;
    NaiveDate::from_ymd_opt(year, month, 1)
}

/// The moment of the local time `local` in `tz`. A time a daylight-saving
/// change repeats is the earlier of the two. A time a change skips is read
/// with the offset in force a day before, which moves it forward by the
/// length of the skip, as PHP does.
fn resolve_local(tz: Tz, local: NaiveDateTime) -> Option<DateTime<Tz>> {
    if let Some(moment) = tz.from_local_datetime(&local).earliest() {
        return Some(moment);
    }
    let day_before = tz
        .from_local_datetime(&local.checked_sub_signed(Duration::days(1))?)
        .earliest()?;
    let offset = i64::from(day_before.offset().fix().local_minus_utc());
    let utc = local.checked_sub_signed(Duration::try_seconds(offset)?)?;
    Some(Utc.from_utc_datetime(&utc).with_timezone(&tz))
}
