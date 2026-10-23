//! PAR-189 on the in-process drivers: `run`, `run_named`, `with_timeout`,
//! `driver`, the registry, and the `tokio` and `sync` drivers.

use crate::support::CRAFTED_NAME;
use crate::tasks::{
    Answer, FailWith, Greeting, Nap, NapThenEcho, PanicOnPurpose, PidThenNap, WritePidThenNap,
};
use serial_test::serial;
use std::time::{Duration, Instant};
use suprnova::concurrency::{Task, TaskFuture, TaskResult};
use suprnova::{Concurrency, ConcurrencyDriver, ConcurrentTask, FrameworkError, async_trait};

fn values(results: &[TaskResult]) -> Vec<String> {
    results
        .iter()
        .map(|result| result.get::<String>().expect("a string"))
        .collect()
}

#[tokio::test]
async fn tokio_runs_two_one_second_tasks_side_by_side_and_answers_in_order() {
    let started = Instant::now();
    let results = Concurrency::driver("tokio")
        .run([
            Task::new::<NapThenEcho>(Nap::new(1000, "first")),
            Task::new::<NapThenEcho>(Nap::new(1000, "second")),
        ])
        .await
        .expect("both tasks succeed");
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_secs(2),
        "side by side: {elapsed:?}"
    );
    assert!(
        elapsed >= Duration::from_secs(1),
        "each one sleeps: {elapsed:?}"
    );
    assert_eq!(values(&results), ["first", "second"]);
}

#[tokio::test]
async fn sync_runs_two_one_second_tasks_one_after_another() {
    let started = Instant::now();
    let results = Concurrency::driver("sync")
        .run([
            Task::new::<NapThenEcho>(Nap::new(1000, "first")),
            Task::new::<NapThenEcho>(Nap::new(1000, "second")),
        ])
        .await
        .expect("both tasks succeed");
    let elapsed = started.elapsed();

    assert!(
        elapsed >= Duration::from_secs(2),
        "in sequence: {elapsed:?}"
    );
    assert_eq!(values(&results), ["first", "second"]);
}

#[tokio::test]
async fn results_follow_the_order_of_the_tasks_not_the_order_they_finish() {
    let results = Concurrency::driver("tokio")
        .run([
            Task::new::<NapThenEcho>(Nap::new(300, "slow")),
            Task::new::<NapThenEcho>(Nap::new(10, "fast")),
        ])
        .await
        .expect("both tasks succeed");

    assert_eq!(values(&results), ["slow", "fast"]);
    assert_eq!(results[0].name(), NapThenEcho::NAME);
}

#[tokio::test]
async fn run_named_answers_each_result_under_its_key_on_tokio_and_sync() {
    for driver in ["tokio", "sync"] {
        let results = Concurrency::driver(driver)
            .run_named([
                ("users", Task::new::<NapThenEcho>(Nap::new(50, "u"))),
                ("orders", Task::new::<NapThenEcho>(Nap::new(0, "o"))),
                ("answer", Task::new::<Answer>(())),
            ])
            .await
            .expect("every task succeeds");

        let keys: Vec<&str> = results.keys().map(String::as_str).collect();
        assert_eq!(keys, ["users", "orders", "answer"], "{driver}");
        assert_eq!(
            results["users"].get::<String>().expect("u"),
            "u",
            "{driver}"
        );
        assert_eq!(
            results["orders"].get::<String>().expect("o"),
            "o",
            "{driver}"
        );
        assert_eq!(results["answer"].get::<u64>().expect("42"), 42, "{driver}");
    }
}

#[tokio::test]
async fn run_named_refuses_a_key_that_names_two_tasks() {
    let error = Concurrency::run_named([
        ("same", Task::new::<Answer>(())),
        ("same", Task::new::<Answer>(())),
    ])
    .await
    .expect_err("two tasks under one key");

    assert!(error.to_string().contains("`same`"), "{error}");
}

#[tokio::test]
async fn a_tokio_task_past_its_timeout_is_ended_with_an_error_naming_it() {
    let dir = tempfile::tempdir().expect("a directory");
    let started = Instant::now();
    let error = Concurrency::driver("tokio")
        .with_timeout(Duration::from_millis(300))
        .run([
            Task::new::<NapThenEcho>(Nap::new(0, "quick")),
            Task::new::<WritePidThenNap>(PidThenNap {
                pid_file: dir.path().join("pid"),
                millis: 10_000,
            }),
        ])
        .await
        .expect_err("ten seconds is past the timeout");

    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    assert!(error.is_timeout(), "{error:?}");
    assert!(error.to_string().contains(WritePidThenNap::NAME), "{error}");
}

#[tokio::test]
async fn a_sync_task_past_its_timeout_is_ended_with_an_error_naming_it() {
    let started = Instant::now();
    let error = Concurrency::driver("sync")
        .with_timeout(Duration::from_millis(300))
        .run([Task::new::<NapThenEcho>(Nap::new(10_000, "late"))])
        .await
        .expect_err("ten seconds is past the timeout");

    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    assert!(error.is_timeout(), "{error:?}");
    assert!(error.to_string().contains(NapThenEcho::NAME), "{error}");
}

