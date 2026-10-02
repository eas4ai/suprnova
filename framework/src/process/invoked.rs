//! A started process: its output, its signals, and its end.

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
/// Dropping it before it is waited on kills it with every process it
/// started.
pub struct InvokedProcess {
    command: String,
    inner: Inner,
}

enum Inner {
    Real(Box<Real>),
    #[cfg(any(test, feature = "testing"))]
    Fake(super::fake::FakeInvoked),
}

struct Real {
    child: tokio::process::Child,
    pid: Option<u32>,
    /// Whether the process leads a process group of its own, so a kill can
    /// reach everything it started.
    grouped: bool,
    captured: Arc<Captured>,
    readers: Vec<JoinHandle<()>>,
    started: Instant,
    timeout: Option<Duration>,
    idle_timeout: Option<Duration>,
    status: Option<ExitStatus>,
    /// Set once the process has been waited on to the end, so a drop
    /// leaves it alone.
    finished: bool,
}

/// The output the readers collect.
struct Captured {
    state: Mutex<CapturedState>,
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
    callback: Option<OutputCallback>,
    out_text: Utf8Stream,
    err_text: Utf8Stream,
}

impl Captured {
    fn lock(&self) -> MutexGuard<'_, CapturedState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Turns chunks of bytes into text without splitting a character that a
/// chunk boundary cut in two.
#[derive(Default)]
struct Utf8Stream {
    pending: Vec<u8>,
}

impl Utf8Stream {
    fn push(&mut self, bytes: &[u8]) -> String {
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

    fn finish(&mut self) -> String {
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
        #[cfg(any(test, feature = "testing"))]
        if let Some(fake) = super::fake::resolve(&pending)? {
            return Ok(Self {
                command,
                inner: Inner::Fake(super::fake::FakeInvoked::new(fake, callback)),
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
        // when the group is killed. Not for a terminal process: a process
        // outside the terminal's foreground group is stopped when it reads
        // the terminal.
        #[cfg(unix)]
        let grouped = !pending.tty;
        #[cfg(not(unix))]
        let grouped = false;
        #[cfg(unix)]
        if grouped {
            tokio_command.process_group(0);
        }

        let mut child = tokio_command
            .spawn()
            .map_err(|source| ProcessError::NotStarted {
                program: pending.command.program(),
                source,
            })?;
        let pid = child.id();
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
                callback: if pending.quietly { None } else { callback },
                out_text: Utf8Stream::default(),
                err_text: Utf8Stream::default(),
            }),
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

        Ok(Self {
            command,
            inner: Inner::Real(Box::new(Real {
                child,
                pid,
                grouped,
                captured,
                readers,
                started: now,
                timeout: pending.timeout,
                idle_timeout: pending.idle_timeout,
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
            Inner::Real(real) => real.pid,
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
                        real.status = Some(status);
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

    /// Everything written to standard output so far.
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
                let from = if latest { *read } else { 0 };
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
            Inner::Real(real) => match real.pid {
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

    /// Whether the process ran past its timeout. If it did, it is killed
    /// with everything it started and the error says so.
    ///
    /// # Errors
    ///
    /// [`ProcessError::TimedOut`] or [`ProcessError::IdleTimedOut`].
    pub async fn ensure_not_timed_out(&mut self) -> Result<(), ProcessError> {
        let real = match &mut self.inner {
            Inner::Real(real) => real,
            #[cfg(any(test, feature = "testing"))]
            Inner::Fake(_) => return Ok(()),
        };
        match real.expired() {
            Some(expiry) => Err(real.kill_for(&self.command, expiry).await),
            None => Ok(()),
        }
    }

    /// Wait for the process to end and return its result.
    ///
    /// # Errors
    ///
    /// [`ProcessError::TimedOut`] or [`ProcessError::IdleTimedOut`] when it
    /// is killed for its timeout while waiting, [`ProcessError::Io`] when
    /// waiting fails, and [`ProcessError::FakeExhausted`] under a fake.
    pub async fn wait(mut self) -> Result<ProcessResult, ProcessError> {
        let command = self.command.clone();
        match &mut self.inner {
            Inner::Real(real) => real.wait(&command, true).await,
            #[cfg(any(test, feature = "testing"))]
            Inner::Fake(fake) => Ok(fake.result(&command)),
        }
    }

    /// Wait until `until` returns `true` for a chunk of output, or the
    /// process ends. Returns whether `until` returned `true`.
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
        loop {
            let (out, err, closed) = {
                let state = real.captured.lock();
                (
                    String::from_utf8_lossy(&state.out[out_seen..]).into_owned(),
                    String::from_utf8_lossy(&state.err[err_seen..]).into_owned(),
                    state.open_streams == 0,
                )
            };
            out_seen += out.len();
            err_seen += err.len();
            if (!out.is_empty() && until(OutputKind::Out, &out))
                || (!err.is_empty() && until(OutputKind::Err, &err))
            {
                return Ok(true);
            }
            if closed && real.status.is_some() {
                return Ok(false);
            }
            if let Some(expiry) = real.expired() {
                return Err(real.kill_for(&command, expiry).await);
            }
            let deadline = real.deadline();
            tokio::select! {
                status = real.child.wait(), if real.status.is_none() => {
                    real.status = Some(status.map_err(|source| ProcessError::Io {
                        command: command.clone(),
                        source,
                    })?);
                }
                () = real.captured.changed.notified() => {}
                () = sleep_until(deadline), if deadline.is_some() => {}
            }
        }
    }

    /// Stop the process: a terminate signal to it and everything it started,
    /// then a kill after `grace` if it has not ended. Returns its result.
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
        if real.status.is_none() {
            real.signal_all(Signal::Term);
            if tokio::time::timeout(grace, real.child.wait())
                .await
                .is_err()
            {
                real.signal_all(Signal::Kill);
            }
        }
        real.wait(&command, false).await
    }
}

/// Which of the two timeouts expired.
#[derive(Clone, Copy)]
enum Expiry {
    Timeout(Duration),
    Idle(Duration),
}

impl Real {
    /// The next moment a timeout can expire.
    fn deadline(&self) -> Option<Instant> {
        let total = self.timeout.map(|timeout| self.started + timeout);
        let idle = self
            .idle_timeout
            .map(|timeout| self.captured.lock().last_output + timeout);
        match (total, idle) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    fn expired(&self) -> Option<Expiry> {
        let now = Instant::now();
        if let Some(timeout) = self.timeout
            && now >= self.started + timeout
        {
            return Some(Expiry::Timeout(timeout));
        }
        if let Some(timeout) = self.idle_timeout
            && now >= self.captured.lock().last_output + timeout
        {
            return Some(Expiry::Idle(timeout));
        }
        None
    }

    /// Send `signal` to the process and, when it leads a group, to
    /// everything it started.
    fn signal_all(&mut self, signal: Signal) {
        match self.pid {
            Some(pid) if self.status.is_none() || self.grouped => {
                if send_signal(pid, self.grouped, signal).is_err() && signal == Signal::Kill {
                    let _ = self.child.start_kill();
                }
            }
            _ => {
                if signal == Signal::Kill {
                    let _ = self.child.start_kill();
                }
            }
        }
    }

    /// Kill the process and everything it started for `expiry`, and return
    /// the error with what it wrote first.
    async fn kill_for(&mut self, command: &str, expiry: Expiry) -> ProcessError {
        self.signal_all(Signal::Kill);
        if self.status.is_none() {
            self.status = self.child.wait().await.ok();
        }
        self.join_readers().await;
        self.finished = true;
        let result = Box::new(self.result(command));
        match expiry {
            Expiry::Timeout(timeout) => ProcessError::TimedOut {
                command: command.to_owned(),
                timeout,
                result,
            },
            Expiry::Idle(timeout) => ProcessError::IdleTimedOut {
                command: command.to_owned(),
                timeout,
                result,
            },
        }
    }

    async fn join_readers(&mut self) {
        for reader in self.readers.drain(..) {
            let _ = tokio::time::timeout(READER_GRACE, reader).await;
        }
    }

    /// Wait for the process to exit and its output to close, killing it
    /// for a timeout when `timeouts` is set.
    async fn wait(&mut self, command: &str, timeouts: bool) -> Result<ProcessResult, ProcessError> {
        loop {
            let closed = self.captured.lock().open_streams == 0;
            if self.status.is_some() && closed {
                break;
            }
            if timeouts && let Some(expiry) = self.expired() {
                return Err(self.kill_for(command, expiry).await);
            }
            let deadline = if timeouts { self.deadline() } else { None };
            tokio::select! {
                status = self.child.wait(), if self.status.is_none() => {
                    self.status = Some(status.map_err(|source| ProcessError::Io {
                        command: command.to_owned(),
                        source,
                    })?);
                }
                () = self.captured.changed.notified(), if !closed => {}
                () = sleep_until(deadline), if deadline.is_some() => {}
            }
        }
        self.join_readers().await;
        self.finished = true;
        Ok(self.result(command))
    }

    fn result(&self, command: &str) -> ProcessResult {
        let state = self.captured.lock();
        let (output, error_output) = if state.quiet {
            (String::new(), String::new())
        } else {
            (
                String::from_utf8_lossy(&state.out).into_owned(),
                String::from_utf8_lossy(&state.err).into_owned(),
            )
        };
        ProcessResult {
            command: command.to_owned(),
            exit_code: self.status.and_then(|status| status.code()),
            output,
            error_output,
        }
    }
}

impl Drop for InvokedProcess {
    fn drop(&mut self) {
        if let Inner::Real(real) = &mut self.inner
            && !real.finished
        {
            real.signal_all(Signal::Kill);
            let _ = real.child.start_kill();
        }
    }
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

/// Read one stream to its end into `captured`.
async fn read_stream<R>(mut stream: R, kind: OutputKind, captured: Arc<Captured>)
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut buffer = vec![0u8; 8192];
    loop {
        let read = match stream.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        let mut state = captured.lock();
        let state = &mut *state;
        state.last_output = Instant::now();
        let text = match kind {
            OutputKind::Out => {
                state.out.extend_from_slice(&buffer[..read]);
                state.out_text.push(&buffer[..read])
            }
            OutputKind::Err => {
                state.err.extend_from_slice(&buffer[..read]);
                state.err_text.push(&buffer[..read])
            }
        };
        if let Some(callback) = state.callback.as_mut()
            && !text.is_empty()
        {
            callback(kind, &text);
        }
        captured.changed.notify_one();
    }
    let mut state = captured.lock();
    let state = &mut *state;
    let rest = match kind {
        OutputKind::Out => state.out_text.finish(),
        OutputKind::Err => state.err_text.finish(),
    };
    if let Some(callback) = state.callback.as_mut()
        && !rest.is_empty()
    {
        callback(kind, &rest);
    }
    state.open_streams = state.open_streams.saturating_sub(1);
    captured.changed.notify_one();
}

#[cfg(unix)]
fn send_signal(pid: u32, group: bool, signal: Signal) -> Result<(), String> {
    use nix::sys::signal::{Signal as Nix, kill, killpg};
    use nix::unistd::Pid;
    let pid =
        Pid::from_raw(i32::try_from(pid).map_err(|_| "the process id is out of range".to_owned())?);
    let signal = match signal {
        Signal::Term => Nix::SIGTERM,
        Signal::Kill => Nix::SIGKILL,
        Signal::Int => Nix::SIGINT,
        Signal::Hup => Nix::SIGHUP,
        Signal::Quit => Nix::SIGQUIT,
        Signal::Usr1 => Nix::SIGUSR1,
        Signal::Usr2 => Nix::SIGUSR2,
    };
    let sent = if group {
        killpg(pid, signal)
    } else {
        kill(pid, signal)
    };
    sent.map_err(|error| error.to_string())
}

#[cfg(not(unix))]
fn send_signal(_pid: u32, _group: bool, signal: Signal) -> Result<(), String> {
    Err(format!("{signal:?} cannot be sent on this platform"))
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
