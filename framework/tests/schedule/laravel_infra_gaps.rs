//! The scheduling surface of the Laravel infrastructure gaps (PAR-143):
//! single-server tasks with no cache store, the day helpers, the time
//! forms, and the environment and maintenance-mode filters.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use suprnova::testing::TestContainer;
use suprnova::{CronExpression, DayOfWeek, Schedule, TaskBuilder};

/// A distinct task name per test: the election lock key holds the name,
/// and the tests of this binary run at once.
fn unique_name(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4())
}

/// The cron expression `configure` leaves on a fresh task.
fn expression_of(configure: impl FnOnce(TaskBuilder) -> TaskBuilder) -> String {
    let mut schedule = Schedule::new();
    let task = configure(schedule.call(|| async { Ok(()) })).name(&unique_name("days"));
    schedule.add(task);
    schedule.tasks()[0].schedule_description().to_owned()
}

// ---------------------------------------------------------------------------
// One server, no cache store
// ---------------------------------------------------------------------------

#[test]
fn a_single_server_task_with_no_cache_store_refuses_to_start() {
    let _container = TestContainer::fake();
    let name = unique_name("billing");
    let mut schedule = Schedule::new();
    let task = schedule
        .call(|| async { Ok(()) })
        .name(&name)
        .every_minute()
        .on_one_server();
    schedule.add(task);

    let error = schedule
        .validate_single_server_locking()
        .expect_err("no cache store is bound, so no server can be elected");
    assert!(
        error.message().contains(&name),
        "the error names the task: {}",
        error.message()
    );
}

#[test]
fn a_schedule_without_single_server_tasks_needs_no_cache_store() {
    let _container = TestContainer::fake();
    let mut schedule = Schedule::new();
    let task = schedule
        .call(|| async { Ok(()) })
        .name(&unique_name("ordinary"))
        .every_minute();
    schedule.add(task);

    assert!(schedule.validate_single_server_locking().is_ok());
}

#[tokio::test]
async fn a_tick_with_no_cache_store_skips_a_single_server_task() {
    let _container = TestContainer::fake();
    let runs = Arc::new(AtomicUsize::new(0));
    let name = unique_name("one-server");

    for _replica in 0..2 {
        let mut schedule = Schedule::new();
        let counter = Arc::clone(&runs);
        let task = schedule
            .call(move || {
                let counter = Arc::clone(&counter);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            })
            .name(&name)
            .every_minute()
            .on_one_server();
        schedule.add(task);

        let results = schedule.run_due_tasks().await;
        assert_eq!(results.len(), 1);
        assert!(results[0].1.is_ok(), "a skipped tick is no failure");
        assert_eq!(
            schedule.tasks()[0].state.skip_count(),
            1,
            "the tick counts as skipped"
        );
    }

    assert_eq!(
        runs.load(Ordering::SeqCst),
        0,
        "with no cache store there is no election, and no server runs the task"
    );
}

// ---------------------------------------------------------------------------
// Days
// ---------------------------------------------------------------------------

#[test]
fn days_replace_only_the_day_of_week_field() {
    assert_eq!(
        expression_of(|t| t.hourly().days(&[DayOfWeek::Monday])),
        "0 * * * 1"
    );
    assert_eq!(
        expression_of(|t| t.daily().at("09:30").mondays()),
        "30 9 * * 1"
    );
    assert_eq!(
        expression_of(|t| t.daily_at("13:00").weekdays()),
        "0 13 * * 1-5"
    );
    assert_eq!(
        expression_of(|t| t.cron("*/5 * * * *").weekends()),
        "*/5 * * * 0,6"
    );
    assert_eq!(
        expression_of(|t| t.monthly_on(15).at("08:00").fridays()),
        "0 8 15 * 5"
    );
    assert_eq!(
        expression_of(|t| t
            .every_minute()
            .days(&[DayOfWeek::Tuesday, DayOfWeek::Thursday])),
        "* * * * 2,4"
    );
}

