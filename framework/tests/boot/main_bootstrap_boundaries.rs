//! `#[suprnova::main]` loads configuration before the runtime, but must not
//! run application bootstrap for console paths that only display metadata.

use std::process::Command;

use suprnova::FrameworkError;

const CHILD_MODE: &str = "SUPRNOVA_BOOTSTRAP_BOUNDARY_CHILD";

#[suprnova::main(flavor = "current_thread")]
async fn run_bootstrap_free_console() -> Result<(), FrameworkError> {
    let mode = std::env::var(CHILD_MODE).expect("child mode");
    suprnova::console::set_version("9.8.7-test");
    let argv = match mode.as_str() {
        "help" => vec!["console".to_string(), "--help".to_string()],
        "version" => vec!["console".to_string(), "--version".to_string()],
        other => panic!("unknown child mode: {other}"),
    };

    suprnova::console::dispatch_argv_with_init(argv, || async {
        panic!("help and version must not run application bootstrap")
    })
    .await
}

#[test]
fn bootstrap_free_console_child() {
    if std::env::var(CHILD_MODE).is_err() {
        return;
    }
    run_bootstrap_free_console().expect("bootstrap-free console path must succeed");
}

fn assert_bootstrap_free_mode(mode: &str) {
    let output = Command::new(std::env::current_exe().expect("current test executable"))
        .args(["--exact", "bootstrap_free_console_child", "--nocapture"])
        .env(CHILD_MODE, mode)
        .env("APP_ENV", "production")
        .env_remove("APP_KEY")
        .env_remove("APP_KEY_PREVIOUS")
        .env_remove("APP_PREVIOUS_KEYS")
        .output()
        .expect("spawn bootstrap-free console child");

    assert!(
        output.status.success(),
        "console --{mode} must work without APP_KEY because it skips application bootstrap; \
         status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn console_help_does_not_require_app_key_in_production() {
    assert_bootstrap_free_mode("help");
}

#[test]
fn console_version_does_not_require_app_key_in_production() {
    assert_bootstrap_free_mode("version");
}
