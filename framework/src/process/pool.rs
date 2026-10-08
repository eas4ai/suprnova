//! Pools of processes run side by side, and pipes run in order.

use super::{InvokedProcess, OutputKind, PendingProcess, ProcessError, ProcessResult, Signal};
use futures_util::stream::{FuturesUnordered, StreamExt};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

/// A callback for the output of a pool or a pipe: the key of the process,
/// the stream, and the chunk.
type KeyedCallback = Arc<dyn Fn(&str, OutputKind, &str) + Send + Sync>;

/// Processes run side by side, from [`Process::pool`](super::Process::pool).
///
/// Each process is added under a key, or under its position when it is
/// pushed without one, and its result comes back under that key. A failing
/// process does not stop the others. With no [`concurrency`](Self::concurrency)
/// set, every process starts at once, as in Laravel.
///
/// ```rust,no_run
/// # async fn ex() {
/// use suprnova::Process;
///
/// let results = Process::pool()
///     .add("assets", Process::command(["npm", "run", "build"]))
///     .add("types", Process::command(["suprnova", "generate-types"]))
///     .concurrency(2)
///     .run()
///     .await;
/// assert!(results.successful());
/// # }
/// ```
#[derive(Default)]
pub struct Pool {
    processes: Vec<(String, PendingProcess)>,
    concurrency: Option<usize>,
}

impl Pool {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Add a process under `key`. A key already in the pool has its process
    /// replaced, as a repeated key does in Laravel.
    pub fn add(mut self, key: impl Into<String>, process: PendingProcess) -> Self {
        insert_keyed(&mut self.processes, key.into(), process);
        self
    }

    /// Add a process under its position in the pool, counted from 0, or
    /// the next free number when a key already holds that one.
    pub fn push(mut self, process: PendingProcess) -> Self {
        let key = next_number(&self.processes);
        self.processes.push((key, process));
        self
    }

    /// Run at most `limit` processes at once; the rest wait for a slot. A
    /// limit of 0 is taken as 1.
    pub fn concurrency(mut self, limit: usize) -> Self {
        self.concurrency = Some(limit.max(1));
        self
    }

    fn limit(&self) -> usize {
        self.concurrency.unwrap_or(self.processes.len()).max(1)
    }

    /// Run every process and wait for all of them.
    pub async fn run(self) -> PoolResults {
        self.run_inner(None).await
    }

    /// [`run`](Self::run), with `output` called with the key, the stream
    /// and each chunk of output as it arrives.
    pub async fn run_with<F>(self, output: F) -> PoolResults
    where
        F: Fn(&str, OutputKind, &str) + Send + Sync + 'static,
    {
        self.run_inner(Some(Arc::new(output))).await
    }

    async fn run_inner(self, output: Option<KeyedCallback>) -> PoolResults {
        self.start_inner(output).wait().await
    }

    /// Start as many processes as the concurrency allows and return the
    /// pool running; the rest start as slots free while it is waited on.
    pub fn start(self) -> InvokedPool {
        self.start_inner(None)
    }

    fn start_inner(self, output: Option<KeyedCallback>) -> InvokedPool {
        let limit = self.limit();
        let mut pool = InvokedPool {
            pending: self
                .processes
                .into_iter()
                .enumerate()
                .map(|(index, (key, process))| (index, key, process))
                .collect(),
            running: Vec::new(),
            exited: Vec::new(),
            finished: Vec::new(),
            limit,
            output,
        };
        pool.fill(0);
        pool
    }
}

/// A pool that has started, from [`Pool::start`].
pub struct InvokedPool {
    pending: VecDeque<(usize, String, PendingProcess)>,
    running: Vec<(usize, String, InvokedProcess)>,
    /// Processes that have exited and wait to be collected; they hold no
    /// slot.
    exited: Vec<(usize, String, InvokedProcess)>,
    finished: Vec<(usize, String, Result<ProcessResult, ProcessError>)>,
    limit: usize,
    output: Option<KeyedCallback>,
}

