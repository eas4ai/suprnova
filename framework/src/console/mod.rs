//! Console - runtime CLI dispatch for user commands and framework
//! builtins.
//!
//! Each Suprnova project ships a `console` binary that calls
//! [`dispatch_argv_with_init`] with its `bootstrap::register()`. Every
//! registered command contributes a [`clap::Command`] subcommand to a
//! single parser tree, so per-command `--help`, typed args, value
//! parsing, and error messages all come from clap rather than being
//! reinvented here.
//!
//! Two registration shapes feed the same registry:
//!
//! - `#[command(name = "...", description = "...")]` on an
//!   `async fn(Vec<String>) -> Result<(), FrameworkError>` - the
//!   simple path; clap captures the trailing positional args via
//!   `trailing_var_arg` and hands them to the handler verbatim.
//! - `#[derive(Command)]` on a `clap::Parser`-deriving struct that
//!   implements [`TypedCommand`] - the typed path; clap parses
//!   the struct fields, the dispatcher calls `parsed.run().await`.
//!
//! Why a per-project console binary instead of a global CLI shell-out:
//! a global `suprnova` binary can't statically link user types
//! (seeders, commands, models) without either cargo-running the
//! project (slow, defeats the purpose) or dynamic loading (too much
//! complexity for v1). Per-project console matches Laravel's
//! `php artisan` model - same script, same process, same address
//! space.

use crate::error::FrameworkError;
use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

pub mod builtins;
mod io;
pub mod output;
mod prompts;
pub mod ssr;
pub mod testing;
mod typed;

pub use io::{
    Verbosity, ask, confirm, error, error_at, error_line, info, line, line_at, verbosity, warn,
};
pub use output::{DETAIL_WIDTH, two_column_detail};
pub use prompts::{
    Form, FormAnswers, FormValue, Progress, ask_with_default, form, multiselect, progress, secret,
    select, select_keyed,
};
pub use testing::{ConsoleRun, ConsoleTest, test};
pub use typed::TypedCommand;

/// fn-pointer-compatible boxed-future returned by every command
/// handler. Receives the per-subcommand `ArgMatches` clap parsed
/// from argv.
pub type CommandHandler =
    fn(&clap::ArgMatches) -> Pin<Box<dyn Future<Output = Result<(), FrameworkError>> + Send>>;

/// Registry entry submitted by `#[command]` / `#[derive(Command)]`.
/// Each entry carries the invocation name, a human-readable
/// description, a clap subcommand builder, and the boxed-future
/// runner.
pub struct CommandEntry {
    /// Subcommand name as it appears in argv (e.g. `make:controller`).
    pub name: &'static str,
    /// The `description` the attribute declared, and empty when it declared
    /// none. The text `--help` shows is [`Self::about`]: a
    /// `#[derive(Command)]` struct with no `description` is described by
    /// its doc comment.
    pub description: &'static str,
    /// Function that builds the clap subcommand definition.
    pub clap_builder: fn() -> clap::Command,
    /// Boxed-future runner invoked when the subcommand is selected.
    pub handler: CommandHandler,
}

impl CommandEntry {
    /// The text the console's help shows for this command, and `None`
    /// when it shows none.
    ///
    /// It is read from the clap command this entry builds, so it is what
    /// `--help` prints, whichever of these it came from: the `description`
    /// of the attribute, the doc comment of a `#[derive(Command)]` struct,
    /// or its `#[command(about = "...")]`. A listing of commands that
    /// read [`Self::description`] would show an empty line for a command
    /// that is described by its doc comment.
    pub fn about(&self) -> Option<String> {
        (self.clap_builder)()
            .get_about()
            .map(ToString::to_string)
            .filter(|about| !about.is_empty())
    }
}

inventory::collect!(CommandEntry);

/// Version string surfaced via `--version` and in `--help` output.
/// Set once at app boot via [`set_version`]; not set ⇒ clap omits
/// the `--version` flag entirely (typing it errors as an unknown
/// argument, which is the honest behavior when no version was
/// declared).
static VERSION: OnceLock<&'static str> = OnceLock::new();

