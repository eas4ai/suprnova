//! Application builder for suprnova framework
//!
//! Provides a fluent builder API to configure and run a suprnova application.
//!
//! # Example
//!
//! ```rust,no_run
//! use suprnova::Application;
//!
//! # mod config { pub fn register_all() {} }
//! # mod bootstrap { pub async fn register() {} }
//! # mod routes {
//! #     pub fn register() -> suprnova::Router { suprnova::Router::new() }
//! # }
//! # mod migrations {
//! #     use sea_orm_migration::prelude::*;
//! #     pub struct Migrator;
//! #     impl MigratorTrait for Migrator {
//! #         fn migrations() -> Vec<Box<dyn MigrationTrait>> { vec![] }
//! #     }
//! # }
//! #[tokio::main]
//! async fn main() {
//!     Application::new()
//!         .config(config::register_all)
//!         .bootstrap(bootstrap::register)
//!         .routes(routes::register)
//!         .migrations::<migrations::Migrator>()
//!         .run()
//!         .await;
//! }
//! ```

use crate::queue::failed_console::Command as FailedJobsCommand;
use crate::schedule::tz_display::DisplayExpressions;
use crate::{FrameworkError, Router, Schedule, Server};
use clap::{Parser, Subcommand};
use sea_orm_migration::prelude::*;
use std::env;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;

pub mod maintenance;
pub mod paths;
pub(crate) mod process_boot;

use process_boot::ProcessBoot;

/// Boxed async bootstrap function (avoids repeating the complex trait-object type).
type BootstrapFn = Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>;

/// Boxed callback run once after the server boots its services.
type BootedFn = Box<dyn FnOnce()>;

/// Boxed function that registers the application's scheduled tasks.
type ScheduleFn = Box<dyn FnOnce(&mut Schedule) + Send>;

/// Boxed future returned by an asynchronous route-construction function.
type RoutesFuture = Pin<Box<dyn Future<Output = Result<Router, FrameworkError>> + Send>>;

/// The application's registered route-construction function, in whichever
/// shape registered it.
///
/// [`Application::routes`], [`Application::try_routes`] and
/// [`Application::try_routes_async`] all write this one slot, so the last
/// call wins across all three rather than each form keeping a rival
/// catalog that the server would have to choose between.
enum RoutesFn {
    /// Registered by [`Application::routes`] or
    /// [`Application::try_routes`]; ready the moment it is called.
    Sync(Box<dyn FnOnce() -> Result<Router, FrameworkError> + Send>),
    /// Registered by [`Application::try_routes_async`]; awaited.
    Async(Box<dyn FnOnce() -> RoutesFuture + Send>),
}

impl RoutesFn {
    /// Run whichever closure is in the slot and return the router it built.
    /// No registration at all yields an empty router, which is what the
    /// server built before any of the three registration methods existed.
    ///
    /// This is the whole of the sync-or-async dispatch and nothing else: it
    /// boots no services, binds no Live runtime, and reads no environment,
    /// so a test can drive it directly. Takes `Option<Self>` rather than
    /// `self` so the no-registration case lives inside the dispatch instead
    /// of at every call site.
    async fn build_router(registered: Option<Self>) -> Result<Router, FrameworkError> {
        match registered {
            Some(Self::Async(routes)) => routes().await,
            Some(Self::Sync(routes)) => routes(),
            None => Ok(Router::new()),
        }
    }

    /// [`Self::build_router`] behind the boot sequence: framework services
    /// and the immutable Live runtime are prepared first, then the
    /// registered closure runs, then the server is assembled around the
    /// router it returned.
    ///
    /// Both shapes go through the asynchronous constructor. That is not a
    /// behaviour change for a synchronous catalog: the two constructors
    /// share one prologue (`prepare_boot`) and one epilogue (`finish_boot`),
    /// and a `Sync` closure simply completes without ever yielding.
    async fn prepare_server(registered: Option<Self>) -> Result<Server, FrameworkError> {
        Server::try_from_config_with_routes_async(move || Self::build_router(registered)).await
    }
}

/// CLI structure for suprnova applications
#[derive(Parser)]
#[command(name = "app")]
#[command(about = "suprnova application server and utilities")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the web server (default command)
    Serve {
        /// Skip running migrations on startup
        #[arg(long)]
        no_migrate: bool,
    },
    /// Run the web server (alias for serve)
    #[command(name = "web:run")]
    WebRun {
        /// Skip running migrations on startup
        #[arg(long)]
        no_migrate: bool,
    },
    /// Run pending database migrations, after loading the schema dump into
    /// a database that has run none
    Migrate {
        /// The schema dump to load instead of database/schema/<engine>-schema.sql
        #[arg(long)]
        schema_path: Option<PathBuf>,
    },
    /// Show migration status
    #[command(name = "migrate:status")]
    MigrateStatus,
    /// Rollback the last migration(s)
    #[command(name = "migrate:rollback")]
    MigrateRollback {
        /// Number of migrations to rollback
        #[arg(default_value = "1")]
        steps: u32,
    },
    /// Drop all tables and re-run all migrations
    #[command(name = "migrate:fresh")]
    MigrateFresh {
        /// Required in production, alongside a typed confirmation.
        #[arg(long)]
        force: bool,
        /// The schema dump to load instead of database/schema/<engine>-schema.sql
        #[arg(long)]
        schema_path: Option<PathBuf>,
    },
    /// Write the database's schema and migration ledger to a dump file
    #[command(name = "schema:dump")]
    SchemaDump {
        /// Where to write the dump instead of database/schema/<engine>-schema.sql
        #[arg(long)]
        path: Option<PathBuf>,
        /// Replace the migrations the dump records with PrunedMigration names
        #[arg(long)]
        prune: bool,
    },
    /// Run the scheduler daemon (checks every minute)
    #[command(name = "schedule:work")]
    ScheduleWork,
    /// Run all due scheduled tasks once
    #[command(name = "schedule:run")]
    ScheduleRun,
    /// List all registered scheduled tasks
    #[command(name = "schedule:list")]
    ScheduleList {
        /// IANA timezone the listing should be read in (default: UTC)
        #[arg(long)]
        timezone: Option<String>,
    },
    /// Run the workflow worker daemon
    #[command(name = "workflow:work")]
    WorkflowWork,
    /// Run the queue worker daemon (drains the configured queue driver)
    #[command(name = "queue:work")]
    QueueWork {
        /// Visibility timeout for popped messages (seconds). Drivers may
        /// interpret this differently; see driver docs.
        #[arg(long, default_value = "60")]
        visibility_timeout: u64,
        /// Poll interval when the queue is empty (milliseconds).
        #[arg(long = "poll", default_value = "100")]
        poll_interval_ms: u64,
        /// Exit cleanly after processing this many jobs. Useful for
        /// release-on-restart deploys (worker exits, supervisor restarts).
        #[arg(long)]
        max_jobs: Option<u64>,
        /// Only drain these queues, comma-separated (e.g. `--queue=billing,default`).
        /// Omit to drain every queue. Jobs with no route count as `default`.
        /// With the `sqs` driver a name is an SQS queue, unrouted jobs are on
        /// `SQS_QUEUE`, and omitting the list drains `SQS_QUEUE` only.
        #[arg(long = "queue", value_delimiter = ',')]
        queues: Vec<String>,
        /// Drain this queue connection. Omit for the default connection.
        #[arg(long)]
        connection: Option<String>,
    },
    /// Pause job processing for a queue (or every queue with `--all`).
    /// Mirrors `php artisan queue:pause`.
    #[command(name = "queue:pause")]
    QueuePause {
        /// Queue to pause. Required unless `--all` is given.
        queue: Option<String>,
        /// Pause job processing for every queue on every connection.
        #[arg(long)]
        all: bool,
        /// Connection the queue is on. Omit for the default connection.
        #[arg(long)]
        connection: Option<String>,
    },
    /// Resume job processing for a paused queue (or every queue with
    /// `--all`). Mirrors `php artisan queue:resume` (alias
    /// `queue:continue`).
    #[command(name = "queue:resume", alias = "queue:continue")]
    QueueResume {
        /// Queue to resume. Required unless `--all` is given.
        queue: Option<String>,
        /// Resume job processing for every queue on every connection.
        /// Does not clear a per-queue pause set by `queue:pause <queue>`.
        #[arg(long)]
        all: bool,
        /// Connection the queue is on. Omit for the default connection.
        #[arg(long)]
        connection: Option<String>,
    },
    /// List the failed jobs. Mirrors `php artisan queue:failed`.
    #[command(name = "queue:failed")]
    QueueFailed,
    /// Push failed jobs back onto the queue. Mirrors
    /// `php artisan queue:retry`.
    #[command(name = "queue:retry")]
    QueueRetry {
        /// Ids of the failed jobs to retry, or `all` for every one.
        #[arg(required = true)]
        ids: Vec<String>,
    },
    /// Delete one failed job. Mirrors `php artisan queue:forget`.
    #[command(name = "queue:forget")]
    QueueForget {
        /// Id of the failed job to delete.
        id: String,
    },
    /// Delete the failed jobs. Mirrors `php artisan queue:flush`.
    #[command(name = "queue:flush")]
    QueueFlush {
        /// Only delete jobs that failed more than this many hours ago.
        #[arg(long)]
        hours: Option<u64>,
    },
    /// Delete the failed jobs older than `--hours`. Mirrors
    /// `php artisan queue:prune-failed`.
    #[command(name = "queue:prune-failed")]
    QueuePruneFailed {
        /// Delete jobs that failed more than this many hours ago.
        #[arg(long, default_value = "24")]
        hours: u64,
    },
    /// Put the application into maintenance mode
    Down {
        /// Seconds for the `Retry-After` header
        #[arg(long)]
        retry: Option<u64>,
        /// Seconds for the browser `Refresh` header
        #[arg(long)]
        refresh: Option<u64>,
        /// Secret URL segment that bypasses maintenance mode
        #[arg(long)]
        secret: Option<String>,
        /// Generate a random bypass secret and print it
        #[arg(long = "with-secret")]
        with_secret: bool,
        /// Redirect visitors to this path instead of serving the 503
        #[arg(long)]
        redirect: Option<String>,
        /// HTTP status code for the maintenance response
        #[arg(long, default_value = "503")]
        status: u16,
        /// A path that stays reachable while down (repeatable)
        #[arg(long = "except")]
        except: Vec<String>,
        /// Plain-text message rendered in the maintenance response body
        #[arg(long)]
        message: Option<String>,
    },
    /// Bring the application out of maintenance mode
    Up,
}

impl Commands {
    /// What this command boots before it runs: the one table every
    /// subcommand's boot is read from.
    ///
    /// `serve` and `web:run` need everything [`ProcessBoot::Work`] names;
    /// they boot it through `Server::from_config` and `Server::run`, which
    /// add the HTTP-only hook and the Live runtime. Every other command
    /// boots through [`Application::boot_process`] with this value.
    fn boot(&self) -> ProcessBoot {
        match self {
            Commands::Migrate { .. }
            | Commands::MigrateStatus
            | Commands::MigrateRollback { .. }
            | Commands::MigrateFresh { .. }
            | Commands::SchemaDump { .. } => ProcessBoot::Migrations,
            Commands::ScheduleList { .. } => ProcessBoot::Core,
            Commands::Down { .. } | Commands::Up => ProcessBoot::Maintenance,
            Commands::Serve { .. }
            | Commands::WebRun { .. }
            | Commands::ScheduleWork
            | Commands::ScheduleRun
            | Commands::WorkflowWork
            | Commands::QueueWork { .. }
            | Commands::QueuePause { .. }
            | Commands::QueueResume { .. }
            | Commands::QueueFailed
            | Commands::QueueRetry { .. }
            | Commands::QueueForget { .. }
            | Commands::QueueFlush { .. }
            | Commands::QueuePruneFailed { .. } => ProcessBoot::Work,
        }
    }
}

/// Application builder for suprnova framework
///
/// Use this to configure and run your suprnova application with a fluent API.
pub struct Application<M = NoMigrator>
where
    M: MigratorTrait,
{
    config_fn: Option<Box<dyn FnOnce()>>,
    bootstrap_fn: Option<BootstrapFn>,
    http_bootstrap_fn: Option<BootstrapFn>,
    routes_fn: Option<RoutesFn>,
    schedule_fn: Option<ScheduleFn>,
    booted_fns: Vec<BootedFn>,
    _migrator: std::marker::PhantomData<M>,
}

/// Placeholder type for when no migrator is configured
pub struct NoMigrator;

impl MigratorTrait for NoMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![]
    }
}

impl Application<NoMigrator> {
    /// Create a new application builder
    pub fn new() -> Self {
        Application {
            config_fn: None,
            bootstrap_fn: None,
            http_bootstrap_fn: None,
            routes_fn: None,
            schedule_fn: None,
            booted_fns: Vec::new(),
            _migrator: std::marker::PhantomData,
        }
    }

    /// The Suprnova framework version this application is built against
    /// (the `suprnova` crate's version).
    pub fn framework_version() -> &'static str {
        crate::VERSION
    }
}

