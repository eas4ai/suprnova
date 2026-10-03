//! A started process: its output, its signals, its timeouts and its end.

use super::{OutputKind, PendingProcess, ProcessError, ProcessResult, Signal};
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio::time::Instant;

/// An output callback.
pub(crate) type OutputCallback = Box<dyn FnMut(OutputKind, &str) + Send>;

/// How long the output readers get to finish after a kill, for a process
/// that left its group and still holds the pipes.
const READER_GRACE: Duration = Duration::from_secs(2);

/// A process that is running, or has run, from [`PendingProcess::start`].
///
/// Its timeout and idle timeout hold whether or not anything waits on it: a
/// watchdog kills it, with everything it started, when one passes, and
/// [`wait`](Self::wait) then returns the timeout error. Dropping it before
/// it is waited on kills it with everything it started.
pub struct InvokedProcess {
    command: String,
    inner: Inner,
}

enum Inner {
    Real(Box<Real>),
    #[cfg(any(test, feature = "testing"))]
    Fake(super::fake::FakeInvoked),
}

/// How a kill reaches everything the program started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    /// The program leads a process group of its own, and the group is
    /// signalled. Unix, for every process that does not use the terminal.
    Group,
    /// The program and its descendants are signalled one by one. A terminal
    /// process stays in the terminal's group, which it must share to read
    /// the terminal, so its descendants are found in the process table.
    /// Windows signals the tree with `taskkill /T`.
    Tree,
}

/// What a kill is aimed at, shared with the watchdog.
#[derive(Debug, Clone, Copy)]
struct Target {
    pid: Option<u32>,
    reach: Reach,
}

impl Target {
    /// Send `signal` to the program and everything it started.
    /// `leader_reaped` says the program has been waited on, so its id may
    /// belong to another process now; `streams_open` says something still
    /// holds its output, so members of its group are alive.
    fn signal_all(&self, signal: Signal, leader_reaped: bool, streams_open: bool) {
        let Some(pid) = self.pid else {
            return;
        };
        match self.reach {
            // A group id cannot be reused while the group has a member, so
            // signalling it is safe while anything holds the output.
            Reach::Group if !leader_reaped || streams_open => {
                let _ = send_signal(pid, true, signal);
            }
            Reach::Tree if !leader_reaped => signal_tree(pid, signal),
            _ => {}
        }
    }
}

struct Real {
    child: tokio::process::Child,
    target: Target,
    captured: Arc<Captured>,
    readers: Vec<JoinHandle<()>>,
    watchdog: Option<JoinHandle<()>>,
    status: Option<ExitStatus>,
    /// Set once the process has been waited on to the end, so a drop
    /// leaves it alone.
    finished: bool,
}

/// The output the readers collect and the timeouts the watchdog keeps.
struct Captured {
    state: Mutex<CapturedState>,
    /// Kept apart from `state`, so a slow callback does not hold up the
    /// timeout checks.
    callback: Mutex<Option<OutputCallback>>,
    changed: Notify,
}

struct CapturedState {
    out: Vec<u8>,
    err: Vec<u8>,
    out_read: usize,
    err_read: usize,
    last_output: Instant,
    open_streams: usize,
    quiet: bool,
    started: Instant,
    timeout: Option<Duration>,
    idle_timeout: Option<Duration>,
    /// Set when the watchdog, or a check, killed the process for a timeout.
    expired: Option<Expiry>,
    /// Set once the program has been waited on, so its id may belong to
    /// another process.
    reaped: bool,
}

