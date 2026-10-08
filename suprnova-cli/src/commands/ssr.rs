//! `suprnova ssr:start`, `ssr:stop` and `ssr:check` - the project's
//! application binary's commands of the same name.
//!
//! The application binary's `ssr:*` commands read the Inertia configuration
//! the application installed, the one first visits dispatch with (PAR-061).
//! Only the application can read it, so the CLI runs the application:
//! `cargo run --bin <package> -- ssr:<command>` from the project directory,
//! as `suprnova serve` runs the backend, with the flags given passed
//! through. The CLI builds no SSR configuration of its own, and links no
//! part of the framework crate for these commands.
//!
//! `ssr:start` keeps the worker in the foreground, under systemd, pm2 or
//! supervisord in production, so the CLI forwards each `SIGINT` and
//! `SIGTERM` it receives to the application, which forwards it to the
//! worker, and exits with the application's status.

use std::path::Path;
use std::process::ExitStatus;

use crate::ui;

/// One of the three commands, with the flags the operator gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SsrCommand {
    /// `ssr:start [--runtime <runtime>]`
    Start {
        /// The runtime to run the bundle with instead of the configured one.
        runtime: Option<String>,
    },
    /// `ssr:stop [--graceful]`
    Stop {
        /// Succeed when no worker is running.
        graceful: bool,
    },
    /// `ssr:check`
    Check,
}

impl SsrCommand {
    /// The arguments the application binary receives, the command name
    /// first.
    ///
    /// A value goes over in one argument with its flag, `--runtime=<value>`,
    /// so the application cannot read a value that starts with a hyphen as a
    /// flag of its own.
    fn app_args(&self) -> Vec<String> {
        match self {
            Self::Start { runtime: None } => vec!["ssr:start".into()],
            Self::Start {
                runtime: Some(runtime),
            } => vec!["ssr:start".into(), format!("--runtime={runtime}")],
            Self::Stop { graceful: false } => vec!["ssr:stop".into()],
            Self::Stop { graceful: true } => vec!["ssr:stop".into(), "--graceful".into()],
            Self::Check => vec!["ssr:check".into()],
        }
    }
}

/// Run `command` through the project's application binary and exit with
/// its status. A command that could not run exits with 1 after printing
/// why: no project here, no package name in its manifest, or a `cargo`
/// that could not be started.
pub fn run(command: SsrCommand) -> ! {
    let status = match run_in_project(&command) {
        Ok(status) => status,
        Err(e) => {
            ui::error(&e);
            1
        }
    };
    std::process::exit(status)
}

/// Find the project in the working directory as `serve` does, and run
/// `command` through its application binary.
fn run_in_project(command: &SsrCommand) -> Result<i32, String> {
    super::serve::validate_suprnova_project(false)?;
    let package = super::serve::get_package_name()?;
    let args = cargo_args(&package, command);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("Unable to start the async runtime: {e}"))?;
    runtime.block_on(run_forwarding(Path::new("."), &args))
}

/// `cargo run --bin <package> -- <command and flags>`, the backend's
/// invocation in `serve` with the command appended.
fn cargo_args(package: &str, command: &SsrCommand) -> Vec<String> {
    ["run", "--bin", package, "--"]
        .into_iter()
        .map(str::to_owned)
        .chain(command.app_args())
        .collect()
}

/// Run `cargo args` in `project` until it exits, forwarding the stop
/// signals the CLI receives, and return the status to exit with.
async fn run_forwarding(project: &Path, args: &[String]) -> Result<i32, String> {
    let described = format!("cargo {}", args.join(" "));
    // Installed before the application exists, so no signal sent while it
    // runs takes the default action and ends the CLI without it.
    let mut signals = StopSignals::install()?;
    let mut child = tokio::process::Command::new("cargo")
        .args(args)
        .current_dir(project)
        .spawn()
        .map_err(|e| format!("Failed to run `{described}`: {e}"))?;
    let status = loop {
        tokio::select! {
            status = child.wait() => break status,
            signal = signals.next() => forward(&child, signal),
        }
    };
    let status = status.map_err(|e| format!("Failed to wait for `{described}`: {e}"))?;
    Ok(exit_code(status))
}