impl Default for Application<NoMigrator> {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a [`Schedule`] by running the registered `schedule_fn` (if any) against
/// a fresh schedule.
///
/// Extracted as a free function (not a method on `Application<M>`) so unit tests
/// can drive the schedule registration flow without instantiating an
/// `Application<NoMigrator>` and without the migrator type bleeding into test
/// expectations.
pub(crate) fn build_schedule(schedule_fn: Option<ScheduleFn>) -> Schedule {
    let mut schedule = Schedule::new();
    if let Some(f) = schedule_fn {
        f(&mut schedule);
    }
    schedule
}

/// Resolve the `--timezone` option into the zone the listing is read in.
///
/// Defaults to UTC rather than the process's local zone: naming the local
/// zone needs an IANA lookup (`iana-time-zone`) that neither `chrono` nor
/// `chrono-tz` re-exports, and the converter needs a *named* zone, not the
/// fixed offset `chrono::Local` can supply. UTC is the one zone every
/// operator can convert from without ambiguity; pass `--timezone` to read
/// the listing in any other.
///
/// # Errors
///
/// When `name` is a non-blank string the bundled tzdb does not know. Blank
/// is treated as unset, matching how the rest of this file reads optional
/// string configuration.
fn resolve_display_timezone(name: Option<&str>) -> Result<chrono_tz::Tz, String> {
    match name.map(str::trim).filter(|n| !n.is_empty()) {
        None => Ok(chrono_tz::Tz::UTC),
        Some(name) => name.parse::<chrono_tz::Tz>().map_err(|_| {
            format!(
                "unknown --timezone `{name}`: expected an IANA zone name such as \
                 `America/New_York` or `UTC`"
            )
        }),
    }
}

/// The next two instants `expr` fires at, starting from `now` in whichever
/// zone the task is evaluated in, normalised to UTC.
///
/// Two are needed, not one: the display converter compares the zone offset
/// at both to detect a DST transition sitting between them, which is the
/// case where no single converted expression can be correct.
fn next_two_runs<Tz: chrono::TimeZone>(
    expr: &crate::schedule::CronExpression,
    now: chrono::DateTime<Tz>,
) -> (
    Option<chrono::DateTime<chrono::Utc>>,
    Option<chrono::DateTime<chrono::Utc>>,
) {
    let first = expr.next_run_after(now);
    let second = first
        .clone()
        .and_then(|first| expr.next_run_after(first))
        .map(|d| d.with_timezone(&chrono::Utc));
    (first.map(|d| d.with_timezone(&chrono::Utc)), second)
}

/// Render the `schedule:list` output for a built [`Schedule`].
///
/// Returns the exact string the handler would print to stdout, so callers can
/// either `print!("{}", …)` from a CLI handler or assert on it from a test
/// without capturing stdout. Trailing newline is included so the caller does
/// not have to worry about whether the schedule is empty.
///
/// `now` is a parameter rather than a `Utc::now()` call so the rendered
/// next-run column is reproducible in tests.
///
/// A task that pinned a timezone has its expression rewritten into
/// `display_tz` where that is possible, and one such task can occupy
/// several lines: an expression that straddles midnight in the display zone
/// needs one cron line per side. A task with no pinned zone is evaluated
/// against the process's local zone, which has no IANA name to convert
/// from, so its expression is printed as written and carries no zone label.
pub(crate) fn format_schedule_listing(
    schedule: &Schedule,
    display_tz: chrono_tz::Tz,
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    if schedule.is_empty() {
        out.push_str("No scheduled tasks registered.\n");
        out.push_str(
            "Define tasks in src/schedule.rs and wire it with \
             `Application::schedule(schedule::register)`.\n",
        );
        return out;
    }
    out.push_str("Registered scheduled tasks:\n");
    for entry in schedule.tasks() {
        // The zone label always names the zone the *printed* fields are in:
        // the display zone once they have been rewritten, the task's own
        // zone when the converter refused and left them as written. That
        // comes from the converter's own verdict rather than from comparing
        // its output against its input, because a genuine rewrite can
        // reproduce the text it started from.
        let (next, expressions, zone_label) = match entry.timezone {
            Some(event_tz) => {
                let (next, next2) = next_two_runs(&entry.expression, now.with_timezone(&event_tz));
                match crate::schedule::tz_display::expressions_for_display(
                    &entry.expression,
                    event_tz,
                    display_tz,
                    next,
                    next2,
                ) {
                    DisplayExpressions::Rewritten(expressions) => {
                        (next, expressions, Some(display_tz))
                    }
                    DisplayExpressions::AsWritten(raw) => (next, vec![raw], Some(event_tz)),
                }
            }
            // No pinned zone: the expression is read against the process's
            // local zone, which has no IANA name for the converter to work
            // from, so only the next-run instant is computed - and only
            // once, since nothing needs the second sample.
            None => (
                entry
                    .expression
                    .next_run_after(now.with_timezone(&chrono::Local))
                    .map(|at| at.with_timezone(&chrono::Utc)),
                vec![entry.expression.expression().to_string()],
                None,
            ),
        };

        let next_text = next.map_or_else(
            || "never".to_string(),
            |at| {
                at.with_timezone(&display_tz)
                    .format("%Y-%m-%d %H:%M %Z")
                    .to_string()
            },
        );

        for expression in &expressions {
            let _ = write!(out, "  {} [{expression}]", entry.name);
            if let Some(zone) = zone_label {
                let _ = write!(out, " ({zone})");
            }
            let _ = write!(out, " next: {next_text}");
            let _ = match &entry.description {
                Some(desc) => writeln!(out, " - {desc}"),
                None => writeln!(out),
            };
        }
    }
    out
}

/// Evaluate every currently-due task in the schedule and collect the results.
///
/// Returns `(results, any_failed)` so the CLI handler can drive the success/
/// failure exit semantics while tests can assert on the structured outcome
/// without intercepting `std::process::exit`.
pub(crate) async fn evaluate_due_once(
    schedule: &Schedule,
) -> (
    Vec<(String, Result<(), crate::error::FrameworkError>)>,
    bool,
) {
    let results = schedule.run_due_tasks().await;
    let any_failed = results.iter().any(|(_, r)| r.is_err());
    (results, any_failed)
}

/// Environment variable that opts the default `serve` / `web:run` auto-migrate
/// path back into the legacy log-and-continue behaviour.
///
/// Unset (or set to any non-truthy value) keeps the production-safe default:
/// migration errors abort the process before the HTTP server boots. Set to
/// `true` / `1` / `yes` / `on` (case-insensitive, trimmed) to log a warning
/// and continue.
pub(crate) const AUTO_MIGRATE_BEST_EFFORT_ENV: &str = "SUPRNOVA_AUTO_MIGRATE_BEST_EFFORT";

/// Parse the truthiness of [`AUTO_MIGRATE_BEST_EFFORT_ENV`].
///
/// Accepts `true`, `1`, `yes`, `on` (case-insensitive, surrounding whitespace
/// stripped). Everything else - including `false`, `0`, empty strings, and the
/// `None` returned by [`std::env::var`] when the variable is unset - yields
/// `false` so the production-safe fail-closed path is the default.
///
/// Extracted as a pure function so the parsing contract is unit-testable
/// without mutating the process-global environment.
pub(crate) fn parse_auto_migrate_best_effort(value: Option<&str>) -> bool {
    value
        .map(|v| {
            let v = v.trim();
            v.eq_ignore_ascii_case("true")
                || v.eq_ignore_ascii_case("yes")
                || v.eq_ignore_ascii_case("on")
                || v == "1"
        })
        .unwrap_or(false)
}

/// Apply the fail-closed-by-default auto-migration policy to a `Migrator::up`
/// result.
///
/// When `best_effort` is `false` (the default), a migration error is returned
/// as-is so the caller can abort the server boot. When `best_effort` is
/// `true`, the error is logged to stderr and swallowed so the caller can
/// continue into the server.
///
/// Extracted as a pure function so the policy is unit-testable without going
/// through `std::process::exit` or spinning up a real `Application::run`.
pub(crate) fn resolve_auto_migration(
    outcome: Result<(), sea_orm::DbErr>,
    best_effort: bool,
) -> Result<(), sea_orm::DbErr> {
    match outcome {
        Ok(()) => Ok(()),
        Err(e) if best_effort => {
            eprintln!(
                "suprnova: WARNING - auto-migrate failed in best-effort mode, server will boot \
                 against the current schema: {e}"
            );
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// Format a single background-task completion for the scheduler daemon's
/// stderr log. Failures (handler `Err`, task panic, or `JoinError` from
/// cancellation) print a single line; success completions are silent so
/// per-minute heartbeats don't drown out real signal.
fn report_background_outcome(
    joined: Result<crate::schedule::ScheduledTaskJoin, tokio::task::JoinError>,
) {
    match joined {
        Ok((_name, Ok(()))) => {}
        Ok((name, Err(e))) => {
            eprintln!("suprnova: scheduled task '{name}' failed: {e}")
        }
        Err(e) => {
            eprintln!("suprnova: scheduled task join error: {e}")
        }
    }
}

/// How long `schedule:work` waits for `run_in_background` tasks on
/// shutdown before aborting them.
///
/// Longer than the server's 5s connection drain on purpose: a background
/// scheduled task is explicitly the long-running kind - a nightly report,
/// a batch export - and cutting it off in five seconds would abandon work
/// that was about to finish. It is still bounded, because the alternative
/// was worse: the drain awaited every task with no deadline at all, so one
/// task that never returns held the process open until somebody sent
/// SIGKILL, and the operator saw a scheduler that "didn't stop".
const SCHEDULER_DRAIN_GRACE: Duration = Duration::from_secs(30);

/// Await every task in `tasks`, reporting each outcome, until `grace`
/// expires. Returns the number still running at the deadline, which are
/// aborted.
///
/// The post-abort `join_next` loop is not redundant: `abort_all` only
/// *requests* cancellation, and a task is not actually stopped until it is
/// polled again. Returning without draining leaves those tasks live while
/// the caller proceeds to exit - the same defect the server's connection
/// drain documents, where abandoned tasks kept emitting spans after the
/// telemetry flush.
async fn drain_with_grace(
    tasks: &mut tokio::task::JoinSet<crate::schedule::ScheduledTaskJoin>,
    grace: Duration,
) -> usize {
    if tasks.is_empty() {
        return 0;
    }
    let deadline = tokio::time::sleep(grace);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            joined = tasks.join_next() => match joined {
                Some(outcome) => report_background_outcome(outcome),
                None => return 0,
            },
            _ = &mut deadline => {
                let abandoned = tasks.len();
                tasks.abort_all();
                while tasks.join_next().await.is_some() {}
                return abandoned;
            }
        }
    }
}

impl<M> Application<M>
where
    M: MigratorTrait,
{
    /// Register a configuration function
    ///
    /// This function is called early during startup to register
    /// application configuration.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use suprnova::Application;
    /// # mod config { pub fn register_all() {} }
    /// # fn ex() {
    /// Application::new()
    ///     .config(config::register_all);
    /// # }
    /// ```
    pub fn config<F>(mut self, f: F) -> Self
    where
        F: FnOnce() + 'static,
    {
        self.config_fn = Some(Box::new(f));
        self
    }

    /// Register a bootstrap function
    ///
    /// This async function is called to register services, middleware,
    /// and other application components. It is process-wide: every
    /// subcommand runs it, not only the server. Register HTTP-only
    /// components - global middleware, `Inertia::install` - with
    /// [`http_bootstrap`](Self::http_bootstrap) instead, which only the
    /// server path runs.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use suprnova::Application;
    /// # mod bootstrap { pub async fn register() {} }
    /// # fn ex() {
    /// Application::new()
    ///     .bootstrap(bootstrap::register);
    /// # }
    /// ```
    pub fn bootstrap<F, Fut>(mut self, f: F) -> Self
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.bootstrap_fn = Some(Box::new(move || Box::pin(f())));
        self
    }

    /// Register an HTTP-only bootstrap function.
    ///
    /// Runs only when this process is the web server (`serve` / `web:run`),
    /// after [`bootstrap`](Self::bootstrap) and before routes are built. The
    /// worker and console subcommands (`queue:work`, `schedule:work`,
    /// `schedule:run`, `workflow:work`, `migrate*`, `down` / `up`) never run
    /// it.
    ///
    /// The split exists because HTTP boot can only succeed on a machine that
    /// serves HTTP: `Inertia::install` fails closed in production when the
    /// built frontend manifest is missing - which is precisely the state of a
    /// worker or console container image that ships no `public/assets`.
    /// Register global middleware and the Inertia layer here; keep
    /// process-wide work - `DB::init`, container bindings, event listeners,
    /// job registration - in [`bootstrap`](Self::bootstrap), which every
    /// subcommand runs.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use suprnova::Application;
    /// # mod bootstrap {
    /// #     pub async fn register() {}
    /// #     pub fn register_http_stack() {}
    /// # }
    /// # fn ex() {
    /// Application::new()
    ///     .bootstrap(bootstrap::register)
    ///     .http_bootstrap(|| async { bootstrap::register_http_stack() });
    /// # }
    /// ```
    pub fn http_bootstrap<F, Fut>(mut self, f: F) -> Self
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.http_bootstrap_fn = Some(Box::new(move || Box::pin(f())));
        self
    }

    /// Register a routes function
    ///
    /// This function returns the application's router configuration.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use suprnova::{Application, Router};
    /// # mod routes {
    /// #     pub fn register() -> suprnova::Router { suprnova::Router::new() }
    /// # }
    /// # fn ex() {
    /// Application::new()
    ///     .routes(routes::register);
    /// # }
    /// ```
    pub fn routes<F>(mut self, f: F) -> Self
    where
        F: FnOnce() -> Router + Send + 'static,
    {
        self.routes_fn = Some(RoutesFn::Sync(Box::new(|| Ok(f()))));
        self
    }

    /// Register a fallible route-construction function.
    ///
    /// Services and the immutable Live runtime are prepared before this
    /// function runs. A registration error therefore aborts startup before a
    /// listener is bound.
    ///
    /// [`routes`](Self::routes), this method and
    /// [`try_routes_async`](Self::try_routes_async) share one slot: the last
    /// of the three that is called is the one the server builds.
    pub fn try_routes<F>(mut self, f: F) -> Self
    where
        F: FnOnce() -> Result<Router, FrameworkError> + Send + 'static,
    {
        self.routes_fn = Some(RoutesFn::Sync(Box::new(f)));
        self
    }

    /// Register a fallible route-construction function that has to await.
    ///
    /// [`try_routes`](Self::try_routes) for a route catalog whose own
    /// construction is asynchronous. `RenderCache::install`
    /// ([`crate::render_cache::RenderCache::install`]) is this workspace's
    /// case: it probes the database for the generation ledger's tables
    /// before it assembles a runtime and appends its middleware, so a
    /// missing migration fails once at boot instead of on every request.
    /// Without this hook such a catalog has nowhere to run - the
    /// process-wide and HTTP boot hooks both run before any router exists,
    /// and a `booted` callback is synchronous and never sees the router.
    ///
    /// Services and the immutable Live runtime are prepared before this
    /// function runs, exactly as for the synchronous form, so a
    /// registration error aborts startup before a listener is bound.
    ///
    /// [`routes`](Self::routes), [`try_routes`](Self::try_routes) and this
    /// method share one slot: the last of the three that is called is the
    /// one the server builds.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use suprnova::{Application, FrameworkError, Router};
    /// # mod routes {
    /// #     pub fn register() -> suprnova::Router { suprnova::Router::new() }
    /// # }
    /// # mod live {
    /// #     use suprnova::{FrameworkError, Router};
    /// #     pub async fn routes_with_render_cache(r: Router) -> Result<Router, FrameworkError> {
    /// #         Ok(r)
    /// #     }
    /// # }
    /// # fn ex() {
    /// Application::new()
    ///     .try_routes_async(|| async { live::routes_with_render_cache(routes::register()).await });
    /// # }
    /// ```
    pub fn try_routes_async<F, Fut>(mut self, f: F) -> Self
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<Router, FrameworkError>> + Send + 'static,
    {
        self.routes_fn = Some(RoutesFn::Async(Box::new(move || Box::pin(f()))));
        self
    }

    /// Register a callback to run once after the server has booted its
    /// services (i.e. after `Server::from_config` has run service
    /// registration), and before it begins accepting connections.
    ///
    /// Unlike [`bootstrap`](Self::bootstrap), which registers services, a
    /// `booted` callback can *resolve* them from the container.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use suprnova::{App, Application};
    /// # #[derive(Clone, Debug)]
    /// # struct MyConfig;
    /// # fn ex() {
    /// Application::new()
    ///     .booted(|| {
    ///         let cfg: MyConfig = App::get().unwrap();
    ///         tracing::info!(?cfg, "services booted");
    ///     });
    /// # }
    /// ```
    pub fn booted<F>(mut self, f: F) -> Self
    where
        F: FnOnce() + 'static,
    {
        self.booted_fns.push(Box::new(f));
        self
    }

    /// Register the application's scheduled tasks.
    ///
    /// The function receives a mutable [`Schedule`] to add tasks to; it is run
    /// by the `schedule:work` (daemon), `schedule:run` (run-due-once), and
    /// `schedule:list` subcommands. Without it, those commands report that no
    /// tasks are registered.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use suprnova::{Application, Schedule};
    /// # mod schedule { pub fn register(_s: &mut suprnova::Schedule) {} }
    /// # fn ex() {
    /// Application::new()
    ///     .schedule(schedule::register);
    /// # }
    /// ```
    pub fn schedule<F>(mut self, f: F) -> Self
    where
        F: FnOnce(&mut Schedule) + Send + 'static,
    {
        self.schedule_fn = Some(Box::new(f));
        self
    }

    /// Configure the migrator type for database migrations
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use suprnova::Application;
    /// # mod migrations {
    /// #     use sea_orm_migration::prelude::*;
    /// #     pub struct Migrator;
    /// #     impl MigratorTrait for Migrator {
    /// #         fn migrations() -> Vec<Box<dyn MigrationTrait>> { vec![] }
    /// #     }
    /// # }
    /// # fn ex() {
    /// Application::new()
    ///     .migrations::<migrations::Migrator>();
    /// # }
    /// ```
    pub fn migrations<NewM>(self) -> Application<NewM>
    where
        NewM: MigratorTrait,
    {
        Application {
            config_fn: self.config_fn,
            bootstrap_fn: self.bootstrap_fn,
            http_bootstrap_fn: self.http_bootstrap_fn,
            routes_fn: self.routes_fn,
            schedule_fn: self.schedule_fn,
            booted_fns: self.booted_fns,
            _migrator: std::marker::PhantomData,
        }
    }

