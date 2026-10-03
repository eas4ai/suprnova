//! `Process::fake`: no process runs while the guard lives, and each one is
//! recorded for the assertions.

use super::invoked::OutputCallback;
use super::{OutputKind, PendingProcess, ProcessError, ProcessResult, Signal};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

/// Serializes the tests that take a fake: the fake is process-global.
static FAKE_LOCK: Mutex<()> = Mutex::new(());

/// The installed fake, while a guard lives.
static FAKE: RwLock<Option<Arc<State>>> = RwLock::new(None);

#[derive(Default)]
struct State {
    handlers: Mutex<Vec<(String, FakeHandler)>>,
    prevent_stray: Mutex<bool>,
    recorded: Mutex<Vec<RecordedProcess>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The guard [`Process::fake`](super::Process::fake) returns. While it
/// lives no process runs: a command matching a pattern given to
/// [`when`](Self::when) gets that result, and any other an empty successful
/// one, unless [`prevent_stray_processes`](Self::prevent_stray_processes)
/// makes it an error. Dropping it lets processes run again.
///
/// ```rust,no_run
/// # async fn ex() {
/// use suprnova::Process;
///
/// let fake = Process::fake();
/// fake.when("git *", Process::result("main\n"));
///
/// let branch = Process::command(["git", "branch", "--show-current"]).run().await.unwrap();
/// assert_eq!(branch.output(), "main\n");
/// fake.assert_ran("git branch --show-current");
/// # }
/// ```
pub struct ProcessFake {
    state: Arc<State>,
    _lock: MutexGuard<'static, ()>,
}

pub(crate) fn install() -> ProcessFake {
    let guard = FAKE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let state = Arc::new(State::default());
    *FAKE
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::clone(&state));
    ProcessFake {
        state,
        _lock: guard,
    }
}

impl Drop for ProcessFake {
    fn drop(&mut self) {
        *FAKE
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }
}

impl ProcessFake {
    /// Answer a command whose command line matches `pattern` with
    /// `handler`. `*` matches any run of characters, and `"*"` alone
    /// matches every command; the first matching pattern, in the order
    /// given, wins. The command line is the arguments joined by spaces, or
    /// the shell line as given.
    pub fn when(&self, pattern: &str, handler: impl Into<FakeHandler>) -> &Self {
        lock(&self.state.handlers).push((pattern.to_owned(), handler.into()));
        self
    }

    /// Make a command no pattern matches an error,
    /// [`ProcessError::Stray`], instead of an empty successful result.
    pub fn prevent_stray_processes(&self) -> &Self {
        *lock(&self.state.prevent_stray) = true;
        self
    }

    /// Every faked process, in the order it ran.
    pub fn recorded(&self) -> Vec<RecordedProcess> {
        lock(&self.state.recorded).clone()
    }

    /// Assert that a process with exactly this command line ran.
    ///
    /// # Panics
    ///
    /// When none did.
    pub fn assert_ran(&self, command: &str) -> &Self {
        assert!(
            self.recorded()
                .iter()
                .any(|process| process.command == command),
            "the process \"{command}\" was not run; ran: {:?}",
            self.command_lines()
        );
        self
    }

    /// Assert that a process for which `check` holds ran.
    ///
    /// # Panics
    ///
    /// When none did.
    pub fn assert_ran_with(&self, check: impl Fn(&RecordedProcess) -> bool) -> &Self {
        assert!(
            self.recorded().iter().any(check),
            "no process that ran matches; ran: {:?}",
            self.command_lines()
        );
        self
    }

    /// Assert that a process with exactly this command line ran `times`
    /// times.
    ///
    /// # Panics
    ///
    /// When it ran another number of times.
    pub fn assert_ran_times(&self, command: &str, times: usize) -> &Self {
        let ran = self
            .recorded()
            .iter()
            .filter(|process| process.command == command)
            .count();
        assert_eq!(
            ran, times,
            "the process \"{command}\" ran {ran} times instead of {times}"
        );
        self
    }

    /// Assert that exactly these command lines ran, in this order.
    ///
    /// # Panics
    ///
    /// When another set ran, or another order.
    pub fn assert_ran_in_order(&self, commands: &[&str]) -> &Self {
        let ran = self.command_lines();
        assert_eq!(
            ran, commands,
            "the processes did not run in the expected order"
        );
        self
    }