impl InvokedPool {
    /// Start waiting processes while there are free slots; `in_flight`
    /// processes are already being waited on and hold slots too.
    fn fill(&mut self, in_flight: usize) {
        while in_flight + self.running.len() < self.limit {
            let Some((index, key, process)) = self.pending.pop_front() else {
                break;
            };
            let started = match &self.output {
                Some(output) => {
                    let output = Arc::clone(output);
                    let name = key.clone();
                    process.start_with(move |kind, chunk| output(&name, kind, chunk))
                }
                None => process.start(),
            };
            match started {
                Ok(invoked) => self.running.push((index, key, invoked)),
                Err(error) => self.finished.push((index, key, Err(error))),
            }
        }
    }

    /// Whether any process is running or still waiting for a slot. Each
    /// call starts waiting processes in the slots that exited ones freed, so
    /// a pool polled with `running` and no `wait` still works through its
    /// queue.
    pub fn running(&mut self) -> bool {
        let mut index = 0;
        while index < self.running.len() {
            if self.running[index].2.running() {
                index += 1;
            } else {
                let exited = self.running.remove(index);
                self.exited.push(exited);
            }
        }
        self.fill(0);
        !self.pending.is_empty() || !self.running.is_empty()
    }

    /// The number of processes, running, waiting or done.
    pub fn len(&self) -> usize {
        self.pending.len() + self.running.len() + self.exited.len() + self.finished.len()
    }

    /// Whether the pool has no processes.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Send `signal` to every running process.
    ///
    /// # Errors
    ///
    /// The first process that could not be signalled, after every process
    /// was tried.
    pub fn signal(&self, signal: Signal) -> Result<(), ProcessError> {
        let mut first = None;
        for (_, _, process) in &self.running {
            if let Err(error) = process.signal(signal)
                && first.is_none()
            {
                first = Some(error);
            }
        }
        first.map_or(Ok(()), Err)
    }

    /// Wait for every process, starting the waiting ones as slots free, and
    /// return the results in the order the processes were added.
    pub async fn wait(mut self) -> PoolResults {
        let mut waits = FuturesUnordered::new();
        for (index, key, process) in self.exited.drain(..) {
            waits.push(wait_keyed(index, key, process));
        }
        // Exited processes hold no slot; fill the free ones first. They
        // resolve at the first poll, so the count of futures in flight is
        // the count of running processes from then on.
        self.fill(0);
        for (index, key, process) in self.running.drain(..) {
            waits.push(wait_keyed(index, key, process));
        }
        while let Some((index, key, result)) = waits.next().await {
            self.finished.push((index, key, result));
            self.fill(waits.len());
            for (index, key, process) in self.running.drain(..) {
                waits.push(wait_keyed(index, key, process));
            }
        }
        self.into_results()
    }

    /// Stop every running process, a terminate signal and then a kill after
    /// `grace`, and drop the ones still waiting for a slot. Returns the
    /// results of the processes that started.
    pub async fn stop(mut self, grace: Duration) -> PoolResults {
        self.pending.clear();
        let mut stopping = std::mem::take(&mut self.running);
        stopping.append(&mut self.exited);
        let stops: FuturesUnordered<_> = stopping
            .into_iter()
            .map(|(index, key, process)| async move { (index, key, process.stop(grace).await) })
            .collect();
        let stopped: Vec<_> = stops.collect().await;
        self.finished.extend(stopped);
        self.into_results()
    }

    fn into_results(mut self) -> PoolResults {
        self.finished.sort_by_key(|(index, _, _)| *index);
        PoolResults {
            entries: self
                .finished
                .into_iter()
                .map(|(_, key, result)| (key, result))
                .collect(),
        }
    }
}

async fn wait_keyed(
    index: usize,
    key: String,
    process: InvokedProcess,
) -> (usize, String, Result<ProcessResult, ProcessError>) {
    (index, key, process.wait().await)
}

/// The results of a pool, under their keys in the order the processes were
/// added. A process that could not start, or was killed for its timeout,
/// holds its error.
#[derive(Debug)]
pub struct PoolResults {
    entries: Vec<(String, Result<ProcessResult, ProcessError>)>,
}