impl CapturedState {
    /// The next moment a timeout can pass; `None` when there is none, or it
    /// lies past what the clock can hold.
    fn deadline(&self) -> Option<Instant> {
        let total = self
            .timeout
            .and_then(|timeout| self.started.checked_add(timeout));
        let idle = self
            .idle_timeout
            .and_then(|timeout| self.last_output.checked_add(timeout));
        match (total, idle) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// The timeout that has passed at `now`, if one has.
    fn due(&self, now: Instant) -> Option<Expiry> {
        if let Some(timeout) = self.timeout
            && self
                .started
                .checked_add(timeout)
                .is_some_and(|at| now >= at)
        {
            return Some(Expiry::Timeout(timeout));
        }
        if let Some(timeout) = self.idle_timeout
            && self
                .last_output
                .checked_add(timeout)
                .is_some_and(|at| now >= at)
        {
            return Some(Expiry::Idle(timeout));
        }
        None
    }
}

impl Captured {
    fn lock(&self) -> MutexGuard<'_, CapturedState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Call the output callback with `text`. A callback that panics is
    /// dropped, so the output keeps being read and the process can end.
    fn emit(&self, kind: OutputKind, text: &str) {
        if text.is_empty() {
            return;
        }
        let mut callback = self
            .callback
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(call) = callback.as_mut() {
            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| call(kind, text)));
            if outcome.is_err() {
                *callback = None;
                tracing::error!(
                    "a process output callback panicked; it is called no more for this process"
                );
            }
        }
    }
}

/// Which of the two timeouts passed.
#[derive(Debug, Clone, Copy)]
enum Expiry {
    Timeout(Duration),
    Idle(Duration),
}

impl Expiry {
    fn error(self, command: &str, result: ProcessResult) -> ProcessError {
        match self {
            Expiry::Timeout(timeout) => ProcessError::TimedOut {
                command: command.to_owned(),
                timeout,
                result: Box::new(result),
            },
            Expiry::Idle(timeout) => ProcessError::IdleTimedOut {
                command: command.to_owned(),
                timeout,
                result: Box::new(result),
            },
        }
    }
}

/// Turns chunks of bytes into text without splitting a character that a
/// chunk boundary cut in two.
#[derive(Default)]
pub(crate) struct Utf8Stream {
    pending: Vec<u8>,
}

impl Utf8Stream {
    pub(crate) fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        match std::str::from_utf8(&self.pending) {
            Ok(text) => {
                let text = text.to_owned();
                self.pending.clear();
                text
            }
            Err(error) if error.error_len().is_none() => {
                let valid = error.valid_up_to();
                let text = String::from_utf8_lossy(&self.pending[..valid]).into_owned();
                self.pending.drain(..valid);
                text
            }
            Err(_) => {
                let text = String::from_utf8_lossy(&self.pending).into_owned();
                self.pending.clear();
                text
            }
        }
    }

    pub(crate) fn finish(&mut self) -> String {
        let text = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        text
    }
}

impl InvokedProcess {
    pub(crate) fn start(
        pending: PendingProcess,
        callback: Option<OutputCallback>,
    ) -> Result<Self, ProcessError> {
        let command = pending.command.line();
        if pending.tty && pending.idle_timeout.is_some() {
            // A terminal process writes to the terminal, so there is no
            // output to watch, and every idle timeout would pass.
            return Err(ProcessError::Unsupported {
                command,
                reason: "an idle timeout needs the output, and a tty process writes to the \
                         terminal instead"
                    .into(),
            });
        }
        let callback = if pending.quietly { None } else { callback };
        #[cfg(any(test, feature = "testing"))]
        if let Some(fake) = super::fake::resolve(&pending)? {
            return Ok(Self {
                command,
                inner: Inner::Fake(super::fake::FakeInvoked::new(
                    fake,
                    callback,
                    pending.quietly,
                )),
            });
        }

        let mut tokio_command = pending.command.to_tokio();
        if let Some(path) = &pending.path {
            tokio_command.current_dir(path);
        }
        for (key, value) in &pending.env {
            tokio_command.env(key, value);
        }
        if pending.tty {
            tokio_command
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());
        } else {
            tokio_command
                .stdin(if pending.input.is_some() {
                    Stdio::piped()
                } else {
                    Stdio::null()
                })
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
        }
        tokio_command.kill_on_drop(true);
        // A process in a group of its own takes everything it starts with it
        // when the group is killed. Not a terminal process: one outside the
        // terminal's foreground group is stopped when it reads the terminal,
        // so its descendants are found and killed one by one instead.
        #[cfg(unix)]
        let reach = if pending.tty {
            Reach::Tree
        } else {
            Reach::Group
        };
        #[cfg(not(unix))]
        let reach = Reach::Tree;
        #[cfg(unix)]
        if reach == Reach::Group {
            tokio_command.process_group(0);
        }

