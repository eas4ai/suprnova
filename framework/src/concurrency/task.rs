//! Concurrent tasks: the registry `#[concurrent_task]` fills, the [`Task`]
//! a caller hands to the facade, and the [`TaskResult`] it answers.
//!
//! A Rust closure cannot cross a process boundary, so a task is a named,
//! registered function with serializable input and output. The name is
//! what the process driver sends to the console binary, which finds the
//! same function in its own registry. The tokio and sync drivers look the
//! name up in this process and run the function here.
//!
//! Every driver passes the input and the output through JSON, the
//! in-process ones included. A task that works under `sync` in a test then
//! works under `process` in production: a value that does not survive the
//! round trip fails on every driver alike.

use crate::error::FrameworkError;
use futures::FutureExt;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::borrow::Cow;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;

/// The future a concurrent task returns, boxed so the registry can hold one
/// plain function pointer per task.
pub type TaskFuture<T> = Pin<Box<dyn Future<Output = Result<T, FrameworkError>> + Send + 'static>>;

/// A function marked `#[concurrent_task]`, as a type.
///
/// The macro implements this on a unit struct named after the function, so
/// a caller names the task by type in [`Task::new`] and the compiler checks
/// the input it is given. Implement it by hand only for a task the macro
/// cannot express; a hand-written implementation also needs a
/// [`ConcurrentTaskEntry`] submitted to the registry, or the console binary
/// cannot find it.
pub trait ConcurrentTask: 'static {
    /// What the task takes. It crosses a process boundary on the process
    /// driver, so it is a serde type.
    type Input: Serialize + DeserializeOwned + Send + 'static;
    /// What the task answers, read back with [`TaskResult::get`].
    type Output: Serialize + DeserializeOwned + Send + 'static;
    /// The name the task is registered under: the function's module path and
    /// name, unless `#[concurrent_task(name = "...")]` gave another. The
    /// process driver sends it to the console binary, so both binaries must
    /// register the task under the same name. It cannot be empty or carry a
    /// control character, for the reason [`Task::named`] gives; the macro
    /// refuses such a `name` at compile time.
    const NAME: &'static str;

    /// Run the task's function with `input`.
    fn call(input: Self::Input) -> TaskFuture<Self::Output>;
}

/// One registered task, as `#[concurrent_task]` submits it to the registry.
///
/// The registry is what lets a console process run a task it is sent by
/// name: a function pointer cannot be sent, a name can.
pub struct ConcurrentTaskEntry {
    name: &'static str,
    run: fn(Value) -> TaskFuture<Value>,
}

impl ConcurrentTaskEntry {
    /// The entry for task `T`. A `const fn`, so it can stand in an
    /// `inventory::submit!` block.
    pub const fn of<T: ConcurrentTask>() -> Self {
        Self {
            name: T::NAME,
            run: run_erased::<T>,
        }
    }

    /// The name the task is registered under.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Run the task with its input as JSON and answer its output as JSON.
    pub(crate) fn run(&self, input: Value) -> TaskFuture<Value> {
        (self.run)(input)
    }
}

impl std::fmt::Debug for ConcurrentTaskEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConcurrentTaskEntry")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

inventory::collect!(ConcurrentTaskEntry);

/// Decode the input, call the task, encode the output.
fn run_erased<T: ConcurrentTask>(input: Value) -> TaskFuture<Value> {
    Box::pin(async move {
        let input: T::Input = serde_json::from_value(input).map_err(|error| {
            FrameworkError::internal(format!(
                "concurrent task `{}`: its input does not decode as {}: {error}",
                T::NAME,
                std::any::type_name::<T::Input>()
            ))
        })?;
        let output = T::call(input).await?;
        serde_json::to_value(output).map_err(|error| {
            FrameworkError::internal(format!(
                "concurrent task `{}`: its output could not be serialized: {error}",
                T::NAME
            ))
        })
    })
}

