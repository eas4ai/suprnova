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
    /// Send `signal` to the program and everything it started. The caller
    /// holds the `released` lock and has seen it unset (see
    /// `Captured::released`): once the program is reaped its id - and, for a
    /// group, the group's id, which is the same number - may belong to
    /// another process.
    ///
    /// Nothing short of the unreaped program proves a group id is still
    /// ours. A process holding the program's output may have left the group,
    /// so an open pipe says nothing about the group having a member, and an
    /// empty group's id is free for reuse once the program is reaped. So the
    /// program is not reaped while its output is open (see
    /// `Real::may_reap`): until then, the program, even exited, keeps its id
    /// and the group's pinned to it.
    ///
    /// The descendants a [`Reach::Tree`] kill finds in the process table are
    /// not this process's children, so nothing pins their ids: one that ends
    /// between the lookup and its signal can still hand its id on.
    fn send_all(&self, signal: Signal) {
        let Some(pid) = self.pid else {
            return;
        };
        match self.reach {
            Reach::Group => {
                let _ = send_signal(pid, true, signal);
            }
            Reach::Tree => signal_tree(pid, signal),
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
    /// Set once the program's id is no longer ours to signal: it has been
    /// reaped, or the process was dropped and Tokio reaps it later.
    ///
    /// A signal is sent only with this lock held and the flag unset, and on
    /// Unix the reap happens with the lock held too, in the step that sets
    /// the flag (see `reap`). So the owner cannot reap between a watchdog's
    /// check and its kill, which would send the kill to an id another
    /// process may have taken.
    released: Mutex<bool>,
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

    fn released(&self) -> MutexGuard<'_, bool> {
        self.released
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
    /// The text `bytes` complete, each invalid sequence replaced with
    /// U+FFFD as `String::from_utf8_lossy` replaces it. Only an incomplete
    /// character at the very end is held back for the next chunk; one
    /// after an invalid byte is not, so `[0xff, 0xe2]` then `[0x82, 0xac]`
    /// is the replacement character and then the euro sign.
    pub(crate) fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut text = String::new();
        let mut start = 0;
        while start < self.pending.len() {
            let rest = &self.pending[start..];
            match std::str::from_utf8(rest) {
                Ok(valid) => {
                    text.push_str(valid);
                    start = self.pending.len();
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    text.push_str(&String::from_utf8_lossy(&rest[..valid]));
                    match error.error_len() {
                        Some(invalid) => {
                            text.push('\u{fffd}');
                            start += valid + invalid;
                        }
                        // The rest is the start of a character the next
                        // chunk may finish.
                        None => {
                            start += valid;
                            break;
                        }
                    }
                }
            }
        }
        self.pending.drain(..start);
        text
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
            }),
            callback: Mutex::new(callback),
            changed: Notify::new(),
            released: Mutex::new(false),
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
    ///
    /// On a platform that cannot look at an exit without collecting it
    /// (macOS, for one), a program that exits while something it started
    /// still holds its output counts as running until that output closes:
    /// collecting it any earlier would free its id for reuse while its
    /// group may still be signalled.
    pub fn running(&mut self) -> bool {
        match &mut self.inner {
            Inner::Real(real) => {
                if real.status.is_some() {
                    return false;
                }
                if !real.may_reap() {
                    return !real.exited_unreaped();
                }
                match try_reap(&mut real.child, &real.captured) {
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
                Some(pid) if real.status.is_none() && !real.exited_unreaped() => {
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
                // The output is complete, so a character it ended inside of
                // is now a replacement character, which the output
                // callback is offered too.
                let (out, err) = (out_text.finish(), err_text.finish());
                return Ok((!out.is_empty() && until(OutputKind::Out, &out))
                    || (!err.is_empty() && until(OutputKind::Err, &err)));
            }
            let reap_now = real.status.is_none() && real.may_reap();
            tokio::select! {
                status = reap(&mut real.child, &real.captured), if reap_now => {
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
        Ok(real.take_result(&command))
    }
}

impl Real {
    /// Keep the status of the program, which `reap` or `try_reap` has
    /// reaped and released.
    fn record(&mut self, status: ExitStatus) {
        self.status = Some(status);
        #[cfg(test)]
        reap_race::reaped(self.target.pid);
    }

    fn streams_open(&self) -> bool {
        self.captured.lock().open_streams > 0
    }

    /// Whether the program may be reaped now: not while its output is open.
    ///
    /// Reaping frees the program's id, and the id of the group it leads.
    /// While something holds its output, the group may still have to be
    /// signalled, and only the unreaped program keeps that id from being
    /// handed to an unrelated process. On Windows the child's handle keeps
    /// its id, so reaping is never early there.
    fn may_reap(&self) -> bool {
        cfg!(not(unix)) || !self.streams_open()
    }

    /// Whether the program has exited, found without reaping it.
    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        all(target_os = "linux", not(target_env = "uclibc")),
    ))]
    fn exited_unreaped(&self) -> bool {
        use nix::sys::wait::{Id, WaitPidFlag, WaitStatus, waitid};
        let Some(pid) = self.target.pid.and_then(|pid| nix_pid(pid).ok()) else {
            return false;
        };
        // `WNOWAIT` leaves the exit to be collected later, so the id stays
        // pinned. An error means there is no such child to wait on any more.
        !matches!(
            waitid(
                Id::Pid(pid),
                WaitPidFlag::WEXITED | WaitPidFlag::WNOHANG | WaitPidFlag::WNOWAIT,
            ),
            Ok(WaitStatus::StillAlive)
        )
    }

    /// Whether the program has exited, found without reaping it. This
    /// platform cannot tell without reaping, so the answer waits for the
    /// reap, which waits for the program's output to close.
    #[cfg(not(any(
        target_os = "android",
        target_os = "freebsd",
        all(target_os = "linux", not(target_env = "uclibc")),
    )))]
    fn exited_unreaped(&self) -> bool {
        false
    }

    fn signal_all(&mut self, signal: Signal) {
        #[cfg(test)]
        reap_race::owner_waiting(self.target.pid);
        let released = self.captured.released();
        if !*released {
            self.target.send_all(signal);
        }
        drop(released);
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
            let reap_now = self.status.is_none() && self.may_reap();
            tokio::select! {
                status = reap(&mut self.child, &self.captured), if reap_now => {
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
        // The watchdog goes before the reap: once the program is reaped, its
        // id is no longer ours to signal.
        if let Some(watchdog) = self.watchdog.take() {
            watchdog.abort();
        }
        if self.status.is_none() {
            let status = reap(&mut self.child, &self.captured)
                .await
                .map_err(|source| ProcessError::Io {
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
            let reap_now = self.status.is_none() && self.may_reap();
            tokio::select! {
                status = reap(&mut self.child, &self.captured), if reap_now => {
                    self.record(status.map_err(|source| ProcessError::Io {
                        command: command.to_owned(),
                        source,
                    })?);
                }
                () = self.captured.changed.notified() => {}
            }
        }
        self.finish_bounded(command).await?;
        Ok(self.take_result(command))
    }

    /// The result so far, copied: the process may still be asked for its
    /// output after a timeout a `wait_until` reports.
    fn result(&self, command: &str) -> ProcessResult {
        let state = self.captured.lock();
        ProcessResult::new(
            command.to_owned(),
            self.status.and_then(|status| status.code()),
            state.out.clone(),
            state.err.clone(),
        )
    }

    /// The result of a process that is finished with: the captured output
    /// moves into it rather than being copied. Only the calls that consume
    /// the process, `wait` and `stop`, take it.
    fn take_result(&mut self, command: &str) -> ProcessResult {
        let mut state = self.captured.lock();
        let out = std::mem::take(&mut state.out);
        let err = std::mem::take(&mut state.err);
        drop(state);
        ProcessResult::new(
            command.to_owned(),
            self.status.and_then(|status| status.code()),
            out,
            err,
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
            // Tokio reaps the program after the drop, without this lock, so a
            // watchdog that is still running must leave its id alone from now.
            *real.captured.released() = true;
        }
    }
}

/// Reap the program if it has exited, and mark its id released in the same
/// step, under the `released` lock: a signal sent under that lock reaches
/// the program, or its group, while the id is still pinned.
fn try_reap(
    child: &mut tokio::process::Child,
    captured: &Captured,
) -> std::io::Result<Option<ExitStatus>> {
    let mut released = captured.released();
    let status = child.try_wait()?;
    if status.is_some() {
        *released = true;
    }
    Ok(status)
}

/// Wait for the program to exit, then reap it with [`try_reap`].
///
/// On Unix the exit is awaited without collecting it: every `SIGCHLD` wakes
/// the wait, and only `try_reap`, under the lock, collects the exit. Tokio's
/// own `wait` would collect it with no lock held, so a watchdog's kill could
/// land after the reap. The listener is in place before the first look, so
/// an exit between a look and the wait still wakes it.
#[cfg(unix)]
async fn reap(
    child: &mut tokio::process::Child,
    captured: &Captured,
) -> std::io::Result<ExitStatus> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut exits = signal(SignalKind::child())?;
    loop {
        if let Some(status) = try_reap(child, captured)? {
            return Ok(status);
        }
        if exits.recv().await.is_none() {
            // The runtime is shutting down and no more exits will be
            // reported; wait the way Tokio does instead.
            let status = child.wait().await?;
            *captured.released() = true;
            return Ok(status);
        }
    }
}

/// Wait for the program to exit and reap it. Windows keeps a program's id
/// for as long as its handle is open, and the handle lives as long as the
/// child, so there is no window to close.
#[cfg(not(unix))]
async fn reap(
    child: &mut tokio::process::Child,
    captured: &Captured,
) -> std::io::Result<ExitStatus> {
    let status = child.wait().await?;
    *captured.released() = true;
    Ok(status)
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
            due.is_some()
        };
        if expired {
            // Checked and sent under one hold of the lock the reap takes, so
            // the program cannot be reaped in between.
            let released = captured.released();
            if !*released {
                #[cfg(test)]
                reap_race::watchdog_parked(target.pid);
                target.send_all(Signal::Kill);
                #[cfg(test)]
                reap_race::watchdog_sent(target.pid);
            }
            drop(released);
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
    #[cfg(all(test, target_os = "linux"))]
    reap_race::sending(pid);
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

/// Test seams for the race between a watchdog's kill and the owner's reap:
/// a test parks the watchdog after it has decided to signal, lets the owner
/// run, and learns whether a signal went to an id that was already reaped.
/// Every hook acts only on the one process a test watches.
#[cfg(test)]
mod reap_race {
    use std::sync::{Condvar, Mutex, MutexGuard};

    #[derive(Default)]
    pub(super) struct State {
        watching: bool,
        pid: Option<u32>,
        hold_watchdog: bool,
        pub(super) watchdog_parked: bool,
        pub(super) watchdog_sent: bool,
        pub(super) reaped: bool,
        pub(super) owner_waiting: bool,
        pub(super) sent_after_reap: bool,
    }

    static STATE: Mutex<Option<State>> = Mutex::new(None);
    static CHANGED: Condvar = Condvar::new();

    fn lock() -> MutexGuard<'static, Option<State>> {
        STATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The watched state, once the watched process is known. A hook that
    /// runs before the test has named it waits for the name.
    fn watched(pid: Option<u32>) -> Option<MutexGuard<'static, Option<State>>> {
        let mut guard = lock();
        loop {
            let (watching, named) = guard.as_ref().map(|state| (state.watching, state.pid))?;
            if !watching {
                return None;
            }
            match named {
                Some(named) if Some(named) == pid => return Some(guard),
                Some(_) => return None,
                None => {
                    guard = CHANGED
                        .wait(guard)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
            }
        }
    }

    fn update(pid: Option<u32>, change: impl FnOnce(&mut State)) {
        if let Some(mut guard) = watched(pid)
            && let Some(state) = guard.as_mut()
        {
            change(state);
            CHANGED.notify_all();
        }
    }

    pub(super) fn watchdog_parked(pid: Option<u32>) {
        let Some(mut guard) = watched(pid) else {
            return;
        };
        if let Some(state) = guard.as_mut() {
            state.watchdog_parked = true;
        }
        CHANGED.notify_all();
        while guard.as_ref().is_some_and(|state| state.hold_watchdog) {
            guard = CHANGED
                .wait(guard)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    pub(super) fn watchdog_sent(pid: Option<u32>) {
        update(pid, |state| state.watchdog_sent = true);
    }

    pub(super) fn reaped(pid: Option<u32>) {
        update(pid, |state| state.reaped = true);
    }

    pub(super) fn owner_waiting(pid: Option<u32>) {
        update(pid, |state| state.owner_waiting = true);
    }

    /// About to signal `pid`: note whether the kernel still holds it as an
    /// unreaped child of this process.
    #[cfg(target_os = "linux")]
    pub(super) fn sending(pid: u32) {
        use nix::sys::wait::{Id, WaitPidFlag, waitid};
        let Ok(raw) = i32::try_from(pid) else {
            return;
        };
        let gone = matches!(
            waitid(
                Id::Pid(nix::unistd::Pid::from_raw(raw)),
                WaitPidFlag::WEXITED | WaitPidFlag::WNOHANG | WaitPidFlag::WNOWAIT,
            ),
            Err(nix::errno::Errno::ECHILD)
        );
        update(Some(pid), |state| state.sent_after_reap |= gone);
    }

    /// Start watching the next process a test names, holding its watchdog
    /// before its signal until [`release_watchdog`].
    pub(super) fn watch() {
        *lock() = Some(State {
            watching: true,
            hold_watchdog: true,
            ..State::default()
        });
    }

    pub(super) fn name(pid: u32) {
        if let Some(state) = lock().as_mut() {
            state.pid = Some(pid);
        }
        CHANGED.notify_all();
    }

    pub(super) fn release_watchdog() {
        if let Some(state) = lock().as_mut() {
            state.hold_watchdog = false;
        }
        CHANGED.notify_all();
    }

    pub(super) fn stop() {
        *lock() = None;
        CHANGED.notify_all();
    }

    /// Block until `ready` holds for the watched state.
    pub(super) fn wait_until(ready: impl Fn(&State) -> bool) {
        let mut guard = lock();
        while !guard.as_ref().is_some_and(&ready) {
            guard = CHANGED
                .wait(guard)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    /// Read the watched state.
    pub(super) fn read<T>(view: impl Fn(&State) -> T) -> Option<T> {
        lock().as_ref().map(view)
    }
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

    /// DRIVERS-048: an invalid byte before a character the chunk boundary
    /// cut must not take the cut character's first bytes down with it.
    #[test]
    fn an_invalid_byte_does_not_discard_a_cut_character_after_it() {
        let mut stream = Utf8Stream::default();
        let mut text = stream.push(&[0xff, 0xe2]);
        text.push_str(&stream.push(&[0x82, 0xac]));
        text.push_str(&stream.finish());
        assert_eq!(text, "\u{fffd}\u{20ac}");
        assert_eq!(text, String::from_utf8_lossy(&[0xff, 0xe2, 0x82, 0xac]));
    }

    /// DRIVERS-048, the end-of-output half: `wait_until` offers the
    /// replacement for an incomplete last character once the process has
    /// closed its output, as the output callback does.
    #[cfg(unix)]
    #[tokio::test]
    async fn wait_until_offers_the_unfinished_last_character_at_the_end() {
        let mut process = crate::process::Process::shell("printf 'a\\342'")
            .start()
            .unwrap();
        let mut seen = String::new();
        let matched = process
            .wait_until(|_, text| {
                seen.push_str(text);
                seen.contains('\u{fffd}')
            })
            .await
            .unwrap();
        assert!(matched, "the replacement reached wait_until: {seen:?}");
        assert_eq!(seen, "a\u{fffd}");
    }

    /// MEM-003: a process that is waited on to the end moves its settled
    /// output into the result rather than copying it.
    #[cfg(unix)]
    #[tokio::test]
    async fn mem_audit_waiting_moves_the_captured_output() {
        let process = crate::process::Process::shell("printf hello")
            .start()
            .unwrap();
        let captured = match &process.inner {
            super::Inner::Real(real) => std::sync::Arc::clone(&real.captured),
            _ => unreachable!("a started process is real"),
        };
        let result = process.wait().await.unwrap();
        assert_eq!(result.output(), "hello");
        assert!(
            captured.state.lock().unwrap().out.is_empty(),
            "the settled output was copied, not moved"
        );
    }

    /// Sol review of DRIVERS-049: the watchdog read `reaped == false`, let go
    /// of the lock, and only then sent its kill. The owner could reap the
    /// program in between, and the kill then went to an id, and a group id,
    /// that were free for another process to take.
    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn the_owner_cannot_reap_between_the_watchdogs_check_and_its_kill() {
        use super::reap_race;
        reap_race::watch();
        let mut process = crate::process::Process::command(["sleep", "30"])
            .timeout(std::time::Duration::from_millis(100))
            .start()
            .unwrap();
        reap_race::name(process.id().expect("a real process has an id"));
        tokio::task::spawn_blocking(|| reap_race::wait_until(|state| state.watchdog_parked))
            .await
            .unwrap();

        // The owner finds the timeout and ends the process: it kills, then
        // reaps, unless reaping has to wait for the watchdog's kill.
        let owner = tokio::spawn(async move {
            let result = process.ensure_not_timed_out().await;
            (process, result)
        });
        tokio::task::spawn_blocking(|| {
            reap_race::wait_until(|state| state.reaped || state.owner_waiting)
        })
        .await
        .unwrap();
        reap_race::release_watchdog();

        let (process, result) = owner.await.unwrap();
        tokio::task::spawn_blocking(|| reap_race::wait_until(|state| state.watchdog_sent))
            .await
            .unwrap();
        let sent_after_reap = reap_race::read(|state| state.sent_after_reap);
        reap_race::stop();
        assert!(
            matches!(result, Err(crate::process::ProcessError::TimedOut { .. })),
            "{result:?}"
        );
        assert_eq!(
            sent_after_reap,
            Some(false),
            "a kill went to an id the owner had already reaped"
        );
        drop(process);
    }
}
