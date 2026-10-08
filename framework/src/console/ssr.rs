//! `ssr:start`, `ssr:stop` and `ssr:check`, Laravel's `inertia:start-ssr`,
//! `inertia:stop-ssr` and `inertia:check-ssr` (PAR-061).
//!
//! The application binary runs these with the Inertia configuration the
//! application installed, and the `suprnova` CLI runs them with one built
//! from its flags and the `SUPRNOVA_SSR_*` environment. One implementation
//! for both means the two commands cannot drift apart: the same
//! configuration gets the same checks, the same messages and the same exit
//! status.
//!
//! Each function writes what Laravel's command prints, success to `out` and
//! failures and warnings to `err`, and returns the exit status the command
//! ends with: [`SUCCESS`], [`FAILURE`], or for `start` the worker's own. A
//! `FrameworkError` means the command could not run at all, such as a signal
//! handler that could not be installed. A failed write to `out` or `err` is
//! ignored, as [`line`](super::line) ignores it: a reader that went away
//! must not stop a running worker.

use std::fmt::Display;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use crate::error::FrameworkError;
use crate::inertia::SsrConfig;

/// The exit status of a command that succeeded.
pub const SUCCESS: i32 = 0;

/// The exit status of a command that failed.
pub const FAILURE: i32 = 1;

/// The longest line of the worker's output read as one line. A longer run
/// of bytes without a line ending is reported in pieces of this size, so a
/// worker that never writes a newline cannot grow the buffer without bound.
const MAX_LINE_BYTES: u64 = 1024 * 1024;

/// How long the output the worker wrote before it exited is still read.
///
/// A worker's output pipes close when it exits, unless a process it started
/// keeps them open. Symfony's `Process` reads what is left and stops; this
/// waits this long for the pipes to close and then stops reading, so a
/// leftover process cannot keep `ssr:start` running after its worker ended.
const OUTPUT_DRAIN: Duration = Duration::from_secs(2);

/// What the worker's error output is logged under.
const LOG_TARGET: &str = "suprnova::ssr";

/// The message for a missing bundle when no path is configured. It names
/// both ways to configure one, since the application binary and the
/// `suprnova` CLI print the same text.
const BUNDLE_NOT_FOUND: &str = "Inertia SSR bundle not found. Set its path with \
     `InertiaConfig::ssr_bundle_path` (`--bundle` or SUPRNOVA_SSR_BUNDLE for the `suprnova` \
     CLI), or build it to frontend/bootstrap/ssr/ssr.js with `vite build --ssr`.";

/// Start the SSR worker in the foreground: Laravel's `inertia:start-ssr`.
///
/// Refuses to start when SSR is not enabled, when no bundle is found (as
/// [`detect_ssr_bundle`](crate::detect_ssr_bundle) finds it for the
/// dispatch), and, with [`SsrConfig::ensure_runtime_exists`], when the
/// runtime cannot be found. A configured bundle that is missing while a
/// conventional one exists is a warning, and the conventional one runs.
///
/// Then it asks a worker still running at [`SsrConfig::url`] to shut down,
/// silently, as Laravel calls `inertia:stop-ssr`. A worker that closed the
/// connection has stopped and one that could not be connected to was not
/// running; one that answered, or kept the connection open past the timeout,
/// is still running, and the start is refused: the running worker stops
/// first, and a second one could not bind the port in any case. Then it
/// runs `runtime bundle`: `runtime_override` (the `--runtime` flag), else
/// [`SsrConfig::runtime`]. The worker's stdout is forwarded to `out`.
/// Each line of its stderr is written to `err` and, when it is not blank,
/// logged as an error, where Laravel reports an `SsrException`.
///
/// `SIGINT` and `SIGTERM` to this process are forwarded to the worker as the
/// same signal, so a worker with its own handlers runs the one the signal
/// names (Laravel's command stops the worker with `SIGTERM` for either); a
/// second one kills the worker. The function returns when the worker exits,
/// with its exit status;
/// a worker ended by the signal this function forwarded returns
/// [`SUCCESS`], as Laravel's command does, and one ended by another signal
/// returns 128 plus the signal number, as a shell reports it.
///
/// # Errors
///
/// When the signal handlers cannot be installed, or waiting for the worker
/// fails.
pub async fn start(
    config: &SsrConfig,
    runtime_override: Option<String>,
    out: &mut (dyn Write + Send),
    err: &mut (dyn Write + Send),
) -> Result<i32, FrameworkError> {
    if !config.enabled {
        say(
            err,
            "Inertia SSR is not enabled. Enable it with `InertiaConfig::ssr(url)` on the \
             configuration passed to `Inertia::install`.",
        );
        return Ok(FAILURE);
    }

    let configured = config.bundle_path.as_deref();
    let Some(bundle) = crate::detect_ssr_bundle(config) else {
        match configured {
            Some(path) => say(err, configured_bundle_missing(path)),
            None => say(err, BUNDLE_NOT_FOUND),
        }
        return Ok(FAILURE);
    };
    if let Some(path) = configured
        && path != bundle
    {
        say(err, configured_bundle_missing(path));
        say(
            err,
            format!("Using a default bundle instead: \"{}\"", bundle.display()),
        );
    }

    let runtime = runtime_override.unwrap_or_else(|| config.runtime.clone());
    if config.ensure_runtime_exists && find_executable(&runtime).is_none() {
        say(
            err,
            format!("SSR runtime \"{runtime}\" could not be found."),
        );
        return Ok(FAILURE);
    }

    // Laravel's `callSilently('inertia:stop-ssr')` asks a worker still
    // running at the URL to shut down. One that closed the connection has
    // stopped and one that could not be connected to was not running; one
    // that answered, or kept the connection open past the timeout, is still
    // running, and no second worker starts beside it (PAR-061): it could
    // not bind the port in any case.
    let shutdown_url = match worker_url(config, "/shutdown") {
        Ok(url) => url,
        Err(message) => {
            say(err, message);
            return Ok(FAILURE);
        }
    };
    if let Shutdown::Other = request_shutdown(&shutdown_url, config.timeout).await? {
        say(
            err,
            format!(
                "The Inertia SSR server at {} did not stop; not starting another.",
                config.url
            ),
        );
        return Ok(FAILURE);
    }

    run_worker(&runtime, &bundle, out, err).await
}