        let mut child = tokio_command
            .spawn()
            .map_err(|source| ProcessError::NotStarted {
                program: pending.command.program(),
                source,
            })?;
        let target = Target {
            pid: child.id(),
            reach,
        };
        let now = Instant::now();
        let captured = Arc::new(Captured {
            state: Mutex::new(CapturedState {
                out: Vec::new(),
                err: Vec::new(),
                out_read: 0,
                err_read: 0,
                last_output: now,
                open_streams: 0,
                quiet: pending.quietly,
                started: now,
                timeout: pending.timeout,
                idle_timeout: pending.idle_timeout,
                expired: None,
                reaped: false,
            }),
            callback: Mutex::new(callback),
            changed: Notify::new(),
        });

        let mut readers = Vec::new();
        if let Some(stdout) = child.stdout.take() {
            captured.lock().open_streams += 1;
            readers.push(tokio::spawn(read_stream(
                stdout,
                OutputKind::Out,
                Arc::clone(&captured),
            )));
        }
        if let Some(stderr) = child.stderr.take() {
            captured.lock().open_streams += 1;
            readers.push(tokio::spawn(read_stream(
                stderr,
                OutputKind::Err,
                Arc::clone(&captured),
            )));
        }
        if let (Some(mut stdin), Some(input)) = (child.stdin.take(), pending.input) {
            // A process that exits without reading all of its input closes
            // the pipe; the write error that leaves is not the process's.
            tokio::spawn(async move {
                let _ = stdin.write_all(&input).await;
                let _ = stdin.shutdown().await;
            });
        }
        let watchdog = (pending.timeout.is_some() || pending.idle_timeout.is_some())
            .then(|| tokio::spawn(watchdog(Arc::clone(&captured), target)));

