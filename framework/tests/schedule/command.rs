//! `Schedule::command`: a console command on the schedule, given the way
//! it is typed.
//!
//! The tests share the record of what ran, so each one uses a tag of its
//! own and looks for that tag alone.

use std::sync::Mutex;

use async_trait::async_trait;
use clap::Parser;
use suprnova::{Command, FrameworkError, Schedule, TypedCommand, command};

static RAN: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn record(what: String) {
    RAN.lock().unwrap_or_else(|p| p.into_inner()).push(what);
}

fn times_ran(what: &str) -> usize {
    RAN.lock()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .filter(|ran| *ran == what)
        .count()
}

/// Record one run under a tag
#[derive(Parser, Command, Debug)]
#[console(name = "scheduled:record")]
struct Record {
    #[arg(long)]
    tag: String,

    /// End with an error after the run was recorded.
    #[arg(long)]
    fail: bool,
}

#[async_trait]
impl TypedCommand for Record {
    async fn run(self) -> Result<(), FrameworkError> {
        record(self.tag.clone());
        if self.fail {
            return Err(FrameworkError::internal(format!(
                "{} failed on purpose",
                self.tag
            )));
        }
        Ok(())
    }
}

#[command(
    name = "scheduled:words",
    description = "Record the words it was given"
)]
async fn words(args: Vec<String>) -> Result<(), FrameworkError> {
    record(format!("words: {}", args.join("|")));
    Ok(())
}

#[tokio::test]
async fn a_command_runs_with_the_arguments_of_its_line() {
    let mut schedule = Schedule::new();
    schedule.add(schedule.command("scheduled:record --tag nightly").daily());

    let result = schedule
        .run_task("scheduled:record --tag nightly")
        .await
        .expect("the task is named by its command line");

    result.expect("the command succeeds");
    assert_eq!(times_ran("nightly"), 1);
}

#[tokio::test]
async fn the_task_is_named_by_the_line_and_described_by_the_command() {
    let mut schedule = Schedule::new();
    schedule.add(
        schedule
            .command("  scheduled:record   --tag   listed ")
            .hourly(),
    );

    let entry = &schedule.tasks()[0];
    assert_eq!(
        entry.name, "scheduled:record --tag listed",
        "one space between the words, whatever the line had"
    );
    assert_eq!(
        entry.description.as_deref(),
        Some("Record one run under a tag"),
        "the about text of the command, which here is its doc comment"
    );
}

#[tokio::test]
async fn the_builder_is_the_builder_of_any_other_task() {
    let mut schedule = Schedule::new();
    schedule.add(
        schedule
            .command("scheduled:record --tag renamed")
            .daily()
            .at("03:00")
            .name("prune-at-night")
            .description("Runs at night")
            .without_overlapping(),
    );

    let entry = schedule.find("prune-at-night").expect("the name given");
    assert_eq!(entry.description.as_deref(), Some("Runs at night"));
    assert!(entry.without_overlapping);
    assert!(schedule.find("scheduled:record --tag renamed").is_none());

    schedule
        .run_task("prune-at-night")
        .await
        .expect("the task exists")
        .expect("the command succeeds");
    assert_eq!(times_ran("renamed"), 1);
}

#[tokio::test]
async fn a_quoted_argument_stays_one_argument() {
    let mut schedule = Schedule::new();
    schedule.add(
        schedule
            .command(r#"scheduled:record --tag "two words""#)
            .daily(),
    );
    schedule.add(
        schedule
            .command("scheduled:words first 'second and third' --flag")
            .daily(),
    );

    for task in schedule.tasks() {
        task.run().await.expect("the command succeeds");
    }

    assert_eq!(times_ran("two words"), 1);
    assert_eq!(
        times_ran("words: first|second and third|--flag"),
        1,
        "a command that takes its arguments as words gets them as they were split"
    );
}

#[tokio::test]
async fn the_error_of_the_command_is_the_result_of_the_task() {
    let mut schedule = Schedule::new();
    schedule.add(
        schedule
            .command("scheduled:record --tag failing --fail")
            .daily(),
    );

    let error = schedule
        .run_task("scheduled:record --tag failing --fail")
        .await
        .expect("the task exists")
        .expect_err("the command fails");

    assert_eq!(error.message(), "failing failed on purpose");
    assert_eq!(times_ran("failing"), 1, "it ran, and then it failed");
}

#[tokio::test]
async fn the_arguments_that_were_checked_at_boot_serve_every_run() {
    // The scheduler runs a task once in a minute, so this calls the
    // handler of the task, which is what the scheduler calls when the
    // next minute has come.
    let mut schedule = Schedule::new();
    schedule.add(schedule.command("scheduled:record --tag again").daily());
    let task = &schedule.tasks()[0].task;

    for _ in 0..3 {
        task.handle().await.expect("the command succeeds");
    }
    assert_eq!(times_ran("again"), 3);
}

#[test]
fn a_name_no_command_has_is_refused_when_the_schedule_is_built() {
    let schedule = Schedule::new();

    let error = schedule
        .try_command("scheduled:recrod --tag typo")
        .err()
        .expect("no command is named scheduled:recrod");

    assert!(
        error.message().contains("scheduled:recrod"),
        "the error must name what was asked for: {}",
        error.message()
    );
    assert!(
        error.message().contains("scheduled:record"),
        "the error must list the commands there are: {}",
        error.message()
    );
    assert_eq!(times_ran("typo"), 0);
}

#[test]
fn arguments_the_command_does_not_take_are_refused_when_the_schedule_is_built() {
    let schedule = Schedule::new();

    let unknown = schedule
        .try_command("scheduled:record --tag flags --forse")
        .err()
        .expect("--forse is not an option of the command");
    assert!(
        unknown.message().contains("--forse"),
        "{}",
        unknown.message()
    );

    let missing = schedule
        .try_command("scheduled:record")
        .err()
        .expect("--tag is required");
    assert!(missing.message().contains("--tag"), "{}", missing.message());

    assert!(
        schedule.try_command("scheduled:record --help").is_err(),
        "help is an answer for a person, and nothing to put on a schedule"
    );
}

#[test]
fn a_line_that_is_empty_or_does_not_end_is_refused() {
    let schedule = Schedule::new();

    for line in ["", "   ", "scheduled:record --tag 'open"] {
        assert!(
            schedule.try_command(line).is_err(),
            "`{line}` must be refused"
        );
    }
}

#[test]
#[should_panic(expected = "no console command is named `scheduled:missing`")]
fn command_stops_the_boot_where_try_command_returns_the_error() {
    let schedule = Schedule::new();
    let _ = schedule.command("scheduled:missing");
}
