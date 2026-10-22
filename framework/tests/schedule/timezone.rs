//! Per-task schedule timezones: due-ness is evaluated in the task's zone;
//! schedule:list converts expressions into a display zone with the #60877
//! expand/shift/collapse algorithm, refusing across DST boundaries.

use chrono::{Datelike, TimeZone, Timelike};
use chrono_tz::Tz;
use suprnova::{CronExpression, Schedule};

#[test]
fn task_timezone_drives_due_ness() {
    // 03:00 in Tokyo is 18:00 UTC the previous day.
    let tokyo: Tz = "Asia/Tokyo".parse().expect("tz");
    let expr = CronExpression::parse("0 3 * * *").expect("cron");

    let due_in_tokyo = tokyo
        .with_ymd_and_hms(2026, 5, 28, 3, 0, 0)
        .single()
        .expect("clock");
    assert!(expr.is_due_at(due_in_tokyo));

    let mut schedule = Schedule::new();
    schedule.add(
        schedule
            .call(|| async { Ok(()) })
            .name("tokyo-daily")
            .daily()
            .at("03:00")
            .timezone(tokyo),
    );
    let entry = schedule.find("tokyo-daily").expect("registered");
    assert_eq!(entry.timezone, Some(tokyo));
}

#[test]
fn try_timezone_rejects_unknown_zones() {
    let schedule = Schedule::new();
    // `.err()` rather than `expect_err`: the `Ok` half is a `TaskBuilder`,
    // which holds a boxed task handler and so cannot implement `Debug`.
    let err = schedule
        .call(|| async { Ok(()) })
        .try_timezone("Mars/Olympus_Mons")
        .err()
        .expect("unknown zone");
    assert!(err.contains("Mars/Olympus_Mons"));
}

#[test]
fn next_run_after_finds_the_next_due_minute() {
    let expr = CronExpression::parse("30 2 * * *").expect("cron");
    let after = chrono::Utc
        .with_ymd_and_hms(2026, 5, 28, 2, 31, 0)
        .single()
        .expect("clock");
    let next = expr.next_run_after(after).expect("satisfiable");
    assert_eq!((next.hour(), next.minute()), (2, 30));
    assert_eq!(
        next.day(),
        29,
        "2:31 is past today's 2:30; next run is tomorrow"
    );
}

#[test]
fn unsatisfiable_expression_yields_none() {
    let expr = CronExpression::parse("0 0 30 2 *").expect("parses"); // Feb 30
    let after = chrono::Utc
        .with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
        .single()
        .expect("clock");
    assert!(expr.next_run_after(after).is_none());
}