    /// Run the application
    ///
    /// This parses CLI arguments and executes the appropriate command:
    /// - `serve` (default): Run the web server
    /// - `web:run`: Run the web server (alias for serve)
    /// - `migrate`: Run pending migrations
    /// - `migrate:status`: Show migration status
    /// - `migrate:rollback`: Rollback migrations
    /// - `migrate:fresh`: Drop and re-run all migrations
    /// - `schedule:*`: Scheduler commands
    /// - `down` / `up`: Enter / leave maintenance mode
    pub async fn run(self) {
        let cli = Cli::parse();

        // Configuration is loaded by `#[suprnova::main]` *before* the
        // runtime exists, not here. Loading it writes to the process
        // environment, which is only sound while the process is
        // single-threaded - and by the time this async fn runs, every
        // Tokio worker thread already exists. See `crate::boot`.
        //
        // This is a hard refusal rather than a warning because the
        // failure it prevents is a silent data race, not a crash: an
        // app that boots "fine" under `#[tokio::main]` is exactly the
        // one that corrupts an env read on some unrelated thread weeks
        // later. A malformed `.env` still fails loudly, just earlier -
        // `load_env_or_exit` owns that message now.
        if let Err(message) = crate::boot::boot_precondition(crate::boot::env_loaded_pre_runtime())
        {
            eprintln!("{message}");
            std::process::exit(1);
        }

        // Register all #[policy] gates collected via inventory::submit!.
        // Called here (before the subcommand match) so background workers,
        // CLI commands, and scheduled tasks all see registered gates - not
        // only the web server path. The inner `Once` makes this idempotent.
        crate::authorization::init_policies();

        // Destructure self to avoid partial move issues
        let Application {
            config_fn,
            bootstrap_fn,
            http_bootstrap_fn,
            routes_fn,
            schedule_fn,
            booted_fns,
            _migrator,
        } = self;

        // Run user's config registration
        if let Some(config_fn) = config_fn {
            config_fn();
        }

        // Every command that ran the process boot may have dispatched events
        // - from jobs, tasks, workflows, or the application's own bootstrap -
        // and a queued listener runs as a task of its own; see the drain
        // after the match.
        // What the command boots, from the one table that says so. The server
        // (no command, `serve`, `web:run`) boots through `Server` itself.
        let boot = cli
            .command
            .as_ref()
            .map_or(ProcessBoot::Work, Commands::boot);
        let booted = !matches!(
            cli.command,
            None | Some(Commands::Serve { .. } | Commands::WebRun { .. })
        ) && boot != ProcessBoot::Migrations;

        match cli.command {
            None
            | Some(Commands::Serve { no_migrate: false })
            | Some(Commands::WebRun { no_migrate: false }) => {
                // Default: run server with auto-migrate
                Self::run_migrations_silent::<M>().await;
                Self::run_server_internal(bootstrap_fn, http_bootstrap_fn, routes_fn, booted_fns)
                    .await;
            }
            Some(Commands::Serve { no_migrate: true })
            | Some(Commands::WebRun { no_migrate: true }) => {
                // Run server without migrations
                Self::run_server_internal(bootstrap_fn, http_bootstrap_fn, routes_fn, booted_fns)
                    .await;
            }
            Some(Commands::Migrate { schema_path }) => {
                Self::run_migrations::<M>(schema_path).await;
            }
            Some(Commands::MigrateStatus) => {
                Self::show_migration_status::<M>().await;
            }
            Some(Commands::MigrateRollback { steps }) => {
                Self::rollback_migrations::<M>(steps).await;
            }
            Some(Commands::MigrateFresh { force, schema_path }) => {
                // The CLI's `suprnova migrate:fresh` gained this gate first,
                // but production deploys run migrations through *this*
                // binary, not the dev CLI - so without the same check here
                // the guard was bypassable by the path that matters most.
                let env = crate::config::Environment::detect();
                if let Err(message) = authorize_migrate_fresh(
                    &env,
                    force,
                    std::io::IsTerminal::is_terminal(&std::io::stdin()),
                    &mut read_confirmation_from_stdin,
                ) {
                    eprintln!("{message}");
                    std::process::exit(1);
                }
                Self::fresh_migrations::<M>(schema_path).await;
            }
            Some(Commands::SchemaDump { path, prune }) => {
                Self::dump_schema::<M>(path, prune).await;
            }
            Some(Commands::ScheduleWork) => {
                Self::run_scheduler_daemon_internal(boot, bootstrap_fn, schedule_fn).await;
            }
            Some(Commands::ScheduleRun) => {
                Self::run_scheduled_tasks_internal(boot, bootstrap_fn, schedule_fn).await;
            }
            Some(Commands::ScheduleList { timezone }) => {
                Self::list_scheduled_tasks(boot, bootstrap_fn, schedule_fn, timezone).await;
            }
            Some(Commands::WorkflowWork) => {
                Self::run_workflow_worker_internal(boot, bootstrap_fn).await;
            }
            Some(Commands::QueueWork {
                visibility_timeout,
                poll_interval_ms,
                max_jobs,
                queues,
                connection,
            }) => {
                Self::run_queue_worker_internal(
                    boot,
                    bootstrap_fn,
                    visibility_timeout,
                    poll_interval_ms,
                    max_jobs,
                    queues,
                    connection,
                )
                .await;
            }
            Some(Commands::QueuePause {
                queue,
                all,
                connection,
            }) => {
                Self::run_queue_pause_internal(boot, bootstrap_fn, queue, all, connection).await;
            }
            Some(Commands::QueueResume {
                queue,
                all,
                connection,
            }) => {
                Self::run_queue_resume_internal(boot, bootstrap_fn, queue, all, connection).await;
            }
            Some(Commands::QueueFailed) => {
                Self::run_failed_jobs_internal(boot, bootstrap_fn, FailedJobsCommand::Failed).await;
            }
            Some(Commands::QueueRetry { ids }) => {
                Self::run_failed_jobs_internal(boot, bootstrap_fn, FailedJobsCommand::Retry(ids))
                    .await;
            }
            Some(Commands::QueueForget { id }) => {
                Self::run_failed_jobs_internal(boot, bootstrap_fn, FailedJobsCommand::Forget(id))
                    .await;
            }
            Some(Commands::QueueFlush { hours }) => {
                Self::run_failed_jobs_internal(
                    boot,
                    bootstrap_fn,
                    FailedJobsCommand::Flush { hours },
                )
                .await;
            }
            Some(Commands::QueuePruneFailed { hours }) => {
                Self::run_failed_jobs_internal(
                    boot,
                    bootstrap_fn,
                    FailedJobsCommand::PruneFailed { hours },
                )
                .await;
            }
            Some(Commands::Down {
                retry,
                refresh,
                secret,
                with_secret,
                redirect,
                status,
                except,
                message,
            }) => {
                Self::run_down(
                    boot,
                    bootstrap_fn,
                    retry,
                    refresh,
                    secret,
                    with_secret,
                    redirect,
                    status,
                    except,
                    message,
                )
                .await;
            }
            Some(Commands::Up) => {
                Self::run_up(boot, bootstrap_fn).await;
            }
        }
        // A booted process waits for its queued listeners as the server does
        // at the end of its graceful shutdown: returning drops the runtime,
        // and every listener still running would end with it.
        if booted {
            crate::events::drain_queued_at_shutdown().await;
        }
        // Every command ends here; the file log channels buffer, and nothing
        // a worker wrote before a clean exit may be lost.
        crate::logging::Log::flush();
    }