    /// Assert that no process with this command line ran.
    ///
    /// # Panics
    ///
    /// When one did.
    pub fn assert_not_ran(&self, command: &str) -> &Self {
        assert!(
            !self
                .recorded()
                .iter()
                .any(|process| process.command == command),
            "the process \"{command}\" was run"
        );
        self
    }

    /// [`assert_not_ran`](Self::assert_not_ran), by Laravel's other name.
    ///
    /// # Panics
    ///
    /// When the process ran.
    pub fn assert_didnt_run(&self, command: &str) -> &Self {
        self.assert_not_ran(command)
    }

    /// Assert that no process ran.
    ///
    /// # Panics
    ///
    /// When one did.
    pub fn assert_nothing_ran(&self) -> &Self {
        let ran = self.command_lines();
        assert!(ran.is_empty(), "processes were run: {ran:?}");
        self
    }

    fn command_lines(&self) -> Vec<String> {
        self.recorded()
            .into_iter()
            .map(|process| process.command)
            .collect()
    }
}

/// A process run under a fake, as the assertions see it.
#[derive(Debug, Clone)]
pub struct RecordedProcess {
    /// The command line.
    pub command: String,
    /// The working directory, when one was set.
    pub path: Option<PathBuf>,
    /// The variables added to the environment.
    pub env: Vec<(String, String)>,
    /// The bytes written to standard input.
    pub input: Vec<u8>,
    /// The faked result; for a started process, the one it ends with.
    pub result: ProcessResult,
}

/// What a faked command gets: a [`FakeResult`], a [`FakeDescription`] or a
/// [`FakeSequence`].
#[derive(Clone)]
pub enum FakeHandler {
    /// A fixed result.
    Result(FakeResult),
    /// A described process.
    Describe(FakeDescription),
    /// Results answered in turn.
    Sequence(FakeSequence),
}

impl From<FakeResult> for FakeHandler {
    fn from(result: FakeResult) -> Self {
        FakeHandler::Result(result)
    }
}

impl From<FakeDescription> for FakeHandler {
    fn from(description: FakeDescription) -> Self {
        FakeHandler::Describe(description)
    }
}

impl From<FakeSequence> for FakeHandler {
    fn from(sequence: FakeSequence) -> Self {
        FakeHandler::Sequence(sequence)
    }
}

/// A faked result, from [`Process::result`](super::Process::result).
#[derive(Debug, Clone, Default)]
pub struct FakeResult {
    output: String,
    error_output: String,
    exit_code: i32,
}

impl FakeResult {
    pub(crate) fn new(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            ..Self::default()
        }
    }

    /// The standard error it reports.
    pub fn error_output(mut self, error_output: impl Into<String>) -> Self {
        self.error_output = error_output.into();
        self
    }

    /// The exit code it reports.
    pub fn exit_code(mut self, exit_code: i32) -> Self {
        self.exit_code = exit_code;
        self
    }
}

/// A faked process described line by line, from
/// [`Process::describe`](super::Process::describe). Each line ends with a
/// newline in the output. A started process reports itself running for
/// [`runs_for`](Self::runs_for) calls of `running`, showing a line of output
/// at each.
#[derive(Debug, Clone, Default)]
pub struct FakeDescription {
    id: Option<u32>,
    output: Vec<String>,
    error_output: Vec<String>,
    exit_code: i32,
    iterations: u32,
}

impl FakeDescription {
    /// The process id the started process reports.
    pub fn id(mut self, id: u32) -> Self {
        self.id = Some(id);
        self
    }

    /// Replace the standard output with the lines of `output`.
    pub fn replace_output(mut self, output: &str) -> Self {
        self.output = output.lines().map(str::to_owned).collect();
        self
    }

    /// Replace the standard error with the lines of `output`.
    pub fn replace_error_output(mut self, output: &str) -> Self {
        self.error_output = output.lines().map(str::to_owned).collect();
        self
    }

    /// Add a line of standard output.
    pub fn output(mut self, line: impl Into<String>) -> Self {
        self.output.push(line.into());
        self
    }

    /// Add a line of standard error.
    pub fn error_output(mut self, line: impl Into<String>) -> Self {
        self.error_output.push(line.into());
        self
    }

    /// The exit code it ends with.
    pub fn exit_code(mut self, exit_code: i32) -> Self {
        self.exit_code = exit_code;
        self
    }