#[test]
fn a_day_helper_alone_keeps_its_midnight() {
    assert_eq!(expression_of(|t| t.mondays()), "0 0 * * 1");
    assert_eq!(expression_of(|t| t.weekdays()), "0 0 * * 1-5");
    assert_eq!(
        expression_of(|t| t.days(&[DayOfWeek::Sunday, DayOfWeek::Saturday])),
        "0 0 * * 0,6"
    );
    assert_eq!(
        expression_of(|t| t.mondays().at("09:00")),
        "0 9 * * 1",
        "a time after the day keeps the day"
    );
    assert_eq!(
        expression_of(|t| t.mondays().at("09:00").tuesdays()),
        "0 9 * * 2",
        "a later day replaces the earlier one and keeps the time"
    );
}

#[test]
fn with_days_of_week_splices_the_fifth_field() {
    let expression = CronExpression::parse("15 3 1 * *")
        .expect("a valid expression")
        .with_days_of_week(&[DayOfWeek::Wednesday, DayOfWeek::Friday]);
    assert_eq!(expression.expression(), "15 3 1 * 3,5");
    assert_eq!(
        CronExpression::hourly().with_days_of_week(&[]).expression(),
        "0 * * * *",
        "no day keeps the expression"
    );
}

// ---------------------------------------------------------------------------
// Times
// ---------------------------------------------------------------------------

#[test]
fn one_segment_is_the_hour_on_the_hour() {
    assert_eq!(
        CronExpression::monthly_on(15).at("9").expression(),
        "0 9 15 * *"
    );
    assert_eq!(CronExpression::daily_at("9").expression(), "0 9 * * *");
    assert_eq!(
        CronExpression::try_daily_at("9")
            .expect("one segment")
            .expression(),
        "0 9 * * *"
    );
    assert_eq!(
        CronExpression::daily()
            .try_at("17")
            .expect("one segment")
            .expression(),
        "0 17 * * *"
    );
    assert_eq!(expression_of(|t| t.daily().at("9")), "0 9 * * *");
}

#[test]
fn three_segments_are_the_hour_and_the_minute() {
    assert_eq!(
        CronExpression::monthly_on(15).at("09:30:00").expression(),
        "30 9 15 * *"
    );
    assert_eq!(
        CronExpression::daily_at("09:30:45").expression(),
        "30 9 * * *"
    );
    assert_eq!(
        CronExpression::daily()
            .try_at("14:30:00")
            .expect("three segments")
            .expression(),
        "30 14 * * *"
    );
    assert_eq!(expression_of(|t| t.daily_at("23:59:59")), "59 23 * * *");
}

#[test]
fn a_non_numeric_segment_keeps_its_old_outcome() {
    assert!(CronExpression::daily().try_at("ab").is_err());
    assert!(CronExpression::daily().try_at("9:cd:00").is_err());
    assert!(CronExpression::daily().try_at("9:30:xx").is_err());
    assert!(
        CronExpression::daily().try_at("1:2:3:4").is_err(),
        "four segments are no time"
    );
    assert_eq!(
        CronExpression::daily().at("ab").expression(),
        "0 0 * * *",
        "`at` warns and keeps the expression"
    );
    assert_eq!(CronExpression::daily_at("ab").expression(), "0 0 * * *");
    assert_eq!(
        CronExpression::try_daily_at("ab:30:00")
            .expect("lenient")
            .expression(),
        "30 0 * * *"
    );
}

// ---------------------------------------------------------------------------
// Environments and maintenance mode
// ---------------------------------------------------------------------------

/// The scenario [`filters_child`] runs.
const FILTERS_CHILD: &str = "SUPRNOVA_INFRA_SCHEDULE_FILTERS_CHILD";
/// What [`filters_child`] prints before the names of the tasks that ran.
const RAN: &str = "ran=";

#[test]
fn a_task_reads_its_environments_and_maintenance_choice() {
    let mut schedule = Schedule::new();
    let limited = schedule
        .call(|| async { Ok(()) })
        .name(&unique_name("limited"))
        .environments([suprnova::Environment::Production])
        .even_in_maintenance_mode();
    schedule.add(limited);
    let open = schedule
        .call(|| async { Ok(()) })
        .name(&unique_name("open"));
    schedule.add(open);

    let [limited, open] = schedule.tasks() else {
        panic!("two tasks");
    };
    assert_eq!(limited.environments(), [suprnova::Environment::Production]);
    assert!(limited.runs_in_environment(&suprnova::Environment::Production));
    assert!(!limited.runs_in_environment(&suprnova::Environment::Local));
    assert!(limited.runs_in_maintenance_mode());
    assert!(open.environments().is_empty());
    assert!(open.runs_in_environment(&suprnova::Environment::Staging));
    assert!(!open.runs_in_maintenance_mode());
}

