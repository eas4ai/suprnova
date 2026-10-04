# Console

Each Suprnova project ships with a `console` binary - the runtime command dispatcher for everything that needs the app's compiled types: database seeders, pruners, one-shot maintenance tasks, anything you'd build with Laravel's `php artisan`. Commands are either typed structs that `#[derive(Command)]` (built on top of `clap::Parser`) or async fns annotated with `#[command]`; the framework collects them via `inventory` at link time, so adding a new command is a single file with no central registry to edit. This is the Suprnova analogue of `php artisan` - same script, same process, same address space, exits when the handler returns.

## Quick Start

The recommended shape uses `#[derive(clap::Parser, Command)]` for typed args:

```rust
use async_trait::async_trait;
use clap::Parser;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(Parser, Command, Debug)]
#[console(name = "greet", description = "Print a friendly greeting")]
pub struct Greet {
    #[arg(short, long, default_value = "world")]
    pub name: String,

    #[arg(long, default_value_t = false)]
    pub loud: bool,
}

#[async_trait]
impl TypedCommand for Greet {
    async fn run(self) -> Result<(), FrameworkError> {
        let prefix = if self.loud { "HELLO" } else { "Hello" };
        suprnova::console::line(format!("{prefix}, {}!", self.name));
        Ok(())
    }
}
```

Drop that in `src/commands/greet.rs`, add `pub mod greet;` to `src/commands/mod.rs`, and run it:

```bash
cargo run --bin console -- greet
# Hello, world!
cargo run --bin console -- greet --name Alice --loud
# HELLO, Alice!
cargo run --bin console -- greet --help
# (clap-generated per-command help, including the typed flags)
```

No central registry to edit. `#[derive(Command)]` submits a `CommandEntry { name, description, clap_builder, handler }` via inventory; the console binary calls `suprnova::console::dispatch_argv_with_init(argv, init)`, which builds one clap parser tree from every registered entry, runs the bootstrap `init` closure only when a real subcommand matches, and routes the parsed `ArgMatches` to the right handler.

### The simpler path: raw `Vec<String>`

For trivial commands that don't need typed args, the `#[command]` attribute on an async fn works too:

```rust
use suprnova::{command, FrameworkError};

#[command(name = "ping", description = "Smoke test")]
pub async fn ping(_args: Vec<String>) -> Result<(), FrameworkError> {
    suprnova::console::line("pong");
    Ok(())
}
```

