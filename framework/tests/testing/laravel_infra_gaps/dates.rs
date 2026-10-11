//! `Date::parse`, `parse_in` and `raw_parse` under a clock frozen at
//! 2026-10-09T12:00:00Z, a Friday.

use chrono::{DateTime, TimeZone, Utc};
use suprnova::Date;
use suprnova::Tz;
use suprnova::testing::{TestClock, TestClockGuard};

fn friday_noon() -> TestClockGuard {
    TestClock::travel_to(utc("2026-10-09T12:00:00Z"))
}

fn utc(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("a test instant")
        .with_timezone(&Utc)
}

fn parsed(text: &str) -> DateTime<Utc> {
    Date::parse(text).unwrap_or_else(|error| panic!("{text:?} did not parse: {error}"))
}

#[test]
fn the_falsifier_forms_read_against_the_clock() {
    let _clock = friday_noon();
    assert_eq!(parsed("tomorrow"), utc("2026-10-10T00:00:00Z"));
    assert_eq!(parsed("+2 days"), utc("2026-10-11T12:00:00Z"));
    assert_eq!(parsed("3 hours ago"), utc("2026-10-09T09:00:00Z"));
    assert_eq!(parsed("next monday"), utc("2026-10-12T00:00:00Z"));
    assert_eq!(parsed("tomorrow 09:30"), utc("2026-10-10T09:30:00Z"));
    assert_eq!(
        parsed("first day of next month"),
        utc("2026-11-01T12:00:00Z")
    );
}

#[test]
fn text_it_does_not_read_is_an_error_naming_it() {
    let _clock = friday_noon();
    let error = Date::parse("soonish").expect_err("soonish is not a date");
    assert!(error.to_string().contains("soonish"), "{error}");
    assert_eq!(error.text(), "soonish");
    for unread in [
        "",
        "next",
        "2 parsecs",
        "2026-02-30",
        "2026-10-09 25:00",
        "@soon",
        "tomorrow 24:00",
        "tomorrow 9:61",
        "first day of next week",
        "last day of month",
        "next tomorrow",
        "+2 days ago",
        "2 days 09:30",
    ] {
        assert!(Date::parse(unread).is_err(), "{unread:?} should be refused");
    }
}

#[test]
fn absolute_forms_read_as_written() {
    let _clock = friday_noon();
    assert_eq!(
        parsed("2026-03-04T05:06:07+02:00"),
        utc("2026-03-04T03:06:07Z")
    );
    assert_eq!(parsed("2026-03-04T05:06:07Z"), utc("2026-03-04T05:06:07Z"));
    assert_eq!(parsed("2026-03-04"), utc("2026-03-04T00:00:00Z"));
    assert_eq!(parsed("2026-03-04 05:06"), utc("2026-03-04T05:06:00Z"));
    assert_eq!(parsed("2026-03-04T05:06"), utc("2026-03-04T05:06:00Z"));
    assert_eq!(parsed("2026-03-04 05:06:07"), utc("2026-03-04T05:06:07Z"));
    assert_eq!(
        parsed("2026-03-04T05:06:07.250"),
        Utc.with_ymd_and_hms(2026, 3, 4, 5, 6, 7).unwrap() + chrono::Duration::milliseconds(250)
    );
    assert_eq!(parsed("@0"), utc("1970-01-01T00:00:00Z"));
    assert_eq!(parsed("@1791547200"), utc("2026-10-09T12:00:00Z"));
    assert_eq!(parsed("@-60"), utc("1969-12-31T23:59:00Z"));
}

#[test]
fn the_words_set_their_time_of_day() {
    let _clock = friday_noon();
    assert_eq!(parsed("now"), utc("2026-10-09T12:00:00Z"));
    assert_eq!(parsed("today"), utc("2026-10-09T00:00:00Z"));
    assert_eq!(parsed("midnight"), utc("2026-10-09T00:00:00Z"));
    assert_eq!(parsed("yesterday"), utc("2026-10-08T00:00:00Z"));
    assert_eq!(parsed("noon"), utc("2026-10-09T12:00:00Z"));
    assert_eq!(parsed("  Tomorrow  "), utc("2026-10-10T00:00:00Z"));
}

#[test]
fn relative_forms_keep_the_time_of_day() {
    let _clock = friday_noon();
    assert_eq!(parsed("+1 second"), utc("2026-10-09T12:00:01Z"));
    assert_eq!(parsed("-30 minutes"), utc("2026-10-09T11:30:00Z"));
    assert_eq!(parsed("5 hours"), utc("2026-10-09T17:00:00Z"));
    assert_eq!(parsed("1 day ago"), utc("2026-10-08T12:00:00Z"));
    assert_eq!(parsed("+1 week"), utc("2026-10-16T12:00:00Z"));
    assert_eq!(parsed("2 weeks ago"), utc("2026-09-25T12:00:00Z"));
    assert_eq!(parsed("+1 fortnight"), utc("2026-10-23T12:00:00Z"));
    assert_eq!(parsed("+3 months"), utc("2027-01-09T12:00:00Z"));
    assert_eq!(parsed("1 year ago"), utc("2025-10-09T12:00:00Z"));
    assert_eq!(parsed("+10 Years"), utc("2036-10-09T12:00:00Z"));
}

