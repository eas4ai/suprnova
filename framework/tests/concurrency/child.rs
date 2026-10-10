//! The console entry of this binary, for the process driver.

use suprnova::console;

/// In a test run, checks that the framework registers `concurrency:run`
/// and keeps it out of the console's help. Started by the process driver,
/// with `concurrency:run <task>` among its arguments, it dispatches that
/// command as an application's console binary does, writes the lines a
/// task left for after the command, and exits with the command's code,
/// before libtest writes anything more.
#[tokio::test]
async fn entry() {
    let arguments: Vec<String> = std::env::args().collect();
    if let Some(at) = arguments
        .iter()
        .position(|argument| argument == "concurrency:run")
    {
        let argv: Vec<String> = std::iter::once("console".to_owned())
            .chain(arguments[at..].iter().cloned())
            .collect();
        let code = match console::dispatch_argv(argv).await {
            Ok(()) => 0,
            Err(error) => i32::from(error.exit_code()),
        };
        // What an application's logging writes during the console's
        // shutdown, after the command wrote its outcome.
        crate::tasks::write_lines_left_for_after_the_command();
        std::process::exit(code);
    }

    let entry = console::find("concurrency:run").expect("the framework registers concurrency:run");
    assert!(
        (entry.clap_builder)().is_hide_set(),
        "concurrency:run is a protocol between processes and stays out of the help"
    );
}