/// The registered task named `name`.
///
/// # Errors
///
/// An error naming the task when no task is registered under the name in
/// this binary, and when two are, since either could be the one meant.
pub fn find(name: &str) -> Result<&'static ConcurrentTaskEntry, FrameworkError> {
    let mut found = inventory::iter::<ConcurrentTaskEntry>
        .into_iter()
        .filter(|entry| entry.name == name);
    match (found.next(), found.next()) {
        (Some(entry), None) => Ok(entry),
        (None, _) => Err(FrameworkError::internal(format!(
            "no concurrent task is registered as `{name}` in this binary"
        ))),
        (Some(_), Some(_)) => Err(FrameworkError::internal(format!(
            "two concurrent tasks are registered as `{name}`; give one of them another \
             name with #[concurrent_task(name = \"...\")]"
        ))),
    }
}

/// The names of every task registered in this binary, sorted.
pub fn registered() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = inventory::iter::<ConcurrentTaskEntry>
        .into_iter()
        .map(ConcurrentTaskEntry::name)
        .collect();
    names.sort_unstable();
    names
}

/// Check a task name: `call` is what the error names, the call that built
/// the task or the console command that received the name.
///
/// A name cannot be empty, since it then names no task, or carry a control
/// character. The process driver sends the name to the console process as
/// an argument, and that process's errors and logging repeat it on the
/// standard output the driver reads the outcome from. A line break in the
/// name would start a line of its own there, which could read as an
/// outcome. The error shows the name escaped, so it adds no line either.
pub(crate) fn check_name(call: &str, name: &str) -> Result<(), FrameworkError> {
    if name.is_empty() {
        return Err(FrameworkError::internal(format!(
            "{call}: the concurrent task name {name:?} is empty; a task is run by its \
             registered name"
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(FrameworkError::internal(format!(
            "{call}: the concurrent task name {name:?} contains a control character; a task \
             name carries no line break, carriage return, tab, NUL or other control character"
        )));
    }
    Ok(())
}

/// The input as JSON, or an error naming the task when it cannot be
/// serialized.
fn serialize_input(name: &str, input: impl Serialize) -> Result<Value, FrameworkError> {
    serde_json::to_value(input).map_err(|error| {
        FrameworkError::internal(format!(
            "concurrent task `{name}`: its input could not be serialized: {error}"
        ))
    })
}

/// One task to run: the registered name and the input, already serialized.
///
/// The input is serialized when the task is built, so every driver sends
/// the same JSON. A value that cannot be serialized is kept as an error and
/// fails the task when it runs, and so is a name that is refused, so
/// building a task with [`new`](Self::new) or [`named`](Self::named) never
/// fails. [`try_named`](Self::try_named) answers both errors at once.
#[derive(Debug, Clone)]
pub struct Task {
    name: Cow<'static, str>,
    input: Result<Value, FrameworkError>,
}

impl Task {
    /// The task `T` with `input`, which the compiler checks against the
    /// task's input type.
    ///
    /// ```rust,ignore
    /// use suprnova::concurrency::{Concurrency, Task};
    ///
    /// let results = Concurrency::run([
    ///     Task::new::<CountUsers>(()),
    ///     Task::new::<CountOrders>(()),
    /// ])
    /// .await?;
    /// ```
    ///
    /// The name is the task's [`ConcurrentTask::NAME`]. The macro refuses a
    /// `name` with a control character at compile time; a name a
    /// hand-written implementation gives is checked here, as
    /// [`named`](Self::named) checks its name.
    pub fn new<T: ConcurrentTask>(input: T::Input) -> Self {
        Self::build("Task::new", Cow::Borrowed(T::NAME), input)
    }

    /// The task registered as `name`, with `input`.
    ///
    /// For a task this binary does not define, such as one only the console
    /// binary links. Nothing checks the input against the task's type until
    /// the task runs.
    ///
    /// The name cannot be empty or carry a control character: a line break,
    /// a carriage return, a tab, NUL or any other. The process driver sends
    /// the name to the console process as an argument, and that process's
    /// errors and logging repeat it on the standard output the driver reads
    /// the outcome from. A line break in the name would start a line of its
    /// own there, which could read as an outcome. This call cannot fail, so
    /// it keeps a refused name as an error that fails the task when it runs,
    /// on every driver, before any console process starts. Use
    /// [`try_named`](Self::try_named) to have the error at once.
    pub fn named(name: impl Into<String>, input: impl Serialize) -> Self {
        Self::build("Task::named", Cow::Owned(name.into()), input)
    }

    /// The task registered as `name`, with `input`, or the error
    /// [`named`](Self::named) would keep until the task runs.
    ///
    /// # Errors
    ///
    /// An error naming this call and the name, escaped, when the name is
    /// empty or carries a control character. An error naming the task when
    /// its input cannot be serialized.
    pub fn try_named(
        name: impl Into<String>,
        input: impl Serialize,
    ) -> Result<Self, FrameworkError> {
        let name = name.into();
        check_name("Task::try_named", &name)?;
        let input = serialize_input(&name, input)?;
        Ok(Self {
            name: Cow::Owned(name),
            input: Ok(input),
        })
    }

    /// The task with its name checked and its input serialized, either
    /// kept as the error the task answers when it runs.
    fn build(call: &str, name: Cow<'static, str>, input: impl Serialize) -> Self {
        let input = check_name(call, &name).and_then(|()| serialize_input(&name, input));
        Self { name, input }
    }

    /// The registered name of the task.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The input as JSON, for a driver that sends it somewhere. A driver
    /// that answers this error as the task's outcome never starts a task
    /// whose name was refused.
    ///
    /// # Errors
    ///
    /// An error naming the call that built the task when its name was
    /// refused. An error naming the task when its input could not be
    /// serialized.
    pub fn input(&self) -> Result<&Value, FrameworkError> {
        self.input.as_ref().map_err(Clone::clone)
    }

    /// Run the task in this process: find it in the registry, decode its
    /// input, await it and encode its output. The tokio and sync drivers
    /// run tasks with it, and a driver of your own can too.
    ///
    /// A panic in the task is caught and answered as an error naming it.
    ///
    /// # Errors
    ///
    /// The task's own error, unchanged, when it fails. An error naming the
    /// call that built the task when its name was refused. An error naming
    /// the task when it is not registered here, when its input could not be
    /// serialized or does not decode, when its output could not be
    /// serialized, and when it panics.
    pub async fn run_in_process(self) -> Result<TaskResult, FrameworkError> {
        let input = self.input()?.clone();
        let entry = find(&self.name)?;
        let output = AssertUnwindSafe(entry.run(input)).catch_unwind().await;
        match output {
            Ok(Ok(value)) => Ok(TaskResult::new(self.name, value)),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(panicked(&self.name)),
        }
    }
}

/// The error for a task that panicked, naming it.
pub(crate) fn panicked(name: &str) -> FrameworkError {
    FrameworkError::internal(format!("concurrent task `{name}` panicked"))
}

/// What one task answered: its output as JSON, decoded on demand to the
/// task's output type.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskResult {
    name: String,
    value: Value,
}

impl TaskResult {
    /// The result of the task named `name`. A driver of your own builds its
    /// results with it.
    pub fn new(name: impl Into<String>, value: Value) -> Self {
        Self {
            name: name.into(),
            value,
        }
    }

    /// The registered name of the task that answered.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The output decoded as `O`, the task's output type.
    ///
    /// # Errors
    ///
    /// An error naming the task and the type when the output does not
    /// decode as `O`.
    pub fn get<O: DeserializeOwned>(&self) -> Result<O, FrameworkError> {
        O::deserialize(&self.value).map_err(|error| {
            FrameworkError::internal(format!(
                "the result of concurrent task `{}` does not decode as {}: {error}",
                self.name,
                std::any::type_name::<O>()
            ))
        })
    }

    /// The output as JSON.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// The output as JSON, taken out of the result.
    pub fn into_value(self) -> Value {
        self.value
    }
}
