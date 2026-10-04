//! The console binary boots the process the way a worker does.
//!
//! `dispatch_argv_with_init` is what a scaffolded `console` binary calls.
//! After its bootstrap closure it used to run the command straight away,
//! so a command could not reach what a queued job reaches: the
//! `#[injectable]` and `#[service]` inventory, the runtime drivers, the
//! `#[policy]` gates. And it returned as soon as the command did, so a
//! queued listener the command started was cut off when the runtime
//! ended. Each test drives one of those through a command of its own.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use suprnova::supervisor::{RestartPolicy, Supervisor, SupervisorRegistry};
use suprnova::{App, Cache, EventFacade, FrameworkError, command, console, injectable};

/// An `#[injectable]` with nothing to inject: it resolves once the
/// inventory has been booted, and not before.
#[injectable]
pub struct ConsoleBootProbe;

#[command(
    name = "process-boot:services",
    description = "Resolves an #[injectable]"
)]
async fn resolves_an_injectable(_args: Vec<String>) -> Result<(), FrameworkError> {
    App::resolve::<ConsoleBootProbe>().map(|_| ())
}

#[command(
    name = "process-boot:cache",
    description = "Writes and reads the cache"
)]
async fn uses_the_cache(_args: Vec<String>) -> Result<(), FrameworkError> {
    Cache::put("process-boot-probe", &7u8, None).await?;
    match Cache::get::<u8>("process-boot-probe").await? {
        Some(7) => Ok(()),
        other => Err(FrameworkError::internal(format!(
            "the cache returned {other:?}"
        ))),
    }
}

/// Set when the `#[policy]` inventory is drained. The macro submits a
/// registration like this one for every policy method.
static POLICIES_DRAINED: AtomicBool = AtomicBool::new(false);

suprnova::inventory::submit! {
    suprnova::authorization::__PolicyRegistration {
        register: || POLICIES_DRAINED.store(true, Ordering::SeqCst),
    }
}

#[command(
    name = "process-boot:policies",
    description = "Needs the policy inventory"
)]
async fn needs_the_policies(_args: Vec<String>) -> Result<(), FrameworkError> {
    if POLICIES_DRAINED.load(Ordering::SeqCst) {
        Ok(())
    } else {
        Err(FrameworkError::internal(
            "the #[policy] inventory was never drained",
        ))
    }
}

/// A queued event, the last thing its command does.
#[derive(Debug, Clone)]
struct ReportFinished;

impl suprnova::Event for ReportFinished {
    fn event_name() -> &'static str {
        "process_boot.ReportFinished"
    }

    fn queued() -> bool {
        true
    }
}

/// Set by the listener, after a delay that outlasts the command.
static REPORT_RECORDED: AtomicBool = AtomicBool::new(false);

struct RecordReport;

#[suprnova::async_trait]
impl suprnova::Listener<ReportFinished> for RecordReport {
    async fn handle(&self, _event: &ReportFinished) -> Result<(), FrameworkError> {
        tokio::time::sleep(Duration::from_millis(200)).await;
        REPORT_RECORDED.store(true, Ordering::SeqCst);
        Ok(())
    }
}

#[command(
    name = "process-boot:queued-event",
    description = "Dispatches a queued event and returns"
)]
async fn dispatches_a_queued_event(_args: Vec<String>) -> Result<(), FrameworkError> {
    EventFacade::dispatch(ReportFinished).await
}

fn argv(command: &str) -> Vec<String> {
    vec!["console".to_string(), command.to_string()]
}

#[tokio::test]
async fn a_console_command_resolves_an_injectable() {
    console::dispatch_argv_with_init(argv("process-boot:services"), || async {})
        .await
        .expect("the #[injectable] inventory is booted before the command runs");
}

#[tokio::test]
async fn a_console_command_reaches_the_runtime_drivers() {
    console::dispatch_argv_with_init(argv("process-boot:cache"), || async {})
        .await
        .expect("the cache driver is booted before the command runs");
}

#[tokio::test]
async fn a_console_command_sees_the_policy_inventory() {
    console::dispatch_argv_with_init(argv("process-boot:policies"), || async {})
        .await
        .expect("the #[policy] inventory is drained before the command runs");
}

/// The listener sleeps past the command's return. The dispatcher waits for
/// it, because the console's runtime ends when `main` returns.
#[tokio::test]
async fn a_queued_listener_finishes_before_the_console_returns() {
    REPORT_RECORDED.store(false, Ordering::SeqCst);
    console::dispatch_argv_with_init(argv("process-boot:queued-event"), || async {
        EventFacade::listen::<ReportFinished, _>(Arc::new(RecordReport)).await;
    })
    .await
    .expect("the command dispatches its event");
    assert!(
        REPORT_RECORDED.load(Ordering::SeqCst),
        "the queued listener was still running when the console returned"
    );
}

/// Set while the console's supervisor runs.
static CONSOLE_SUPERVISOR_RUNNING: AtomicBool = AtomicBool::new(false);

/// A supervisor the console's bootstrap starts: it runs until its token is
/// cancelled.
struct ConsoleSupervisor;

#[suprnova::async_trait]
impl Supervisor for ConsoleSupervisor {
    fn name(&self) -> &'static str {
        "console_supervisor"
    }

    async fn run(&self, cancel: tokio_util::sync::CancellationToken) -> Result<(), FrameworkError> {
        CONSOLE_SUPERVISOR_RUNNING.store(true, Ordering::SeqCst);
        cancel.cancelled().await;
        CONSOLE_SUPERVISOR_RUNNING.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn restart_policy(&self) -> RestartPolicy {
        RestartPolicy::Never
    }
}

#[command(name = "process-boot:noop", description = "Does nothing")]
async fn does_nothing(_args: Vec<String>) -> Result<(), FrameworkError> {
    Ok(())
}

/// The console stops and drains the supervisors its bootstrap started
/// before it returns. It used to return with them still running, and the
/// end of `main` cut them off mid-work.
///
/// In a process where an earlier test already shut the supervisors down,
/// the spawn is refused and nothing runs; the assertion still holds.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_console_command_drains_the_supervisors_its_bootstrap_started() {
    console::dispatch_argv_with_init(argv("process-boot:noop"), || async {
        SupervisorRegistry::spawn(Arc::new(ConsoleSupervisor)).await;
        for _ in 0..100 {
            if CONSOLE_SUPERVISOR_RUNNING.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the command runs");
    assert!(
        !CONSOLE_SUPERVISOR_RUNNING.load(Ordering::SeqCst),
        "the console returned with its bootstrap's supervisor still running"
    );
}
