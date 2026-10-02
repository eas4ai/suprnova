# Processes

The `Process` facade runs other programs: a build tool, `git`, an image
converter, a script. It captures what the program writes, ends it when it
runs too long, cleans up everything it started, runs several side by side,
and fakes them all in tests. It is Laravel's `Process` facade with the
command and the shell line kept apart.

```rust
use suprnova::Process;

let status = Process::command(["git", "status", "--short"])
    .path("/srv/app")
    .run()
    .await?;

if status.successful() {
    println!("{}", status.output());
}
```

## Commands and shell lines

`Process::command` takes the program and its arguments. Each argument
reaches the program as it is, through no shell, so a value from a form or
a file name with spaces is never split or run:

```rust
use suprnova::Process;

// `name` might hold "x; rm -rf ~": it is still one file name to convert.
Process::command(["convert", name.as_str(), "thumb.png"]).run().await?;
```

`Process::shell` takes a command line and runs it through the system
shell, `sh -c` (`cmd /C` on Windows), the way Laravel runs a string
command. Pipes, `&&`, redirects, `$VAR` and globs work:

```rust
use suprnova::Process;

Process::shell("npm ci && npm run build > build.log").run().await?;
```

Never build a shell line from input you do not control: the shell runs
whatever the line holds. Use `Process::command` for that.

## Results

`run` waits for the program and returns a `ProcessResult`:

| Method | Returns |
|---|---|
| `successful()` / `failed()` | whether the exit code is 0 |
| `exit_code()` | `Some(code)`, or `None` when a signal ended it |
| `output()` / `error_output()` | everything written to standard output and standard error |
| `see_in_output(text)` / `see_in_error_output(text)` | whether the output contains `text` |
| `command()` | the command line that ran |

A nonzero exit is a result that reports failure, not an error. Call
`throw()` to turn a failed result into a `ProcessError::Failed` that
carries the exit code and both outputs, so `?` stops on it:

```rust
let built = Process::command(["cargo", "build"]).run().await?.throw()?;
```

`run` returns an error only when the program did not give a result: it
could not be started (`ProcessError::NotStarted`, which names the
program), or it was killed for a timeout.

## Options

Every option takes the builder and returns it:

| Option | Effect |
|---|---|
| `path(dir)` | the working directory |
| `env(key, value)` | adds a variable to the environment the program inherits |
| `input(bytes)` | writes to standard input, then closes it; without it, standard input is empty |
| `timeout(duration)` | the longest it may run; 60 seconds unless set |
| `forever()` | no timeout |
| `idle_timeout(duration)` | the longest it may go without writing output |
| `quietly()` | captures no output and calls no output callback |
| `tty()` | hands the program this terminal, for a program that talks to the user; nothing is captured. `Process::supports_tty()` says whether there is one |

`run_with(callback)` calls the callback with each chunk of output as it
arrives, marked `OutputKind::Out` or `OutputKind::Err`:

```rust
use suprnova::{OutputKind, Process};

Process::command(["npm", "run", "build"])
    .run_with(|kind, chunk| match kind {
        OutputKind::Out => print!("{chunk}"),
        OutputKind::Err => print!("[stderr] {chunk}"),
    })
    .await?;
```

## Timeouts and cleanup

A program that runs past its timeout is killed and `run` returns
`ProcessError::TimedOut`, naming the command and the timeout. One that
writes nothing for its idle timeout is killed the same way, with
`ProcessError::IdleTimedOut`. Both errors carry the output written before
the kill.

On Unix each program gets a process group of its own, and a kill reaches
the whole group: a script that started background jobs takes them with
it. The same cleanup happens when the future of `run` is dropped before
it completes, as `tokio::time::timeout` or a `select!` drops it, and when
a started process is dropped. A program that exits by itself is waited on
until its output closes, and nothing it left behind is killed. Elsewhere
only the program itself is killed.

## Started processes

`start` returns the program running, as an `InvokedProcess`:

```rust
use std::time::Duration;
use suprnova::{Process, Signal};

let mut worker = Process::command(["./app", "queue:work"])
    .forever()
    .start()?;

println!("pid {:?}", worker.id());
while worker.running() {
    print!("{}", worker.latest_output());
    tokio::time::sleep(Duration::from_secs(1)).await;
}
let result = worker.wait().await?;
```