/// The status the CLI exits with for the application's: its exit code, or
/// for an application ended by a signal 128 plus the signal number, as a
/// shell reports it.
fn exit_code(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    1
}

/// Forward a stop signal the CLI received to the application as the same
/// signal. The application's `ssr:start` stops its worker at the first and
/// kills it at the second, so every signal is forwarded. An application
/// already reaped has no process id, so a reused one is never signalled.
#[cfg(unix)]
fn forward(child: &tokio::process::Child, signal: nix::sys::signal::Signal) {
    if let Some(pid) = child.id().and_then(|pid| i32::try_from(pid).ok()) {
        let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), signal);
    }
}

/// The console delivers Ctrl-C to the application as well, so there is
/// nothing to forward.
#[cfg(not(unix))]
fn forward(_child: &tokio::process::Child, (): ()) {}

/// The stop signals the CLI receives while the application runs.
struct StopSignals {
    #[cfg(unix)]
    interrupt: tokio::signal::unix::Signal,
    #[cfg(unix)]
    terminate: tokio::signal::unix::Signal,
}

impl StopSignals {
    /// Install the handlers now, rather than on the first poll as
    /// `tokio::signal::ctrl_c` does, so a signal that arrives before the
    /// first poll is not lost.
    #[cfg(unix)]
    fn install() -> Result<Self, String> {
        use tokio::signal::unix::{SignalKind, signal};
        let handler = |kind: SignalKind, name: &str| {
            signal(kind).map_err(|e| format!("Unable to handle {name}: {e}"))
        };
        Ok(Self {
            interrupt: handler(SignalKind::interrupt(), "SIGINT")?,
            terminate: handler(SignalKind::terminate(), "SIGTERM")?,
        })
    }

    #[cfg(not(unix))]
    fn install() -> Result<Self, String> {
        Ok(Self {})
    }

    /// The next stop signal, as the signal to forward. A handler whose
    /// stream ended is not polled again, and with both ended this never
    /// resolves, rather than resolving at once in a loop.
    #[cfg(unix)]
    async fn next(&mut self) -> nix::sys::signal::Signal {
        use nix::sys::signal::Signal;
        tokio::select! {
            Some(()) = self.interrupt.recv() => Signal::SIGINT,
            Some(()) = self.terminate.recv() => Signal::SIGTERM,
            else => std::future::pending().await,
        }
    }

    /// The next Ctrl-C. A handler that cannot be installed never resolves,
    /// rather than resolving at once in a loop.
    #[cfg(not(unix))]
    async fn next(&mut self) {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_command_goes_over_with_the_flags_given() {
        assert_eq!(
            SsrCommand::Start { runtime: None }.app_args(),
            ["ssr:start"]
        );
        assert_eq!(
            SsrCommand::Start {
                runtime: Some("bun".into())
            }
            .app_args(),
            ["ssr:start", "--runtime=bun"]
        );
        assert_eq!(
            SsrCommand::Stop { graceful: false }.app_args(),
            ["ssr:stop"]
        );
        assert_eq!(
            SsrCommand::Stop { graceful: true }.app_args(),
            ["ssr:stop", "--graceful"]
        );
        assert_eq!(SsrCommand::Check.app_args(), ["ssr:check"]);
    }

    #[test]
    fn a_runtime_that_starts_with_a_hyphen_stays_the_flags_value() {
        assert_eq!(
            cargo_args(
                "app",
                &SsrCommand::Start {
                    runtime: Some("-weird".into())
                }
            ),
            ["run", "--bin", "app", "--", "ssr:start", "--runtime=-weird"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_application_ended_by_a_signal_exits_with_128_plus_the_signal() {
        use std::os::unix::process::ExitStatusExt;
        // The wait(2) encoding: the signal in the low bits, the exit code
        // in the next byte.
        assert_eq!(exit_code(ExitStatus::from_raw(9)), 137);
        assert_eq!(exit_code(ExitStatus::from_raw(3 << 8)), 3);
    }
}
