//! Laravel's `date_format`, `after`, `after_or_equal`, `before` and
//! `before_or_equal`.

use chrono::{NaiveDate, TimeZone, Utc};
use suprnova::rules::{After, AfterOrEqual, Before, BeforeOrEqual, DateBound, DateFormat};
use suprnova::testing::TestClock;
use suprnova::{ContextualRule, FormContext, Rule};

fn ctx(pairs: &[(&str, &str)]) -> FormContext {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn day(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).expect("a real day")
}

#[test]
fn date_format_passes_a_value_matching_any_of_its_formats() {
    let rule = DateFormat(&["%Y-%m-%d", "%d/%m/%Y"]);
    assert!(rule.passes("2026-09-30").is_ok());
    assert!(rule.passes("30/09/2026").is_ok());
}

#[test]
fn date_format_fails_a_value_matching_none_of_its_formats() {
    let rule = DateFormat(&["%Y-%m-%d"]);
    let err = rule.passes("09/30/2026").unwrap_err();
    assert_eq!(err.key, "validation-date-format");
    assert_eq!(err.args.get("format"), Some(&"%Y-%m-%d".into()));
    assert!(rule.passes("2026-09-30 10:00").is_err(), "trailing input");
    assert!(rule.passes("").is_err());
}

/// Laravel formats the parsed date back and compares it with the input,
/// so an unpadded month and an impossible day both fail.
#[test]
fn date_format_is_as_strict_as_laravel() {
    let rule = DateFormat(&["%Y-%m-%d"]);
    assert!(rule.passes("2026-9-30").is_err(), "unpadded month");
    assert!(rule.passes("2026-02-31").is_err(), "February 31st");
    assert!(rule.passes("2026-13-01").is_err(), "month 13");
    assert!(rule.passes("2028-02-29").is_ok(), "a real leap day");
}

#[test]
fn date_format_takes_a_partial_format() {
    let month = DateFormat(&["%Y-%m"]);
    assert!(month.passes("2026-09").is_ok());
    assert!(month.passes("2026-9").is_err());
    assert!(month.passes("2026-13").is_err());

    let expiry = DateFormat(&["%m/%y"]);
    assert!(expiry.passes("09/27").is_ok(), "a two-digit year");

    let time = DateFormat(&["%H:%M"]);
    assert!(time.passes("23:59").is_ok());
    assert!(time.passes("24:00").is_err());

    // The missing year is 1970, as PHP's `!` makes it, so Laravel and
    // Suprnova both refuse a leap day without a year.
    let leap = DateFormat(&["%m-%d"]);
    assert!(leap.passes("02-28").is_ok());
    assert!(leap.passes("02-29").is_err());
}

#[test]
fn date_format_takes_a_format_with_an_offset() {
    let rule = DateFormat(&["%Y-%m-%dT%H:%M:%S%:z"]);
    assert!(rule.passes("2026-09-30T08:15:00+02:00").is_ok());
    assert!(rule.passes("2026-09-30T08:15:00").is_err());
}

#[test]
fn date_format_with_a_format_chrono_cannot_read_names_it() {
    let rule = DateFormat(&["%Y-%Q"]);
    let err = rule.passes("2026-09").unwrap_err();
    assert!(err.key.is_empty(), "an operator message, not a keyed one");
    assert!(err.fallback.contains("%Y-%Q"), "got: {}", err.fallback);
}

#[test]
fn after_compares_with_a_fixed_day_as_its_midnight() {
    let rule = After::new(DateBound::Date(day(2026, 9, 30)));
    let none = FormContext::new();
    assert!(rule.passes("2026-10-01", &none).is_ok());
    assert!(rule.passes("2026-09-30 00:00:01", &none).is_ok());
    let err = rule.passes("2026-09-30", &none).unwrap_err();
    assert_eq!(err.key, "validation-after");
    assert_eq!(err.args.get("date"), Some(&"2026-09-30".into()));
    assert!(rule.passes("2026-09-29T23:59", &none).is_err());
}

#[test]
fn equal_moments_pass_only_the_or_equal_rules() {
    let bound = || DateBound::Date(day(2026, 9, 30));
    let none = FormContext::new();
    assert!(After::new(bound()).passes("2026-09-30", &none).is_err());
    assert!(
        AfterOrEqual::new(bound())
            .passes("2026-09-30", &none)
            .is_ok()
    );
    assert!(Before::new(bound()).passes("2026-09-30", &none).is_err());
    assert!(
        BeforeOrEqual::new(bound())
            .passes("2026-09-30", &none)
            .is_ok()
    );
    assert!(
        BeforeOrEqual::new(bound())
            .passes("2026-10-01", &none)
            .is_err()
    );
    assert!(Before::new(bound()).passes("2026-09-29", &none).is_ok());
}

#[test]
fn an_offset_is_converted_to_utc_before_comparing() {
    let rule = Before::new(DateBound::DateTime(
        Utc.with_ymd_and_hms(2026, 9, 30, 12, 0, 0).unwrap(),
    ));
    let none = FormContext::new();
    // 13:30 at +02:00 is 11:30 UTC.
    assert!(rule.passes("2026-09-30T13:30:00+02:00", &none).is_ok());
    // 11:30 at -02:00 is 13:30 UTC.
    assert!(rule.passes("2026-09-30T11:30:00-02:00", &none).is_err());
}