    async fn run_server_internal(
        bootstrap_fn: Option<BootstrapFn>,
        http_bootstrap_fn: Option<BootstrapFn>,
        routes_fn: Option<RoutesFn>,
        booted_fns: Vec<BootedFn>,
    ) {
        // Run the process-wide hook, then the HTTP-only one.
        Self::run_boot_hooks(bootstrap_fn, http_bootstrap_fn).await;

        // Prepare framework services and the immutable Live runtime before
        // invoking fallible route construction.
        //
        // `from_config` returns Err when APP_KEY is required (any
        // non-development environment) but unset or malformed. The
        // error type carries the user-facing remediation (it points at
        // `suprnova key:generate`); we surface it on stderr without a
        // panic stack-trace wrapper so production boot logs stay clean.
        let server = match RoutesFn::prepare_server(routes_fn).await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("suprnova: failed to start server: {e}");
                std::process::exit(1);
            }
        };

        // Services are booted now (Server::from_config ran service
        // registration); fire the registered `booted` callbacks before
        // the server begins accepting connections.
        for booted in booted_fns {
            booted();
        }

        if let Err(e) = server.run().await {
            eprintln!("suprnova: server exited with error: {e}");
            std::process::exit(1);
        }
    }

    async fn get_database_connection() -> sea_orm::DatabaseConnection {
        let database_url = Self::database_url();
        sea_orm::Database::connect(crate::database::config::driver_url(&database_url).as_ref())
            .await
            .unwrap_or_else(|e| {
                eprintln!("suprnova: failed to connect to the database: {e}");
                std::process::exit(1);
            })
    }

    /// `DATABASE_URL`, with a SQLite file created when it does not exist
    /// yet. Exits when the variable is unset or the file cannot be made.
    fn database_url() -> String {
        let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| {
            eprintln!(
                "suprnova: DATABASE_URL is not set. \
                 Configure DATABASE_URL in your environment (e.g. .env) \
                 before running a database subcommand."
            );
            std::process::exit(1);
        });

        // For SQLite, ensure the database file can be created. Surface
        // filesystem errors (permission denied, no such path, read-only fs)
        // with an actionable message rather than letting them re-surface
        // later as a generic "failed to connect" panic.
        //
        // `normalize_sqlite_url` splits the file portion from any query
        // string the caller already supplied (e.g. `?mode=rwc`,
        // `?cache=shared`) so the filesystem ops below run on the bare
        // file path, and rebuilds the connect URL with `mode=rwc` merged
        // exactly once instead of double-suffixing it.
        if database_url.starts_with("sqlite://") {
            let (path, connect_url) =
                crate::database::connection::normalize_sqlite_url(&database_url);

            if path != ":memory:" && !path.starts_with(":memory:") {
                if let Some(parent) = Path::new(&path).parent()
                    && !parent.as_os_str().is_empty()
                    && let Err(e) = std::fs::create_dir_all(parent)
                {
                    eprintln!(
                        "suprnova: failed to create SQLite parent directory \
                         {parent}: {e}. Check that the path is writable and the \
                         enclosing filesystem is not read-only.",
                        parent = parent.display(),
                    );
                    std::process::exit(1);
                }

                if !Path::new(&path).exists()
                    && let Err(e) = std::fs::File::create(&path)
                {
                    eprintln!(
                        "suprnova: failed to create SQLite database file {path}: \
                         {e}. Check filesystem permissions on the target path.",
                    );
                    std::process::exit(1);
                }
            }

            connect_url
        } else {
            database_url
        }
    }

    /// Auto-migrate path for the default `serve` / `web:run` arms.
    ///
    /// **Fails closed by default.** If `Migrator::up` returns an error the
    /// process aborts with `exit(1)` rather than booting the HTTP server
    /// against a partially-migrated schema. The matching no-server paths
    /// (`migrate`, `migrate:status`, `migrate:rollback`, `migrate:fresh`)
    /// already exit on error; this brings the server entry into the same
    /// contract.
    ///
    /// Operators who deliberately want the old log-and-continue behaviour
    /// can opt in by setting [`AUTO_MIGRATE_BEST_EFFORT_ENV`] to one of the
    /// truthy values accepted by [`parse_auto_migrate_best_effort`]. The
    /// process then logs a warning and continues into the server boot.
    async fn run_migrations_silent<Migrator: MigratorTrait>() {
        // If the configured migrator has no migrations (the default
        // `NoMigrator`, or any app-defined migrator with an empty set),
        // skip the database connection entirely. This is the default
        // `serve`/`web:run` arm, and a framework app without a database
        // should boot successfully without `DATABASE_URL` being set.
        // Explicit subcommands like `migrate` continue to require it.
        if Migrator::migrations().is_empty() {
            return;
        }
        let best_effort =
            parse_auto_migrate_best_effort(env::var(AUTO_MIGRATE_BEST_EFFORT_ENV).ok().as_deref());
        let url = Self::database_url();
        // Connecting and loading a schema dump fail closed whatever the
        // best-effort setting says: it covers a failing migration, not a
        // database that cannot be reached or a dump that did not load.
        let db = match crate::SchemaDump::prepare::<Migrator>(&url, None).await {
            Ok((db, _)) => db,
            Err(e) => {
                eprintln!("suprnova: migration failed: {e}");
                eprintln!(
                    "suprnova: refusing to start the server: the database could not be reached \
                     or its schema dump did not load."
                );
                std::process::exit(1);
            }
        };
        let outcome = Migrator::up(&db, None).await;
        if let Err(e) = resolve_auto_migration(outcome, best_effort) {
            eprintln!("suprnova: migration failed: {e}");
            eprintln!(
                "suprnova: refusing to start the server against a partially-migrated schema. \
                 Fix the failing migration, or set {AUTO_MIGRATE_BEST_EFFORT_ENV}=true to keep \
                 the previous best-effort behaviour, or pass --no-migrate to skip auto-migration."
            );
            std::process::exit(1);
        }
    }

    async fn run_migrations<Migrator: MigratorTrait>(schema_path: Option<PathBuf>) {
        println!("Running migrations...");
        let url = Self::database_url();
        match crate::SchemaDump::migrate::<Migrator>(&url, schema_path.as_deref()).await {
            Ok(loaded) => {
                if let Some(path) = loaded {
                    println!("Loaded the schema dump {}", path.display());
                }
            }
            Err(e) => {
                eprintln!("suprnova: migration failed: {e}");
                std::process::exit(1);
            }
        }
        println!("Migrations completed successfully!");
    }

    async fn show_migration_status<Migrator: MigratorTrait>() {
        println!("Migration status:");
        let db = Self::get_database_connection().await;
        if let Err(e) = Migrator::status(&db).await {
            eprintln!("suprnova: failed to read migration status: {e}");
            std::process::exit(1);
        }
    }

    async fn rollback_migrations<Migrator: MigratorTrait>(steps: u32) {
        println!("Rolling back {} migration(s)...", steps);
        let db = Self::get_database_connection().await;
        if let Err(e) = Migrator::down(&db, Some(steps)).await {
            eprintln!("suprnova: rollback failed: {e}");
            std::process::exit(1);
        }
        println!("Rollback completed successfully!");
    }

    async fn fresh_migrations<Migrator: MigratorTrait>(schema_path: Option<PathBuf>) {
        println!("WARNING: Dropping all tables and re-running migrations...");
        let url = Self::database_url();
        match crate::SchemaDump::fresh::<Migrator>(&url, schema_path.as_deref()).await {
            Ok(loaded) => {
                if let Some(path) = loaded {
                    println!("Loaded the schema dump {}", path.display());
                }
            }
            Err(e) => {
                eprintln!("suprnova: database refresh failed: {e}");
                std::process::exit(1);
            }
        }
        println!("Database refreshed successfully!");
    }

    /// `schema:dump`: the schema and ledger to `path` or the engine's
    /// default dump, then, with `prune`, the dumped migrations in
    /// `src/migrations` replaced by their names.
    async fn dump_schema<Migrator: MigratorTrait>(path: Option<PathBuf>, prune: bool) {
        let url = Self::database_url();
        let path = match path {
            Some(path) => path,
            None => crate::SchemaDump::default_path(&url)
                .await
                .unwrap_or_else(|e| {
                    eprintln!("suprnova: schema dump failed: {e}");
                    std::process::exit(1);
                }),
        };
        let outcome = if prune {
            crate::SchemaDump::dump_and_prune::<Migrator>(
                &url,
                &path,
                &crate::base_path("src/migrations"),
            )
            .await
            .map(Some)
        } else {
            crate::SchemaDump::dump::<Migrator>(&url, &path)
                .await
                .map(|()| None)
        };
        match outcome {
            Ok(pruned) => {
                println!("Database schema dumped to {}", path.display());
                if let Some(pruned) = pruned {
                    println!(
                        "Pruned {} migration(s); rebuild the app before it migrates again",
                        pruned.len()
                    );
                }
            }
            Err(e) => {
                eprintln!("suprnova: schema dump failed: {e}");
                std::process::exit(1);
            }
        }
    }

    /// `schedule:work`: run the scheduler as a long-lived daemon.
    ///
    /// The first tick is aligned to the next minute boundary, then due tasks
    /// are evaluated once per minute (matching Laravel's per-minute cron
    /// evaluation). Runs the app's `bootstrap_fn`, the container's services
    /// and the runtime drivers (see [`Self::boot_process`]) so tasks can
    /// resolve services; stops on Ctrl-C or SIGTERM.
    async fn run_scheduler_daemon_internal(
        boot: ProcessBoot,
        bootstrap_fn: Option<BootstrapFn>,
        schedule_fn: Option<ScheduleFn>,
    ) {
        let shutdown = Self::start_daemon();
        Self::boot_or_exit("schedule:work", boot, bootstrap_fn).await;
        let schedule = build_schedule(schedule_fn);
        // Before any task runs: a production deployment that asks for
        // single-server execution with a per-process cache would get every
        // replica running every task, silently. Fail the boot instead.
        if let Err(e) = schedule.validate_single_server_locking() {
            eprintln!("suprnova: {e}");
            std::process::exit(1);
        }

        println!("==============================================");
        println!("  suprnova Scheduler Daemon");
        println!("==============================================");
        println!(
            "  {} task(s) registered. Stop with Ctrl+C or SIGTERM.",
            schedule.len()
        );
        println!("==============================================");

        // Align the first tick to the next minute boundary, then tick once a
        // minute. Cron expressions are evaluated against the wall clock at each
        // tick, so alignment keeps a `* * * * *` task firing at :00.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let until_next_minute = Duration::from_secs(60 - (now.as_secs() % 60))
            .saturating_sub(Duration::from_nanos(now.subsec_nanos() as u64));
        let mut tick = tokio::time::interval_at(
            tokio::time::Instant::now() + until_next_minute,
            Duration::from_secs(60),
        );
        // A task run that overruns a minute must not trigger a catch-up burst
        // that re-evaluates the same wall-clock minute (double-firing tasks);
        // skip missed ticks and resume on the next aligned boundary.
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // Long-lived JoinSet for `.run_in_background()` tasks. These tasks
        // are fire-and-forget within a tick - the loop polls completed ones
        // before each tick and on shutdown awaits the rest before exit, so a
        // slow background task never gets dropped mid-flight.
        let mut bg_tasks: tokio::task::JoinSet<crate::schedule::ScheduledTaskJoin> =
            tokio::task::JoinSet::new();

        loop {
            tokio::select! {
                _ = tick.tick() => {
                    // Surface any background tasks that completed since the
                    // last tick. `try_join_next` is non-blocking - anything
                    // still running stays in the set for the next sweep.
                    while let Some(joined) = bg_tasks.try_join_next() {
                        report_background_outcome(joined);
                    }
                    // Run this tick's due tasks. Inline tasks complete
                    // before we return; `run_in_background` tasks land in
                    // `bg_tasks` and are observed on the next tick or at
                    // shutdown.
                    for (name, result) in schedule.run_due_tasks_into(&mut bg_tasks).await {
                        if let Err(e) = result {
                            eprintln!("suprnova: scheduled task '{name}' failed: {e}");
                        }
                    }
                }
                signal = shutdown.fired() => {
                    println!("suprnova: scheduler shutting down ({}).", signal.as_str());
                    // Admission is closed by construction: this arm breaks the
                    // loop, so no further tick can spawn into `bg_tasks`.
                    if !bg_tasks.is_empty() {
                        println!(
                            "suprnova: waiting up to {}s for {} background task(s) to finish…",
                            SCHEDULER_DRAIN_GRACE.as_secs(),
                            bg_tasks.len()
                        );
                    }
                    let abandoned = drain_with_grace(&mut bg_tasks, SCHEDULER_DRAIN_GRACE).await;
                    if abandoned > 0 {
                        eprintln!(
                            "suprnova: aborted {abandoned} background task(s) still running after \
                             the {}s shutdown grace",
                            SCHEDULER_DRAIN_GRACE.as_secs()
                        );
                    }
                    break;
                }
            }
        }
    }

    /// `schedule:run`: evaluate and run the due tasks once, then exit. Exits
    /// non-zero if any task failed.
    async fn run_scheduled_tasks_internal(
        boot: ProcessBoot,
        bootstrap_fn: Option<BootstrapFn>,
        schedule_fn: Option<ScheduleFn>,
    ) {
        // Logging only. `schedule:run` evaluates the due tasks once and
        // exits, so there is no long-lived loop for a stop signal to
        // interrupt - the default disposition is right for a one-shot
        // command, and installing a handler it never reads would only
        // make Ctrl-C stop working.
        Self::install_daemon_logging();
        Self::boot_or_exit("schedule:run", boot, bootstrap_fn).await;
        let schedule = build_schedule(schedule_fn);
        // Before any task runs: a production deployment that asks for
        // single-server execution with a per-process cache would get every
        // replica running every task, silently. Fail the boot instead.
        if let Err(e) = schedule.validate_single_server_locking() {
            eprintln!("suprnova: {e}");
            std::process::exit(1);
        }

        println!("Running due scheduled tasks...");
        let (results, any_failed) = evaluate_due_once(&schedule).await;
        // The tasks may have started queued listeners, and this process ends
        // next, by returning or by the failure exit below, which would skip
        // the drain the long-running workers get after the match.
        crate::events::drain_queued_at_shutdown().await;
        if results.is_empty() {
            println!("No tasks were due.");
            return;
        }
        for (name, result) in &results {
            match result {
                Ok(()) => println!("  ✓ {name}"),
                Err(e) => eprintln!("  ✗ {name}: {e}"),
            }
        }
        if any_failed {
            std::process::exit(1);
        }
    }

    /// `schedule:list`: print every registered task and its cron expression.
    ///
    /// Boots the core of the process first, so the schedule it builds is the
    /// one the daemon would run: a schedule function that reads a binding or
    /// a path the `bootstrap` hook installed sees it here too.
    async fn list_scheduled_tasks(
        boot: ProcessBoot,
        bootstrap_fn: Option<BootstrapFn>,
        schedule_fn: Option<ScheduleFn>,
        timezone: Option<String>,
    ) {
        let display_tz = match resolve_display_timezone(timezone.as_deref()) {
            Ok(tz) => tz,
            Err(message) => {
                eprintln!("suprnova: {message}");
                std::process::exit(1);
            }
        };
        Self::boot_or_exit("schedule:list", boot, bootstrap_fn).await;
        let schedule = build_schedule(schedule_fn);
        print!(
            "{}",
            format_schedule_listing(&schedule, display_tz, crate::clock::now())
        );
    }

    async fn run_workflow_worker_internal(boot: ProcessBoot, bootstrap_fn: Option<BootstrapFn>) {
        let shutdown = Self::start_daemon();
        Self::boot_or_exit("workflow:work", boot, bootstrap_fn).await;

        let worker = crate::workflow::WorkflowWorker::new();
        let cancel = tokio_util::sync::CancellationToken::new();

        println!("==============================================");
        println!("  suprnova Workflow Worker");
        println!("==============================================");
        println!("  worker_id: {}", worker.worker_id());
        println!("  Stop with Ctrl+C or SIGTERM (in-flight workflows will drain)");
        println!("==============================================");

        // Mirror the queue worker shutdown pattern: spawn the worker on a
        // task so we can race it against the stop signal without blocking
        // the signal future. On signal we cancel the token and await the
        // task; the worker's drain loop awaits every in-flight workflow
        // before returning Ok(()).
        let cancel_for_worker = cancel.clone();
        let mut handle =
            tokio::spawn(async move { worker.run_with_cancel(cancel_for_worker).await });

        tokio::select! {
            signal = shutdown.fired() => {
                println!("suprnova: workflow worker shutting down ({}).", signal.as_str());
                cancel.cancel();
                match handle.await {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => {
                        eprintln!("Workflow worker error during drain: {e}");
                        std::process::exit(1);
                    }
                    Err(e) => {
                        eprintln!("Workflow worker task panicked during drain: {e}");
                        std::process::exit(1);
                    }
                }
            }
            res = &mut handle => {
                match res {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => {
                        eprintln!("Workflow worker error: {e}");
                        std::process::exit(1);
                    }
                    Err(e) => {
                        eprintln!("Workflow worker task panicked: {e}");
                        std::process::exit(1);
                    }
                }
            }
        }
    }

    /// `queue:work`: drain a queue connection until cancelled. The default
    /// connection unless `--connection` names another.
    ///
    /// Runs the app's `bootstrap_fn`, the container's services and the
    /// runtime drivers (see [`Self::boot_process`] for why that order), so
    /// popped jobs can resolve services from the container. Honours Ctrl-C and SIGTERM
    /// cleanly via
    /// `CancellationToken`: the cancel fires at the next pop boundary, so an
    /// in-flight handler runs to completion (bounded by its own per-job
    /// `timeout()` if set) before the worker exits.
    async fn run_queue_worker_internal(
        boot: ProcessBoot,
        bootstrap_fn: Option<BootstrapFn>,
        visibility_timeout: u64,
        poll_interval_ms: u64,
        max_jobs: Option<u64>,
        queues: Vec<String>,
        connection: Option<String>,
    ) {
        let shutdown = Self::start_daemon();
        Self::boot_or_exit("queue:work", boot, bootstrap_fn).await;

        // Resolved here as well as inside the worker: a connection nobody
        // registered has to stop the process before the banner promises a
        // worker. The banner names the connection the worker is labelled
        // with, which is the default one while no connection is registered.
        let requested = connection.unwrap_or_else(crate::queue::Queue::connection_name);
        let (driver, connection) = match crate::queue::connections::target(&requested) {
            Ok(target) => (target.driver, target.label),
            Err(e) => {
                eprintln!("suprnova: the queue worker cannot start: {e}");
                std::process::exit(1);
            }
        };

        let cfg = crate::queue::worker::WorkerConfig {
            visibility_timeout: Duration::from_secs(visibility_timeout),
            poll_interval: Duration::from_millis(poll_interval_ms),
            max_jobs,
            queues,
        };

        let cancel = tokio_util::sync::CancellationToken::new();

        println!("==============================================");
        println!("  suprnova Queue Worker");
        println!("==============================================");
        println!("  connection:         {connection}");
        println!("  driver:             {}", driver.name());
        println!("  visibility timeout: {visibility_timeout}s");
        println!("  poll interval:      {poll_interval_ms}ms");
        if let Some(m) = max_jobs {
            println!("  max jobs:           {m} (exits after)");
        } else {
            println!("  max jobs:           unlimited");
        }
        println!("  Stop with Ctrl+C or SIGTERM (in-flight jobs will drain)");
        println!("==============================================");

        // Registered after the banner so the first transition prints below it.
        // A worker that goes quiet because somebody paused its queue now says
        // so, instead of looking hung.
        crate::events::EventFacade::listen::<crate::queue::events::WorkerQueuePaused, _>(
            std::sync::Arc::new(WorkerQueuePausedPrinter),
        )
        .await;
        crate::events::EventFacade::listen::<crate::queue::events::WorkerQueueResumed, _>(
            std::sync::Arc::new(WorkerQueueResumedPrinter),
        )
        .await;

        let cancel_for_worker = cancel.clone();
        let mut worker = tokio::spawn(async move {
            if let Err(e) =
                crate::queue::worker::run_worker_on(&connection, cfg, cancel_for_worker).await
            {
                eprintln!("suprnova: queue worker could not start: {e}");
                std::process::exit(1);
            }
        });

        // Either a stop signal fires (then we cancel and wait for in-flight
        // to settle) or the worker exits on its own (max_jobs reached).
        tokio::select! {
            signal = shutdown.fired() => {
                println!("suprnova: queue worker shutting down ({}).", signal.as_str());
                cancel.cancel();
                if let Err(e) = worker.await {
                    eprintln!("suprnova: queue worker task error during drain: {e}");
                    std::process::exit(1);
                }
            }
            res = &mut worker => {
                if let Err(e) = res {
                    eprintln!("suprnova: queue worker task error: {e}");
                    std::process::exit(1);
                }
            }
        }
    }

    /// `queue:pause`: mark a queue (or, with `--all`, every queue on every
    /// connection) as paused.
    ///
    /// Refuses and exits non-zero when `QUEUE_PAUSABLE=false` - Laravel's
    /// `PauseCommand` refuses the same way when `Worker::$pausable` is
    /// false, so an operator who disabled pausing finds out immediately
    /// rather than issuing a pause a running worker will ignore.
    /// `queue:resume` has no equivalent check; see
    /// [`Self::run_queue_resume_internal`].
    async fn run_queue_pause_internal(
        boot: ProcessBoot,
        bootstrap_fn: Option<BootstrapFn>,
        queue: Option<String>,
        all: bool,
        connection: Option<String>,
    ) {
        if !crate::queue::pausable_from_env() {
            eprintln!("suprnova: queue pausing is currently disabled (QUEUE_PAUSABLE=false).");
            std::process::exit(1);
        }
        let target = match resolve_pause_target(queue, all) {
            Ok(t) => t,
            Err(message) => {
                eprintln!("suprnova: {message}");
                std::process::exit(1);
            }
        };
        Self::boot_or_exit("queue:pause", boot, bootstrap_fn).await;
        match target {
            PauseTarget::All => {
                if let Err(e) = crate::queue::Queue::pause_all().await {
                    eprintln!("suprnova: failed to pause all queues: {e}");
                    std::process::exit(1);
                }
                println!("Job processing on all queues across all connections has been paused.");
            }
            PauseTarget::Named(queue) => {
                let connection = match Self::pause_connection(connection) {
                    Ok(connection) => connection,
                    Err(e) => {
                        eprintln!("suprnova: {e}");
                        std::process::exit(1);
                    }
                };
                if let Err(e) = crate::queue::Queue::pause(&connection, &queue).await {
                    eprintln!("suprnova: failed to pause queue [{connection}:{queue}]: {e}");
                    std::process::exit(1);
                }
                println!("Job processing on queue [{connection}:{queue}] has been paused.");
            }
        }
    }

    /// `queue:failed`, `queue:retry`, `queue:forget`, `queue:flush` and
    /// `queue:prune-failed`: the operator's view of the failed-job store.
    ///
    /// Boots the way a worker does, because the store and the queue a retry
    /// pushes to are both wired by that boot. The command itself lives in
    /// [`crate::queue::failed_console`] and returns what to print. A request
    /// that was only met in part, such as a retry of an id that names no
    /// job, exits non-zero after printing.
    async fn run_failed_jobs_internal(
        boot: ProcessBoot,
        bootstrap_fn: Option<BootstrapFn>,
        command: FailedJobsCommand,
    ) {
        let name = command.name();
        Self::boot_or_exit(name, boot, bootstrap_fn).await;
        match crate::queue::failed_console::run(command).await {
            Ok(report) => {
                for line in &report.lines {
                    println!("{line}");
                }
                if !report.succeeded {
                    std::process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("suprnova: {name}: {e}");
                std::process::exit(1);
            }
        }
    }

    /// `queue:resume` (alias `queue:continue`): clear a queue's pause (or,
    /// with `--all`, the global pause). Never gated by `QUEUE_PAUSABLE` -
    /// disabling the ability to *create* a pause must not also disable the
    /// ability to *clear* one, which would leave an operator stuck with no
    /// way to undo an earlier pause once the switch is off.
    async fn run_queue_resume_internal(
        boot: ProcessBoot,
        bootstrap_fn: Option<BootstrapFn>,
        queue: Option<String>,
        all: bool,
        connection: Option<String>,
    ) {
        let target = match resolve_pause_target(queue, all) {
            Ok(t) => t,
            Err(message) => {
                eprintln!("suprnova: {message}");
                std::process::exit(1);
            }
        };
        Self::boot_or_exit("queue:resume", boot, bootstrap_fn).await;
        match target {
            PauseTarget::All => {
                if let Err(e) = crate::queue::Queue::resume_all().await {
                    eprintln!("suprnova: failed to resume all queues: {e}");
                    std::process::exit(1);
                }
                println!("Job processing on all queues across all connections has been resumed.");
            }
            PauseTarget::Named(queue) => {
                // A name that is no connection any more is resumed as it
                // was typed. Refusing it would leave a pause set for a
                // connection that was since removed with no way to clear
                // it, and the pause would be back in force the day the
                // connection is registered again.
                let connection = Self::pause_connection(connection.clone())
                    .unwrap_or_else(|_| connection.unwrap_or_default());
                if let Err(e) = crate::queue::Queue::resume(&connection, &queue).await {
                    eprintln!("suprnova: failed to resume queue [{connection}:{queue}]: {e}");
                    std::process::exit(1);
                }
                println!("Job processing on queue [{connection}:{queue}] has been resumed.");
            }
        }
    }

    /// The connection a `queue:pause` or `queue:resume` of one queue acts
    /// on: `--connection`, else the default connection.
    ///
    /// The answer is the label of the connection, the name its worker
    /// carries, because that is the name the worker looks a pause up by.
    /// A name that is no connection is an error: a pause keyed by it would
    /// reach no worker while reporting success.
    fn pause_connection(requested: Option<String>) -> Result<String, FrameworkError> {
        match requested {
            Some(name) => crate::queue::connections::target(&name).map(|target| target.label),
            None => Ok(crate::queue::Queue::connection_name()),
        }
    }

    /// Give a daemon process a tracing subscriber.
    ///
    /// `serve` gets one from `init_telemetry`; the daemons come through a
    /// different path and used to get nothing, so every `tracing::` line
    /// they emit went nowhere and `LOG_LEVEL` was inert for them. That is
    /// most of what they have to say - a worker dead-lettering a job, a
    /// scheduler skipping a tick it lost, a lock it could not release. In
    /// a container the only visible output was the startup banner, and the
    /// process looked idle while it was doing all of it.
    ///
    /// Called from the four daemon entry points rather than from
    /// [`Self::boot_process`], which they share: that helper is
    /// also exercised directly by a unit test, and installing a global
    /// subscriber inside a test binary poisons `tracing_test`'s one-shot
    /// initialiser for every capture-based test that runs afterwards. The
    /// subscriber belongs to the process that is about to run for hours,
    /// not to the bootstrap step it happens to share.
    ///
    /// Also not in [`Self::run`], which executes before the subcommand is
    /// chosen: a plain subscriber installed there would win the `try_init`
    /// race ahead of `serve`'s telemetry one and cost OTel builds their
    /// layers.
    fn install_daemon_logging() {
        // The channel is checked in `boot_process`, once the
        // bootstrap that may define it has run; until then the bootstrap's
        // own events go to the channel if it is built in, else stdout.
        crate::logging::init_subscriber(crate::logging::LogConfig::from_env());
    }

    /// Everything a daemon needs installed before it does anything else.
    ///
    /// Returns the shutdown listener, and returning it is the point: the
    /// signal handler has to exist *before* the bootstrap that can take
    /// seconds and before the banner that promises SIGTERM works. It used
    /// to be created just above the `select!`, several awaits later, and
    /// a stop arriving in that window found no handler and killed the
    /// process outright - losing the drain in exactly the situation the
    /// drain exists for, a supervisor stopping a container mid-start.
    ///
    /// Safe to create this early because the listener publishes through a
    /// `watch`: a signal that arrives before anyone waits is still there
    /// when the `select!` finally asks.
    fn start_daemon() -> crate::signals::ShutdownListener {
        Self::install_daemon_logging();
        crate::signals::spawn_shutdown_listener()
    }

    /// Boot a non-server process: the application's `bootstrap` hook, then
    /// what `boot` names (see [`process_boot::boot_after_hook`]).
    ///
    /// Every subcommand but `serve` boots here, with the [`ProcessBoot`]
    /// that [`Commands::boot`] gives it, so a worker, a queue command and
    /// `down` cannot each assemble a different subset of the boot again.
    /// The hook runs first: `QUEUE_DRIVER=database` resolves its connection
    /// out of the `DB` the hook initialized, and a log channel the hook
    /// defines can be `LOG_CHANNEL`. A bootstrap that installs a driver by
    /// hand is overridden by the environment in exactly the same way under
    /// `serve` and under `queue:work`; the storage disk of the environment
    /// is the one that gives way, so a disk the bootstrap registered under
    /// its name is kept.
    async fn boot_process(
        boot: ProcessBoot,
        bootstrap_fn: Option<BootstrapFn>,
    ) -> Result<(), process_boot::BootError> {
        if boot == ProcessBoot::Migrations {
            return Ok(());
        }
        if let Some(bootstrap_fn) = bootstrap_fn {
            bootstrap_fn().await;
        }
        process_boot::boot_after_hook(boot).await
    }

    /// [`Self::boot_process`], or report the failure and exit non-zero.
    async fn boot_or_exit(command: &str, boot: ProcessBoot, bootstrap_fn: Option<BootstrapFn>) {
        if let Err(e) = Self::boot_process(boot, bootstrap_fn).await {
            eprintln!("suprnova: {command} bootstrap error: {e}");
            std::process::exit(1);
        }
    }

    /// Run the boot hooks for an HTTP server process: the process-wide hook
    /// first, then the HTTP-only one.
    ///
    /// The order is the contract - everything the HTTP hook registers may
    /// assume the process-wide hook has already run. Worker processes never
    /// come through here; [`Self::boot_process`] takes only the
    /// process-wide hook, which is what keeps the HTTP stack (and its
    /// fail-closed Inertia manifest check) off machines that ship no
    /// frontend assets.
    async fn run_boot_hooks(
        bootstrap_fn: Option<BootstrapFn>,
        http_bootstrap_fn: Option<BootstrapFn>,
    ) {
        if let Some(bootstrap_fn) = bootstrap_fn {
            bootstrap_fn().await;
        }
        if let Some(http_bootstrap_fn) = http_bootstrap_fn {
            http_bootstrap_fn().await;
        }
    }

    /// `down`: record the maintenance payload via the configured driver.
    ///
    /// Runs the application's `bootstrap` hook first, as every subcommand
    /// does: a storage path or a cache store the hook installs is the one
    /// the serving process reads, so `down` must write to it too.
    #[allow(clippy::too_many_arguments)]
    async fn run_down(
        boot: ProcessBoot,
        bootstrap_fn: Option<BootstrapFn>,
        retry: Option<u64>,
        refresh: Option<u64>,
        secret: Option<String>,
        with_secret: bool,
        redirect: Option<String>,
        status: u16,
        except: Vec<String>,
        message: Option<String>,
    ) {
        Self::boot_or_exit("down", boot, bootstrap_fn).await;

        let secret = match (secret, with_secret) {
            (Some(s), _) => Some(s),
            (None, true) => Some(maintenance::random_secret()),
            (None, false) => None,
        };

        let payload = maintenance::MaintenancePayload {
            except,
            redirect,
            retry,
            refresh,
            secret: secret.clone(),
            status,
            template: message,
        };

        match maintenance::maintenance_mode().activate(&payload).await {
            Ok(()) => {
                println!("Application is now in maintenance mode.");
                if let Some(secret) = secret {
                    println!("Bypass maintenance mode by visiting: /{secret}");
                }
            }
            Err(e) => {
                eprintln!("suprnova: failed to enter maintenance mode: {e}");
                std::process::exit(1);
            }
        }
    }

    /// `up`: clear maintenance state via the configured driver, after the
    /// same boot `down` runs.
    async fn run_up(boot: ProcessBoot, bootstrap_fn: Option<BootstrapFn>) {
        Self::boot_or_exit("up", boot, bootstrap_fn).await;

        match maintenance::maintenance_mode().deactivate().await {
            Ok(()) => println!("Application is now live."),
            Err(e) => {
                eprintln!("suprnova: failed to bring the application up: {e}");
                std::process::exit(1);
            }
        }
    }
}

