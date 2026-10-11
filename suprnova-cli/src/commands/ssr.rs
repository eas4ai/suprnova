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
//! supervisord in production, and the CLI exits with the application's
//! status. The application runs in a process group of its own, so a
//! terminal's Ctrl-C, which goes to the CLI's group, reaches it once,
//! through the CLI. The CLI forwards each `SIGINT`, `SIGTERM`, `SIGHUP`,
//! `SIGQUIT`, `SIGTSTP` and `SIGCONT` it receives, and stops with the
//! application at a `SIGTSTP`, so Ctrl-C, Ctrl-\, Ctrl-Z and `fg` act on the
//! application and its worker as they act on the CLI.

use std::path::Path;
use std::process::{ExitStatus, Stdio};

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

/// Run `cargo args` in `project` until it exits, forwarding the signals
/// the CLI receives, and return the status to exit with.
async fn run_forwarding(project: &Path, args: &[String]) -> Result<i32, String> {
    let described = format!("cargo {}", args.join(" "));
    // Installed before the application exists, so no signal sent while it
    // runs takes the default action and ends the CLI without it.
    let mut signals = ForwardedSignals::install()?;
    let mut command = tokio::process::Command::new("cargo");
    command
        .args(args)
        .current_dir(project)
        // The application reads nothing, and in a process group that is not
        // the terminal's foreground one a read would stop it.
        .stdin(Stdio::null());
    // A terminal sends Ctrl-C to its foreground process group. In the CLI's
    // group the application would get it twice, from the terminal and from
    // the CLI, and its `ssr:start` kills the worker at a second signal.
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
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

/// Forward a signal the CLI received to the application as the same
/// signal, each time one arrives.
///
/// `SIGINT` and `SIGTERM` go to the application alone: its `ssr:start`
/// forwards them to the worker, stopping it at the first and killing it at
/// the second. The others go to the application's whole process group, the
/// worker included, as a terminal delivers them to the job in its
/// foreground: the application does not handle them, so `SIGHUP` and
/// `SIGQUIT` would end it by their default action and leave the worker
/// running, and `SIGTSTP` and `SIGCONT` must stop and resume the worker with
/// it. After a `SIGTSTP` the CLI stops itself as well, as that signal's
/// default action would; with the signal handled it stops by `SIGSTOP`, and
/// the `SIGCONT` that `fg` sends resumes it and is forwarded in turn. An
/// application already reaped has no process id, so a reused one is never
/// signalled.
#[cfg(unix)]
fn forward(child: &tokio::process::Child, signal: nix::sys::signal::Signal) {
    use nix::sys::signal::{Signal, kill, killpg, raise};
    let Some(pid) = child.id().and_then(|pid| i32::try_from(pid).ok()) else {
        return;
    };
    // The application leads its own process group, so its id names it.
    let pid = nix::unistd::Pid::from_raw(pid);
    let _ = match signal {
        Signal::SIGINT | Signal::SIGTERM => kill(pid, signal),
        _ => killpg(pid, signal),
    };
    if signal == Signal::SIGTSTP {
        let _ = raise(Signal::SIGSTOP);
    }
}

/// The console delivers Ctrl-C to the application as well, so there is
/// nothing to forward.
#[cfg(not(unix))]
fn forward(_child: &tokio::process::Child, (): ()) {}

/// The signals the CLI receives while the application runs and forwards to
/// it.
struct ForwardedSignals {
    #[cfg(unix)]
    interrupt: tokio::signal::unix::Signal,
    #[cfg(unix)]
    terminate: tokio::signal::unix::Signal,
    #[cfg(unix)]
    hangup: tokio::signal::unix::Signal,
    #[cfg(unix)]
    quit: tokio::signal::unix::Signal,
    #[cfg(unix)]
    stop: tokio::signal::unix::Signal,
    #[cfg(unix)]
    resume: tokio::signal::unix::Signal,
}

impl ForwardedSignals {
    /// Install the handlers now, rather than on the first poll as
    /// `tokio::signal::ctrl_c` does, so a signal that arrives before the
    /// first poll is not lost.
    #[cfg(unix)]
    fn install() -> Result<Self, String> {
        use nix::sys::signal::Signal;
        use tokio::signal::unix::{SignalKind, signal};
        let handler = |kind: SignalKind, name: &str| {
            signal(kind).map_err(|e| format!("Unable to handle {name}: {e}"))
        };
        Ok(Self {
            interrupt: handler(SignalKind::interrupt(), "SIGINT")?,
            terminate: handler(SignalKind::terminate(), "SIGTERM")?,
            hangup: handler(SignalKind::hangup(), "SIGHUP")?,
            quit: handler(SignalKind::quit(), "SIGQUIT")?,
            stop: handler(SignalKind::from_raw(Signal::SIGTSTP as i32), "SIGTSTP")?,
            resume: handler(SignalKind::from_raw(Signal::SIGCONT as i32), "SIGCONT")?,
        })
    }

    #[cfg(not(unix))]
    fn install() -> Result<Self, String> {
        Ok(Self {})
    }

    /// The next signal to forward. A handler whose
    /// stream ended is not polled again, and with all ended this never
    /// resolves, rather than resolving at once in a loop.
    #[cfg(unix)]
    async fn next(&mut self) -> nix::sys::signal::Signal {
        use nix::sys::signal::Signal;
        tokio::select! {
            Some(()) = self.interrupt.recv() => Signal::SIGINT,
            Some(()) = self.terminate.recv() => Signal::SIGTERM,
            Some(()) = self.hangup.recv() => Signal::SIGHUP,
            Some(()) = self.quit.recv() => Signal::SIGQUIT,
            Some(()) = self.stop.recv() => Signal::SIGTSTP,
            Some(()) = self.resume.recv() => Signal::SIGCONT,
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