/// Register the version string the console exposes via `--version`
/// and in its top-level `--help` output. Call once at the start of
/// the app's `console` binary `main`, typically with
/// `env!("CARGO_PKG_VERSION")` so the value reflects the user's
/// project, not the framework.
///
/// Subsequent calls are silently ignored (`OnceLock` semantics) -
/// the first registration wins. Tests and programmatic callers that
/// don't call this just get no `--version` support, which is fine.
pub fn set_version(version: &'static str) {
    let _ = VERSION.set(version);
}

/// Look up a registered command by name.
pub fn find(name: &str) -> Option<&'static CommandEntry> {
    inventory::iter::<CommandEntry>
        .into_iter()
        .find(|entry| entry.name == name)
}

/// All registered commands, sorted alphabetically by name.
pub fn list() -> Vec<&'static CommandEntry> {
    let mut entries: Vec<&'static CommandEntry> =
        inventory::iter::<CommandEntry>.into_iter().collect();
    entries.sort_by_key(|entry| entry.name);
    entries
}

/// Build the top-level `clap::Command` with every registered
/// subcommand attached. Name is the static literal "console" -
/// help output reads "Usage: console <COMMAND>" regardless of where
/// the binary lives on disk. Clap won't accept a runtime-owned
/// `String` here because `clap::builder::Str` only converts from
/// `&'static str` or `Box<str>`, and we'd rather not leak per call.
fn build_root() -> clap::Command {
    let mut root = with_verbosity_flags(
        clap::Command::new("console")
            .about("Suprnova console - per-project command dispatch")
            .arg_required_else_help(true)
            .subcommand_required(false),
    );
    if let Some(v) = VERSION.get() {
        root = root.version(*v);
    }
    for entry in list() {
        root = root.subcommand(command_with_verbosity_flags((entry.clap_builder)()));
    }
    root
}

/// The id of the `-q` / `--quiet` flag the console adds. Prefixed so it
/// cannot meet the id of a command's own argument.
const QUIET: &str = "__suprnova_quiet";

/// The id of the `-v` / `--verbose` flag the console adds, counted.
const VERBOSE: &str = "__suprnova_verbose";

/// The id of a raw `#[command]`'s trailing arguments.
const TRAILING_ARGS: &str = "__suprnova_trailing_args";

/// `cmd` with the console's `-q` and `-v` flags, each left out where `cmd`
/// already declares a flag of that short or long name.
///
/// The flags are added to each command rather than made global: clap's
/// debug assertions refuse a global flag that a subcommand declares too,
/// and a command's own `-v` keeps its meaning.
fn with_verbosity_flags(cmd: clap::Command) -> clap::Command {
    let declares = |short: char, long: &str| {
        cmd.get_arguments().any(|arg| {
            arg.get_short() == Some(short)
                || arg.get_long() == Some(long)
                || arg
                    .get_all_short_aliases()
                    .is_some_and(|aliases| aliases.contains(&short))
                || arg
                    .get_all_aliases()
                    .is_some_and(|aliases| aliases.contains(&long))
        })
    };
    let quiet = !declares('q', "quiet");
    let verbose = !declares('v', "verbose");
    let mut cmd = cmd;
    if quiet {
        cmd = cmd.arg(
            clap::Arg::new(QUIET)
                .short('q')
                .long("quiet")
                .action(clap::ArgAction::SetTrue)
                .help("Write nothing but the error of a failed command"),
        );
    }
    if verbose {
        cmd = cmd.arg(
            clap::Arg::new(VERBOSE)
                .short('v')
                .long("verbose")
                .action(clap::ArgAction::Count)
                .help("Write more detail: -v, -vv or -vvv"),
        );
    }
    cmd
}

/// A registered command with the console's flags after its name, unless
/// it takes raw arguments: those reach its handler as they were typed, a
/// `-v` among them, so the flags come before the command's name instead.
/// Its subcommands get the flags too.
fn command_with_verbosity_flags(cmd: clap::Command) -> clap::Command {
    let takes_raw_arguments = cmd
        .get_arguments()
        .any(|arg| arg.get_id() == TRAILING_ARGS || arg.is_trailing_var_arg_set());
    let cmd = if takes_raw_arguments {
        cmd
    } else {
        with_verbosity_flags(cmd)
    };
    cmd.mut_subcommands(command_with_verbosity_flags)
}

