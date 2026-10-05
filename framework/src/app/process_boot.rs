//! The boot every process shares, except the HTTP server.
//!
//! `serve` boots through [`crate::Server`], which installs the container's
//! services, the `#[policy]` gates and the runtime drivers on its own path.
//! Every other process - the queue, schedule and workflow workers, the
//! queue and maintenance commands, `schedule:list`, and the per-project
//! console binary - used to assemble its own subset of that boot, and each
//! subset left something out: the workers and the console never booted the
//! `#[injectable]` and `#[service]` inventory, the console never booted the
//! runtime drivers or the policies, and `down`/`up` never ran the
//! application's `bootstrap` hook at all. A job that resolved an action
//! worked under `serve` and failed with `ServiceNotFound` under
//! `queue:work`.
//!
//! So there is one function, [`boot_after_hook`], and every one of those
//! processes calls it right after the application's own `bootstrap` hook.
//! What differs between them is only which drivers they need, and that is
//! the [`ProcessBoot`] value they pass.

use crate::container::App;

/// The error a process boot returns; every step's own error converts into it.
pub(crate) type BootError = Box<dyn std::error::Error + Send + Sync>;

/// What a process installs after the application's `bootstrap` hook.
///
/// Every value but [`Self::Migrations`] boots the same core - the log
/// channels, the container's services, the `#[policy]` gates - and then
/// the drivers its commands use. The drivers are not booted where nothing
/// uses them: a driver whose backend is down would otherwise stop `down`,
/// the command an operator reaches for during exactly that outage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProcessBoot {
    /// `migrate`, `migrate:*` and `schema:dump`. They run no application
    /// hook, the same way `serve` runs its migrations before the hook: a
    /// fresh database has to be migratable even when the hook reads tables
    /// the migrations have not created yet.
    Migrations,
    /// The core only: `schedule:list`, which builds the schedule the
    /// daemon would run and prints it.
    Core,
    /// The core and the drivers maintenance mode reads: the cache when
    /// `MAINTENANCE_DRIVER=cache`, and localization. `down` and `up`.
    Maintenance,
    /// The core and every runtime driver: Cache, Localization, the
    /// environment's disks, Queue, RateLimit, Mail. The workers, the queue
    /// commands, and `schedule:run`. A driver whose backend does not come up
    /// stops the boot: these processes exist to do work through them.
    Work,
    /// The core and every runtime driver, for the console binary. A driver
    /// that does not come up is reported on stderr and does not stop the
    /// boot. A console command is often a one-off that uses none of the
    /// drivers, and a mail driver missing its key must not block `db:seed`;
    /// a command that does use the broken driver fails when it reaches it,
    /// after the report has named the cause.
    Console,
}

/// Boot what `boot` names, after the application's `bootstrap` hook ran.
///
/// The order matches the server's. The log channels are checked first,
/// because the hook may define the channel `LOG_CHANNEL` names. The
/// services come next, after the hook, so a binding the hook installed by
/// hand wins over the inventory default (`singleton_if_absent`). The
/// drivers come last: `QUEUE_DRIVER=database` resolves its connection out
/// of the `DB` the hook initialized.
///
/// # Errors
///
/// When the log channel does not exist, when a service's dependencies
/// cannot be resolved, or when a driver's backend does not come up.
pub(crate) async fn boot_after_hook(boot: ProcessBoot) -> Result<(), BootError> {
    if boot == ProcessBoot::Migrations {
        return Ok(());
    }
    crate::logging::check_channels()?;
    App::init();
    App::boot_services()?;
    crate::authorization::init_policies();
    match boot {
        ProcessBoot::Migrations | ProcessBoot::Core => {}
        ProcessBoot::Maintenance => bootstrap_maintenance_drivers().await?,
        ProcessBoot::Work => bootstrap_runtime_drivers(false).await?,
        ProcessBoot::Console => bootstrap_runtime_drivers(true).await?,
    }
    Ok(())
}

/// How long the end of a booted process waits for the supervisors its
/// bootstrap started: the grace `Server::run` gives them.
const SUPERVISOR_DRAIN: std::time::Duration = std::time::Duration::from_secs(5);

/// End a booted process the way `Server::run` ends its graceful
/// shutdown: stop the supervisors and drain them, then wait for the
/// queued event listeners still running, which the supervisors may have
/// started on their way down.
///
/// The application's bootstrap starts supervisors in every process, not
/// only the server's, and returning from `main` drops the runtime and
/// every task on it. The workers, the commands, and the console used to
/// return with their supervisors still running, so the teardown cut them
/// off mid-work instead of letting them see their cancel token.
pub(crate) async fn finish_process() {
    crate::supervisor::SupervisorRegistry::shutdown(SUPERVISOR_DRAIN).await;
    crate::events::drain_queued_at_shutdown().await;
}

/// End a booted process that failed, with `code`, after [`finish_process`]
/// and a flush of the file log channels.
///
/// `std::process::exit` lets no task finish. A command whose boot or work
/// failed after the application's bootstrap started its supervisors used to
/// exit straight away and cut them off, while the same command succeeding
/// drained them. Every exit after the bootstrap hook goes through here.
pub(crate) async fn exit_after_boot(code: i32) -> ! {
    finish_process().await;
    crate::logging::Log::flush();
    std::process::exit(code)
}

/// Every runtime driver, in the order `Server::run` boots them.
///
/// `report_failures` turns a driver that does not come up into a warning
/// on stderr and the log, and goes on with the next one; otherwise the
/// first failure is the boot's error.
async fn bootstrap_runtime_drivers(report_failures: bool) -> Result<(), BootError> {
    let settle = |driver: &str, outcome: Result<(), BootError>| -> Result<(), BootError> {
        match outcome {
            Err(e) if report_failures => {
                tracing::warn!(driver, error = %e, "a runtime driver did not boot");
                crate::console::error_line(format!(
                    "warning: the {driver} driver did not boot: {e}"
                ));
                Ok(())
            }
            outcome => outcome,
        }
    };
    settle(
        "cache",
        crate::cache::Cache::bootstrap().await.map_err(Into::into),
    )?;
    #[cfg(feature = "localization")]
    settle(
        "localization",
        crate::localization::Localization::bootstrap()
            .await
            .map_err(Into::into),
    )?;
    // The disks first: the `sqs` queue driver checks its overflow disk.
    #[cfg(feature = "filesystem")]
    settle(
        "filesystem",
        crate::filesystem::bootstrap_from_env().map_err(Into::into),
    )?;
    settle(
        "queue",
        crate::queue::bootstrap_from_env().await.map_err(Into::into),
    )?;
    settle(
        "rate limit",
        crate::rate_limit::bootstrap_from_env()
            .await
            .map_err(Into::into),
    )?;
    settle(
        "mail",
        crate::mail::boot::bootstrap_from_env().map_err(Into::into),
    )?;
    Ok(())
}

/// The drivers `down` and `up` read.
///
/// The cache only when the cache driver holds the maintenance state. The
/// localization always: the commands print user-facing status text, and a
/// custom driver may call `Lang::get`.
async fn bootstrap_maintenance_drivers() -> Result<(), BootError> {
    if std::env::var("MAINTENANCE_DRIVER").as_deref() == Ok("cache") {
        crate::cache::Cache::bootstrap()
            .await
            .map_err(|e| format!("maintenance (cache driver) bootstrap failed: {e}"))?;
    }
    #[cfg(feature = "localization")]
    crate::localization::Localization::bootstrap()
        .await
        .map_err(|e| format!("localization bootstrap failed: {e}"))?;
    Ok(())
}