/// Run `scenario` of [`filters_child`] with `env` set, and return the
/// names of the tasks it ran.
fn ran_in_child(scenario: &str, env: &[(&str, &str)]) -> Vec<String> {
    let base = tempfile::tempdir().expect("a base path");
    let mut child = std::process::Command::new(std::env::current_exe().expect("the test binary"));
    child
        .args([
            "--exact",
            "laravel_infra_gaps::filters_child",
            "--nocapture",
        ])
        .env(FILTERS_CHILD, scenario)
        .env("APP_BASE_PATH", base.path())
        .env_remove("MAINTENANCE_DRIVER")
        .env_remove("APP_ENV");
    for (key, value) in env {
        child.env(key, value);
    }
    let output = child.output().expect("run the child");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let line = stdout
        .lines()
        .find_map(|line| line.strip_prefix(RAN))
        .unwrap_or_else(|| panic!("the child printed no result: {stdout}"));
    line.split(',')
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The child half of the environment and maintenance tests: one schedule
/// of three every-minute tasks, `production-only`, `in-maintenance` and
/// `plain`, run once, and the names of the tasks that ran.
#[test]
fn filters_child() {
    let Ok(scenario) = std::env::var(FILTERS_CHILD) else {
        return;
    };
    suprnova::boot::load_env().expect("load the configuration");
    let _container = TestContainer::fake();
    if scenario != "unreadable" {
        TestContainer::bind::<dyn suprnova::CacheStore>(Arc::new(suprnova::InMemoryCache::new()));
    }
    let ran = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime")
        .block_on(async {
            if scenario == "down" {
                suprnova::maintenance_mode()
                    .activate(&suprnova::MaintenancePayload::new())
                    .await
                    .expect("go down");
            }
            let names = Arc::new(std::sync::Mutex::new(Vec::new()));
            let mut schedule = Schedule::new();
            for (name, configure) in [
                (
                    "production-only",
                    (|t: TaskBuilder| t.environments([suprnova::Environment::Production]))
                        as fn(TaskBuilder) -> TaskBuilder,
                ),
                ("in-maintenance", |t: TaskBuilder| {
                    t.even_in_maintenance_mode()
                }),
                ("plain", |t: TaskBuilder| t),
            ] {
                let names = Arc::clone(&names);
                let task = schedule
                    .call(move || {
                        let names = Arc::clone(&names);
                        async move {
                            names
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .push(name.to_owned());
                            Ok(())
                        }
                    })
                    .every_minute()
                    .name(name);
                schedule.add(configure(task));
            }
            for (name, result) in schedule.run_due_tasks().await {
                result.unwrap_or_else(|e| panic!("{name}: {}", e.message()));
            }
            let mut ran = names.lock().unwrap_or_else(|p| p.into_inner()).clone();
            ran.sort();
            ran
        });
    println!("{RAN}{}", ran.join(","));
}

#[test]
fn a_task_limited_to_production_does_not_run_in_local() {
    let ran = ran_in_child("up", &[("APP_ENV", "local")]);
    assert_eq!(ran, ["in-maintenance", "plain"]);

    let ran = ran_in_child("up", &[("APP_ENV", "production")]);
    assert_eq!(ran, ["in-maintenance", "plain", "production-only"]);
}

#[test]
fn in_maintenance_mode_only_the_tasks_that_ask_run() {
    let ran = ran_in_child(
        "down",
        &[("APP_ENV", "production"), ("MAINTENANCE_DRIVER", "cache")],
    );
    assert_eq!(ran, ["in-maintenance"]);
}

#[test]
fn a_maintenance_state_that_cannot_be_read_counts_as_down() {
    // `MAINTENANCE_DRIVER=cache` with no cache store bound: the state read
    // fails.
    let ran = ran_in_child(
        "unreadable",
        &[("APP_ENV", "production"), ("MAINTENANCE_DRIVER", "cache")],
    );
    assert_eq!(ran, ["in-maintenance"]);
}