        Ok(Self {
            command,
            inner: Inner::Real(Box::new(Real {
                child,
                target,
                captured,
                readers,
                watchdog,
                status: None,
                finished: false,
            })),
        })
    }

    /// The command line.
    pub fn command(&self) -> &str {
        &self.command
    }

    /// The process id, while the process has one.
    pub fn id(&self) -> Option<u32> {
        match &self.inner {
            Inner::Real(real) => real.target.pid,
            #[cfg(any(test, feature = "testing"))]
            Inner::Fake(fake) => Some(fake.id()),
        }
    }

    /// Whether the process is still running.
    pub fn running(&mut self) -> bool {
        match &mut self.inner {
            Inner::Real(real) => {
                if real.status.is_some() {
                    return false;
                }
                match real.child.try_wait() {
                    Ok(Some(status)) => {
                        real.record(status);
                        false
                    }
                    Ok(None) => true,
                    Err(_) => false,
                }
            }
            #[cfg(any(test, feature = "testing"))]
            Inner::Fake(fake) => fake.running(),
        }
    }

    /// Everything written to standard output so far; empty under
    /// [`quietly`](PendingProcess::quietly).
    pub fn output(&self) -> String {
        self.read(OutputKind::Out, false)
    }

    /// Everything written to standard error so far.
    pub fn error_output(&self) -> String {
        self.read(OutputKind::Err, false)
    }

    /// What was written to standard output since the last call.
    pub fn latest_output(&self) -> String {
        self.read(OutputKind::Out, true)
    }

    /// What was written to standard error since the last call.
    pub fn latest_error_output(&self) -> String {
        self.read(OutputKind::Err, true)
    }

    fn read(&self, kind: OutputKind, latest: bool) -> String {
        match &self.inner {
            Inner::Real(real) => {
                let mut state = real.captured.lock();
                let state = &mut *state;
                let (bytes, read) = match kind {
                    OutputKind::Out => (&state.out, &mut state.out_read),
                    OutputKind::Err => (&state.err, &mut state.err_read),
                };
                let from = if latest { (*read).min(bytes.len()) } else { 0 };
                let text = String::from_utf8_lossy(&bytes[from..]).into_owned();
                if latest {
                    *read = bytes.len();
                }
                text
            }
            #[cfg(any(test, feature = "testing"))]
            Inner::Fake(fake) => fake.read(kind, latest),
        }
    }

    /// Send `signal` to the process itself; what it started is not
    /// signalled. [`stop`](Self::stop) ends everything.
    ///
    /// # Errors
    ///
    /// [`ProcessError::Signal`] when the process has exited or the platform
    /// cannot send the signal.
    pub fn signal(&self, signal: Signal) -> Result<(), ProcessError> {
        match &self.inner {
            Inner::Real(real) => match real.target.pid {
                Some(pid) if real.status.is_none() => {
                    send_signal(pid, false, signal).map_err(|message| ProcessError::Signal {
                        command: self.command.clone(),
                        message,
                    })
                }
                _ => Err(ProcessError::Signal {
                    command: self.command.clone(),
                    message: "the process has exited".into(),
                }),
            },
            #[cfg(any(test, feature = "testing"))]
            Inner::Fake(fake) => {
                fake.signal(signal);
                Ok(())
            }
        }
    }

    /// Whether a faked process was sent `signal`, for a test to assert on.
    /// A real process always answers `false`: the signal went to the
    /// operating system.
    pub fn has_received_signal(&self, signal: Signal) -> bool {
        match &self.inner {
            Inner::Real(_) => {
                let _ = signal;
                false
            }
            #[cfg(any(test, feature = "testing"))]
            Inner::Fake(fake) => fake.has_received_signal(signal),
        }
    }

    /// Fail when the process ran past a timeout. If it did, it has been
    /// killed with everything it started, and the error says which timeout
    /// passed.
    ///
    /// # Errors
    ///
    /// [`ProcessError::TimedOut`] or [`ProcessError::IdleTimedOut`].
    pub async fn ensure_not_timed_out(&mut self) -> Result<(), ProcessError> {
        let command = self.command.clone();
        let real = match &mut self.inner {
            Inner::Real(real) => real,
            #[cfg(any(test, feature = "testing"))]
            Inner::Fake(_) => return Ok(()),
        };
        match real.expire_if_due() {
            Some(expiry) => Err(real.finish_killed(&command, expiry).await),
            None => Ok(()),
        }
    }

    /// Wait for the process to end and return its result.
    ///
    /// # Errors
    ///
    /// [`ProcessError::TimedOut`] or [`ProcessError::IdleTimedOut`] when a
    /// timeout passed and the process was killed, [`ProcessError::Io`] when
    /// waiting fails.
    pub async fn wait(mut self) -> Result<ProcessResult, ProcessError> {
        let command = self.command.clone();
        match &mut self.inner {
            Inner::Real(real) => real.wait(&command).await,
            #[cfg(any(test, feature = "testing"))]
            Inner::Fake(fake) => Ok(fake.result(&command)),
        }
    }

    /// Wait until `until` returns `true` for a chunk of output, or the
    /// process ends. Returns whether `until` returned `true`. Output the
    /// process wrote before the call is offered too.
    ///
    /// # Errors
    ///
    /// As [`wait`](Self::wait), for a timeout while waiting.
    pub async fn wait_until<F>(&mut self, mut until: F) -> Result<bool, ProcessError>
    where
        F: FnMut(OutputKind, &str) -> bool,
    {
        let command = self.command.clone();
        let real = match &mut self.inner {
            Inner::Real(real) => real,
            #[cfg(any(test, feature = "testing"))]
            Inner::Fake(fake) => return Ok(fake.wait_until(&mut until)),
        };
        let (mut out_seen, mut err_seen) = (0, 0);
        let (mut out_text, mut err_text) = (Utf8Stream::default(), Utf8Stream::default());
        loop {
            let (out, err, closed, expired) = {
                let state = real.captured.lock();
                let out = state.out.get(out_seen..).unwrap_or_default().to_vec();
                let err = state.err.get(err_seen..).unwrap_or_default().to_vec();
                (out, err, state.open_streams == 0, state.expired)
            };
            out_seen += out.len();
            err_seen += err.len();
            let (out, err) = (out_text.push(&out), err_text.push(&err));
            if (!out.is_empty() && until(OutputKind::Out, &out))
                || (!err.is_empty() && until(OutputKind::Err, &err))
            {
                return Ok(true);
            }
            if let Some(expiry) = expired {
                return Err(real.finish_killed(&command, expiry).await);
            }
            if closed && real.status.is_some() {
                return Ok(false);
            }
            tokio::select! {
                status = real.child.wait(), if real.status.is_none() => {
                    real.record(status.map_err(|source| ProcessError::Io {
                        command: command.clone(),
                        source,
                    })?);
                }
                () = real.captured.changed.notified() => {}
            }
        }
    }

    /// Stop the process: a terminate signal to it and everything it started,
    /// then, after `grace`, a kill to whatever is left. Returns its result.
    ///
    /// # Errors
    ///
    /// [`ProcessError::Io`] when waiting fails.
    pub async fn stop(mut self, grace: Duration) -> Result<ProcessResult, ProcessError> {
        let command = self.command.clone();
        let real = match &mut self.inner {
            Inner::Real(real) => real,
            #[cfg(any(test, feature = "testing"))]
            Inner::Fake(fake) => {
                fake.stop();
                return Ok(fake.result(&command));
            }
        };
        if let Some(watchdog) = real.watchdog.take() {
            watchdog.abort();
        }
        real.signal_all(Signal::Term);
        let ended = tokio::time::timeout(grace, real.wait_for_end(&command)).await;
        if !matches!(ended, Ok(Ok(()))) {
            real.signal_all(Signal::Kill);
        }
        real.finish_bounded(&command).await?;
        Ok(real.result(&command))
    }
}

