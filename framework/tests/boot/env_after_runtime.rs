//! Loading `.env` again once `#[suprnova::main]` has built its runtime is
//! refused.
//!
//! Writing the process environment is sound only while no other thread can
//! read it. `#[suprnova::main]` loads the environment and then builds the
//! runtime, whose worker threads read it through `getenv` from then on. A
//! later `Config::init` or `config::load_dotenv` from a plain thread is not
//! inside the runtime, so a check for the current runtime does not see it;
//! these safe functions used to write the environment anyway. The child
//! process below makes exactly that call.

use std::process::Command;

const CHILD_MODE: &str = "SUPRNOVA_ENV_AFTER_RUNTIME_CHILD";

#[suprnova::main(flavor = "multi_thread", worker_threads = 2)]
async fn reload_from_a_plain_thread() -> Result<(), String> {
    let root = tempfile::tempdir().map_err(|e| e.to_string())?;
    std::fs::write(
        root.path().join(".env"),
        "SUPRNOVA_ENV_AFTER_RUNTIME=written\n",
    )
    .map_err(|e| e.to_string())?;
    let dir = root.path().to_path_buf();
    let outcome = std::thread::spawn(move || {
        (
            suprnova::Config::init(&dir).map(|_| ()),
            suprnova::config::load_dotenv(&dir).map(|_| ()),
        )
    })
    .join()
    .map_err(|_| "the loading thread panicked".to_string())?;
    match outcome {
        (Err(init), Err(load)) => {
            if std::env::var("SUPRNOVA_ENV_AFTER_RUNTIME").is_ok() {
                return Err("a refused load wrote the environment".to_string());
            }
            for message in [init.to_string(), load.to_string()] {
                if !message.contains("#[suprnova::main]") {
                    return Err(format!("the refusal does not say why: {message}"));
                }
            }
            Ok(())
        }
        other => Err(format!(
            "the environment was written while the runtime's threads ran: {other:?}"
        )),
    }
}

#[test]
fn env_after_runtime_child() {
    if std::env::var(CHILD_MODE).is_err() {
        return;
    }
    reload_from_a_plain_thread().expect("a reload after the runtime started must be refused");
}

#[test]
fn a_reload_after_the_runtime_started_is_refused() {
    let output = Command::new(std::env::current_exe().expect("current test executable"))
        .args([
            "--exact",
            "env_after_runtime::env_after_runtime_child",
            "--nocapture",
        ])
        .env(CHILD_MODE, "1")
        .env_remove("SUPRNOVA_ENV_AFTER_RUNTIME")
        .output()
        .expect("spawn the child");

    assert!(
        output.status.success(),
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("running 1 test"),
        "child filter matched no test; stdout:\n{}",
        String::from_utf8_lossy(&output.stdout),
    );
}