    /// Report the started process running for this many `running` calls.
    pub fn runs_for(mut self, iterations: u32) -> Self {
        self.iterations = iterations;
        self
    }
}

/// Faked results answered in turn, from
/// [`Process::sequence`](super::Process::sequence). Once they run out, a
/// run is an error, [`ProcessError::FakeExhausted`], unless
/// [`dont_fail_when_empty`](Self::dont_fail_when_empty) makes it an empty
/// successful result.
#[derive(Clone)]
pub struct FakeSequence {
    items: Arc<Mutex<VecDeque<FakeHandler>>>,
    fail_when_empty: bool,
    when_empty: Option<Box<FakeHandler>>,
}

impl FakeSequence {
    pub(crate) fn new<I, H>(results: I) -> Self
    where
        I: IntoIterator<Item = H>,
        H: Into<FakeHandler>,
    {
        Self {
            items: Arc::new(Mutex::new(results.into_iter().map(Into::into).collect())),
            fail_when_empty: true,
            when_empty: None,
        }
    }

    /// Add a result after the others.
    pub fn push(self, result: impl Into<FakeHandler>) -> Self {
        lock(&self.items).push_back(result.into());
        self
    }

    /// Answer `result` once the sequence runs out, instead of an error.
    pub fn when_empty(mut self, result: impl Into<FakeHandler>) -> Self {
        self.fail_when_empty = false;
        self.when_empty = Some(Box::new(result.into()));
        self
    }

    /// Whether every result has been answered.
    pub fn is_empty(&self) -> bool {
        lock(&self.items).is_empty()
    }

    /// Answer an empty successful result once the sequence runs out.
    pub fn dont_fail_when_empty(mut self) -> Self {
        self.fail_when_empty = false;
        self
    }
}

/// What a faked process does, whatever handler gave it.
#[derive(Debug, Clone, Default)]
pub(crate) struct Canned {
    id: Option<u32>,
    output: String,
    error_output: String,
    output_lines: Vec<String>,
    exit_code: i32,
    iterations: u32,
}

impl Canned {
    fn result(&self, command: &str) -> ProcessResult {
        ProcessResult::new(
            command.to_owned(),
            Some(self.exit_code),
            self.output.clone().into_bytes(),
            self.error_output.clone().into_bytes(),
        )
    }
}

fn lines(lines: &[String]) -> String {
    lines.iter().map(|line| format!("{line}\n")).collect()
}

impl FakeHandler {
    /// What this handler answers for one run.
    fn answer(&self, command: &str) -> Result<Canned, ProcessError> {
        match self {
            FakeHandler::Result(result) => Ok(Canned {
                id: None,
                output: result.output.clone(),
                error_output: result.error_output.clone(),
                output_lines: vec![result.output.clone()],
                exit_code: result.exit_code,
                iterations: 0,
            }),
            FakeHandler::Describe(description) => Ok(Canned {
                id: description.id,
                output: lines(&description.output),
                error_output: lines(&description.error_output),
                output_lines: description
                    .output
                    .iter()
                    .map(|line| format!("{line}\n"))
                    .collect(),
                exit_code: description.exit_code,
                iterations: description.iterations,
            }),
            FakeHandler::Sequence(sequence) => {
                let next = lock(&sequence.items).pop_front();
                match next {
                    Some(handler) => handler.answer(command),
                    None if sequence.fail_when_empty => Err(ProcessError::FakeExhausted {
                        command: command.to_owned(),
                    }),
                    None => match &sequence.when_empty {
                        Some(handler) => handler.answer(command),
                        None => Ok(Canned::default()),
                    },
                }
            }
        }
    }
}