/// One line of `queue:work` output for a queue that paused or resumed.
///
/// Split from the listener so the shape can be pinned by a unit test - the
/// listener itself writes to stdout from inside a long-lived daemon. Mirrors
/// Laravel's `WorkCommand::writeQueueStatus` output, minus its ANSI colors
/// (the worker's other lines are uncolored too) and minus its `--json` mode,
/// which `queue:work` does not have.
fn format_worker_queue_status(
    queue: Option<&str>,
    paused: bool,
    at: chrono::DateTime<chrono::Utc>,
) -> String {
    let status = if paused { "PAUSED" } else { "RESUMED" };
    let stamp = at.format("%Y-%m-%d %H:%M:%S");
    match queue {
        Some(name) => format!("  {stamp} Queue {name} {status}"),
        None => format!("  {stamp} All queues {status}"),
    }
}

/// Prints one line whenever the running worker observes a queue pausing.
///
/// Registered by `queue:work` only. Without it a paused worker produces no
/// output at all, which is indistinguishable from a hung one - the operability
/// hole Laravel closed in #61142.
struct WorkerQueuePausedPrinter;

#[crate::async_trait]
impl crate::events::Listener<crate::queue::events::WorkerQueuePaused> for WorkerQueuePausedPrinter {
    async fn handle(
        &self,
        event: &crate::queue::events::WorkerQueuePaused,
    ) -> Result<(), crate::FrameworkError> {
        println!(
            "{}",
            format_worker_queue_status(event.queue.as_deref(), true, crate::clock::now())
        );
        Ok(())
    }
}

/// The mirror of [`WorkerQueuePausedPrinter`], for the way back.
struct WorkerQueueResumedPrinter;

#[crate::async_trait]
impl crate::events::Listener<crate::queue::events::WorkerQueueResumed>
    for WorkerQueueResumedPrinter
{
    async fn handle(
        &self,
        event: &crate::queue::events::WorkerQueueResumed,
    ) -> Result<(), crate::FrameworkError> {
        println!(
            "{}",
            format_worker_queue_status(event.queue.as_deref(), false, crate::clock::now())
        );
        Ok(())
    }
}

/// Decide whether a `migrate:fresh` may proceed.
///
/// Outside production this is always `Ok` - dropping a local database is
/// routine. In production it demands two *different* kinds of proof:
///
/// 1. `--force`, which proves intent at the moment the command was typed.
/// 2. A typed confirmation on an interactive terminal, which proves a
///    human is present.
///
/// The TTY requirement is the point of the second condition. Without it,
/// `echo production | app migrate:fresh --force` in a deploy script would
/// satisfy the prompt automatically and the confirmation would be just
/// another flag - which is exactly what this guard exists to prevent.
///
/// Split out from the subcommand arm and given the reader as a parameter
/// so the policy is testable without a terminal, a database, or a
/// process exit. Mirrors `suprnova migrate:fresh`'s gate in the CLI
/// crate; production runs migrations through this binary, so the two
/// have to agree.
fn authorize_migrate_fresh(
    env: &crate::config::Environment,
    force: bool,
    stdin_is_tty: bool,
    read_confirmation: &mut dyn FnMut() -> Result<String, String>,
) -> Result<(), String> {
    if !env.is_production() {
        return Ok(());
    }
    let expected = env.to_string();

    if !force {
        return Err(format!(
            "Refusing to run migrate:fresh with APP_ENV={expected}: it drops every \
             table in the database and the data is not recoverable.\n  If you are \
             certain, re-run it as `migrate:fresh --force` from an interactive \
             terminal and type the environment name when asked."
        ));
    }

    if !stdin_is_tty {
        return Err(format!(
            "Refusing to run migrate:fresh with APP_ENV={expected}: --force alone is \
             not enough, it also needs a typed confirmation, and stdin is not a \
             terminal.\n  Run it from an interactive shell. Piping the answer in \
             would make the confirmation just another flag, which is the thing this \
             guard exists to prevent."
        ));
    }

    eprintln!("About to DROP ALL TABLES in the {expected} database. This cannot be undone.");
    eprintln!("Type `{expected}` to confirm, or anything else to abort:");

    let typed = read_confirmation()?;
    if typed.trim() != expected {
        return Err(
            "Confirmation did not match - nothing was dropped and no migrations ran.".to_string(),
        );
    }
    Ok(())
}

/// Read one line from the real stdin. Only ever called on a TTY.
fn read_confirmation_from_stdin() -> Result<String, String> {
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| format!("Failed to read the confirmation from stdin: {e}"))?;
    Ok(line)
}

/// What a `queue:pause` / `queue:resume` invocation targets, resolved
/// from its parsed CLI arguments.
#[derive(Debug, PartialEq, Eq)]
enum PauseTarget {
    /// `--all` was given: every queue on every connection.
    All,
    /// A specific queue name was given.
    Named(String),
}

/// Validate a `queue:pause` / `queue:resume` invocation's `[queue]` /
/// `--all` arguments. Both commands require exactly one of them - `--all`
/// on its own, or a non-empty queue name - and share this validation.
///
/// Split out from the subcommand arms and given plain arguments, for the
/// same reason [`authorize_migrate_fresh`] is split out just above: the
/// policy is testable without a process exit. An empty-string queue name
/// (`queue:pause ""`) counts as "no queue given," matching Laravel's
/// falsy check on the same argument (`! $this->argument('queue')`).
fn resolve_pause_target(queue: Option<String>, all: bool) -> Result<PauseTarget, String> {
    if all {
        return Ok(PauseTarget::All);
    }
    match queue {
        Some(q) if !q.trim().is_empty() => Ok(PauseTarget::Named(q)),
        _ => Err("A queue name is required unless the --all option is used.".to_string()),
    }
}

#[cfg(test)]
mod worker_boot_order_tests {
    use super::*;
    use serial_test::serial;