#[tokio::test]
async fn a_zero_timeout_is_no_timeout() {
    let results = Concurrency::with_timeout(Duration::ZERO)
        .driver("tokio")
        .run([Task::new::<NapThenEcho>(Nap::new(50, "done"))])
        .await
        .expect("no timeout");
    assert_eq!(values(&results), ["done"]);
}

#[tokio::test]
async fn a_failing_task_fails_the_run_with_its_own_error_after_the_others_end() {
    for driver in ["tokio", "sync"] {
        let error = Concurrency::driver(driver)
            .run([
                Task::new::<NapThenEcho>(Nap::new(10, "fine")),
                Task::new::<FailWith>("the report broke".to_owned()),
            ])
            .await
            .expect_err("the second task fails");
        assert!(
            error.to_string().contains("the report broke"),
            "{driver}: {error}"
        );
    }
}

#[tokio::test]
async fn a_panicking_task_is_an_error_naming_it() {
    for driver in ["tokio", "sync"] {
        let error = Concurrency::driver(driver)
            .run([Task::new::<PanicOnPurpose>(())])
            .await
            .expect_err("the task panicked");
        let text = error.to_string();
        assert!(text.contains(PanicOnPurpose::NAME), "{driver}: {text}");
        assert!(text.contains("panicked"), "{driver}: {text}");
    }
}

#[tokio::test]
async fn an_unregistered_task_name_fails_in_process_naming_it() {
    for driver in ["tokio", "sync"] {
        let error = Concurrency::driver(driver)
            .run([Task::named("concurrency-tests:nowhere", ())])
            .await
            .expect_err("no such task");
        assert!(
            error.to_string().contains("concurrency-tests:nowhere"),
            "{driver}: {error}"
        );
    }
}

#[tokio::test]
async fn a_named_task_with_a_control_character_in_its_name_fails_in_process_naming_it() {
    for driver in ["tokio", "sync"] {
        let error = Concurrency::driver(driver)
            .run([Task::named(CRAFTED_NAME, ())])
            .await
            .expect_err("the name carries line breaks");
        let text = error.to_string();
        assert!(text.contains("Task::named"), "{driver}: {text}");
        assert!(
            text.contains(&format!("{CRAFTED_NAME:?}")),
            "{driver}: {text}"
        );
        assert!(
            !text.contains('\n'),
            "{driver}: the name is escaped: {text}"
        );
    }
}

#[test]
fn try_named_refuses_an_empty_name_and_every_control_character_naming_itself() {
    let error = Task::try_named(CRAFTED_NAME, ()).expect_err("the name carries line breaks");
    let text = error.to_string();
    assert!(text.contains("Task::try_named"), "{text}");
    assert!(text.contains(&format!("{CRAFTED_NAME:?}")), "{text}");
    assert!(!text.contains('\n'), "the name is escaped: {text}");

    for name in [
        "",
        "line\nbreak",
        "carriage\rreturn",
        "a\ttab",
        "nul\0",
        "escape\u{1b}[31m",
        "delete\u{7f}",
        "next-line\u{85}",
    ] {
        let error = Task::try_named(name, ()).expect_err("the name is refused");
        let text = error.to_string();
        assert!(text.contains("Task::try_named"), "{name:?}: {text}");
        assert!(text.contains(&format!("{name:?}")), "{name:?}: {text}");
    }
}

#[tokio::test]
async fn try_named_builds_a_task_with_a_plain_name_that_runs() {
    let task = Task::try_named("concurrency-tests:greet", "ada").expect("a plain name");
    assert_eq!(task.name(), "concurrency-tests:greet");
    let results = Concurrency::driver("sync")
        .run([task])
        .await
        .expect("the task is registered");
    assert_eq!(results[0].get::<String>().expect("text"), "hello ada");
}

#[test]
fn try_named_refuses_an_input_that_does_not_serialize_naming_the_task() {
    // JSON object keys are strings; a tuple key cannot become one.
    let input = std::collections::BTreeMap::from([((1_u8, 2_u8), 3_u8)]);
    let error = Task::try_named("concurrency-tests:greet", input).expect_err("not JSON");
    let text = error.to_string();
    assert!(text.contains("concurrency-tests:greet"), "{text}");
    assert!(text.contains("could not be serialized"), "{text}");
}

#[test]
fn every_registered_task_name_is_one_try_named_accepts() {
    // The macro's default names are module paths; none carries a control
    // character, so every registered task can be named.
    let registered = suprnova::concurrency::registered();
    assert!(!registered.is_empty());
    for name in registered {
        if let Err(error) = Task::try_named(name, ()) {
            panic!("{name:?}: {error}");
        }
    }
}

/// A task implemented by hand, with a name the macro would refuse.
struct HandWritten;

impl ConcurrentTask for HandWritten {
    type Input = ();
    type Output = ();
    const NAME: &'static str = "hand\nwritten";