#[test]
fn a_month_past_the_end_of_the_next_month_overflows_as_php_does() {
    let _clock = TestClock::travel_to(utc("2026-01-31T08:00:00Z"));
    assert_eq!(parsed("+1 month"), utc("2026-03-03T08:00:00Z"));
}

#[test]
fn weekday_forms_land_on_midnight() {
    let _clock = friday_noon();
    assert_eq!(
        parsed("friday"),
        utc("2026-10-09T00:00:00Z"),
        "today when it is that day"
    );
    assert_eq!(parsed("monday"), utc("2026-10-12T00:00:00Z"));
    assert_eq!(parsed("next friday"), utc("2026-10-16T00:00:00Z"));
    assert_eq!(parsed("last friday"), utc("2026-10-02T00:00:00Z"));
    assert_eq!(parsed("last monday"), utc("2026-10-05T00:00:00Z"));
    assert_eq!(parsed("Sunday"), utc("2026-10-11T00:00:00Z"));
}

#[test]
fn first_and_last_day_forms_keep_the_time_of_day() {
    let _clock = friday_noon();
    assert_eq!(
        parsed("first day of this month"),
        utc("2026-10-01T12:00:00Z")
    );
    assert_eq!(
        parsed("last day of this month"),
        utc("2026-10-31T12:00:00Z")
    );
    assert_eq!(
        parsed("last day of next month"),
        utc("2026-11-30T12:00:00Z")
    );
    assert_eq!(
        parsed("first day of last month"),
        utc("2026-09-01T12:00:00Z")
    );
    assert_eq!(
        parsed("last day of last month"),
        utc("2026-09-30T12:00:00Z")
    );
}

#[test]
fn a_word_form_followed_by_a_time_takes_that_time() {
    let _clock = friday_noon();
    assert_eq!(parsed("today 18:45"), utc("2026-10-09T18:45:00Z"));
    assert_eq!(parsed("now 07:05"), utc("2026-10-09T07:05:00Z"));
    assert_eq!(parsed("noon 13:00"), utc("2026-10-09T13:00:00Z"));
    assert_eq!(parsed("yesterday 9:15"), utc("2026-10-08T09:15:00Z"));
    assert_eq!(parsed("next monday 08:00"), utc("2026-10-12T08:00:00Z"));
    assert_eq!(
        parsed("last day of this month 23:59"),
        utc("2026-10-31T23:59:00Z")
    );
}

#[test]
fn parse_in_reads_the_same_forms_in_a_time_zone() {
    // 12:00 UTC is 14:00 in Paris on 2026-10-09 (summer time, +02:00).
    let _clock = friday_noon();
    let paris: Tz = "Europe/Paris".parse().expect("a zone");
    let tomorrow = Date::parse_in("tomorrow", paris).expect("parses");
    assert_eq!(tomorrow.with_timezone(&Utc), utc("2026-10-09T22:00:00Z"));
    assert_eq!(tomorrow.timezone(), paris);
    let local = Date::parse_in("2026-10-09 09:30", paris).expect("parses");
    assert_eq!(local.with_timezone(&Utc), utc("2026-10-09T07:30:00Z"));
    let offset = Date::parse_in("2026-10-09T09:30:00Z", paris).expect("parses");
    assert_eq!(offset.with_timezone(&Utc), utc("2026-10-09T09:30:00Z"));
    let ahead = Date::parse_in("+2 hours", paris).expect("parses");
    assert_eq!(ahead.with_timezone(&Utc), utc("2026-10-09T14:00:00Z"));
    // The day after 2026-10-24 crosses the end of summer time, and a day
    // keeps the wall-clock time.
    let _later = TestClock::travel_to(utc("2026-10-24T10:00:00Z"));
    let next_day = Date::parse_in("+1 day", paris).expect("parses");
    assert_eq!(next_day.with_timezone(&Utc), utc("2026-10-25T11:00:00Z"));
    assert!(Date::parse_in("soonish", paris).is_err());
}

#[test]
fn parse_in_moves_a_skipped_local_time_forward_by_the_skip() {
    let _clock = friday_noon();
    let paris: Tz = "Europe/Paris".parse().expect("a zone");
    // Paris skips 02:00 to 03:00 on 2026-03-29.
    let skipped = Date::parse_in("2026-03-29 02:30", paris).expect("parses");
    assert_eq!(skipped.with_timezone(&Utc), utc("2026-03-29T01:30:00Z"));
    // 02:30 happens twice on 2026-10-25; the earlier one is chosen.
    let repeated = Date::parse_in("2026-10-25 02:30", paris).expect("parses");
    assert_eq!(repeated.with_timezone(&Utc), utc("2026-10-25T00:30:00Z"));
}

#[test]
fn raw_parse_equals_parse() {
    let _clock = friday_noon();
    for text in [
        "tomorrow",
        "+2 days",
        "2026-03-04",
        "@0",
        "next monday 08:00",
    ] {
        assert_eq!(Date::raw_parse(text).unwrap(), parsed(text), "{text}");
    }
    assert!(Date::raw_parse("soonish").is_err());
}
