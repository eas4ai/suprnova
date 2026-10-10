# Concurrency

The `Concurrency` facade runs several slow tasks that don't depend on one
another at the same time, and gives you their results. It is Laravel's
`Concurrency` facade with tasks in place of closures, and Tokio tasks as
the default driver.

```rust
use suprnova::concurrency::Task;
use suprnova::{Concurrency, DB, FrameworkError, concurrent_task};

#[concurrent_task]
async fn count_rows(table: String) -> Result<u64, FrameworkError> {
    DB::table(table).count().await
}

let results = Concurrency::run([
    Task::new::<CountRows>("users".to_owned()),
    Task::new::<CountRows>("orders".to_owned()),
])
.await?;

let users: u64 = results[0].get()?;
let orders: u64 = results[1].get()?;
```

## Define a task

A task is an `async fn` marked `#[concurrent_task]`. It takes one input
and returns `Result<T, FrameworkError>`. Both the input and the output are
serde types, because the process driver sends them to another process. A
task without a parameter takes `()`:

```rust
use serde::{Deserialize, Serialize};
use suprnova::{FrameworkError, concurrent_task};

#[derive(Serialize, Deserialize)]
pub struct Export {
    pub account_id: u64,
    pub format: String,
}

#[concurrent_task]
pub async fn build_export(input: Export) -> Result<String, FrameworkError> {
    // ...
    Ok(format!("exports/{}.{}", input.account_id, input.format))
}

#[concurrent_task]
pub async fn warm_caches() -> Result<(), FrameworkError> {
    // ...
    Ok(())
}
```

The function stays as you wrote it, so you can still call it directly.
The macro adds a unit struct named after the function in UpperCamelCase,
`BuildExport` and `WarmCaches` here. You pass that struct to `Task::new`,
and the compiler checks the input against the function's parameter:

```rust
use suprnova::concurrency::Task;

let task = Task::new::<BuildExport>(Export {
    account_id: 7,
    format: "csv".to_owned(),
});
```

The macro also registers the task at compile time under a name: the
function's module path and name, such as `app::exports::build_export`. The
process driver sends that name to the console binary, which finds the same
function in its own registry. Two attributes change the defaults:

| Attribute | Effect |
|---|---|
| `name = "exports:build"` | registers the task under this name instead |
| `type_name = "ExportTask"` | names the struct this instead, when the UpperCamelCase name is taken, for example by the input type |

`Task::named(name, input)` builds a task from its registered name, for a
task that only the console binary links. Nothing checks its input until the
task runs. A name can't be empty or contain a control character, such as a
line break or a tab: `Task::named` keeps a refused name as an error that
fails the task when it runs, and `Task::try_named(name, input)` returns
that error at once.

## Run tasks

`Concurrency::run` takes the tasks, runs them at the same time, and returns
a `Vec<TaskResult>` in the order you gave the tasks, whatever order they
finish in. `TaskResult::get::<T>()` decodes a result as the task's output
type:

```rust
let results = Concurrency::run([
    Task::new::<BuildExport>(csv),
    Task::new::<WarmCaches>(()),
])
.await?;

let path: String = results[0].get()?;
```

Every task runs to its end before `run` returns, even when one of them
fails. `run` then returns the first failed task's error, in the order of
the tasks. A task that panics is an error that names it.

### Named results

`Concurrency::run_named` takes `(key, task)` pairs and returns an
`IndexMap<String, TaskResult>` with each result under its key, in the order
you gave them:

```rust
let results = Concurrency::run_named([
    ("users", Task::new::<CountRows>("users".to_owned())),
    ("orders", Task::new::<CountRows>("orders".to_owned())),
])
.await?;

let users: u64 = results["users"].get()?;
```

A key that names two tasks is an error, and no task runs.

### Task timeouts

`Concurrency::with_timeout` ends a task that runs longer than the duration
you give it. The task's error is a `FrameworkError::Timeout` that names
the task, so `error.is_timeout()` tells it from a failure of the task
itself:

```rust
use std::time::Duration;

let results = Concurrency::with_timeout(Duration::from_secs(30))
    .run([Task::new::<BuildExport>(csv)])
    .await?;
```