impl PoolResults {
    /// The result under `key`.
    pub fn get(&self, key: &str) -> Option<&Result<ProcessResult, ProcessError>> {
        self.entries
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, result)| result)
    }

    /// The keys, in the order the processes were added.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(key, _)| key.as_str())
    }

    /// Each key with its result, in the order the processes were added.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Result<ProcessResult, ProcessError>)> {
        self.entries
            .iter()
            .map(|(key, result)| (key.as_str(), result))
    }

    /// The number of results.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether every process ran and exited with code 0.
    pub fn successful(&self) -> bool {
        self.entries
            .iter()
            .all(|(_, result)| matches!(result, Ok(result) if result.successful()))
    }

    /// The keys of the processes that failed, errored or did not start.
    pub fn failed(&self) -> impl Iterator<Item = &str> {
        self.entries
            .iter()
            .filter(|(_, result)| !matches!(result, Ok(result) if result.successful()))
            .map(|(key, _)| key.as_str())
    }
}

/// Processes run in order, from [`Process::pipe`](super::Process::pipe):
/// each gets the previous one's output as its input, and the last result
/// comes back. The first process that fails ends the pipe, and its result
/// comes back instead; the processes after it do not run. This is Laravel's
/// pipe: each process runs to its end before the next starts.
///
/// ```rust,no_run
/// # async fn ex() -> Result<(), suprnova::ProcessError> {
/// use suprnova::Process;
///
/// let sorted = Process::pipe()
///     .push(Process::command(["cat", "words.txt"]))
///     .push(Process::command(["sort"]))
///     .run()
///     .await?;
/// # Ok(()) }
/// ```
#[derive(Default)]
pub struct Pipe {
    processes: Vec<(String, PendingProcess)>,
}

impl Pipe {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Add a process under `key`, for the output callback. A key already in
    /// the pipe has its process replaced.
    pub fn add(mut self, key: impl Into<String>, process: PendingProcess) -> Self {
        insert_keyed(&mut self.processes, key.into(), process);
        self
    }

    /// Add a process under its position, or the next free number, as
    /// [`Pool::push`] does.
    pub fn push(mut self, process: PendingProcess) -> Self {
        let key = next_number(&self.processes);
        self.processes.push((key, process));
        self
    }

    /// Run the processes in order.
    ///
    /// # Errors
    ///
    /// The error of a process that could not run or was killed for its
    /// timeout; the processes after it do not run. A pipe with no processes
    /// returns an empty successful result.
    pub async fn run(self) -> Result<ProcessResult, ProcessError> {
        self.run_inner(None).await
    }

    /// [`run`](Self::run), with `output` called with the key, the stream
    /// and each chunk of output as it arrives.
    ///
    /// # Errors
    ///
    /// As [`run`](Self::run).
    pub async fn run_with<F>(self, output: F) -> Result<ProcessResult, ProcessError>
    where
        F: Fn(&str, OutputKind, &str) + Send + Sync + 'static,
    {
        self.run_inner(Some(Arc::new(output))).await
    }

    async fn run_inner(self, output: Option<KeyedCallback>) -> Result<ProcessResult, ProcessError> {
        let mut previous: Option<ProcessResult> = None;
        for (key, process) in self.processes {
            let process = match &previous {
                Some(result) => process.input(result.output_bytes().to_vec()),
                None => process,
            };
            let result = match &output {
                Some(output) => {
                    let output = Arc::clone(output);
                    process
                        .run_with(move |kind, chunk| output(&key, kind, chunk))
                        .await?
                }
                None => process.run().await?,
            };
            if result.failed() {
                return Ok(result);
            }
            previous = Some(result);
        }
        Ok(previous
            .unwrap_or_else(|| ProcessResult::new(String::new(), Some(0), Vec::new(), Vec::new())))
    }
}

/// Put `process` under `key`, replacing a process already there in its
/// place.
fn insert_keyed(
    processes: &mut Vec<(String, PendingProcess)>,
    key: String,
    process: PendingProcess,
) {
    match processes.iter_mut().find(|(existing, _)| *existing == key) {
        Some(slot) => slot.1 = process,
        None => processes.push((key, process)),
    }
}

/// The position the next process takes, counted from 0, or the next number
/// after it that no key holds.
fn next_number(processes: &[(String, PendingProcess)]) -> String {
    let mut number = processes.len();
    while processes.iter().any(|(key, _)| *key == number.to_string()) {
        number += 1;
    }
    number.to_string()
}
