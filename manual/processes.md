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
| `output()` / `error_output()` | everything written to standard output and standard error, as text |
| `output_bytes()` / `error_output_bytes()` | the same, byte for byte, for output that is not text |
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
program and command line), or it was killed for a timeout.

You see arguments quoted as a POSIX shell would quote them in results and
errors. `Process::command(["printf", "a b"])` reports `printf 'a b'`.
The quoting only describes the command; you still pass arguments directly.

## Options

Every option takes the builder and returns it:

| Option | Effect |
|---|---|
| `path(dir)` | the working directory |
| `env(key, value)` | adds a variable to the inherited environment; `env("HOME", None)` removes it |
| `input(bytes)` | writes to standard input, then closes it; without it, standard input is empty |
| `timeout(duration)` | the longest it may run; 60 seconds unless set, and zero means none, as in Laravel |
| `forever()` | no timeout |
| `idle_timeout(duration)` | the longest it may go without writing output |
| `quietly()` | keeps no output: it is read and thrown away, and no output callback is called |
| `tty()` | hands the program this terminal, for a program that talks to the user; nothing is captured, and an idle timeout is refused, since nothing can watch the terminal. Redirected standard input or standard output returns an error naming that stream. `Process::supports_tty()` says whether there is a terminal |

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

A kill reaches everything the program started: a script that started
background jobs takes them with it. On Unix each program gets a process
group of its own, and the group is killed. A `tty()` program stays in the
terminal's group, which it must share to read the terminal, so its
descendants are found in the process table and killed one by one. On
Windows `taskkill /T` ends the tree. The same cleanup happens when the
future of `run` is dropped before it completes, as `tokio::time::timeout`
or a `select!` drops it, and when a started process is dropped. A program
that exits by itself is waited on until its output closes, and nothing it
left behind is killed.

On Linux, cleanup reads the process tree from procfs and kills descendants
that called `setsid` or otherwise left the group. It sweeps twice to find
children forked during cleanup.
You cannot rely on cleanup to find a descendant that reparented to init before
the first tree snapshot.
When you wait for a program that exits by itself, it is not collected until
its output closes, so its group id cannot pass to an unrelated process that
a later kill would hit.

A child in a group of its own does not get the `SIGINT` a terminal sends
on Ctrl-C. The server and the workers end their children when they shut
down; a console command that Ctrl-C kills outright leaves its children
running unless it handles the signal, for instance with
`tokio::signal::ctrl_c()` in a `select!` beside the run.

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
call. You use `signal(Signal::Term)` to signal the program itself.
You use `stop(grace, signal)` to signal it and everything it started,
then kill whatever remains after the grace period and return the result.
`stop(None, None)` gives it 10 seconds and sends `Signal::Term`.
`stop(Duration::from_secs(1), Signal::Interrupt)` gives it one second
after an interrupt.
`wait_until(|kind, chunk| ...)` waits until the callback returns `true` for
a chunk of output. The timeouts hold for a started process whether or not
anything waits on it: a watchdog kills it at its timeout, and `wait` then
returns the timeout error. Call `start` inside a Tokio runtime: the output
is read by tasks of its own.

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
A pushed process is keyed by its position, or the next free number when a
key already holds it, and adding a key twice replaces the first process.
With `concurrency(n)` at most `n` run at once and the rest wait for a
slot; without it every process starts at once. `start()` returns the pool
running, as an `InvokedPool`, with `running`, `signal`, `stop` and
`wait`; each `running()` call starts waiting processes in the slots that
freed, so a pool can be polled to the end.

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
processes after it do not run. The output passes byte for byte, so a pipe
can carry an archive or an image. Each process runs to its end before the
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
fake.assert_ran_times("npm run build");
fake.assert_not_ran("git push --force");
```

The command line is the arguments joined by spaces, or the shell line as
given, and `*` matches any run of characters; the first matching pattern
wins. You use `assert_ran_times(command)` to assert one run and
`assert_ran_count(command, count)` to assert another count.
`fake.prevent_stray_processes()` turns an unmatched command into
`ProcessError::Stray`.

You can answer a matching command with a closure that receives the pending
process and returns a `FakeResult`:

```rust
fake.when("git *", |pending: &suprnova::PendingProcess| {
    assert_eq!(pending.arguments().map(|args| args[0].as_str()), Some("git"));
    Process::result("main\n")
});
```

You inspect `command_line()`, `arguments()`, `working_directory()`,
`environment()` and `standard_input()` in the closure. Shell commands
return `None` from `arguments()`. Environment entries hold `Some(value)`
for a set variable and `None` for a removed variable.

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

You receive described output and error output in the order you add their
lines, including mixed streams. Each described line ends with one newline,
including an empty line or a line you already end with a newline.

A faked started process reveals its output a line at a time, as a real one
would write it, so you can test code that polls a running process. Each call
of `output()` or `latest_output()` reveals the next standard output line:
`output()` returns every line revealed so far, and `latest_output()` returns
the line it revealed, or the empty string when no line is left. The two share
one count. `error_output()` and `latest_error_output()` do the same for the
error output:

```rust
fake.when("worker", Process::describe().output("one").output("two"));