/// Ask the SSR worker to shut down: Laravel's `inertia:stop-ssr`.
///
/// Sends `GET {url}/shutdown` with [`SsrConfig::timeout`]. The Inertia SSR
/// server exits on that request without answering, so a connection closed
/// with no response is success. With `graceful`, a worker that cannot be
/// connected to is not running, which is success too. An answer, a
/// connection that stays open past the timeout, or a connection without
/// `graceful` that cannot be made is a failure: the worker did not stop.
///
/// # Errors
///
/// When the HTTP client cannot be built.
pub async fn stop(
    config: &SsrConfig,
    graceful: bool,
    out: &mut (dyn Write + Send),
    err: &mut (dyn Write + Send),
) -> Result<i32, FrameworkError> {
    let url = match worker_url(config, "/shutdown") {
        Ok(url) => url,
        Err(message) => {
            say(err, message);
            return Ok(FAILURE);
        }
    };
    match request_shutdown(&url, config.timeout).await? {
        Shutdown::Closed => {
            say(out, "Inertia SSR server stopped.");
            Ok(SUCCESS)
        }
        Shutdown::NotRunning if graceful => {
            say(out, "Inertia SSR server is not running.");
            Ok(SUCCESS)
        }
        Shutdown::NotRunning | Shutdown::Other => {
            say(err, "Unable to connect to Inertia SSR server.");
            Ok(FAILURE)
        }
    }
}

/// Ask the SSR gateway whether the worker is healthy: Laravel's
/// `inertia:check-ssr`.
///
/// The gateway is the one the application bound as `dyn SsrGateway`, else
/// the HTTP gateway (see [`ssr_gateway`](crate::ssr_gateway)), so the check
/// asks the same gateway first visits dispatch through. A gateway without a
/// health check fails, as one that is not a `HasHealthCheck` does in
/// Laravel.
///
/// # Errors
///
/// This function returns no error of its own; the `Result` matches
/// [`start`] and [`stop`], so a caller handles the three alike.
pub async fn check(
    config: &SsrConfig,
    out: &mut (dyn Write + Send),
    err: &mut (dyn Write + Send),
) -> Result<i32, FrameworkError> {
    if let Err(message) = worker_url(config, "/health") {
        say(err, message);
        return Ok(FAILURE);
    }
    match crate::ssr_gateway().is_healthy(config).await {
        None => {
            say(err, "The SSR gateway does not support health checks.");
            Ok(FAILURE)
        }
        Some(true) => {
            say(out, "Inertia SSR server is running.");
            Ok(SUCCESS)
        }
        Some(false) => {
            say(err, "Inertia SSR server is not running.");
            Ok(FAILURE)
        }
    }
}

/// Write one line and flush it, so a line reaches the terminal while the
/// worker runs.
fn say(to: &mut (dyn Write + Send), text: impl Display) {
    let _ = writeln!(to, "{text}").and_then(|()| to.flush());
}