impl Real {
    fn record(&mut self, status: ExitStatus) {
        self.status = Some(status);
        self.captured.lock().reaped = true;
    }

    fn streams_open(&self) -> bool {
        self.captured.lock().open_streams > 0
    }

    fn signal_all(&mut self, signal: Signal) {
        let streams_open = self.streams_open();
        self.target
            .signal_all(signal, self.status.is_some(), streams_open);
        if signal == Signal::Kill && self.status.is_none() {
            let _ = self.child.start_kill();
        }
    }

    /// Record and act on a timeout that has passed and the watchdog has not
    /// handled yet.
    fn expire_if_due(&mut self) -> Option<Expiry> {
        let expiry = {
            let mut state = self.captured.lock();
            if state.expired.is_none() {
                state.expired = state.due(Instant::now());
            }
            state.expired
        };
        if expiry.is_some() {
            self.signal_all(Signal::Kill);
        }
        expiry
    }

    /// Wait until the program has exited and its output has closed.
    async fn wait_for_end(&mut self, command: &str) -> Result<(), ProcessError> {
        loop {
            if self.status.is_some() && !self.streams_open() {
                return Ok(());
            }
            tokio::select! {
                status = self.child.wait(), if self.status.is_none() => {
                    self.record(status.map_err(|source| ProcessError::Io {
                        command: command.to_owned(),
                        source,
                    })?);
                }
                () = self.captured.changed.notified() => {}
            }
        }
    }

    /// Reap a killed program and give its output readers a bounded time to
    /// finish, for a process that left its group and holds the pipes.
    async fn finish_bounded(&mut self, command: &str) -> Result<(), ProcessError> {
        if self.status.is_none() {
            let status = self.child.wait().await.map_err(|source| ProcessError::Io {
                command: command.to_owned(),
                source,
            })?;
            self.record(status);
        }
        for reader in self.readers.drain(..) {
            let abort = reader.abort_handle();
            if tokio::time::timeout(READER_GRACE, reader).await.is_err() {
                abort.abort();
            }
        }
        if let Some(watchdog) = self.watchdog.take() {
            watchdog.abort();
        }
        self.finished = true;
        Ok(())
    }