/// Whether `pattern`, with `*` for any run of characters, matches all of
/// `text`.
fn matches(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !text.starts_with(first) || text.len() < first.len() + last.len() || !text.ends_with(last) {
        return false;
    }
    let mut rest = &text[first.len()..text.len() - last.len()];
    for part in &parts[1..parts.len() - 1] {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    true
}

/// Under an installed fake, what `pending` gets instead of running; `None`
/// when no fake is installed. The process is recorded.
pub(crate) fn resolve(pending: &PendingProcess) -> Result<Option<Canned>, ProcessError> {
    let Some(state) = FAKE
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
    else {
        return Ok(None);
    };
    let command = pending.command.line();
    let handler = lock(&state.handlers)
        .iter()
        .find(|(pattern, _)| pattern == "*" || matches(pattern, &command))
        .map(|(_, handler)| handler.clone());
    let canned = match handler {
        Some(handler) => handler.answer(&command)?,
        None if *lock(&state.prevent_stray) => {
            return Err(ProcessError::Stray { command });
        }
        None => Canned::default(),
    };
    lock(&state.recorded).push(RecordedProcess {
        command: command.clone(),
        path: pending.path.clone(),
        env: pending.env.clone(),
        input: pending.input.clone().unwrap_or_default(),
        result: canned.result(&command),
    });
    Ok(Some(canned))
}

/// A started process under a fake.
pub(crate) struct FakeInvoked {
    canned: Canned,
    remaining: u32,
    stopped: bool,
    /// How many lines of output `running` has shown.
    shown: usize,
    out_read: AtomicUsize,
    err_read: AtomicUsize,
    callback: Mutex<Option<OutputCallback>>,
    signals: Mutex<Vec<Signal>>,
    /// Under `quietly` the faked output is kept back, as a real run's is.
    quiet: bool,
}

impl FakeInvoked {
    pub(crate) fn new(canned: Canned, callback: Option<OutputCallback>, quiet: bool) -> Self {
        Self {
            remaining: canned.iterations,
            canned,
            stopped: false,
            shown: 0,
            out_read: AtomicUsize::new(0),
            err_read: AtomicUsize::new(0),
            callback: Mutex::new(callback),
            signals: Mutex::new(Vec::new()),
            quiet,
        }
    }

    pub(crate) fn id(&self) -> u32 {
        self.canned.id.unwrap_or(0)
    }

    pub(crate) fn signal(&self, signal: Signal) {
        lock(&self.signals).push(signal);
    }

    pub(crate) fn has_received_signal(&self, signal: Signal) -> bool {
        lock(&self.signals).contains(&signal)
    }

    /// Show the next line of output, if any is left.
    fn show_next(&mut self) -> bool {
        let Some(line) = self.canned.output_lines.get(self.shown).cloned() else {
            return false;
        };
        self.shown += 1;
        if let Some(callback) = lock(&self.callback).as_mut() {
            callback(OutputKind::Out, &line);
        }
        true
    }

    fn show_all(&mut self) {
        while self.show_next() {}
        if let Some(callback) = lock(&self.callback).as_mut()
            && !self.canned.error_output.is_empty()
        {
            callback(OutputKind::Err, &self.canned.error_output);
        }
    }

    pub(crate) fn running(&mut self) -> bool {
        if self.stopped {
            return false;
        }
        if self.remaining == 0 {
            self.show_all();
            return false;
        }
        self.show_next();
        self.remaining -= 1;
        true
    }

    pub(crate) fn read(&self, kind: OutputKind, latest: bool) -> String {
        if self.quiet {
            return String::new();
        }
        let text = match kind {
            OutputKind::Out => self.canned.output.clone(),
            OutputKind::Err => self.canned.error_output.clone(),
        };
        if !latest {
            return text;
        }
        let read = match kind {
            OutputKind::Out => &self.out_read,
            OutputKind::Err => &self.err_read,
        };
        let from = read.swap(text.len(), Ordering::Relaxed).min(text.len());
        text[from..].to_owned()
    }

    pub(crate) fn stop(&mut self) {
        self.stopped = true;
    }

    pub(crate) fn wait_until(&mut self, until: &mut dyn FnMut(OutputKind, &str) -> bool) -> bool {
        let lines = self.canned.output_lines.clone();
        lines.iter().any(|line| until(OutputKind::Out, line))
            || (!self.canned.error_output.is_empty()
                && until(OutputKind::Err, &self.canned.error_output))
    }

    pub(crate) fn result(&mut self, command: &str) -> ProcessResult {
        if !self.stopped {
            self.show_all();
        }
        let result = self.canned.result(command);
        if self.quiet {
            ProcessResult::new(result.command, result.exit_code, Vec::new(), Vec::new())
        } else {
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn a_star_matches_any_run_of_characters() {
        assert!(matches("git *", "git status --short"));
        assert!(matches("*status*", "git status --short"));
        assert!(matches("git status", "git status"));
        assert!(!matches("git status", "git status --short"));
        assert!(!matches("npm *", "git status"));
        assert!(matches("a*b*c", "a-x-b-y-c"));
        assert!(!matches("a*b*c", "a-x-c-y-b"));
        assert!(matches("*", ""));
    }
}