    /// Saves and restores the process-global env this test rewrites.
    struct EnvGuard {
        saved: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn set(pairs: &[(&'static str, &str)]) -> Self {
            let saved = pairs
                .iter()
                .map(|(k, _)| (*k, std::env::var(k).ok()))
                .collect();
            for (k, v) in pairs {
                // SAFETY: `#[serial]` keeps any other test from reading or
                // writing these vars concurrently.
                unsafe {
                    std::env::set_var(k, v);
                }
            }
            Self { saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (k, v) in &self.saved {
                // SAFETY: same as above.
                unsafe {
                    match v {
                        Some(value) => std::env::set_var(k, value),
                        None => std::env::remove_var(k),
                    }
                }
            }
        }
    }

    /// `QUEUE_DRIVER=database` is the ordering tripwire: the driver resolves
    /// its connection from `DB`, so it can only be built after the app's
    /// bootstrap ran `DB::init`. Booting the drivers first - which every
    /// worker subcommand used to do - fails here with "requires DB::init()
    /// to run first", so a green run *is* the ordering assertion.
    #[tokio::test]
    #[serial]
    async fn worker_boot_runs_app_bootstrap_before_the_env_drivers() {
        let _env = EnvGuard::set(&[("QUEUE_DRIVER", "database"), ("QUEUE_DB_TABLE", "jobs")]);

        let bootstrap: BootstrapFn = Box::new(|| {
            Box::pin(async {
                crate::database::DB::init_with(
                    crate::database::DatabaseConfig::builder()
                        .url("sqlite::memory:")
                        .build(),
                )
                .await
                .expect("bootstrap must be able to initialise the database");
            })
        });

        Application::<NoMigrator>::boot_process(ProcessBoot::Work, Some(bootstrap))
            .await
            .expect("the database queue driver must find an initialised connection");

        assert_eq!(
            crate::queue::Queue::driver_name().expect("driver registered"),
            "database",
        );

        // Leave the global driver as the harmless default for anything that
        // runs after this test in the same process.
        crate::queue::Queue::set_driver(std::sync::Arc::new(
            crate::queue::memory::MemoryQueueDriver::new(),
        ));
    }

    /// A worker resolves its log channel after the application's bootstrap,
    /// so a channel the bootstrap defines can be `LOG_CHANNEL`; one that
    /// nothing defines still stops the boot, naming it.
    #[tokio::test]
    #[serial]
    async fn worker_boot_takes_a_log_channel_the_bootstrap_defines() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("worker.log");
        let name = "bootstrap-defined-log-channel";
        let _env = EnvGuard::set(&[("LOG_CHANNEL", name)]);
        let bootstrap: BootstrapFn = Box::new(move || {
            let path = path.clone();
            Box::pin(async move {
                crate::logging::Log::define(name, crate::logging::LogChannel::single(path));
            })
        });
        Application::<NoMigrator>::boot_process(ProcessBoot::Work, Some(bootstrap))
            .await
            .expect("the channel the bootstrap defines is the default");
        assert_eq!(crate::logging::Log::default_channel(), name);

        let _missing = EnvGuard::set(&[("LOG_CHANNEL", "no-such-log-channel")]);
        let error = Application::<NoMigrator>::boot_process(ProcessBoot::Work, None)
            .await
            .expect_err("a channel nothing defines stops the worker");
        assert!(error.to_string().contains("no-such-log-channel"), "{error}");
        let _ = crate::logging::Log::set_default_channel("stdout");
        crate::queue::Queue::set_driver(std::sync::Arc::new(
            crate::queue::memory::MemoryQueueDriver::new(),
        ));
    }

    /// The `sqs` queue driver checks its overflow disk as it boots, so the
    /// worker boot must register the disks of the environment first: here
    /// the overflow disk is the `s3` disk `S3_BUCKET` describes, and booting
    /// the queue before the filesystem fails with "SQS_OVERFLOW_DISK names
    /// the disk 's3', which is not registered".
    #[cfg(feature = "queue-sqs")]
    #[tokio::test]
    #[serial]
    async fn worker_boot_registers_the_environment_disks_before_the_queue() {
        let _storage = crate::filesystem::Storage::fake();
        let _env = EnvGuard::set(&[
            ("QUEUE_DRIVER", "sqs"),
            (
                "SQS_PREFIX",
                "https://sqs.us-east-1.amazonaws.com/123456789012",
            ),
            ("SQS_QUEUE", "default"),
            ("AWS_DEFAULT_REGION", "us-east-1"),
            ("AWS_ACCESS_KEY_ID", "AKIDSUPRNOVATEST"),
            ("AWS_SECRET_ACCESS_KEY", "suprnova-test-secret"),
            ("SQS_OVERFLOW_ENABLED", "true"),
            ("SQS_OVERFLOW_DISK", "s3"),
            ("S3_BUCKET", "suprnova-boot-order"),
            ("S3_REGION", "us-east-1"),
            ("S3_ENDPOINT", "http://127.0.0.1:9"),
            ("S3_ACCESS_KEY", "suprnova-test-key"),
            ("S3_SECRET_KEY", "suprnova-test-secret"),
        ]);

        Application::<NoMigrator>::boot_process(ProcessBoot::Work, None)
            .await
            .expect("the overflow disk the environment describes is registered in time");

        assert_eq!(
            crate::queue::Queue::driver_name().expect("driver registered"),
            "sqs",
        );
        crate::queue::Queue::set_driver(std::sync::Arc::new(
            crate::queue::memory::MemoryQueueDriver::new(),
        ));
    }

    // Every subcommand but `serve` boots through `boot_process` with the
    // value `Commands::boot` gives it. The tests below parse each
    // subcommand's argv, boot what it names, and look at what came up:
    //
    // - the hook: the application's `bootstrap`, which `down`, `up` and
    //   `schedule:list` used to skip;
    // - the services: the `#[injectable]` and `#[service]` inventory, which
    //   no worker and no console booted - a job that resolved an action
    //   failed with `ServiceNotFound` under `queue:work`;
    // - the policies: the `#[policy]` inventory;
    // - the drivers: `QUEUE_DRIVER=database` is set, so a boot that builds
    //   the drivers leaves the `database` queue driver behind, and one that
    //   does not leaves the `memory` driver each test starts from.

    thread_local! {
        /// How many times this thread booted the singleton inventory. The
        /// probe entry below counts itself; a thread-local so a boot in a
        /// test running in parallel cannot be mistaken for this test's.
        static SERVICE_BOOTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    fn count_service_boot() -> Result<(), String> {
        SERVICE_BOOTS.with(|boots| boots.set(boots.get() + 1));
        Ok(())
    }

    inventory::submit!(crate::container::provider::SingletonEntry {
        register: count_service_boot,
        name: "process_boot_probe",
    });

    /// Set once the `#[policy]` inventory has been drained.
    static POLICIES_DRAINED: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);

    inventory::submit!(crate::authorization::__PolicyRegistration {
        register: || POLICIES_DRAINED.store(true, std::sync::atomic::Ordering::SeqCst),
    });

    /// What one subcommand's boot brought up.
    #[derive(Debug, PartialEq, Eq)]
    struct Booted {
        hook: bool,
        services: bool,
        policies: bool,
        queue_driver: String,
    }

    /// Parse `argv`, boot what its subcommand names, and report what came up.
    async fn boot(argv: &[&str]) -> Booted {
        let command = Cli::try_parse_from(argv)
            .expect("the argv parses")
            .command
            .expect("the argv names a subcommand");
        crate::queue::Queue::set_driver(std::sync::Arc::new(
            crate::queue::memory::MemoryQueueDriver::new(),
        ));
        let _env = EnvGuard::set(&[("QUEUE_DRIVER", "database"), ("QUEUE_DB_TABLE", "jobs")]);
        let hook_ran = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ran = std::sync::Arc::clone(&hook_ran);
        let bootstrap: BootstrapFn = Box::new(move || {
            Box::pin(async move {
                crate::database::DB::init_with(
                    crate::database::DatabaseConfig::builder()
                        .url("sqlite::memory:")
                        .build(),
                )
                .await
                .expect("the bootstrap initialises the database");
                ran.store(true, std::sync::atomic::Ordering::SeqCst);
            })
        });
        let before = SERVICE_BOOTS.with(std::cell::Cell::get);
        Application::<NoMigrator>::boot_process(command.boot(), Some(bootstrap))
            .await
            .expect("the boot succeeds");
        let booted = Booted {
            hook: hook_ran.load(std::sync::atomic::Ordering::SeqCst),
            services: SERVICE_BOOTS.with(std::cell::Cell::get) > before,
            policies: POLICIES_DRAINED.load(std::sync::atomic::Ordering::SeqCst),
            queue_driver: crate::queue::Queue::driver_name()
                .expect("a queue driver is set")
                .to_string(),
        };
        crate::queue::Queue::set_driver(std::sync::Arc::new(
            crate::queue::memory::MemoryQueueDriver::new(),
        ));
        booted
    }

    fn work() -> Booted {
        Booted {
            hook: true,
            services: true,
            policies: true,
            queue_driver: "database".to_string(),
        }
    }

    fn without_drivers() -> Booted {
        Booted {
            queue_driver: "memory".to_string(),
            ..work()
        }
    }

    async fn assert_migrations_boot_nothing(argv: &[&str]) {
        let booted = boot(argv).await;
        assert!(!booted.hook, "{argv:?} runs no application hook");
        assert!(!booted.services, "{argv:?} boots no services");
        assert_eq!(booted.queue_driver, "memory", "{argv:?} builds no driver");
    }

    #[tokio::test]
    #[serial]
    async fn queue_work_boots_the_hook_services_policies_and_drivers() {
        assert_eq!(boot(&["app", "queue:work"]).await, work());
    }

    #[tokio::test]
    #[serial]
    async fn queue_pause_boots_the_hook_services_policies_and_drivers() {
        assert_eq!(boot(&["app", "queue:pause", "default"]).await, work());
    }

    #[tokio::test]
    #[serial]
    async fn queue_resume_boots_the_hook_services_policies_and_drivers() {
        assert_eq!(boot(&["app", "queue:resume", "default"]).await, work());
    }

    #[tokio::test]
    #[serial]
    async fn queue_failed_boots_the_hook_services_policies_and_drivers() {
        assert_eq!(boot(&["app", "queue:failed"]).await, work());
    }

    #[tokio::test]
    #[serial]
    async fn queue_retry_boots_the_hook_services_policies_and_drivers() {
        assert_eq!(boot(&["app", "queue:retry", "all"]).await, work());
    }

    #[tokio::test]
    #[serial]
    async fn queue_forget_boots_the_hook_services_policies_and_drivers() {
        assert_eq!(boot(&["app", "queue:forget", "1"]).await, work());
    }

    #[tokio::test]
    #[serial]
    async fn queue_flush_boots_the_hook_services_policies_and_drivers() {
        assert_eq!(boot(&["app", "queue:flush"]).await, work());
    }

    #[tokio::test]
    #[serial]
    async fn queue_prune_failed_boots_the_hook_services_policies_and_drivers() {
        assert_eq!(boot(&["app", "queue:prune-failed"]).await, work());
    }

    #[tokio::test]
    #[serial]
    async fn schedule_work_boots_the_hook_services_policies_and_drivers() {
        assert_eq!(boot(&["app", "schedule:work"]).await, work());
    }

    #[tokio::test]
    #[serial]
    async fn schedule_run_boots_the_hook_services_policies_and_drivers() {
        assert_eq!(boot(&["app", "schedule:run"]).await, work());
    }

    #[tokio::test]
    #[serial]
    async fn workflow_work_boots_the_hook_services_policies_and_drivers() {
        assert_eq!(boot(&["app", "workflow:work"]).await, work());
    }

    /// `schedule:list` builds the schedule the daemon would run, so it
    /// runs the hook that schedule may read; it runs no task, so it needs
    /// no driver.
    #[tokio::test]
    #[serial]
    async fn schedule_list_boots_the_hook_services_and_policies() {
        assert_eq!(boot(&["app", "schedule:list"]).await, without_drivers());
    }

    /// `down` runs the hook: a storage path or a cache store the hook
    /// installs is the one the serving process reads its maintenance state
    /// from. The queue driver is not built - a broken queue backend must
    /// not stop the command an operator runs during the outage.
    #[tokio::test]
    #[serial]
    async fn down_boots_the_hook_services_and_policies() {
        assert_eq!(boot(&["app", "down"]).await, without_drivers());
    }

    #[tokio::test]
    #[serial]
    async fn up_boots_the_hook_services_and_policies() {
        assert_eq!(boot(&["app", "up"]).await, without_drivers());
    }

    /// The migrations run no hook, as `serve` runs its own migrations
    /// before the hook: a fresh database has to be migratable even when the
    /// hook reads a table the migrations have not created yet.
    #[tokio::test]
    #[serial]
    async fn migrate_boots_nothing() {
        assert_migrations_boot_nothing(&["app", "migrate"]).await;
    }

    #[tokio::test]
    #[serial]
    async fn migrate_status_boots_nothing() {
        assert_migrations_boot_nothing(&["app", "migrate:status"]).await;
    }

    #[tokio::test]
    #[serial]
    async fn migrate_rollback_boots_nothing() {
        assert_migrations_boot_nothing(&["app", "migrate:rollback"]).await;
    }

    #[tokio::test]
    #[serial]
    async fn migrate_fresh_boots_nothing() {
        assert_migrations_boot_nothing(&["app", "migrate:fresh"]).await;
    }

    #[tokio::test]
    #[serial]
    async fn schema_dump_boots_nothing() {
        assert_migrations_boot_nothing(&["app", "schema:dump"]).await;
    }
}

#[cfg(test)]
mod migrate_fresh_gate_tests {
    use super::authorize_migrate_fresh;
    use crate::config::Environment;

    /// Never called - reaching it means the gate asked for a confirmation
    /// on a path that should have refused before prompting.
    fn unreachable_reader() -> Result<String, String> {
        panic!("the gate must refuse before prompting on this path");
    }

    #[test]
    fn non_production_needs_no_force_and_no_prompt() {
        for env in [
            Environment::Local,
            Environment::Development,
            Environment::Testing,
            Environment::Staging,
        ] {
            assert!(
                authorize_migrate_fresh(&env, false, false, &mut unreachable_reader).is_ok(),
                "{env} must not be gated - dropping a non-production database is routine"
            );
        }
    }

    #[test]
    fn production_without_force_refuses_without_prompting() {
        let err = authorize_migrate_fresh(
            &Environment::Production,
            false,
            true,
            &mut unreachable_reader,
        )
        .expect_err("production without --force must refuse");
        assert!(err.contains("--force"), "the message names the flag: {err}");
    }

    /// The case that matters for deploy scripts: `--force` is present, but
    /// stdin is a pipe. Refusing here is what stops
    /// `echo production | app migrate:fresh --force` from working.
    #[test]
    fn production_with_force_but_no_tty_refuses_without_prompting() {
        let err = authorize_migrate_fresh(
            &Environment::Production,
            true,
            false,
            &mut unreachable_reader,
        )
        .expect_err("--force without a TTY must refuse");
        assert!(
            err.contains("not a terminal"),
            "the message explains why piping is rejected: {err}"
        );
    }

    #[test]
    fn production_with_force_and_matching_confirmation_proceeds() {
        let mut reader = || Ok("production\n".to_string());
        assert!(
            authorize_migrate_fresh(&Environment::Production, true, true, &mut reader).is_ok(),
            "the fully-authorized path must proceed"
        );
    }

    #[test]
    fn production_with_wrong_confirmation_refuses() {
        let mut reader = || Ok("yes\n".to_string());
        let err = authorize_migrate_fresh(&Environment::Production, true, true, &mut reader)
            .expect_err("a mismatched confirmation must abort");
        assert!(
            err.contains("nothing was dropped"),
            "the message states nothing happened: {err}"
        );
    }
}

#[cfg(test)]
mod queue_pause_target_tests {
    //! `resolve_pause_target` is what `queue:pause` / `queue:resume`
    //! delegate to before touching the queue driver or exiting the
    //! process. Tested here - not in `framework/tests/queue_pause.rs` -
    //! for the same reason `migrate_fresh_gate_tests` above is: it's a
    //! private free function, invisible to an integration-test crate,
    //! and the process-exit paths wrapping it can't be exercised without
    //! killing the test binary. `framework/tests/queue_pause.rs` proves
    //! everything reachable through the public `Queue` / worker API;
    //! this module proves the CLI-argument policy in front of it.
    use super::{PauseTarget, resolve_pause_target};

    #[test]
    fn all_flag_wins_regardless_of_queue() {
        assert_eq!(resolve_pause_target(None, true), Ok(PauseTarget::All));
        assert_eq!(
            resolve_pause_target(Some("billing".to_string()), true),
            Ok(PauseTarget::All)
        );
    }

    #[test]
    fn a_named_queue_is_accepted() {
        assert_eq!(
            resolve_pause_target(Some("billing".to_string()), false),
            Ok(PauseTarget::Named("billing".to_string()))
        );
    }

    #[test]
    fn neither_a_queue_nor_all_is_an_error() {
        let err =
            resolve_pause_target(None, false).expect_err("no queue and no --all must be refused");
        assert!(
            err.contains("--all"),
            "the message names the escape hatch: {err}"
        );
    }

    #[test]
    fn a_blank_queue_name_is_treated_as_no_queue() {
        let err = resolve_pause_target(Some("   ".to_string()), false)
            .expect_err("a blank queue name must be refused, matching Laravel's falsy check");
        assert!(err.contains("--all"));
    }
}

#[cfg(test)]
mod queue_command_line_tests {
    //! The queue commands as an operator types them. The failed-job
    //! commands and `--connection` are read by `clap` from the definitions
    //! above, so a renamed flag or a lost argument shows here.

    use super::{Cli, Commands};
    use clap::Parser;

    fn parse(args: &[&str]) -> Commands {
        Cli::try_parse_from(std::iter::once("app").chain(args.iter().copied()))
            .unwrap_or_else(|e| panic!("{args:?} should parse: {e}"))
            .command
            .expect("a subcommand")
    }

    #[test]
    fn queue_work_takes_a_connection_and_defaults_to_none() {
        match parse(&["queue:work", "--connection", "durable", "--queue=a,b"]) {
            Commands::QueueWork {
                connection, queues, ..
            } => {
                assert_eq!(connection.as_deref(), Some("durable"));
                assert_eq!(queues, ["a", "b"]);
            }
            _ => panic!("queue:work did not parse as the worker command"),
        }
        match parse(&["queue:work"]) {
            Commands::QueueWork { connection, .. } => assert_eq!(connection, None),
            _ => panic!("queue:work did not parse as the worker command"),
        }
    }

    #[test]
    fn queue_pause_and_resume_take_a_connection() {
        match parse(&["queue:pause", "billing", "--connection", "durable"]) {
            Commands::QueuePause {
                queue,
                all,
                connection,
            } => {
                assert_eq!(queue.as_deref(), Some("billing"));
                assert!(!all);
                assert_eq!(connection.as_deref(), Some("durable"));
            }
            _ => panic!("queue:pause did not parse as the pause command"),
        }
        match parse(&["queue:continue", "billing"]) {
            Commands::QueueResume { connection, .. } => assert_eq!(connection, None),
            _ => panic!("queue:continue did not parse as the resume command"),
        }
    }

    #[test]
    fn the_failed_job_commands_parse_their_arguments() {
        assert!(matches!(parse(&["queue:failed"]), Commands::QueueFailed));
        match parse(&["queue:retry", "a", "b"]) {
            Commands::QueueRetry { ids } => assert_eq!(ids, ["a", "b"]),
            _ => panic!("queue:retry did not parse as the retry command"),
        }
        assert!(
            Cli::try_parse_from(["app", "queue:retry"]).is_err(),
            "queue:retry with nothing to retry must be refused"
        );
        match parse(&["queue:forget", "a"]) {
            Commands::QueueForget { id } => assert_eq!(id, "a"),
            _ => panic!("queue:forget did not parse as the forget command"),
        }
        match parse(&["queue:flush", "--hours", "48"]) {
            Commands::QueueFlush { hours } => assert_eq!(hours, Some(48)),
            _ => panic!("queue:flush did not parse as the flush command"),
        }
        match parse(&["queue:prune-failed"]) {
            Commands::QueuePruneFailed { hours } => assert_eq!(hours, 24),
            _ => panic!("queue:prune-failed did not parse as the prune command"),
        }
    }
}

#[cfg(test)]
mod worker_queue_status_tests {
    //! `format_worker_queue_status` is what the `queue:work` listener prints
    //! when the worker observes a queue pausing or resuming. Tested here for
    //! the reason `queue_pause_target_tests` above documents: the listener
    //! itself writes to stdout from inside a long-lived daemon, so the shape
    //! of the line is the only part worth pinning, and it is a private free
    //! function invisible to an integration-test crate.
    use super::format_worker_queue_status;
    use chrono::TimeZone;

    fn at() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc
            .with_ymd_and_hms(2026, 8, 25, 14, 3, 11)
            .single()
            .expect("fixed clock")
    }

