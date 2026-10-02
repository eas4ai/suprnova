//! PAR-021: running a program with its arguments and capturing its output.

use serial_test::serial;
use std::sync::{Arc, Mutex};
use suprnova::{OutputKind, Process, ProcessError};

use crate::support::sh;

#[tokio::test]
#[serial]
async fn a_failing_exit_is_a_result_with_both_outputs() {
    let result = Process::command(sh("printf out; printf err >&2; exit 3"))
        .run()
        .await
        .expect("a nonzero exit is a result, not an error");

    assert_eq!(result.output(), "out");
    assert_eq!(result.error_output(), "err");
    assert_eq!(result.exit_code(), Some(3));
    assert!(result.failed());
    assert!(!result.successful());
}

#[tokio::test]
#[serial]
async fn a_successful_exit_is_successful() {
    let result = Process::command(["printf", "%s", "hello"])
        .run()
        .await
        .unwrap();
    assert!(result.successful());
    assert_eq!(result.exit_code(), Some(0));
    assert_eq!(result.output(), "hello");
}

#[tokio::test]
#[serial]
async fn arguments_reach_the_program_as_they_are_through_no_shell() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("marker");
    let argument = format!("; touch {}", marker.display());

    let result = Process::command(["printf", "%s", argument.as_str()])
        .run()
        .await
        .unwrap();

    assert_eq!(result.output(), argument, "the argument is printed whole");
    assert!(!marker.exists(), "no shell ran the argument");
}

#[tokio::test]
#[serial]
async fn throw_turns_a_failed_result_into_an_error_with_its_outputs() {
    let result = Process::command(sh("printf out; printf err >&2; exit 4"))
        .run()
        .await
        .unwrap();

    let error = result.throw().expect_err("a failed result throws");
    assert!(matches!(error, ProcessError::Failed { .. }), "{error:?}");
    let text = error.to_string();
    assert!(text.contains('4'), "the exit code: {text}");
    assert!(text.contains("out") && text.contains("err"), "{text}");

    let ok = Process::command(["true"]).run().await.unwrap();
    assert!(ok.throw().is_ok(), "a successful result does not throw");
}

#[tokio::test]
#[serial]
async fn path_env_and_input_reach_the_process() {
    let dir = tempfile::tempdir().unwrap();
    let result = Process::command(sh("pwd; printf '%s\\n' \"$SUPRNOVA_PROCESS_TEST\"; cat"))
        .path(dir.path())
        .env("SUPRNOVA_PROCESS_TEST", "from-env")
        .input("from-stdin")
        .run()
        .await
        .unwrap();

    let lines: Vec<&str> = result.output().lines().collect();
    let expected_dir = std::fs::canonicalize(dir.path()).unwrap();
    assert_eq!(
        std::fs::canonicalize(lines[0]).unwrap(),
        expected_dir,
        "the working directory"
    );
    assert_eq!(lines[1], "from-env");
    assert_eq!(lines[2], "from-stdin");
}

#[tokio::test]
#[serial]
async fn the_environment_is_inherited() {
    let result = Process::command(sh("printf '%s' \"$PATH\""))
        .run()
        .await
        .unwrap();
    assert!(!result.output().is_empty(), "PATH came from the parent");
}

#[tokio::test]
#[serial]
async fn the_output_callback_gets_every_chunk_as_it_arrives() {
    let seen: Arc<Mutex<Vec<(OutputKind, String)>>> = Arc::default();
    let sink = Arc::clone(&seen);
    let result = Process::command(sh(
        "printf one; sleep 0.2; printf two >&2; sleep 0.2; printf three",
    ))
    .run_with(move |kind, chunk| sink.lock().unwrap().push((kind, chunk.to_owned())))
    .await
    .unwrap();

    let seen = seen.lock().unwrap().clone();
    let out: String = seen
        .iter()
        .filter(|(kind, _)| *kind == OutputKind::Out)
        .map(|(_, chunk)| chunk.as_str())
        .collect();
    let err: String = seen
        .iter()
        .filter(|(kind, _)| *kind == OutputKind::Err)
        .map(|(_, chunk)| chunk.as_str())
        .collect();
    assert_eq!(out, "onethree");
    assert_eq!(err, "two");
    assert!(seen.len() >= 3, "the chunks came apart: {seen:?}");
    assert_eq!(result.output(), "onethree");
}

#[tokio::test]
#[serial]
async fn quietly_calls_no_callback() {
    let calls = Arc::new(Mutex::new(0));
    let count = Arc::clone(&calls);
    Process::command(sh("printf out; printf err >&2"))
        .quietly()
        .run_with(move |_, _| *count.lock().unwrap() += 1)
        .await
        .unwrap();
    assert_eq!(*calls.lock().unwrap(), 0);
}

#[tokio::test]
#[serial]
async fn a_program_that_cannot_start_is_an_error_naming_it() {
    let error = Process::command(["suprnova-no-such-program-4f1c", "arg"])
        .run()
        .await
        .expect_err("a missing program is an error");
    assert!(
        matches!(error, ProcessError::NotStarted { .. }),
        "{error:?}"
    );
    assert!(
        error.to_string().contains("suprnova-no-such-program-4f1c"),
        "{error}"
    );
}

#[tokio::test]
#[serial]
async fn the_result_names_the_command() {
    let result = Process::command(["printf", "a b"]).run().await.unwrap();
    assert_eq!(result.command(), "printf a b");
}

#[tokio::test]
#[serial]
async fn tty_captures_nothing() {
    let result = Process::command(["true"]).tty().run().await.unwrap();
    assert!(result.successful());
    assert_eq!(result.output(), "", "the terminal has the output");
    assert_eq!(result.error_output(), "");
}

#[tokio::test]
#[serial]
async fn a_shell_line_runs_through_the_shell() {
    let result = Process::shell("printf 'b\\na\\n' | sort")
        .run()
        .await
        .unwrap();
    assert_eq!(result.output(), "a\nb\n", "the pipe ran in the shell");
    assert_eq!(result.command(), "printf 'b\\na\\n' | sort");
}

#[tokio::test]
#[serial]
async fn a_shell_line_sees_the_environment_and_the_working_directory() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.log"), "").unwrap();
    std::fs::write(dir.path().join("b.log"), "").unwrap();
    let result = Process::shell("echo $SUPRNOVA_PROCESS_TEST *.log")
        .path(dir.path())
        .env("SUPRNOVA_PROCESS_TEST", "expanded")
        .run()
        .await
        .unwrap();
    assert_eq!(result.output(), "expanded a.log b.log\n");
}