    /// End a process killed for `expiry` and return the error with what it
    /// wrote first.
    async fn finish_killed(&mut self, command: &str, expiry: Expiry) -> ProcessError {
        self.signal_all(Signal::Kill);
        if let Err(error) = self.finish_bounded(command).await {
            return error;
        }
        expiry.error(command, self.result(command))
    }

    async fn wait(&mut self, command: &str) -> Result<ProcessResult, ProcessError> {
        loop {
            let (closed, expired) = {
                let state = self.captured.lock();
                (state.open_streams == 0, state.expired)
            };
            if let Some(expiry) = expired {
                return Err(self.finish_killed(command, expiry).await);
            }
            if self.status.is_some() && closed {
                break;
            }
            tokio::select! {
                status = self.child.wait(), if self.status.is_none() => {
                    self.record(status.map_err(|source| ProcessError::Io {
                        command: command.to_owned(),
                        source,
                    })?);
                }
                () = self.captured.changed.notified() => {}
            }
        }
        self.finish_bounded(command).await?;
        Ok(self.result(command))
    }

    fn result(&self, command: &str) -> ProcessResult {
        let state = self.captured.lock();
        ProcessResult::new(
            command.to_owned(),
            self.status.and_then(|status| status.code()),
            state.out.clone(),
            state.err.clone(),
        )
    }
}

impl Drop for InvokedProcess {
    fn drop(&mut self) {
        if let Inner::Real(real) = &mut self.inner
            && !real.finished
        {
            if let Some(watchdog) = real.watchdog.take() {
                watchdog.abort();
            }
            real.signal_all(Signal::Kill);
        }
    }
}

/// Kill the process, with everything it started, when a timeout passes,
/// whether or not anything waits on it.
async fn watchdog(captured: Arc<Captured>, target: Target) {
    loop {
        let deadline = {
            let state = captured.lock();
            if state.expired.is_some() {
                return;
            }
            state.deadline()
        };
        let Some(deadline) = deadline else {
            return;
        };
        tokio::time::sleep_until(deadline).await;
        let expired = {
            let mut state = captured.lock();
            let due = state.due(Instant::now());
            if due.is_some() {
                state.expired = due;
            }
            due.map(|_| (state.reaped, state.open_streams > 0))
        };
        if let Some((reaped, streams_open)) = expired {
            target.signal_all(Signal::Kill, reaped, streams_open);
            captured.changed.notify_one();
            return;
        }
    }
}

/// Lowers the count of open streams when a reader ends, however it ends.
struct StreamClosed(Arc<Captured>);

impl Drop for StreamClosed {
    fn drop(&mut self) {
        let mut state = self.0.lock();
        state.open_streams = state.open_streams.saturating_sub(1);
        drop(state);
        self.0.changed.notify_one();
    }
}

/// Read one stream to its end into `captured`.
async fn read_stream<R>(mut stream: R, kind: OutputKind, captured: Arc<Captured>)
where
    R: tokio::io::AsyncRead + Unpin,
{
    let _closed = StreamClosed(Arc::clone(&captured));
    let mut text = Utf8Stream::default();
    let mut buffer = vec![0u8; 8192];
    loop {
        let read = match stream.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        let quiet = {
            let mut state = captured.lock();
            state.last_output = Instant::now();
            if !state.quiet {
                match kind {
                    OutputKind::Out => state.out.extend_from_slice(&buffer[..read]),
                    OutputKind::Err => state.err.extend_from_slice(&buffer[..read]),
                }
            }
            state.quiet
        };
        if !quiet {
            captured.emit(kind, &text.push(&buffer[..read]));
        }
        captured.changed.notify_one();
    }
    captured.emit(kind, &text.finish());
}

/// Send `signal` to `pid`, or to the process group it leads.
#[cfg(unix)]
fn send_signal(pid: u32, group: bool, signal: Signal) -> Result<(), String> {
    use nix::sys::signal::{kill, killpg};
    let pid = nix_pid(pid)?;
    let sent = if group {
        killpg(pid, nix_signal(signal))
    } else {
        kill(pid, nix_signal(signal))
    };
    sent.map_err(|error| error.to_string())
}