`output()` and `error_output()` return everything so far, and
`latest_output()` and `latest_error_output()` what came since the last
call. `signal(Signal::Term)` signals the program itself; `stop(grace)`
sends a terminate signal to it and everything it started, then a kill
after `grace`, and returns the result. `wait_until(|kind, chunk| ...)`
waits until the callback returns `true` for a chunk of output. The timeout
still applies to a started process: `wait` and `ensure_not_timed_out`
enforce it. Call `start` inside a Tokio runtime: the output is read by
tasks of its own.

## Pools

`Process::pool()` runs processes side by side. Add each under a key, or
push it to be keyed by its position:

```rust
use suprnova::Process;

let results = Process::pool()
    .add("assets", Process::command(["npm", "run", "build"]))
    .add("types", Process::command(["suprnova", "generate-types"]))
    .push(Process::command(["cargo", "check"]))
    .concurrency(2)
    .run()
    .await;

if !results.successful() {
    for key in results.failed() {
        eprintln!("{key} failed");
    }
}
```

`results.get(key)` returns that process's result, or its error when it
could not run or was killed for its timeout. A failure stops nothing else.
With `concurrency(n)` at most `n` run at once and the rest wait for a
slot; without it every process starts at once. `start()` returns the pool
running, as an `InvokedPool`, with `running`, `signal`, `stop` and
`wait`.

## Pipes

`Process::pipe()` runs processes in order, each with the previous one's
output as its input, and returns the last result:

```rust
use suprnova::Process;

let sorted = Process::pipe()
    .push(Process::command(["cat", "words.txt"]))
    .push(Process::command(["sort", "-u"]))
    .run()
    .await?;
```

The first process that fails ends the pipe and its result comes back; the
processes after it do not run. Each process runs to its end before the
next starts, as in Laravel. For a streaming pipe, use `Process::shell`
with `|`.

## Testing

`Process::fake()` stops every process from running while its guard lives.
A command whose command line matches a pattern gets that result; any other
gets an empty successful one:

```rust
use suprnova::Process;

let fake = Process::fake();
fake.when("git *", Process::result("main\n"));
fake.when("npm run *", Process::result("").error_output("failed").exit_code(1));

deploy().await?; // runs git and npm

fake.assert_ran("git branch --show-current");
fake.assert_ran_times("npm run build", 1);
fake.assert_not_ran("git push --force");
```

The command line is the arguments joined by spaces, or the shell line as
given, and `*` matches any run of characters; the first matching pattern
wins. `fake.prevent_stray_processes()` turns an unmatched command into
`ProcessError::Stray`.

`Process::describe()` builds a process line by line, for a started process
that reports itself running for a number of `running()` calls:

```rust
fake.when(
    "worker *",
    Process::describe()
        .output("processing 1")
        .output("processing 2")
        .exit_code(0)
        .runs_for(2),
);
```

`.id(n)` sets the process id a described process reports, and
`.replace_output(text)` and `.replace_error_output(text)` set all of its
lines at once. A faked started process records the signals sent to it, for
`has_received_signal(Signal::Term)`.

`Process::sequence([...])` answers its results in turn, one a run, and
`.push(result)` adds one at the end. A run after the last is
`ProcessError::FakeExhausted`, unless `.dont_fail_when_empty()` makes it
an empty success or `.when_empty(result)` gives the result to answer.

The assertions are `assert_ran`, `assert_ran_with(|process| ...)`,
`assert_ran_times`, `assert_ran_in_order`, `assert_not_ran` (and
`assert_didnt_run`) and `assert_nothing_ran`; `recorded()` returns every
faked process with its command line, working directory, environment,
input and result. Runs, starts, pools and pipes are all faked and
recorded.

The fake is process-global, like `Storage::fake`, and its guard
serializes the tests that take one. A test that runs real processes at
the same time as a faked test would be faked too, so keep real-process
tests out of a binary whose tests fake, or mark them `#[serial]`.

### Why Suprnova diverges

- **The command and the shell line are two methods.** Laravel's `run`
  takes either a string, run through a shell, or an array, run as it is,
  and the difference is easy to miss when a value is interpolated.
  `command` and `shell` make the choice visible.
- **A kill reaches everything the program started.** Laravel kills the
  program; a background job it started keeps running.
- **Pools take a concurrency limit.** Laravel starts every process in a
  pool at once.
- **The fake is a guard.** `Process::fake()` returns the fake, and the
  assertions are its methods, so a test cannot assert against a fake it
  did not install.

## Next

- [Queues](queues.md) - running the work in the background instead
- [Task Scheduling](scheduling.md) - running a command on a schedule
- [Testing](testing.md) - the other fakes