/// The clock is moved far from the real date, so a bound that read the
/// system clock instead would answer differently.
#[test]
fn the_relative_bounds_read_the_framework_clock() {
    let _clock = TestClock::travel_to(Utc.with_ymd_and_hms(2031, 3, 14, 15, 0, 0).unwrap());
    let none = FormContext::new();

    assert!(
        After::new(DateBound::Today)
            .passes("2031-03-14 00:00:01", &none)
            .is_ok()
    );
    assert!(
        After::new(DateBound::Today)
            .passes("2031-03-14", &none)
            .is_err()
    );
    assert!(
        After::new(DateBound::Now)
            .passes("2031-03-14 15:00:01", &none)
            .is_ok()
    );
    assert!(
        After::new(DateBound::Now)
            .passes("2031-03-14 14:59", &none)
            .is_err()
    );
    assert!(
        AfterOrEqual::new(DateBound::Tomorrow)
            .passes("2031-03-15", &none)
            .is_ok()
    );
    assert!(
        AfterOrEqual::new(DateBound::Tomorrow)
            .passes("2031-03-14 23:59", &none)
            .is_err()
    );
    assert!(
        Before::new(DateBound::Yesterday)
            .passes("2031-03-12", &none)
            .is_ok()
    );
    assert!(
        Before::new(DateBound::Yesterday)
            .passes("2031-03-13", &none)
            .is_err()
    );

    let err = After::new(DateBound::Today)
        .passes("2031-03-01", &none)
        .unwrap_err();
    assert_eq!(err.args.get("date"), Some(&"today".into()));
}

#[test]
fn a_sibling_bound_is_named_by_its_field_not_its_value() {
    let rule = After::new(DateBound::Field("starts_on"));
    let form = ctx(&[("starts_on", "2026-09-30")]);
    assert!(rule.passes("2026-10-02", &form).is_ok());
    let err = rule.passes("2026-09-01", &form).unwrap_err();
    assert_eq!(err.args.get("date"), Some(&"starts_on".into()));
    assert!(
        !err.fallback.contains("2026-09-30"),
        "got: {}",
        err.fallback
    );
}

#[test]
fn a_value_or_sibling_that_is_not_a_date_fails_the_field() {
    let rule = After::new(DateBound::Field("starts_on"));
    assert!(
        rule.passes("2026-10-02", &ctx(&[("starts_on", "soon")]))
            .is_err()
    );
    assert!(
        rule.passes("next week", &ctx(&[("starts_on", "2026-09-30")]))
            .is_err()
    );
    assert!(
        Before::new(DateBound::Today)
            .passes("", &FormContext::new())
            .is_err()
    );
}

/// Laravel reads an absent sibling as null and passes the comparison.
#[test]
fn a_sibling_the_form_did_not_send_passes() {
    let rule = After::new(DateBound::Field("starts_on"));
    assert!(rule.passes("2026-10-02", &FormContext::new()).is_ok());
    assert!(
        rule.passes("2026-10-02", &ctx(&[("starts_on", " ")]))
            .is_ok()
    );
}

#[test]
fn a_format_reads_the_value_and_a_sibling_bound() {
    let _clock = TestClock::travel_to(Utc.with_ymd_and_hms(2031, 3, 14, 15, 0, 0).unwrap());
    let none = FormContext::new();
    let rule = After::new(DateBound::Today).format("%d/%m/%Y");
    assert!(rule.passes("15/03/2031", &none).is_ok());
    assert!(rule.passes("13/03/2031", &none).is_err());
    assert!(
        rule.passes("2031-03-15", &none).is_err(),
        "not in the format"
    );

    let ends = AfterOrEqual::new(DateBound::Field("starts_on")).format("%d/%m/%Y");
    let form = ctx(&[("starts_on", "20/03/2031")]);
    assert!(ends.passes("20/03/2031", &form).is_ok());
    assert!(ends.passes("19/03/2031", &form).is_err());
}

#[test]
fn a_comparison_format_chrono_cannot_read_names_it() {
    let err = Before::new(DateBound::Now)
        .format("%Q")
        .passes("2026-09-30", &FormContext::new())
        .unwrap_err();
    assert!(err.key.is_empty(), "an operator message, not a keyed one");
    assert!(err.fallback.contains("%Q"), "got: {}", err.fallback);
}

/// Laravel compares Unix timestamps, so two moments in the same second
/// are equal.
#[test]
fn moments_in_the_same_second_are_equal() {
    let _clock = TestClock::travel_to(Utc.with_ymd_and_hms(2031, 3, 14, 15, 0, 0).unwrap());
    let none = FormContext::new();
    assert!(
        After::new(DateBound::Now)
            .passes("2031-03-14 15:00:00.900", &none)
            .is_err()
    );
    assert!(
        AfterOrEqual::new(DateBound::Now)
            .passes("2031-03-14 15:00:00.900", &none)
            .is_ok()
    );
    assert!(
        After::new(DateBound::Now)
            .passes("2031-03-14 15:00:01", &none)
            .is_ok()
    );
}