let worker = Process::command(["worker"]).start()?;
assert_eq!(worker.latest_output(), "one\n");
assert_eq!(worker.output(), "one\ntwo\n");
assert_eq!(worker.latest_output(), "");
```

A line that `running()` showed counts as revealed, and every line is revealed
once `running()` returns `false`. The result `wait()` returns always holds the
whole output.

`.id(n)` sets the process id a described process reports, and
`.replace_output(text)` and `.replace_error_output(text)` set all of its
lines at once. A faked started process records the signals sent to it, for
`has_received_signal(Signal::Term)`.

`Process::sequence([...])` answers its results in turn, one a run, and
`.push(result)` adds one at the end. A run after the last is
`ProcessError::FakeExhausted`, unless `.dont_fail_when_empty()` makes it
an empty success or `.when_empty(result)` gives the result to answer.

The assertions are `assert_ran`, `assert_ran_with(|process| ...)`,
`assert_ran_times`, `assert_ran_count`, `assert_ran_in_order`, `assert_not_ran` (and
`assert_didnt_run`) and `assert_nothing_ran`; `recorded()` returns every
faked process with its command line, argument list, quoted command line,
working directory, environment, input and result. Runs, starts, pools and
pipes are all faked and recorded.

`assert_ran` and the patterns match the arguments joined by spaces, so
`["printf", "a b"]` and `["printf", "a", "b"]` look the same to them. When
the split matters, `assert_ran_args(&["printf", "a b"])` compares the
argument list exactly, and `assert_ran_command_line("printf 'a b'")` compares
the command line with each argument quoted for a POSIX shell, as
`InvokedProcess::command()` reports it. A shell line has no argument list, so
`assert_ran_args` never matches one; its command line is the line as given:

```rust
Process::command(["printf", "a b"]).run().await?;

fake.assert_ran("printf a b");
fake.assert_ran_args(&["printf", "a b"]);
fake.assert_ran_command_line("printf 'a b'");
```

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
- **A started process's timeout holds without a wait.** Laravel checks it
  only while something waits on the process.
- **The fake is a guard.** `Process::fake()` returns the fake, and the
  assertions are its methods, so a test cannot assert against a fake it
  did not install.
- **`assert_ran` matches the joined line.** Laravel's `assertRan` compares
  an array command as an array. Here `assert_ran` and the patterns match the
  arguments joined by spaces, which reads the way you type the command, and
  `assert_ran_args` compares the list when the split matters.

## Next

- [Queues](queues.md) - running the work in the background instead
- [Task Scheduling](scheduling.md) - running a command on a schedule
- [Testing](testing.md) - the other fakes