A zero duration means no timeout. Without a timeout, a task runs until it
finishes, on every driver.

## Drivers

A driver decides where the tasks run. Three drivers are built in:

| Driver | Runs each task | Use it for |
|---|---|---|
| `tokio` (default) | as a Tokio task in this process | most work: it's the cheapest |
| `process` | in a console process of its own | work that needs isolation: its own memory, and a timeout that kills it |
| `sync` | one after another, in the caller's task | tests that want concurrency out of the way |

`CONCURRENCY_DRIVER` names the default driver, and `tokio` is the default
when the variable is unset or empty. The variable is read on each call. To
pick a driver for one call, use `Concurrency::driver`:

```rust
let results = Concurrency::driver("process")
    .with_timeout(Duration::from_secs(60))
    .run([Task::new::<BuildExport>(csv)])
    .await?;
```

A name that no driver has is an error that names it.

On every driver, the input and output pass through JSON, the in-process
drivers included. A task that works under `sync` in a test then works
under `process` in production.

### The tokio driver

The `tokio` driver spawns each task on the runtime and waits for them all.
The tasks share the caller's container scope, so a scoped binding the
request resolved is the one the task sees. A timeout ends a task at its next
`.await`; a task that blocks its thread runs on until it yields.

### The process driver

The `process` driver starts the console binary once for each task, through
the [`Process` facade](processes.md), as Laravel starts an `artisan` process
for each closure. The binary runs the hidden `concurrency:run` command with
the task's name, reads the input as JSON from its standard input, runs the
task, and writes the outcome to its standard output. The command is
registered by the framework, so every console binary has it, and it stays
out of the console's help. The outcome line carries a random token that the
driver generates for each run, so text your application logs to standard
output can't pass for a task's result.

The driver starts the first of these that exists:

1. The command you set with `Concurrency::console_binary(path)`, or
   `Concurrency::console_command([program, args...])` for a console that
   another program starts.
2. The `console` binary in the same directory as the running executable.
   A Suprnova project builds `app` and `console` side by side, so the server
   finds its console there.
3. The running executable, which is right when the console binary itself
   runs the tasks.

```rust
use suprnova::Concurrency;

// In bootstrap::register(), when the image keeps the console elsewhere.
Concurrency::console_binary("/srv/app/bin/console")?;

// In development, through Cargo.
Concurrency::console_command(["cargo", "run", "--quiet", "--bin", "console", "--"])?;
```

A timeout kills the console process and every process it started. Both
binaries must register the task under the same name, which they do when the
task is in the application's library crate. A name that the console binary
doesn't register is an error that names the task. Starting a process costs
far more than spawning a Tokio task, so use this driver for heavy or risky
work, not for small queries.

### Your own driver

A driver implements `ConcurrencyDriver`, one method that runs the tasks
and returns one outcome for each, in order. `Task::run_in_process` runs a
task in this process, `Task::name` and `Task::input` give what a remote
worker needs, and `concurrency::timed_out` builds the timeout error.
Register the driver with `Concurrency::extend`:

```rust
use std::time::Duration;
use suprnova::concurrency::{Task, TaskResult};
use suprnova::{Concurrency, ConcurrencyDriver, FrameworkError, async_trait};

struct InlineDriver;

#[async_trait]
impl ConcurrencyDriver for InlineDriver {
    async fn run(
        &self,
        tasks: Vec<Task>,
        _timeout: Option<Duration>,
    ) -> Vec<Result<TaskResult, FrameworkError>> {
        let mut outcomes = Vec::new();
        for task in tasks {
            outcomes.push(task.run_in_process().await);
        }
        outcomes
    }
}

Concurrency::extend("inline", InlineDriver)?;
```

A driver added under `tokio`, `process`, or `sync` replaces the built-in
one.

## Defer tasks

`Concurrency::defer` runs tasks after the response has been sent, and
discards their results. Use it for work the user doesn't wait for, such
as reporting metrics:

```rust
use suprnova::concurrency::Task;
use suprnova::{Concurrency, HttpResponse, Request, Response};

async fn checkout(_request: Request) -> Response {
    // ...
    Concurrency::defer([
        Task::new::<ReportMetric>("checkout".to_owned()),
        Task::new::<WarmCaches>(()),
    ])
    .map_err(HttpResponse::from)?;

    Ok(HttpResponse::text("ordered"))
}
```

Inside a request, `defer` returns at once. The tasks start on the selected
driver after the response's last bytes have been written and flushed,
through the same mechanism as
[`Bus::dispatch_after_response`](bus.md). A task that fails is logged with
its name and error, and the response, already sent, isn't changed. `defer`
returns an error only when the driver doesn't exist.

Outside a request, in a console command or a worker, there is no response
to wait for. `defer` returns a `DeferredTasks` handle: awaiting it runs the
tasks and waits for them, and dropping it starts them on the runtime. A
process that ends right after the drop can end before the tasks finish, so
await the handle at the end of a console command:

```rust
Concurrency::defer([Task::new::<WarmCaches>(())])?.await;
```

In every request the framework serves, the in-memory test client
included, the tasks run after the response. A request with no response
writer in reach, such as one from the test client or a custom Hyper loop
that doesn't use `after_response_connection`, starts them when the
response's body ends instead of after its bytes flush. Only outside a
request does the handle run the tasks when you await it or drop it.
`DeferredTasks::after_response()` tells you which path the tasks took: it
returns `true` inside a request and `false` outside one.

## Testing

Set `CONCURRENCY_DRIVER=sync` in the test environment, or call
`Concurrency::driver("sync")`, to run tasks one after another in the test's
own task. The input and output still pass through JSON, so a task that
can't cross a process boundary fails in the test as well.

A test that uses the process driver needs a console binary that registers
its tasks. `Process::fake()` fakes the driver's console processes as it
fakes every other process.

A faked console process answers the way a real one does. The outcome is one
line on standard output: `suprnova-concurrency-result:`, the run's token, a
colon and `{"ok":<value>}` or `{"error":"<message>"}`, and the process exits
with code 0 for a success. The driver gives each console process its own
token in `SUPRNOVA_CONCURRENCY_TOKEN` and takes no outcome line without it,
so a fixed `Process::result(..)` can't answer a task. A closure reads the
token from the pending process:

```rust
use suprnova::{PendingProcess, Process};

fake.when("*concurrency:run*", |pending: &PendingProcess| {
    let token = pending
        .environment()
        .iter()
        .find(|(key, _)| key.as_str() == "SUPRNOVA_CONCURRENCY_TOKEN")
        .and_then(|(_, value)| value.clone())
        .unwrap_or_default();
    Process::result(format!("suprnova-concurrency-result:{token}:{{\"ok\":42}}\n"))
});
```

Most tests don't need the fake: the `sync` driver runs the task itself.

### Why Suprnova diverges

- **A task is a registered function, not a closure.** Laravel serializes
  closures with `laravel/serializable-closure`. A Rust closure can't be
  serialized, so a task is an `async fn` registered by name with serde input
  and output, and the console binary finds it by that name.
- **Tokio tasks are the default driver.** Laravel defaults to `process`
  because PHP has no threads in a request. Tokio runs tasks side by side in
  one process, so `process` is the driver you pick for isolation.
- **There is no `fork` driver.** The `process` driver is the isolating one.
- **A timeout applies on every driver, and there is none by default.**
  Laravel's timeout applies only to the `process` driver, and a process
  there gets the Process facade's 60-second default when you set none.
- **Named results come from `run_named`.** Laravel's `run` returns keys
  when you pass an associative array. Rust has no such array, so keyed
  tasks go through their own method, and a repeated key is an error.
- **Deferred tasks run without a request.** Laravel's `defer` waits for the
  end of the request or the command. Outside a request, the handle runs the
  tasks when you await it or drop it.

## Next

- [Processes](processes.md) - running other programs, which the process driver uses
- [Queues](queues.md) - running work in the background, with retries
- [Command Bus](bus.md) - dispatching a command after the response