    #[test]
    fn a_named_queue_is_reported_by_name() {
        assert_eq!(
            format_worker_queue_status(Some("billing"), true, at()),
            "  2026-08-25 14:03:11 Queue billing PAUSED"
        );
        assert_eq!(
            format_worker_queue_status(Some("billing"), false, at()),
            "  2026-08-25 14:03:11 Queue billing RESUMED"
        );
    }

    #[test]
    fn an_unfiltered_worker_reports_every_queue() {
        assert_eq!(
            format_worker_queue_status(None, true, at()),
            "  2026-08-25 14:03:11 All queues PAUSED",
            "an unfiltered worker has no queue name, and must not invent one"
        );
    }
}

#[cfg(test)]
mod tests {
    //! End-to-end coverage for the `schedule:list` / `schedule:run` /
    //! `schedule:work` integration points. The free helpers
    //! [`build_schedule`], [`format_schedule_listing`], and
    //! [`evaluate_due_once`] are the exact code the three CLI subcommand
    //! handlers delegate to, so exercising them here proves the
    //! `Application::schedule(f)` registration flow reaches the user's
    //! `schedule_fn` and produces the same artefacts the binary would emit.
    use super::*;
    use crate::error::FrameworkError;
    use crate::schedule::{Schedule, Task, TaskResult};
    use async_trait::async_trait;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn build_schedule_with_none_returns_empty_schedule() {
        let schedule = build_schedule(None);
        assert!(
            schedule.is_empty(),
            "no schedule_fn should produce an empty Schedule",
        );
    }

    #[test]
    fn build_schedule_runs_user_fn_against_fresh_schedule() {
        let f: ScheduleFn = Box::new(|sched: &mut Schedule| {
            let b = sched.call(|| async { Ok(()) }).every_minute().name("a");
            sched.add(b);
            let b = sched.call(|| async { Ok(()) }).hourly().name("b");
            sched.add(b);
        });
        let schedule = build_schedule(Some(f));
        assert_eq!(schedule.len(), 2);
        assert!(schedule.find("a").is_some());
        assert!(schedule.find("b").is_some());
    }

    /// Fixed clock for the listing tests. Any instant works; pinning one
    /// keeps the rendered `next:` column reproducible.
    fn listing_clock() -> chrono::DateTime<chrono::Utc> {
        use chrono::TimeZone as _;
        chrono::Utc
            .with_ymd_and_hms(2026, 5, 28, 12, 0, 0)
            .single()
            .expect("test clock must be unambiguous")
    }

    #[test]
    fn format_schedule_listing_empty_includes_registration_hint() {
        let schedule = build_schedule(None);
        let out = format_schedule_listing(&schedule, chrono_tz::Tz::UTC, listing_clock());
        assert!(
            out.contains("No scheduled tasks registered."),
            "empty listing should announce no tasks: {out:?}",
        );
        assert!(
            out.contains("Application::schedule(schedule::register)"),
            "empty listing should suggest the registration call: {out:?}",
        );
    }

    #[test]
    fn format_schedule_listing_renders_name_expression_and_description() {
        let f: ScheduleFn = Box::new(|sched: &mut Schedule| {
            let b = sched
                .call(|| async { Ok(()) })
                .every_minute()
                .name("nightly-cleanup")
                .description("Remove stale upload temp files");
            sched.add(b);
            let b = sched
                .call(|| async { Ok(()) })
                .hourly()
                .name("plain-hourly");
            sched.add(b);
        });
        let schedule = build_schedule(Some(f));
        let out = format_schedule_listing(&schedule, chrono_tz::Tz::UTC, listing_clock());
        assert!(out.starts_with("Registered scheduled tasks:\n"));
        assert!(out.contains("nightly-cleanup"));
        assert!(out.contains("[* * * * *]"));
        assert!(out.contains(" - Remove stale upload temp files"));
        assert!(out.contains("plain-hourly"));
        assert!(out.contains("[0 * * * *]"));
        // No task pinned a zone, so no zone label is printed - but the
        // next-run column is unconditional.
        assert!(
            !out.contains('('),
            "an unpinned task must not carry a zone label: {out:?}",
        );
        assert_eq!(
            out.matches("next: ").count(),
            2,
            "every listed task gets a next-run column: {out:?}",
        );
    }

    /// A pinned zone changes three things at once: the expression is
    /// rewritten into the display zone, a zone label appears, and the
    /// next-run column is the *same instant* rendered in the display zone.
    #[test]
    fn format_schedule_listing_converts_and_labels_a_pinned_timezone() {
        let f: ScheduleFn = Box::new(|sched: &mut Schedule| {
            let b = sched
                .call(|| async { Ok(()) })
                .cron("0 3 * * *")
                .name("tokyo-report")
                .timezone(chrono_tz::Asia::Tokyo);
            sched.add(b);
        });
        let schedule = build_schedule(Some(f));
        let out = format_schedule_listing(&schedule, chrono_tz::Tz::UTC, listing_clock());
        // 03:00 Tokyo is 18:00 UTC the day before; the clock is
        // 2026-05-28 12:00 UTC = 2026-05-28 21:00 JST, so the next 03:00
        // JST is 2026-05-29, which is 2026-05-28 18:00 UTC.
        assert_eq!(
            out,
            "Registered scheduled tasks:\n  \
             tokyo-report [0 18 * * *] (UTC) next: 2026-05-28 18:00 UTC\n",
        );
    }

    /// A conversion the algorithm refuses prints the expression the user
    /// wrote, labelled with the task's own zone rather than the display
    /// zone - the label always names the zone the printed fields are in.
    #[test]
    fn format_schedule_listing_labels_a_refused_conversion_with_the_task_zone() {
        let f: ScheduleFn = Box::new(|sched: &mut Schedule| {
            // 03:00 Tokyo is 18:00 UTC the *previous* day, and that day
            // roll would have to move a restricted day-of-month and a
            // restricted day-of-week at once, which cron ORs rather than
            // ANDs. Refuse.
            let b = sched
                .call(|| async { Ok(()) })
                .cron("0 3 1 * 1")
                .name("month-start-monday")
                .timezone(chrono_tz::Asia::Tokyo);
            sched.add(b);
        });
        let schedule = build_schedule(Some(f));
        let out = format_schedule_listing(&schedule, chrono_tz::Tz::UTC, listing_clock());
        assert!(
            out.contains("[0 3 1 * 1] (Asia/Tokyo)"),
            "a refused conversion keeps the raw expression and its own zone: {out:?}",
        );
    }

    /// One event, several lines: an expression that straddles midnight in
    /// the display zone needs one cron line per side (Laravel's `flatMap`).
    #[test]
    fn format_schedule_listing_renders_one_line_per_converted_expression() {
        let f: ScheduleFn = Box::new(|sched: &mut Schedule| {
            let b = sched
                .call(|| async { Ok(()) })
                .cron("0 14,20 * * 1")
                .name("monday-twice")
                .timezone(chrono_tz::Tz::UTC);
            sched.add(b);
        });
        let schedule = build_schedule(Some(f));
        let out = format_schedule_listing(&schedule, chrono_tz::Asia::Tokyo, listing_clock());
        // The clock is Thursday 2026-05-28; the next Monday 14:00 UTC is
        // 2026-06-01, which is 23:00 JST the same day. The next-run column
        // belongs to the task, so it repeats on both lines.
        assert_eq!(
            out,
            "Registered scheduled tasks:\n  \
             monday-twice [0 23 * * 1] (Asia/Tokyo) next: 2026-06-01 23:00 JST\n  \
             monday-twice [0 5 * * 2] (Asia/Tokyo) next: 2026-06-01 23:00 JST\n",
        );
    }

    /// An expression that never fires renders `never` rather than a
    /// fabricated date.
    #[test]
    fn format_schedule_listing_reports_an_unsatisfiable_expression_as_never() {
        let f: ScheduleFn = Box::new(|sched: &mut Schedule| {
            let b = sched
                .call(|| async { Ok(()) })
                .cron("0 0 30 2 *")
                .name("never-runs")
                .timezone(chrono_tz::Tz::UTC);
            sched.add(b);
        });
        let schedule = build_schedule(Some(f));
        let out = format_schedule_listing(&schedule, chrono_tz::Tz::UTC, listing_clock());
        assert!(out.contains("next: never"), "{out:?}");
    }

    /// A February 29 task is *matchable* - it runs every four years - so it
    /// must show a real date and a converted expression.
    ///
    /// This is the end-to-end shape of the one-year-scan-bound defect: the
    /// next leap day is more than a year past the clock, so `next_run_after`
    /// returned `None`; that made the listing print `next: never` for a task
    /// that genuinely runs, and because the converter needs two real run
    /// instants to sample zone offsets from, the missing `next` also
    /// short-circuited it into refusing the (perfectly convertible)
    /// expression and printing it raw with the wrong zone label.
    #[test]
    fn format_schedule_listing_converts_a_february_29_task_years_ahead() {
        let f: ScheduleFn = Box::new(|sched: &mut Schedule| {
            let b = sched
                .call(|| async { Ok(()) })
                .cron("0 3 29 2 *")
                .name("leap-day-audit")
                .timezone(chrono_tz::Tz::UTC);
            sched.add(b);
        });
        let schedule = build_schedule(Some(f));
        let out = format_schedule_listing(&schedule, chrono_tz::Asia::Tokyo, listing_clock());
        // 03:00 UTC is 12:00 JST the same day - no day carry, so the Feb 29
        // refusal is never reached. The clock is 2026-05-28, so the next
        // leap day is 2028-02-29, roughly 641 days out.
        assert_eq!(
            out,
            "Registered scheduled tasks:\n  \
             leap-day-audit [0 12 29 2 *] (Asia/Tokyo) next: 2028-02-29 12:00 JST\n",
        );
    }

    #[test]
    fn resolve_display_timezone_defaults_to_utc_and_rejects_unknown_zones() {
        assert_eq!(resolve_display_timezone(None), Ok(chrono_tz::Tz::UTC));
        assert_eq!(resolve_display_timezone(Some("  ")), Ok(chrono_tz::Tz::UTC));
        assert_eq!(
            resolve_display_timezone(Some("Asia/Tokyo")),
            Ok(chrono_tz::Asia::Tokyo)
        );
        let err = resolve_display_timezone(Some("Mars/Olympus_Mons"))
            .expect_err("an unknown zone must be refused");
        assert!(
            err.contains("Mars/Olympus_Mons"),
            "the message names the rejected zone: {err}",
        );
    }