/// The level the console's flags ask for, from the root and every
/// subcommand on the way to the one that runs: `-q` anywhere is quiet, and
/// the `-v` counts add up.
fn verbosity_of(matches: &clap::ArgMatches) -> io::Verbosity {
    let mut quiet = false;
    let mut count: u8 = 0;
    let mut current = Some(matches);
    while let Some(m) = current {
        if m.try_get_one::<bool>(QUIET).ok().flatten() == Some(&true) {
            quiet = true;
        }
        if let Ok(Some(n)) = m.try_get_one::<u8>(VERBOSE) {
            count = count.saturating_add(*n);
        }
        current = m.subcommand().map(|(_, sub)| sub);
    }
    if quiet {
        io::Verbosity::Quiet
    } else {
        io::Verbosity::from_count(count)
    }
}

/// Dispatch argv to a registered command and nothing else: no
/// bootstrap, no framework boot, no wait for queued listeners.
///
/// For tests and programmatic callers that set up the process
/// themselves. A test that faked the queue or the mailer keeps its fake,
/// because nothing here installs the drivers of the environment over it.
/// The console binary calls [`dispatch_argv_with_init`] instead.
pub async fn dispatch_argv(argv: Vec<String>) -> Result<(), FrameworkError> {
    dispatch(argv, None::<fn() -> std::future::Ready<()>>).await
}

/// Dispatch the process's argv to a registered command, booting the
/// process the way `Application::run` boots a worker first.
///
/// Between clap's argv parse and the matched handler this runs
/// `lazy_init` - the application's `config::register_all` and
/// `bootstrap::register` - and then the framework's own process boot:
/// the container's `#[injectable]` and `#[service]` inventory, the
/// `#[policy]` gates, and the runtime drivers (Cache, Localization, the
/// environment's disks, Queue, RateLimit, Mail). A command therefore
/// resolves the same services and reaches the same drivers a queued job
/// does. A driver that does not come up is reported on stderr and does
/// not stop the command, which may use none of them; a worker refuses to
/// start instead. After the handler returns, and when the framework boot
/// fails after the bootstrap ran, the dispatcher stops the supervisors the
/// bootstrap started (up to five seconds) and waits for the queued event
/// listeners still running (up to ten), because the console's runtime ends
/// when `main` returns and would cut them off.
///
/// None of it runs unless clap matches a real registered subcommand -
/// help, version, missing-subcommand, and parse-error paths all skip
/// it, so `console --help` doesn't require `DATABASE_URL` to be set.
///
/// The full clap tree (every registered subcommand) is built each
/// call; clap then parses argv and routes to the right entry.
/// Help flags (`--help`, `-h`, missing subcommand) are clap's
/// responsibility - handled via `handle_clap_error` which prints
/// formatted output and returns Ok (for help/version) or a silent
/// Err (for parse failures) so `main` doesn't double-print.
pub async fn dispatch_argv_with_init<F, Fut>(
    argv: Vec<String>,
    lazy_init: F,
) -> Result<(), FrameworkError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    dispatch(argv, Some(lazy_init)).await
}