fn configured_bundle_missing(path: &Path) -> String {
    format!(
        "Inertia SSR bundle not found at the configured path: \"{}\"",
        path.display()
    )
}

/// `{url}{path}` for the worker at [`SsrConfig::url`], or why the URL cannot
/// be one: Laravel's `getProductionUrl`, which trims the trailing slash.
///
/// A URL without `http://` or `https://` is refused here, with the URL in
/// the message, rather than read as a worker that is not running.
fn worker_url(config: &SsrConfig, path: &str) -> Result<String, String> {
    let valid = url::Url::parse(&config.url)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host().is_some());
    if valid {
        Ok(format!("{}{path}", config.url.trim_end_matches('/')))
    } else {
        Err(format!(
            "The Inertia SSR URL must start with http:// or https:// and name a host: \"{}\"",
            config.url
        ))
    }
}

/// How a `GET /shutdown` ended.
enum Shutdown {
    /// The worker closed the connection without an answer: curl's
    /// `CURLE_GOT_NOTHING`, which Laravel reads as stopped.
    Closed,
    /// No connection could be made: nothing listens at the URL.
    NotRunning,
    /// An answer, a timeout, or a connection that failed after it was made.
    Other,
}

async fn request_shutdown(url: &str, timeout: Duration) -> Result<Shutdown, FrameworkError> {
    // No proxy: the worker is the application's own process, and a proxy
    // from the environment would answer in its place.
    let client = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(timeout)
        .timeout(timeout)
        .build()
        .map_err(|e| FrameworkError::internal(format!("ssr:stop: the HTTP client: {e}")))?;
    Ok(match client.get(url).send().await {
        Ok(_) => Shutdown::Other,
        Err(e) if e.is_connect() => Shutdown::NotRunning,
        Err(e) if closed_without_response(&e) => Shutdown::Closed,
        Err(_) => Shutdown::Other,
    })
}

/// Whether the connection closed before any response arrived. hyper reports
/// that as an incomplete message; a reset or a timeout is something else.
fn closed_without_response(error: &reqwest::Error) -> bool {
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        if let Some(hyper) = cause.downcast_ref::<hyper::Error>() {
            return hyper.is_incomplete_message();
        }
        source = cause.source();
    }
    false
}

/// The runtime as Symfony's `ExecutableFinder` finds it: a name with a
/// directory in it is a path, found when it is an executable file; a bare
/// name is looked up in each `PATH` directory.
fn find_executable(runtime: &str) -> Option<PathBuf> {
    if runtime.is_empty() {
        return None;
    }
    let path = Path::new(runtime);
    if path
        .parent()
        .is_some_and(|parent| !parent.as_os_str().is_empty())
    {
        return is_executable(path).then(|| path.to_path_buf());
    }
    let dirs = std::env::var_os("PATH")?;
    std::env::split_paths(&dirs)
        .map(|dir| {
            if dir.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                dir
            }
        })
        .flat_map(|dir| executable_names(runtime).map(move |name| dir.join(name)))
        .find(|candidate| is_executable(candidate))
}

/// The file names `runtime` may have in a `PATH` directory.
#[cfg(not(windows))]
fn executable_names(runtime: &str) -> impl Iterator<Item = String> {
    std::iter::once(runtime.to_owned())
}

/// The file names `runtime` may have in a `PATH` directory: the name, and
/// the name with each `PATHEXT` extension, as Windows finds `node.exe` for
/// `node`.
#[cfg(windows)]
fn executable_names(runtime: &str) -> impl Iterator<Item = String> {
    let extensions = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned());
    let runtime = runtime.to_owned();
    std::iter::once(runtime.clone()).chain(
        extensions
            .split(';')
            .filter(|ext| !ext.is_empty())
            .map(|ext| format!("{runtime}{ext}"))
            .collect::<Vec<_>>(),
    )
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// Which of the worker's streams a line came from.
#[derive(Clone, Copy)]
enum Stream {
    Out,
    Err,
}