    /// `evaluate_due_once` is what `schedule:run` delegates to. The handler
    /// uses the returned `any_failed` flag to choose its process exit code; a
    /// test that asserts the flag covers the success-path contract end-to-end
    /// without spawning a child process.
    #[tokio::test]
    async fn evaluate_due_once_executes_due_tasks_and_marks_success() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = Arc::clone(&calls);
        let f: ScheduleFn = Box::new(move |sched: &mut Schedule| {
            let counter = Arc::clone(&calls_clone);
            let b = sched
                .call(move || {
                    let counter = Arc::clone(&counter);
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    }
                })
                .every_minute()
                .name("ok-task");
            sched.add(b);
        });
        let schedule = build_schedule(Some(f));
        let (results, any_failed) = evaluate_due_once(&schedule).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "ok-task");
        assert!(results[0].1.is_ok());
        assert!(
            !any_failed,
            "no failed tasks should report any_failed=false"
        );
    }

    #[tokio::test]
    async fn evaluate_due_once_reports_failure_via_any_failed_flag() {
        let f: ScheduleFn = Box::new(|sched: &mut Schedule| {
            let b = sched
                .call(|| async { Err(FrameworkError::internal("boom")) })
                .every_minute()
                .name("boom-task");
            sched.add(b);
            let b = sched
                .call(|| async { Ok(()) })
                .every_minute()
                .name("ok-task");
            sched.add(b);
        });
        let schedule = build_schedule(Some(f));
        let (results, any_failed) = evaluate_due_once(&schedule).await;
        assert_eq!(results.len(), 2);
        assert!(any_failed, "a failing task must flip any_failed");
        let by_name: std::collections::BTreeMap<_, _> = results
            .iter()
            .map(|(n, r)| (n.as_str(), r.is_err()))
            .collect();
        assert_eq!(by_name.get("boom-task"), Some(&true));
        assert_eq!(by_name.get("ok-task"), Some(&false));
    }

    #[tokio::test]
    async fn evaluate_due_once_with_empty_schedule_returns_empty_results() {
        let schedule = build_schedule(None);
        let (results, any_failed) = evaluate_due_once(&schedule).await;
        assert!(results.is_empty());
        assert!(!any_failed);
    }

    /// Trait-based tasks must reach the same registration / listing /
    /// evaluation pipeline as closure-based ones - proves
    /// `Schedule::task(T)` and `Schedule::call(|| ...)` both round-trip
    /// through the CLI helpers.
    #[tokio::test]
    async fn application_pipeline_handles_trait_based_tasks() {
        struct CleanupTask {
            ran: Arc<AtomicUsize>,
        }

        #[async_trait]
        impl Task for CleanupTask {
            async fn handle(&self) -> TaskResult {
                self.ran.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }

        let ran = Arc::new(AtomicUsize::new(0));
        let ran_clone = Arc::clone(&ran);
        let f: ScheduleFn = Box::new(move |sched: &mut Schedule| {
            let task = CleanupTask { ran: ran_clone };
            let b = sched.task(task).every_minute().name("cleanup");
            sched.add(b);
        });
        let schedule = build_schedule(Some(f));
        let listing = format_schedule_listing(&schedule, chrono_tz::Tz::UTC, listing_clock());
        assert!(listing.contains("cleanup"));
        assert!(listing.contains("[* * * * *]"));

        let (results, any_failed) = evaluate_due_once(&schedule).await;
        assert_eq!(results.len(), 1);
        assert!(!any_failed);
        assert_eq!(ran.load(Ordering::SeqCst), 1);
    }

    /// `schedule:run` semantics: a `.run_in_background()` task must still
    /// surface in the returned `(results, any_failed)` tuple, so the
    /// handler reports success/failure consistently regardless of how the
    /// task was executed.
    #[tokio::test]
    async fn evaluate_due_once_drains_background_tasks_before_returning() {
        let f: ScheduleFn = Box::new(|sched: &mut Schedule| {
            let b = sched
                .call(|| async { Ok(()) })
                .every_minute()
                .name("inline-ok");
            sched.add(b);
            let b = sched
                .call(|| async { Ok(()) })
                .every_minute()
                .name("bg-ok")
                .run_in_background();
            sched.add(b);
            let b = sched
                .call(|| async { Err(FrameworkError::internal("bg-failure")) })
                .every_minute()
                .name("bg-err")
                .run_in_background();
            sched.add(b);
        });
        let schedule = build_schedule(Some(f));
        let (results, any_failed) = evaluate_due_once(&schedule).await;
        assert_eq!(results.len(), 3);
        assert!(any_failed, "a failing background task must flip any_failed");

        let by_name: std::collections::BTreeMap<_, _> = results
            .iter()
            .map(|(n, r)| (n.as_str(), r.is_ok()))
            .collect();
        assert_eq!(by_name.get("inline-ok"), Some(&true));
        assert_eq!(by_name.get("bg-ok"), Some(&true));
        assert_eq!(by_name.get("bg-err"), Some(&false));
    }

    /// `Application::new().run()` defaults to `NoMigrator`, whose
    /// `migrations()` returns an empty vec. The default `serve`/`web:run`
    /// arm calls `run_migrations_silent::<M>()` before booting the server;
    /// a framework app without a database must boot without `DATABASE_URL`
    /// being set.
    ///
    /// Without the empty-migrations short-circuit in `run_migrations_silent`,
    /// `get_database_connection()` calls `std::process::exit(1)` when the
    /// env var is missing - that would terminate the entire test binary,
    /// not just fail this single test, so a passing run is itself the
    /// regression signal.
    ///
    /// The `remove_var` is load-bearing: if the ambient environment has
    /// `DATABASE_URL` set, the unfixed path would skip the exit and
    /// silently succeed, making the test green against the bug. We gate
    /// with `#[serial_test::serial]` because the env is process-wide.
    #[tokio::test]
    #[serial_test::serial]
    async fn no_migrator_default_serve_does_not_require_database_url() {
        let prior = env::var("DATABASE_URL").ok();
        // SAFETY: edition 2024 marks env mutation `unsafe`; we serialize
        // with `#[serial_test::serial]` so no concurrent test reads it,
        // and we restore the prior value before returning.
        unsafe {
            env::remove_var("DATABASE_URL");
        }

        // With the fix in place this call returns immediately because
        // `NoMigrator::migrations()` is empty; without the fix this would
        // terminate the test binary via `std::process::exit(1)` inside
        // `get_database_connection`.
        Application::<NoMigrator>::run_migrations_silent::<NoMigrator>().await;

        // SAFETY: same justification as above.
        unsafe {
            if let Some(prior) = prior {
                env::set_var("DATABASE_URL", prior);
            }
        }
    }

    /// `SUPRNOVA_AUTO_MIGRATE_BEST_EFFORT` parsing: unset, empty, and
    /// non-truthy values must keep the production-safe fail-closed default.
    #[test]
    fn parse_auto_migrate_best_effort_defaults_to_false() {
        assert!(!parse_auto_migrate_best_effort(None));
        assert!(!parse_auto_migrate_best_effort(Some("")));
        assert!(!parse_auto_migrate_best_effort(Some("   ")));
        assert!(!parse_auto_migrate_best_effort(Some("false")));
        assert!(!parse_auto_migrate_best_effort(Some("0")));
        assert!(!parse_auto_migrate_best_effort(Some("no")));
        assert!(!parse_auto_migrate_best_effort(Some("off")));
    }

    /// The full truthy alphabet: `true` / `1` / `yes` / `on`, mixed case,
    /// trimmed of surrounding whitespace.
    #[test]
    fn parse_auto_migrate_best_effort_accepts_truthy_values() {
        for v in [
            "true", "TRUE", "True", "  true  ", "1", " 1 ", "yes", "YES", "on", "On",
        ] {
            assert!(
                parse_auto_migrate_best_effort(Some(v)),
                "{v:?} should enable best-effort mode",
            );
        }
    }

    /// Success outcomes pass through both modes; this pins the contract so
    /// future refactors can't accidentally swap the arms.
    #[test]
    fn resolve_auto_migration_passes_success_through() {
        assert!(resolve_auto_migration(Ok(()), false).is_ok());
        assert!(resolve_auto_migration(Ok(()), true).is_ok());
    }

    /// Regression for `app-serve-fails-open`: with the default
    /// (best_effort=false), a migration error must propagate so the caller
    /// (`run_migrations_silent`) can `exit(1)` instead of booting the server
    /// against a half-migrated schema.
    #[test]
    fn resolve_auto_migration_default_fails_closed_on_error() {
        let err = sea_orm::DbErr::Migration("create_users_table: column already exists".into());
        let outcome = resolve_auto_migration(Err(err), false);
        assert!(
            outcome.is_err(),
            "default mode must surface the migration error so the server aborts",
        );
    }

    /// Best-effort opt-in (the SUPRNOVA_AUTO_MIGRATE_BEST_EFFORT=true escape
    /// hatch) preserves the legacy log-and-continue behaviour for operators
    /// who explicitly want it.
    #[test]
    fn resolve_auto_migration_best_effort_swallows_error() {
        let err = sea_orm::DbErr::Migration("create_users_table: column already exists".into());
        let outcome = resolve_auto_migration(Err(err), true);
        assert!(
            outcome.is_ok(),
            "best-effort mode must swallow the migration error so the server still boots",
        );
    }

    /// End-to-end variant of the fix: route a real `Migrator::up` failure
    /// through the same policy gate `run_migrations_silent` uses. Uses a
    /// migrator whose first migration deliberately fails so the result is a
    /// real `DbErr`, not a hand-rolled one. Connects to `sqlite::memory:`
    /// directly to avoid `get_database_connection`'s `exit(1)` on missing
    /// `DATABASE_URL`.
    #[tokio::test]
    async fn resolve_auto_migration_routes_real_migrator_failure() {
        struct FailingMigration;

        impl MigrationName for FailingMigration {
            fn name(&self) -> &str {
                "m_app_serve_fails_open_regression_failing_migration"
            }
        }

        #[async_trait]
        impl MigrationTrait for FailingMigration {
            async fn up(&self, _manager: &SchemaManager) -> Result<(), sea_orm::DbErr> {
                Err(sea_orm::DbErr::Migration(
                    "intentional failure for app-serve-fails-open regression test".into(),
                ))
            }

            async fn down(&self, _manager: &SchemaManager) -> Result<(), sea_orm::DbErr> {
                Ok(())
            }
        }

        struct FailingMigrator;

        #[async_trait]
        impl MigratorTrait for FailingMigrator {
            fn migrations() -> Vec<Box<dyn MigrationTrait>> {
                vec![Box::new(FailingMigration)]
            }
        }

        let db = sea_orm::Database::connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should connect");

        // Default mode: server boot would abort.
        let outcome = FailingMigrator::up(&db, None).await;
        assert!(
            resolve_auto_migration(outcome, false).is_err(),
            "default fail-closed mode must propagate a real Migrator::up failure",
        );

        // Best-effort opt-in: same error, swallowed.
        let outcome = FailingMigrator::up(&db, None).await;
        assert!(
            resolve_auto_migration(outcome, true).is_ok(),
            "best-effort opt-in must swallow a real Migrator::up failure",
        );
    }

    /// The scheduler's shutdown drain used to await every background task with
    /// no deadline, so one task that never returns held the process open
    /// until SIGKILL. It must now give up and abort.
    #[tokio::test]
    async fn the_drain_abandons_tasks_that_outlive_the_grace() {
        let mut tasks: tokio::task::JoinSet<crate::schedule::ScheduledTaskJoin> =
            tokio::task::JoinSet::new();
        tasks.spawn(async { ("quick".to_string(), Ok(())) });
        tasks.spawn(async {
            std::future::pending::<()>().await;
            unreachable!()
        });

        let started = std::time::Instant::now();
        let abandoned = drain_with_grace(&mut tasks, Duration::from_millis(150)).await;

        assert_eq!(abandoned, 1, "the hung task must be reported as abandoned");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the drain must return at its deadline, not wait for the hung task"
        );
    }

    /// The ordinary path: everything finishes inside the grace, nothing is
    /// abandoned, and the drain does not sit until the deadline.
    #[tokio::test]
    async fn the_drain_returns_as_soon_as_every_task_finishes() {
        let mut tasks: tokio::task::JoinSet<crate::schedule::ScheduledTaskJoin> =
            tokio::task::JoinSet::new();
        for i in 0..4 {
            tasks.spawn(async move { (format!("task-{i}"), Ok(())) });
        }

        let started = std::time::Instant::now();
        let abandoned = drain_with_grace(&mut tasks, Duration::from_secs(30)).await;

        assert_eq!(abandoned, 0);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the drain must not wait out the full grace when the set is empty"
        );
    }

    /// An empty set is the common case - most shutdowns have no background
    /// work in flight - and must not cost a scheduler timer at all.
    #[tokio::test]
    async fn draining_an_empty_set_is_free() {
        let mut tasks: tokio::task::JoinSet<crate::schedule::ScheduledTaskJoin> =
            tokio::task::JoinSet::new();
        assert_eq!(
            drain_with_grace(&mut tasks, Duration::from_secs(30)).await,
            0
        );
    }
}

#[cfg(test)]
mod boot_hook_tests {
    //! The serve path runs the process-wide hook then the HTTP hook, in
    //! that order; worker paths compile against a signature that cannot
    //! receive the HTTP hook at all. These tests pin the runtime half of
    //! that contract - ordering and None-tolerance - because a regression
    //! here strands either the middleware chain (HTTP hook skipped) or
    //! everything that assumes `DB::init` ran first (order flipped),
    //! without any compile error.
    use super::*;
    use std::sync::{Arc, Mutex};

    fn recording(order: &Arc<Mutex<Vec<&'static str>>>, tag: &'static str) -> BootstrapFn {
        let order = Arc::clone(order);
        Box::new(move || {
            Box::pin(async move {
                order.lock().expect("order mutex").push(tag);
            })
        })
    }

    #[tokio::test]
    async fn server_boot_runs_process_wide_then_http_in_order() {
        let order = Arc::new(Mutex::new(Vec::new()));
        Application::<NoMigrator>::run_boot_hooks(
            Some(recording(&order, "process-wide")),
            Some(recording(&order, "http")),
        )
        .await;
        assert_eq!(
            *order.lock().expect("order mutex"),
            vec!["process-wide", "http"],
            "HTTP boot must be able to assume process-wide boot already ran"
        );
    }

    #[tokio::test]
    async fn missing_http_hook_boots_like_before_the_split() {
        let order = Arc::new(Mutex::new(Vec::new()));
        Application::<NoMigrator>::run_boot_hooks(Some(recording(&order, "process-wide")), None)
            .await;
        assert_eq!(*order.lock().expect("order mutex"), vec!["process-wide"]);
    }

    #[tokio::test]
    async fn http_hook_alone_still_runs() {
        let order = Arc::new(Mutex::new(Vec::new()));
        Application::<NoMigrator>::run_boot_hooks(None, Some(recording(&order, "http"))).await;
        assert_eq!(*order.lock().expect("order mutex"), vec!["http"]);
    }
}

#[cfg(test)]
mod route_registration_tests {
    //! `routes`, `try_routes` and `try_routes_async` write one slot, and
    //! `RoutesFn::build_router` runs whichever closure is in it. A
    //! regression that gave the asynchronous form a field of its own would
    //! leave `suprnova serve` quietly building the synchronous catalog the
    //! application meant to replace: no compile error, no missing route,
    //! only a middleware that was never installed.
    //!
    //! These tests drive `build_router` and nothing else. They boot no
    //! services, bind no Live runtime, and read no environment, so they
    //! cannot be broken by what another test in this binary left in the
    //! process-global container - which is exactly how an earlier version
    //! of this module, which called `prepare_server`, failed under the
    //! repository gate while passing when run alone. `prepare_server` is
    //! `build_router` handed to `Server::try_from_config_with_routes_async`,
    //! and the boot order that wrapper owes is proven where a real
    //! container is legitimate: `framework/tests/live_boot.rs`.
    use super::*;
    use std::sync::{Arc, Mutex};

    fn recorder() -> Arc<Mutex<Vec<&'static str>>> {
        Arc::new(Mutex::new(Vec::new()))
    }

    #[tokio::test]
    async fn the_last_registered_route_builder_wins_across_both_forms() {
        let ran = recorder();

        let sync_first = Arc::clone(&ran);
        let async_second = Arc::clone(&ran);
        let async_last = Application::new()
            .try_routes(move || {
                sync_first.lock().expect("recorder").push("sync");
                Ok(Router::new())
            })
            .try_routes_async(move || {
                let recorded = Arc::clone(&async_second);
                async move {
                    recorded.lock().expect("recorder").push("async");
                    Ok(Router::new())
                }
            });
        RoutesFn::build_router(async_last.routes_fn)
            .await
            .expect("the asynchronous catalog must build a router");
        assert_eq!(
            *ran.lock().expect("recorder"),
            vec!["async"],
            "try_routes_async must replace an earlier try_routes, not run beside it"
        );

        ran.lock().expect("recorder").clear();

        let async_first = Arc::clone(&ran);
        let sync_second = Arc::clone(&ran);
        let sync_last = Application::new()
            .try_routes_async(move || {
                let recorded = Arc::clone(&async_first);
                async move {
                    recorded.lock().expect("recorder").push("async");
                    Ok(Router::new())
                }
            })
            .try_routes(move || {
                sync_second.lock().expect("recorder").push("sync");
                Ok(Router::new())
            });
        RoutesFn::build_router(sync_last.routes_fn)
            .await
            .expect("the synchronous catalog must build a router");
        assert_eq!(
            *ran.lock().expect("recorder"),
            vec!["sync"],
            "the slot is shared in both directions: a later try_routes must replace \
             an earlier try_routes_async"
        );
    }

    /// The infallible builder shares the slot too. `routes` is the oldest
    /// of the three and the one an existing application is most likely to
    /// be holding when it adds `try_routes_async`, so its half of the
    /// last-wins rule is worth its own test rather than an inference from
    /// the fallible one.
    #[tokio::test]
    async fn the_infallible_routes_builder_shares_the_slot_with_the_async_one() {
        let ran = recorder();

        let infallible_first = Arc::clone(&ran);
        let async_second = Arc::clone(&ran);
        let async_last = Application::new()
            .routes(move || {
                infallible_first.lock().expect("recorder").push("routes");
                Router::new()
            })
            .try_routes_async(move || {
                let recorded = Arc::clone(&async_second);
                async move {
                    recorded.lock().expect("recorder").push("async");
                    Ok(Router::new())
                }
            });
        RoutesFn::build_router(async_last.routes_fn)
            .await
            .expect("the asynchronous catalog must build a router");
        assert_eq!(
            *ran.lock().expect("recorder"),
            vec!["async"],
            "try_routes_async must replace an earlier infallible routes builder"
        );

        ran.lock().expect("recorder").clear();

        let async_first = Arc::clone(&ran);
        let infallible_second = Arc::clone(&ran);
        let infallible_last = Application::new()
            .try_routes_async(move || {
                let recorded = Arc::clone(&async_first);
                async move {
                    recorded.lock().expect("recorder").push("async");
                    Ok(Router::new())
                }
            })
            .routes(move || {
                infallible_second.lock().expect("recorder").push("routes");
                Router::new()
            });
        RoutesFn::build_router(infallible_last.routes_fn)
            .await
            .expect("the infallible catalog must build a router");
        assert_eq!(
            *ran.lock().expect("recorder"),
            vec!["routes"],
            "a later infallible routes builder must replace an earlier try_routes_async"
        );
    }

    /// The router each closure returns is the router the dispatch hands
    /// back, whichever shape registered it. This is the unit-level half of
    /// "the server serves the router the closure returned"; the other half,
    /// that the wrapper carries this router into the assembled server, is
    /// `finish_boot`, shared with the synchronous constructor and covered
    /// in `framework/tests/live_boot.rs`.
    #[tokio::test]
    async fn each_form_hands_back_the_router_its_own_closure_built() {
        let from_async = RoutesFn::build_router(
            Application::new()
                .try_routes_async(|| async {
                    Ok(Router::new()
                        .get("/async-built", |_req| async { crate::http::text("async") })
                        .into())
                })
                .routes_fn,
        )
        .await
        .expect("the asynchronous catalog must build a router");
        assert!(
            from_async
                .match_route(&hyper::Method::GET, "/async-built")
                .is_some(),
            "the awaited closure's own router must come back, not a fresh one"
        );

        let from_sync = RoutesFn::build_router(
            Application::new()
                .try_routes(|| {
                    Ok(Router::new()
                        .get("/sync-built", |_req| async { crate::http::text("sync") })
                        .into())
                })
                .routes_fn,
        )
        .await
        .expect("the synchronous catalog must build a router");
        assert!(
            from_sync
                .match_route(&hyper::Method::GET, "/sync-built")
                .is_some(),
            "the synchronous closure's own router must come back unchanged"
        );
        assert!(
            from_sync
                .match_route(&hyper::Method::GET, "/async-built")
                .is_none(),
            "sanity: the two dispatches must not share a router"
        );
    }

    #[tokio::test]
    async fn an_error_from_the_async_route_closure_surfaces_like_the_sync_one() {
        let from_sync = RoutesFn::build_router(
            Application::new()
                .try_routes(|| Err(FrameworkError::internal("route catalog rejected")))
                .routes_fn,
        )
        .await;
        let from_async = RoutesFn::build_router(
            Application::new()
                .try_routes_async(|| async {
                    Err(FrameworkError::internal("route catalog rejected"))
                })
                .routes_fn,
        )
        .await;

        let sync_error = match from_sync {
            Ok(_) => panic!("a synchronous route-construction error must abort the dispatch"),
            Err(error) => error,
        };
        let async_error = match from_async {
            Ok(_) => panic!("an asynchronous route-construction error must abort the dispatch"),
            Err(error) => error,
        };
        assert!(
            async_error.to_string().contains("route catalog rejected"),
            "the closure's own error must survive: {async_error}"
        );
        assert_eq!(
            async_error.to_string(),
            sync_error.to_string(),
            "an awaited catalog must fail with exactly what the synchronous one fails with"
        );
        assert_eq!(async_error.status_code(), sync_error.status_code());
    }

    #[tokio::test]
    async fn registering_no_route_builder_still_builds_an_empty_router() {
        let empty = RoutesFn::build_router(Application::new().routes_fn)
            .await
            .expect("an application that registers no routes must still build one");
        assert!(
            empty.match_route(&hyper::Method::GET, "/").is_none(),
            "the no-registration path must yield a bare Router::new()"
        );
    }
}