#[cfg(unix)]
fn nix_pid(pid: u32) -> Result<nix::unistd::Pid, String> {
    Ok(nix::unistd::Pid::from_raw(i32::try_from(pid).map_err(
        |_| "the process id is out of range".to_owned(),
    )?))
}

#[cfg(unix)]
fn nix_signal(signal: Signal) -> nix::sys::signal::Signal {
    use nix::sys::signal::Signal as Nix;
    match signal {
        Signal::Term => Nix::SIGTERM,
        Signal::Kill => Nix::SIGKILL,
        Signal::Int => Nix::SIGINT,
        Signal::Hup => Nix::SIGHUP,
        Signal::Quit => Nix::SIGQUIT,
        Signal::Usr1 => Nix::SIGUSR1,
        Signal::Usr2 => Nix::SIGUSR2,
    }
}

/// Signal `pid` and every process descended from it, found in the process
/// table with `ps`. For a kill the program is stopped first, so it starts
/// nothing new while its descendants are listed.
#[cfg(unix)]
fn signal_tree(pid: u32, signal: Signal) {
    use nix::sys::signal::{Signal as Nix, kill};
    let Ok(root) = nix_pid(pid) else {
        return;
    };
    if signal == Signal::Kill {
        let _ = kill(root, Nix::SIGSTOP);
    }
    for descendant in descendants(pid) {
        if let Ok(descendant) = nix_pid(descendant) {
            let _ = kill(descendant, nix_signal(signal));
        }
    }
    let _ = kill(root, nix_signal(signal));
}

/// Every process descended from `root`, from `ps -A -o pid= -o ppid=`.
#[cfg(unix)]
fn descendants(root: u32) -> Vec<u32> {
    let Ok(listing) = std::process::Command::new("ps")
        .args(["-A", "-o", "pid=", "-o", "ppid="])
        .stderr(Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    let pairs: Vec<(u32, u32)> = String::from_utf8_lossy(&listing.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            Some((fields.next()?.parse().ok()?, fields.next()?.parse().ok()?))
        })
        .collect();
    let mut found = Vec::new();
    let mut frontier = vec![root];
    while let Some(parent) = frontier.pop() {
        for (pid, ppid) in &pairs {
            if *ppid == parent && *pid != root && !found.contains(pid) {
                found.push(*pid);
                frontier.push(*pid);
            }
        }
    }
    found
}

/// On Windows, `taskkill` ends a process (`/F` forces it) and, for the
/// group, its tree (`/T`). Other signals have no Windows meaning.
#[cfg(not(unix))]
fn send_signal(pid: u32, tree: bool, signal: Signal) -> Result<(), String> {
    let mut command = std::process::Command::new("taskkill");
    command.args(["/PID", &pid.to_string()]);
    if tree {
        command.arg("/T");
    }
    match signal {
        Signal::Kill => {
            command.arg("/F");
        }
        Signal::Term => {}
        other => return Err(format!("{other:?} cannot be sent on Windows")),
    }
    let status = command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| error.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("taskkill exited with {status}"))
    }
}

#[cfg(not(unix))]
fn signal_tree(pid: u32, signal: Signal) {
    let _ = send_signal(pid, true, signal);
}

#[cfg(test)]
mod tests {
    use super::Utf8Stream;

    #[test]
    fn a_character_cut_by_a_chunk_boundary_comes_out_whole() {
        let mut stream = Utf8Stream::default();
        let bytes = "é".as_bytes();
        assert_eq!(stream.push(&bytes[..1]), "");
        assert_eq!(stream.push(&bytes[1..]), "é");
        assert_eq!(stream.finish(), "");
    }

    #[test]
    fn invalid_bytes_are_replaced() {
        let mut stream = Utf8Stream::default();
        assert_eq!(stream.push(&[b'a', 0xff, b'b']), "a\u{fffd}b");
    }
}