A command prints with `suprnova::console::line`, not `println!`. See [Printing and asking](#printing-and-asking).

Under the hood both paths land in the same `CommandEntry` registry; the raw shape just uses a clap subcommand with a `trailing_var_arg` to capture argv into the `Vec<String>`. Prefer the typed shape for any command with arguments - you get per-command `--help`, value parsing, default values, and short/long flag pairs without writing a parser by hand.

## The Console Binary

`suprnova new` scaffolds two binaries into every new project:

- **`<project>`** (`cmd/main.rs` or `src/main.rs`) - the HTTP server, started by `cargo run` or `suprnova serve`. Long-running; serves until killed.
- **`console`** (`src/bin/console.rs`) - the runtime command dispatcher. One-shot; exits when the handler returns.

The console binary's `main` is small and predictable:

```rust
use std::process::ExitCode;

#[suprnova::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    // Surface this project's version via `--version` / `--help`.
    // env! resolves to the user's app version, not the framework's.
    suprnova::console::set_version(env!("CARGO_PKG_VERSION"));

    let argv: Vec<String> = std::env::args().collect();
    let result = suprnova::console::dispatch_argv_with_init(argv, || async {
        my_app::config::register_all();
        my_app::bootstrap::register().await;
    })
    .await;

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}
```

Tokio runs in `current_thread` flavor - there's no work to parallelize across cores in a one-shot command, and the multi-threaded runtime's worker pool would just be overhead.

Two things to notice:

- **Bootstrap is lazy.** The closure passed to `dispatch_argv_with_init` only runs when clap matches a real registered subcommand. `console --help`, `console --version`, missing-subcommand, and parse-error paths all skip it - so `console --help` works on a fresh checkout that doesn't have `DATABASE_URL` set yet.
- **A command gets what a queued job gets.** After the closure, `dispatch_argv_with_init` boots the rest of the process the way `queue:work` does: the `#[injectable]` and `#[service]` inventory, the `#[policy]` gates, and the runtime drivers (Cache, Localization, the environment's disks, Queue, RateLimit, Mail). A command can resolve an injectable action, write the cache, or check a policy with no setup of its own. A driver whose backend does not come up stops the command with an error, before it runs.
- **Queued listeners finish.** After the command returns, the dispatcher waits up to ten seconds for the queued event listeners still running, so an event the command dispatches last is handled before the process exits.
- **`main` doesn't print errors.** `dispatch_argv_with_init` owns all user-facing stderr - it writes the handler's error message as `error: <message>` (unless the error is silent, like a clap parse failure that clap already printed) and prints clap's own help / version / parse-error output. `main` is pure `Result → ExitCode` translation; adding a redundant `eprintln!` would double-print.

If you want a particular command to skip an expensive bootstrap step entirely, gate the step itself on an env var rather than threading a "lazy bootstrap" flag through the framework.

## Built-in Commands

The framework registers a small set of commands itself. Linking the framework into a project pulls them in automatically.

| Command       | What it does                              |
|---------------|-------------------------------------------|
| `db:seed`     | Run every registered `Seeder` in order. Accepts `--class=<Name>` (or a bare positional) to run a single named seeder, matching `php artisan db:seed --class=UserSeeder`. |
| `model:prune` | Walk the `PrunerEntry` registry and force-delete every row each registered `Prunable` / `MassPrunable` scope returns. `--model=<Name>` restricts to one type; `--pretend` reports rowcount without modifying any rows. |
| `db:monitor`  | Print how many connections the database server of each connection has. `--max=<n>` also dispatches `DatabaseBusy` for each server at or over that number. See [Database](database.md#busy-database---dbmonitor-and-dbmonitor). |
| `--help` / `-h` | List available commands; per-subcommand `--help` is built by clap from the typed args. |
| `--version`   | Print the version registered by `set_version` (typically your app's `CARGO_PKG_VERSION`). Omitted entirely if `set_version` was never called. |

`db:seed` runs whatever you've registered in `bootstrap::register()` with `suprnova::seed::register::<MySeeder>()`. On an empty registry a bare `db:seed` prints a warning and returns `Ok(())` - invoking `db:seed` before registering seeders is a benign user mistake, not a programmer error. A targeted run, `db:seed --class=<Name>`, fails with the not-found error when no seeder of that name is registered, whether or not other seeders are.

`db:seed` reports progress on a targeted run using
`suprnova::two_column_detail`, which renders a name, a dot leader, and a
status as one 80-column line. Your own commands can call it for the same
look.

> The worker daemons (`queue:work`, `schedule:run`, `schedule:work`, `schedule:list`, `workflow:work`) are **not** on the console binary. They live on the app/server binary's clap parser (the same binary that serves HTTP). The global `suprnova` CLI shells into `cargo run --quiet -- <name>` for those. See the [Asymmetry section](#asymmetry-with-suprnova-migrate) below.

## Defining Commands

Two macros, one registry. Pick whichever fits the command's shape.

### `#[derive(Command)]` - typed args (recommended)

Goes on top of `#[derive(clap::Parser)]`. The struct fields are the command's args; clap parses argv into the struct; the framework calls your `TypedCommand::run(self)`.

```rust
use async_trait::async_trait;
use clap::Parser;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(Parser, Command, Debug)]
#[console(name = "users:purge", description = "Purge users older than N days")]
pub struct UsersPurge {
    #[arg(long)]
    pub older_than_days: u32,

    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
}

#[async_trait]
impl TypedCommand for UsersPurge {
    async fn run(self) -> Result<(), FrameworkError> {
        // self.older_than_days, self.dry_run - typed, validated by clap
        Ok(())
    }
}
```

Attributes:

| Attribute    | Required | Purpose                                       |
|--------------|----------|-----------------------------------------------|
| `#[console(name = "...")]` | yes | The invocation name on the CLI (`"users:purge"`, `"mail:send"`, `"greet"`). |
| `#[console(description = "...")]` | no | One-line description shown in top-level help. Without it, the command keeps the about text clap has for the struct: its doc comment, or `#[command(about = "...")]`. A `description` overrides both. |
| `#[arg(...)]` (clap) | n/a | Clap's own field attributes for short/long flags, defaults, value parsers, etc. |

You also get clap's auto-generated per-command help (`console users:purge --help`) for free.

A command can take its about text from the doc comment instead of the attribute:

```rust
use async_trait::async_trait;
use clap::Parser;
use suprnova::{Command, FrameworkError, TypedCommand};

/// Rebuild the search index
#[derive(Parser, Command, Debug)]
#[console(name = "search:rebuild")]
pub struct SearchRebuild {}

#[async_trait]
impl TypedCommand for SearchRebuild {
    async fn run(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}
```

`CommandEntry::about()` returns the text the help shows for a command, whichever of the three sources it came from, and `None` when the command has none. To list commands with their descriptions, read `about()`. `CommandEntry::description` holds only the text of the `description` attribute and is empty when the attribute is missing.

```rust
for entry in suprnova::console::list() {
    let about = entry.about().unwrap_or_default();
    suprnova::console::line(format!("{:<20} {about}", entry.name));
}
```

### `#[command]` - raw `Vec<String>` (simple cases)

For commands that take no arguments or only consume positionals as a list, the attribute on an async fn is enough:

```rust
use suprnova::{command, FrameworkError};

#[command(name = "cache:clear", description = "Drop every entry from the cache")]
pub async fn cache_clear(_args: Vec<String>) -> Result<(), FrameworkError> {
    suprnova::Cache::flush().await
}
```

The annotated function must be `async fn(Vec<String>) -> Result<(), FrameworkError>`. The macro preserves the original function, so you can also call it directly from Rust - useful for unit tests that don't want to thread argv strings through the dispatcher.

Names in both shapes support Laravel-style namespacing: `mail:send`, `queue:work`, `db:fresh`. The colon is purely cosmetic - it's a string the dispatcher matches against `argv[1]`.

## Printing and asking

Print with `suprnova::console::line` and `suprnova::console::error_line`, and ask with `suprnova::console::ask` and `suprnova::console::confirm`. In the console binary they use the standard streams. Under [`console::test`](#testing-a-command) they use a buffer and a list of prepared answers, so the same command body runs in both.

| Function | What it does |
|---|---|
| `line(text)` | Print one line on the standard output. Takes anything that implements `Display`. |
| `error_line(text)` | Print one line on the standard error: a warning, or a note that must stay out of piped output. |
| `ask(question) -> Result<String, FrameworkError>` | Print the question, wait on the standard input, and return the line typed, without its line ending. |
| `confirm(question, default) -> Result<bool, FrameworkError>` | Ask a yes or no question. `y` and `yes` are yes, `n` and `no` are no, in any case. An empty line is `default`, and the `[Y/n]` or `[y/N]` hint shows which one that is. |

```rust
use async_trait::async_trait;
use clap::Parser;
use suprnova::console;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(Parser, Command, Debug)]
#[console(name = "users:purge", description = "Delete users older than N days")]
pub struct UsersPurge {
    #[arg(long)]
    pub older_than_days: u32,
}

#[async_trait]
impl TypedCommand for UsersPurge {
    async fn run(self) -> Result<(), FrameworkError> {
        let question = format!("Delete users older than {} days?", self.older_than_days);
        if !console::confirm(&question, false)? {
            console::line("nothing deleted");
            return Ok(());
        }
        console::line("deleted 12 users");
        Ok(())
    }
}
```

Three rules keep a command honest:

- `line` does not panic when the write fails. `println!` panics when the reader of a pipe has gone away, as in `console list | head`.
- `ask` and `confirm` return an error when the input ends before a line was read, which is what a command gets with no terminal and nothing piped in. The command stops instead of acting on an answer nobody gave. `confirm` also returns an error for an answer that is neither yes nor no.
- Return the error of a failed command. The console prints it as `error: <message>`, so printing it with `error_line` as well shows it twice.

`ask` blocks the thread while it waits on the standard input. That is right for a console command, which has nothing else to do until you answer, and wrong inside a server.

What a command prints with `println!` reaches the standard output of the process, and a test cannot read it. The framework's own commands, `db:seed` and `model:prune` among them, print through the console.

## Testing a command

`suprnova::console::test(argv)` runs a command through the dispatcher the console binary uses and collects what it printed. `argv` is what you type after the name of the binary. `.expects_question(question, answer)` prepares an answer, and `.run().await` returns a `ConsoleRun`.

```rust
use suprnova::console;

#[tokio::test]
async fn purge_asks_before_it_deletes() {
    let run = console::test(["users:purge", "--older-than-days", "30"])
        .expects_question("Delete users older than 30 days?", "yes")
        .run()
        .await;

    run.assert_successful().assert_every_question_was_asked();
    run.assert_output_contains("deleted 12 users");
}
```

The test binary has to link the module that holds your commands. Make sure the crate declares `pub mod commands;` in `src/lib.rs`, and reference the crate from the test. `console::test` runs no bootstrap: `dispatch_argv` has no init closure, so set up the database and the container in the test, as in [Testing](testing.md).

`ConsoleRun` has these methods:

| Method | Returns or asserts |
|---|---|
| `output()` | The standard output as a `&str`. Questions are part of it, one line each. |
| `errors()` | The standard error as a `&str`. The error of a failed command is in it, as `error: <message>`. |
| `exit_code()` | `0` when the command succeeded, `1` when it failed or its arguments did not parse. Help and the version end with `0`. |
| `error()` | The `FrameworkError` the run ended with, as an `Option`. |
| `unasked_questions()` | The questions with a prepared answer that the command did not ask, in the order you gave them. |
| `assert_successful()`, `assert_failed()` | Assert that the run ended with exit code `0`, or with a failure. |
| `assert_output_contains(text)`, `assert_errors_contain(text)` | Assert that a stream contains `text`. |
| `assert_every_question_was_asked()` | Assert that `unasked_questions()` is empty. |

The assert methods return `&Self`, so you can chain them.

Help, the version, parse errors and the error of a failed command are collected too, so `console::test(["--help"])` and an argument the command does not take are testable.

Answers are given in order. A question that comes out of order, or one with no prepared answer, makes `ask` return an error and the command fails. An answer that went to another question than the one it was written for would let a test pass while the command deleted something the test never agreed to. For `confirm`, the question is the text without the `[y/N]` hint.

The collection belongs to the task the command runs on. What a task that the command spawned prints is not collected.

### Why Suprnova diverges

Laravel collects the output of a command in `$this->artisan(...)` and checks it with `expectsOutput` and `expectsQuestion`. Suprnova follows the same shape, but commands are plain Rust functions, so nothing can hook `println!`. A command has to print through `console::line`, `console::error_line`, `console::ask` and `console::confirm` for a test to see it, and a test states its questions in the order the command asks them.

## `suprnova make:command`

The CLI generator drops a runnable stub. The generated file uses the **typed shape** (`#[derive(Parser, Command)]` + `impl TypedCommand`) - that's the recommended default, and it gives you per-command `--help` for free:

```bash
suprnova make:command cache:clear
# → src/commands/cache_clear.rs (pub struct CacheClear with #[console(name = "cache:clear")])
# → src/commands/mod.rs gets `pub mod cache_clear;` appended (created if missing)
```

The stub prints with `suprnova::console::line`, so a test can read it. It is runnable as-is - `cargo run --bin console -- cache:clear` will print a line that names the command and says it is not implemented, and return `Ok(())` so you can wire it in and iterate. Fill in fields on the struct for typed args and replace the body of `TypedCommand::run`.

Name normalization:

| Input          | File              | Command name   |
|----------------|-------------------|----------------|
| `greet`        | `greet.rs`        | `greet`        |
| `CleanCache`   | `clean_cache.rs`  | `clean-cache`  |
| `clean-cache`  | `clean_cache.rs`  | `clean-cache`  |
| `mail:send`    | `mail_send.rs`    | `mail:send`    |

If the input contains `:`, the colon namespace is preserved verbatim. Otherwise the Rust fn name is snake_case and the command name is kebab-case.

Make sure `pub mod commands;` is declared in `src/lib.rs` so the inventory submission is link-reachable from the console binary. The generator scaffolds this for new projects and emits a loud warning if it's missing; if you removed it, the new file's `inventory::submit!` block will compile but never end up in the registry.

### Why Suprnova diverges

A global binary can't statically load your app's seeders, factories, or `#[command]` async fns, so the work runs in your project's own binaries. The `suprnova` CLI forwards framework tasks (`migrate`, `db:seed`, `schedule:list`, and the rest) to them through `cargo run`, which compiles on first use. Your own `#[command]`s live in the project's `console` binary. Run it directly:

```bash
./target/debug/console db:seed
./target/release/console greet Alice
cargo run --bin console -- mail:send
```

Laravel solves the same problem with `php artisan` - a per-project script that boots the framework and dispatches to user-defined commands. PHP can do this dynamically because the framework code lives next to the user's at runtime. Rust's compile-and-link model rules that out, so we ship the dispatcher as a library (`suprnova::console::*`) and let each project link its own one-line `console` binary.

### Asymmetry with `suprnova migrate`

There are three distinct command-invocation paths in a Suprnova project, and the asymmetry is **structural** - don't try to unify them:

| Command surface                                   | Invocation                                              | Why                                                 |
|---------------------------------------------------|---------------------------------------------------------|-----------------------------------------------------|
| `suprnova new`, `suprnova make:*`, `suprnova serve`, `suprnova key:generate`, … | Global CLI binary (installed via `cargo install --git`) | File-only generators and scaffolders; don't need user code. |
| `suprnova migrate`, `suprnova migrate:status`, `suprnova schedule:run`, `suprnova schedule:work`, `suprnova schedule:list`, `suprnova workflow:work` | Global CLI shells into `cargo run --quiet -- <name>` against the app/server binary | Long-running daemons and schema work that the same `Application::run` clap parser owns. The server binary's `queue:work` lives here too - `cargo run --bin <app> -- queue:work`. |
| `console db:seed`, `console model:prune`, `console <your-command>` | Per-project `console` binary (`src/bin/console.rs`) | One-shot commands that need user types (seeders, commands, prunable models) compiled into the user's crate. |

`suprnova db:seed` and `suprnova model:prune` are forwards to the console binary: the CLI runs `cargo run --quiet --bin console -- <name>` and checks no name itself. See [CLI Overview](cli.md#database).

The split is intentional. The server binary already needs a clap parser to choose between `serve`, `migrate`, `queue:work`, etc.; daemons that share its lifecycle live there. The console binary exists for everything else - short-lived, user-defined, type-rich. New runtime commands belong in `#[command]` / `#[derive(Command)]` dispatched by the project's `console` binary.

## Best Practices

### Keep handlers small; reach for shared services through the container

A `#[command]` is the CLI-shaped wrapper; the business logic should live in an `Action`, a service, or a method on a model. The handler parses args, resolves the service from the container, and forwards. That keeps the same logic testable from a unit test, an HTTP route, and the console.

```rust
#[command(name = "users:purge")]
pub async fn users_purge(args: Vec<String>) -> Result<(), FrameworkError> {
    let action = App::resolve::<PurgeStaleUsers>()?;
    action.execute(parse(args)?).await
}
```

`App::resolve` returns `Result<T, FrameworkError::ServiceUnresolved(_)>` - the `?` flavor of `App::get` (which returns `Option`). See [Service Container](container.md) for the full surface.

### Use namespaces for related commands

Group with `:`: `mail:send`, `mail:retry`, `mail:queue:work`. The dispatcher treats it as opaque, but humans scan `mail:*` better than `send-mail`, `retry-mail`, `mail-queue-work`.

### Don't print structured data - return it

Console handlers print to stdout for human-readable output. If a downstream tool needs to consume the output, write a `console <name> --json` variant that emits machine-readable JSON to stdout and a status line to stderr. Don't make the human-readable path responsible for both audiences.

### Treat exit codes as the contract

`FrameworkError` → `ExitCode::FAILURE` is the only failure path. Don't `std::process::exit(custom_code)` from inside a handler - return `Err(...)` and let the binary's `main` translate. Future tooling (CI gates, supervised workers) only has to read the exit code.

## Reference

| Symbol                                    | Purpose                                       |
|-------------------------------------------|-----------------------------------------------|
| `suprnova::Command` (derive)              | Register a `clap::Parser`-deriving struct as a typed console command. Pairs with `TypedCommand`. |
| `suprnova::TypedCommand` (trait)          | Trait with `async fn run(self) -> Result<(), FrameworkError>` - the body of a typed command. |
| `suprnova::command` (attribute)           | Register an async fn taking `Vec<String>` as a raw-args console command. |
| `suprnova::console::dispatch_argv(argv)`  | Build the clap parser tree from every registered entry, parse argv, route to the handler. No bootstrap and no framework boot - for tests and programmatic callers that set up the process themselves, so a fake they installed is kept. |
| `suprnova::console::dispatch_argv_with_init(argv, init)` | Same as `dispatch_argv` but runs the `init` closure, then the framework's process boot (services, policies, runtime drivers), between clap's argv parse and the matched handler, and waits for queued listeners after it. None of it fires unless a real subcommand matches - `--help` / `--version` / parse-error paths skip it. This is what the scaffolded `console` binary uses. |
| `suprnova::console::set_version(&'static str)` | Register the version string surfaced via `--version` and in `--help`. Call once at the start of `main`. First registration wins. |
| `suprnova::console::find(name)`           | Look up a registered command by exact name.   |
| `suprnova::two_column_detail(left, right)` | Render a name, a dot leader, and a status word as one 80-column progress line. Mirrors Laravel's `$this->components->twoColumnDetail(...)`. |
| `suprnova::console::list()`               | All registered commands, sorted by name.      |
| `suprnova::CommandEntry`                  | Inventory record: `{ name, description, clap_builder, handler }`. Submitted by both macros. `about()` returns the text the help shows. |
| `suprnova::console::line(text)`, `error_line(text)` | Print one line on the standard output or the standard error. A test reads them. |
| `suprnova::console::ask(question)`, `confirm(question, default)` | Read an answer from the standard input, or from the answers a test prepared. |
| `suprnova::console::test(argv)`           | Prepare a run of the console for a test. Returns a `ConsoleTest`; `.expects_question(..)` prepares an answer and `.run().await` returns a `ConsoleRun`. |
| `suprnova::CommandHandler`                | The handler fn-pointer type: `fn(&clap::ArgMatches) -> Pin<Box<dyn Future<...>>>`. |
| `FrameworkError::silent()` / `.is_silent()` | Construct / detect an error that the dispatcher will NOT print to stderr. Used internally to suppress double-prints when clap already wrote a parse error to the terminal. |

## Next

- [Application Bootstrap](bootstrap.md) - what runs inside the `dispatch_argv_with_init` closure
- [Service Container](container.md) - `App::resolve` vs `App::get`, and how a handler reaches shared services
- [Seeding](seeding.md) - what `db:seed` actually invokes
- [Eloquent](eloquent.md) - `Prunable`, `MassPrunable`, and how `model:prune` walks the registry
- [Scheduling](scheduling.md) - the asymmetry: scheduler daemons live on the app binary, not the console