/// Run `runtime bundle` until it exits, forwarding its output and the stop
/// signals this process receives.
async fn run_worker(
    runtime: &str,
    bundle: &Path,
    out: &mut (dyn Write + Send),
    err: &mut (dyn Write + Send),
) -> Result<i32, FrameworkError> {
    // Installed before the worker exists, so no signal sent once it runs
    // can take the default action and end this process without it.
    let mut signals = StopSignals::install()?;

    let mut child = match Command::new(runtime)
        .arg(bundle)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            say(
                err,
                format!(
                    "Unable to start the Inertia SSR server: \"{runtime}\" \"{}\": {e}",
                    bundle.display()
                ),
            );
            return Ok(FAILURE);
        }
    };

    let (sender, mut lines) = mpsc::channel::<(Stream, String)>(64);
    let mut readers = Vec::with_capacity(2);
    if let Some(stdout) = child.stdout.take() {
        readers.push(tokio::spawn(read_lines(
            stdout,
            Stream::Out,
            sender.clone(),
        )));
    }
    if let Some(stderr) = child.stderr.take() {
        readers.push(tokio::spawn(read_lines(stderr, Stream::Err, sender)));
    }

    let mut stop_sent = false;
    let status = loop {
        tokio::select! {
            Some(line) = lines.recv() => emit(line, out, err),
            status = child.wait() => break status,
            signal = signals.next() => {
                if stop_sent {
                    let _ = child.start_kill();
                } else {
                    ask_to_stop(&mut child, signal);
                    stop_sent = true;
                }
            }
        }
    };

    let drain = async {
        while let Some(line) = lines.recv().await {
            emit(line, out, err);
        }
    };
    let _ = tokio::time::timeout(OUTPUT_DRAIN, drain).await;
    for reader in readers {
        reader.abort();
    }

    let status = status.map_err(|e| {
        FrameworkError::internal(format!(
            "ssr:start: waiting for the Inertia SSR server failed: {e}"
        ))
    })?;
    Ok(exit_status(status, stop_sent))
}

/// Send each line of `stream` to `lines`, until the stream ends or nothing
/// receives any more.
async fn read_lines<R>(stream: R, kind: Stream, lines: mpsc::Sender<(Stream, String)>)
where
    R: AsyncRead + Unpin,
{
    let mut reader = BufReader::new(stream);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match (&mut reader)
            .take(MAX_LINE_BYTES)
            .read_until(b'\n', &mut buf)
            .await
        {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let line = String::from_utf8_lossy(&buf)
            .trim_end_matches(['\r', '\n'])
            .to_owned();
        if lines.send((kind, line)).await.is_err() {
            return;
        }
    }
}

fn emit(
    (kind, line): (Stream, String),
    out: &mut (dyn Write + Send),
    err: &mut (dyn Write + Send),
) {
    match kind {
        Stream::Out => say(out, &line),
        Stream::Err => {
            if !line.trim().is_empty() {
                tracing::error!(target: LOG_TARGET, output = %line, "Inertia SSR server error");
            }
            say(err, &line);
        }
    }
}

/// The status `ssr:start` exits with for the worker's `status`.
fn exit_status(status: ExitStatus, stop_sent: bool) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    if stop_sent {
        return SUCCESS;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    FAILURE
}

/// Forward the stop signal this process received to the worker as the same
/// signal, so a worker with its own `SIGINT` and `SIGTERM` handlers runs the
/// one the signal names. A worker already reaped has no process id, so a
/// reused one is never signalled.
#[cfg(unix)]
fn ask_to_stop(child: &mut Child, signal: nix::sys::signal::Signal) {
    if let Some(pid) = child.id().and_then(|pid| i32::try_from(pid).ok()) {
        let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), signal);
    }
}

/// Without a signal to forward, the stop is the kill.
#[cfg(not(unix))]
fn ask_to_stop(child: &mut Child, (): ()) {
    let _ = child.start_kill();
}

/// The stop signals this process receives while the worker runs.
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
    fn install() -> Result<Self, FrameworkError> {
        use tokio::signal::unix::{SignalKind, signal};
        let handler = |kind: SignalKind, name: &str| {
            signal(kind).map_err(|e| {
                FrameworkError::internal(format!("ssr:start: cannot handle {name}: {e}"))
            })
        };
        Ok(Self {
            interrupt: handler(SignalKind::interrupt(), "SIGINT")?,
            terminate: handler(SignalKind::terminate(), "SIGTERM")?,
        })
    }

    #[cfg(not(unix))]
    fn install() -> Result<Self, FrameworkError> {
        Ok(Self {})
    }

    /// The next stop signal, as the signal to forward to the worker. A
    /// handler whose stream ended never resolves again, rather than
    /// resolving at once in a loop.
    #[cfg(unix)]
    async fn next(&mut self) -> nix::sys::signal::Signal {
        use nix::sys::signal::Signal;
        let interrupt = async {
            if self.interrupt.recv().await.is_none() {
                std::future::pending::<()>().await;
            }
        };
        let terminate = async {
            if self.terminate.recv().await.is_none() {
                std::future::pending::<()>().await;
            }
        };
        tokio::select! {
            () = interrupt => Signal::SIGINT,
            () = terminate => Signal::SIGTERM,
        }
    }

    #[cfg(not(unix))]
    async fn next(&mut self) {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}