    fn call(_input: ()) -> TaskFuture<()> {
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn a_hand_written_task_name_with_a_control_character_fails_naming_the_call() {
    let error = Concurrency::driver("sync")
        .run([Task::new::<HandWritten>(())])
        .await
        .expect_err("the name carries a line break");
    let text = error.to_string();
    assert!(text.contains("Task::new"), "{text}");
    assert!(text.contains(r#""hand\nwritten""#), "{text}");
}

#[tokio::test]
async fn an_unknown_driver_is_an_error_naming_it() {
    let error = Concurrency::driver("fork")
        .run([Task::new::<Answer>(())])
        .await
        .expect_err("there is no fork driver");
    assert!(error.to_string().contains("`fork`"), "{error}");

    let error = Concurrency::driver("fork")
        .defer([Task::new::<Answer>(())])
        .expect_err("defer resolves the driver at the call");
    assert!(error.to_string().contains("`fork`"), "{error}");
}

#[tokio::test]
async fn the_registered_name_is_the_module_path_unless_one_is_given() {
    assert_eq!(NapThenEcho::NAME, "concurrency::tasks::nap_then_echo");
    assert_eq!(Greeting::NAME, "concurrency-tests:greet");
    let registered = suprnova::concurrency::registered();
    assert!(
        registered.contains(&"concurrency::tasks::nap_then_echo"),
        "{registered:?}"
    );
    assert!(
        registered.contains(&"concurrency-tests:greet"),
        "{registered:?}"
    );

    let results = Concurrency::driver("tokio")
        .run([
            Task::named("concurrency-tests:greet", "ada"),
            Task::new::<Answer>(()),
        ])
        .await
        .expect("both are registered");
    assert_eq!(results[0].get::<String>().expect("text"), "hello ada");
    assert_eq!(results[1].get::<u64>().expect("a number"), 42);
}

#[tokio::test]
async fn a_result_read_as_another_type_is_an_error_naming_the_task() {
    let results = Concurrency::driver("sync")
        .run([Task::new::<Answer>(())])
        .await
        .expect("the task succeeds");
    let error = results[0]
        .get::<Vec<String>>()
        .expect_err("a number is not a list");
    assert!(error.to_string().contains(Answer::NAME), "{error}");
}

#[tokio::test]
async fn an_input_that_does_not_decode_as_the_tasks_input_is_an_error() {
    let error = Concurrency::driver("tokio")
        .run([Task::named(NapThenEcho::NAME, "not a nap")])
        .await
        .expect_err("a string is not a Nap");
    assert!(error.to_string().contains(NapThenEcho::NAME), "{error}");
}

#[tokio::test]
async fn no_tasks_answer_no_results() {
    let results = Concurrency::driver("tokio")
        .run(Vec::new())
        .await
        .expect("nothing to run");
    assert!(results.is_empty());
}

/// A driver that answers each task's name instead of running it.
struct NameDriver;

#[async_trait]
impl ConcurrencyDriver for NameDriver {
    async fn run(
        &self,
        tasks: Vec<Task>,
        _timeout: Option<Duration>,
    ) -> Vec<Result<TaskResult, FrameworkError>> {
        tasks
            .into_iter()
            .map(|task| {
                Ok(TaskResult::new(
                    task.name(),
                    serde_json::Value::String(task.name().to_owned()),
                ))
            })
            .collect()
    }
}

/// A driver that loses a task.
struct LosingDriver;

#[async_trait]
impl ConcurrencyDriver for LosingDriver {
    async fn run(
        &self,
        _tasks: Vec<Task>,
        _timeout: Option<Duration>,
    ) -> Vec<Result<TaskResult, FrameworkError>> {
        Vec::new()
    }
}

#[tokio::test]
#[serial]
async fn extend_adds_a_driver_the_facade_selects_by_name() {
    Concurrency::extend("names", NameDriver).expect("registered");
    let results = Concurrency::driver("names")
        .run([Task::new::<Answer>(())])
        .await
        .expect("the driver answers");
    assert_eq!(results[0].get::<String>().expect("text"), Answer::NAME);

    Concurrency::extend("losing", LosingDriver).expect("registered");
    let error = Concurrency::driver("losing")
        .run([Task::new::<Answer>(())])
        .await
        .expect_err("one outcome per task");
    assert!(error.to_string().contains("`losing`"), "{error}");
}

#[tokio::test]
#[tracing_test::traced_test]
async fn a_deferred_task_with_a_refused_name_is_logged_with_the_name_escaped() {
    Concurrency::driver("sync")
        .defer([Task::named(CRAFTED_NAME, ())])
        .expect("the driver exists")
        .await;

    assert!(logs_contain("deferred concurrent task failed"));
    // The whole name on the one log line, its line breaks escaped: a name
    // logged as it is would start a line of its own that reads as an
    // outcome.
    assert!(
        logs_contain(&format!("task={CRAFTED_NAME:?}")),
        "the task name is logged in its escaped form"
    );
}