/// The dispatcher both entry points share. `boot` is the application's
/// bootstrap; when it is given, the framework's process boot follows it
/// and the queued listeners are awaited after the command.
async fn dispatch<F, Fut>(argv: Vec<String>, boot: Option<F>) -> Result<(), FrameworkError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let root = build_root();
    let matches = match root.try_get_matches_from(argv) {
        Ok(m) => m,
        Err(e) => return handle_clap_error(e),
    };
    io::set_verbosity(verbosity_of(&matches));

    if let Some((name, sub_matches)) = matches.subcommand() {
        if let Some(entry) = find(name) {
            let booted = boot.is_some();
            if let Some(boot) = boot {
                // Boxed, as `Application::run` holds its bootstrap: awaited
                // inline, an application's bootstrap nests its whole state
                // machine inside this one, and a deep one pushes the
                // console's `main` past rustc's query depth limit in a
                // release build.
                Box::pin(boot()).await;
                if let Err(e) = crate::app::process_boot::boot_after_hook(
                    crate::app::process_boot::ProcessBoot::Console,
                )
                .await
                {
                    // The bootstrap already ran and may have started
                    // supervisors: they are drained as after a command.
                    crate::app::process_boot::finish_process().await;
                    let error = FrameworkError::internal(format!("console bootstrap failed: {e}"));
                    // Written past the verbosity: `-q` silences the
                    // command, and never the reason it did not run.
                    io::write_errors(&format!("error: {}\n", error.message()));
                    crate::logging::Log::flush();
                    return Err(error);
                }
            }
            // One command is one unit of work: it runs in a container scope
            // of its own, so its scoped bindings are built for it and
            // dropped when it returns. The boot registers bindings and
            // stays outside.
            let command = (entry.handler)(sub_matches);
            let result = crate::container::scope::run_in_new_scope(command).await;
            // Laravel's console kernel reports the error before it renders
            // it (PAR-111). `report` passes over `AlreadyReported`, which
            // the user has seen. Before the drain below, so the queued
            // listeners a reporter starts are awaited too.
            if let Err(ref e) = result {
                crate::error::Exceptions::report(e);
            }
            // A supervisor the bootstrap started and a queued listener both
            // run as tasks of their own, and the console's runtime ends when
            // `main` returns: stop and drain them first, as the server's
            // shutdown does, so neither is cut off mid-work.
            if booted {
                crate::app::process_boot::finish_process().await;
            }
            // The file log channels buffer; a command's last records reach
            // the file before the process exits.
            crate::logging::Log::flush();
            if let Err(ref e) = result
                && !e.is_silent()
            {
                // Past the verbosity, as the bootstrap failure above.
                io::write_errors(&format!("error: {}\n", e.message()));
            }
            return result;
        }
        // Unreachable by construction: `build_root()` adds a subcommand
        // for every entry returned by `inventory::iter::<CommandEntry>`,
        // and `find(name)` searches the same iterator. Clap therefore
        // cannot match a name that `find` then misses unless those two
        // call sites disagree about the registry - a contract violation,
        // not a runtime condition. Panic so the breakage surfaces
        // immediately rather than silently exiting non-zero.
        unreachable!(
            "clap matched subcommand '{name}' but the inventory registry has no entry \
             by that name - build_root() and find() are out of sync"
        );
    }

    Ok(())
}

/// Translate a clap parse/help error into the right
/// `Result<(), FrameworkError>` shape. Help-shaped clap errors
/// (`--help`, `--version`, missing-subcommand) print to stdout and
/// resolve to `Ok(())`. Real parse errors print to stderr and
/// resolve to `Err(FrameworkError::internal(...))` so the binary's
/// `main` returns the right exit code.
fn handle_clap_error(err: clap::Error) -> Result<(), FrameworkError> {
    use clap::error::ErrorKind;
    // Clap formats the error / help / version output and writes it
    // to the right stream (stdout for help, stderr for errors). We
    // never let `main` add a redundant second print - for clap-shaped
    // failures the returned Err carries an empty message; the binary
    // skips its own eprintln and just translates to a non-zero
    // ExitCode.
    if io::is_captured() {
        // A test reads the text. `render` is the text `print` writes,
        // and `use_stderr` is the stream `print` writes it to.
        let text = err.render().to_string();
        if err.use_stderr() {
            io::write_errors(&text);
        } else {
            io::write_output(&text);
        }
    } else {
        let _ = err.print();
    }
    match err.kind() {
        ErrorKind::DisplayHelp
        | ErrorKind::DisplayVersion
        | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => Ok(()),
        _ => Err(FrameworkError::silent()),
    }
}

/// Helper for `#[command]` macro expansion - extracts the trailing
/// positional args (clap parsed via `trailing_var_arg`) into a
/// `Vec<String>` for the legacy raw-fn handler shape.
#[doc(hidden)]
pub fn collect_trailing_args(matches: &clap::ArgMatches) -> Vec<String> {
    matches
        .get_many::<String>(TRAILING_ARGS)
        .map(|values| values.cloned().collect())
        .unwrap_or_default()
}

/// Helper for `#[command]` macro expansion - builds the clap
/// subcommand for a raw `fn(Vec<String>)` handler. The single
/// trailing-var-arg captures every positional after the command
/// name; `.allow_hyphen_values(true)` lets users pass `-x` style
/// flags through to the handler without clap intercepting them.
#[doc(hidden)]
pub fn raw_clap_builder(name: &'static str, description: &'static str) -> clap::Command {
    clap::Command::new(name).about(description).arg(
        clap::Arg::new(TRAILING_ARGS)
            .action(clap::ArgAction::Append)
            .num_args(0..)
            .trailing_var_arg(true)
            .allow_hyphen_values(true),
    )
}
