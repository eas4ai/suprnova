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

use crate::{Router, Schedule, Server};
use clap::{Parser, Subcommand};
use sea_orm_migration::prelude::*;
use std::env;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::time::Duration;

pub mod maintenance;
pub mod paths;

/// Boxed async bootstrap function (avoids repeating the complex trait-object type).
type BootstrapFn = Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>;

/// Boxed callback run once after the server boots its services.
type BootedFn = Box<dyn FnOnce()>;

/// Boxed function that registers the application's scheduled tasks.
type ScheduleFn = Box<dyn FnOnce(&mut Schedule) + Send>;

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
    /// Run pending database migrations
    Migrate,
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
    },
    /// Run the scheduler daemon (checks every minute)
    #[command(name = "schedule:work")]
    ScheduleWork,
    /// Run all due scheduled tasks once
    #[command(name = "schedule:run")]
    ScheduleRun,
    /// List all registered scheduled tasks
    #[command(name = "schedule:list")]
    ScheduleList,
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
        #[arg(long = "queue", value_delimiter = ',')]
        queues: Vec<String>,
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

/// Application builder for suprnova framework
///
/// Use this to configure and run your suprnova application with a fluent API.
pub struct Application<M = NoMigrator>
where
    M: MigratorTrait,
{
    config_fn: Option<Box<dyn FnOnce()>>,
    bootstrap_fn: Option<BootstrapFn>,
    routes_fn: Option<Box<dyn FnOnce() -> Router + Send>>,
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

/// Render the `schedule:list` output for a built [`Schedule`].
///
/// Returns the exact string the handler would print to stdout, so callers can
/// either `print!("{}", …)` from a CLI handler or assert on it from a test
/// without capturing stdout. Trailing newline is included so the caller does
/// not have to worry about whether the schedule is empty.
pub(crate) fn format_schedule_listing(schedule: &Schedule) -> String {
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
        let expr = entry.expression.expression();
        let _ = match &entry.description {
            Some(desc) => writeln!(out, "  {} [{expr}] — {desc}", entry.name),
            None => writeln!(out, "  {} [{expr}]", entry.name),
        };
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
/// stripped). Everything else — including `false`, `0`, empty strings, and the
/// `None` returned by [`std::env::var`] when the variable is unset — yields
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
                "suprnova: WARNING — auto-migrate failed in best-effort mode, server will boot \
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
/// scheduled task is explicitly the long-running kind — a nightly report,
/// a batch export — and cutting it off in five seconds would abandon work
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
/// the caller proceeds to exit — the same defect the server's connection
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
    /// and other application components.
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
        self.routes_fn = Some(Box::new(f));
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
        // single-threaded — and by the time this async fn runs, every
        // Tokio worker thread already exists. See `crate::boot`.
        //
        // This is a hard refusal rather than a warning because the
        // failure it prevents is a silent data race, not a crash: an
        // app that boots "fine" under `#[tokio::main]` is exactly the
        // one that corrupts an env read on some unrelated thread weeks
        // later. A malformed `.env` still fails loudly, just earlier —
        // `load_env_or_exit` owns that message now.
        if let Err(message) = crate::boot::boot_precondition(crate::boot::env_loaded_pre_runtime())
        {
            eprintln!("{message}");
            std::process::exit(1);
        }

        // Register all #[policy] gates collected via inventory::submit!.
        // Called here (before the subcommand match) so background workers,
        // CLI commands, and scheduled tasks all see registered gates — not
        // only the web server path. The inner `Once` makes this idempotent.
        crate::authorization::init_policies();

        // Destructure self to avoid partial move issues
        let Application {
            config_fn,
            bootstrap_fn,
            routes_fn,
            schedule_fn,
            booted_fns,
            _migrator,
        } = self;

        // Run user's config registration
        if let Some(config_fn) = config_fn {
            config_fn();
        }

        match cli.command {
            None
            | Some(Commands::Serve { no_migrate: false })
            | Some(Commands::WebRun { no_migrate: false }) => {
                // Default: run server with auto-migrate
                Self::run_migrations_silent::<M>().await;
                Self::run_server_internal(bootstrap_fn, routes_fn, booted_fns).await;
            }
            Some(Commands::Serve { no_migrate: true })
            | Some(Commands::WebRun { no_migrate: true }) => {
                // Run server without migrations
                Self::run_server_internal(bootstrap_fn, routes_fn, booted_fns).await;
            }
            Some(Commands::Migrate) => {
                Self::run_migrations::<M>().await;
            }
            Some(Commands::MigrateStatus) => {
                Self::show_migration_status::<M>().await;
            }
            Some(Commands::MigrateRollback { steps }) => {
                Self::rollback_migrations::<M>(steps).await;
            }
            Some(Commands::MigrateFresh { force }) => {
                // The CLI's `suprnova migrate:fresh` gained this gate first,
                // but production deploys run migrations through *this*
                // binary, not the dev CLI — so without the same check here
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
                Self::fresh_migrations::<M>().await;
            }
            Some(Commands::ScheduleWork) => {
                Self::run_scheduler_daemon_internal(bootstrap_fn, schedule_fn).await;
            }
            Some(Commands::ScheduleRun) => {
                Self::run_scheduled_tasks_internal(bootstrap_fn, schedule_fn).await;
            }
            Some(Commands::ScheduleList) => {
                Self::list_scheduled_tasks(schedule_fn).await;
            }
            Some(Commands::WorkflowWork) => {
                Self::run_workflow_worker_internal(bootstrap_fn).await;
            }
            Some(Commands::QueueWork {
                visibility_timeout,
                poll_interval_ms,
                max_jobs,
                queues,
            }) => {
                Self::run_queue_worker_internal(
                    bootstrap_fn,
                    visibility_timeout,
                    poll_interval_ms,
                    max_jobs,
                    queues,
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
                Self::run_up().await;
            }
        }
    }

    async fn run_server_internal(
        bootstrap_fn: Option<BootstrapFn>,
        routes_fn: Option<Box<dyn FnOnce() -> Router + Send>>,
        booted_fns: Vec<BootedFn>,
    ) {
        // Run bootstrap
        if let Some(bootstrap_fn) = bootstrap_fn {
            bootstrap_fn().await;
        }

        // Get router
        let router = if let Some(routes_fn) = routes_fn {
            routes_fn()
        } else {
            Router::new()
        };

        // Create server with configuration from environment.
        //
        // `from_config` returns Err when APP_KEY is required (any
        // non-development environment) but unset or malformed. The
        // error type carries the user-facing remediation (it points at
        // `suprnova key:generate`); we surface it on stderr without a
        // panic stack-trace wrapper so production boot logs stay clean.
        let server = match Server::from_config(router) {
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
        let database_url = if database_url.starts_with("sqlite://") {
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
        };

        sea_orm::Database::connect(&database_url)
            .await
            .unwrap_or_else(|e| {
                eprintln!("suprnova: failed to connect to the database: {e}");
                std::process::exit(1);
            })
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
        let db = Self::get_database_connection().await;
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

    async fn run_migrations<Migrator: MigratorTrait>() {
        println!("Running migrations...");
        let db = Self::get_database_connection().await;
        if let Err(e) = Migrator::up(&db, None).await {
            eprintln!("suprnova: migration failed: {e}");
            std::process::exit(1);
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

    async fn fresh_migrations<Migrator: MigratorTrait>() {
        println!("WARNING: Dropping all tables and re-running migrations...");
        let db = Self::get_database_connection().await;
        if let Err(e) = Migrator::fresh(&db).await {
            eprintln!("suprnova: database refresh failed: {e}");
            std::process::exit(1);
        }
        println!("Database refreshed successfully!");
    }

    /// `schedule:work`: run the scheduler as a long-lived daemon.
    ///
    /// The first tick is aligned to the next minute boundary, then due tasks
    /// are evaluated once per minute (matching Laravel's per-minute cron
    /// evaluation). Runs the app's `bootstrap_fn` and then the runtime drivers
    /// (see [`Self::boot_worker_process`]) so tasks can resolve services;
    /// stops on Ctrl-C or SIGTERM.
    async fn run_scheduler_daemon_internal(
        bootstrap_fn: Option<BootstrapFn>,
        schedule_fn: Option<ScheduleFn>,
    ) {
        let shutdown = Self::start_daemon();
        if let Err(e) = Self::boot_worker_process(bootstrap_fn).await {
            eprintln!("suprnova: scheduler bootstrap error: {e}");
            std::process::exit(1);
        }
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
        // are fire-and-forget within a tick — the loop polls completed ones
        // before each tick and on shutdown awaits the rest before exit, so a
        // slow background task never gets dropped mid-flight.
        let mut bg_tasks: tokio::task::JoinSet<crate::schedule::ScheduledTaskJoin> =
            tokio::task::JoinSet::new();

        loop {
            tokio::select! {
                _ = tick.tick() => {
                    // Surface any background tasks that completed since the
                    // last tick. `try_join_next` is non-blocking — anything
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
        bootstrap_fn: Option<BootstrapFn>,
        schedule_fn: Option<ScheduleFn>,
    ) {
        // Logging only. `schedule:run` evaluates the due tasks once and
        // exits, so there is no long-lived loop for a stop signal to
        // interrupt — the default disposition is right for a one-shot
        // command, and installing a handler it never reads would only
        // make Ctrl-C stop working.
        Self::install_daemon_logging();
        if let Err(e) = Self::boot_worker_process(bootstrap_fn).await {
            eprintln!("suprnova: scheduler bootstrap error: {e}");
            std::process::exit(1);
        }
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
    async fn list_scheduled_tasks(schedule_fn: Option<ScheduleFn>) {
        let schedule = build_schedule(schedule_fn);
        print!("{}", format_schedule_listing(&schedule));
    }

    async fn run_workflow_worker_internal(bootstrap_fn: Option<BootstrapFn>) {
        let shutdown = Self::start_daemon();
        if let Err(e) = Self::boot_worker_process(bootstrap_fn).await {
            eprintln!("Workflow worker bootstrap error: {e}");
            std::process::exit(1);
        }

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

    /// `queue:work`: drain the configured queue driver until cancelled.
    ///
    /// Runs the app's `bootstrap_fn` and then the runtime drivers (see
    /// [`Self::boot_worker_process`] for why that order), so popped jobs can
    /// resolve services from the container. Honours Ctrl-C and SIGTERM
    /// cleanly via
    /// `CancellationToken`: the cancel fires at the next pop boundary, so an
    /// in-flight handler runs to completion (bounded by its own per-job
    /// `timeout()` if set) before the worker exits.
    async fn run_queue_worker_internal(
        bootstrap_fn: Option<BootstrapFn>,
        visibility_timeout: u64,
        poll_interval_ms: u64,
        max_jobs: Option<u64>,
        queues: Vec<String>,
    ) {
        let shutdown = Self::start_daemon();
        if let Err(e) = Self::boot_worker_process(bootstrap_fn).await {
            eprintln!("suprnova: queue worker bootstrap error: {e}");
            std::process::exit(1);
        }

        let driver = match crate::queue::Queue::driver() {
            Ok(d) => d,
            Err(e) => {
                eprintln!("suprnova: no queue driver configured: {e}");
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

        let cancel_for_worker = cancel.clone();
        let mut worker = tokio::spawn(async move {
            crate::queue::worker::run_worker(driver, cfg, cancel_for_worker).await;
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

    /// Shared bootstrap for non-server subcommands that still need the
    /// runtime drivers: Cache, Queue, RateLimit, Mail. Mirrors the
    /// driver-bootstrap order in `Server::run` (telemetry / encryption
    /// keys / authorization init are subcommand-specific and stay out
    /// of this helper).
    async fn bootstrap_runtime_drivers() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        crate::cache::Cache::bootstrap().await?;
        crate::queue::bootstrap_from_env().await?;
        crate::rate_limit::bootstrap_from_env().await?;
        crate::mail::boot::bootstrap_from_env()?;
        Ok(())
    }

    /// Full boot for the long-running non-server subcommands (`queue:work`,
    /// `schedule:work`, `schedule:run`, `workflow:work`).
    ///
    /// The app's `bootstrap_fn` runs **first**, then the env-driven drivers.
    /// That order is not cosmetic: `QUEUE_DRIVER=database` resolves its
    /// connection out of `DB`, which only exists once the app's bootstrap has
    /// called `DB::init`. Booting the drivers first made every worker
    /// subcommand die with "requires DB::init() to run first" before it could
    /// pop a single job. `Server::run` already boots the drivers after
    /// `bootstrap_fn`; this makes the worker paths agree with it, which also
    /// means a `bootstrap_fn` that installs a driver by hand is overridden by
    /// the environment in exactly the same way under `serve` and under
    /// `queue:work`.
    /// Give a daemon process a tracing subscriber.
    ///
    /// `serve` gets one from `init_telemetry`; the daemons come through a
    /// different path and used to get nothing, so every `tracing::` line
    /// they emit went nowhere and `LOG_LEVEL` was inert for them. That is
    /// most of what they have to say — a worker dead-lettering a job, a
    /// scheduler skipping a tick it lost, a lock it could not release. In
    /// a container the only visible output was the startup banner, and the
    /// process looked idle while it was doing all of it.
    ///
    /// Called from the four daemon entry points rather than from
    /// [`Self::boot_worker_process`], which they share: that helper is
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
        crate::logging::init_subscriber(crate::logging::LogConfig::from_env());
    }

    /// Everything a daemon needs installed before it does anything else.
    ///
    /// Returns the shutdown listener, and returning it is the point: the
    /// signal handler has to exist *before* the bootstrap that can take
    /// seconds and before the banner that promises SIGTERM works. It used
    /// to be created just above the `select!`, several awaits later, and
    /// a stop arriving in that window found no handler and killed the
    /// process outright — losing the drain in exactly the situation the
    /// drain exists for, a supervisor stopping a container mid-start.
    ///
    /// Safe to create this early because the listener publishes through a
    /// `watch`: a signal that arrives before anyone waits is still there
    /// when the `select!` finally asks.
    fn start_daemon() -> crate::signals::ShutdownListener {
        Self::install_daemon_logging();
        crate::signals::spawn_shutdown_listener()
    }

    async fn boot_worker_process(
        bootstrap_fn: Option<BootstrapFn>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(bootstrap_fn) = bootstrap_fn {
            bootstrap_fn().await;
        }
        Self::bootstrap_runtime_drivers().await
    }

    /// `down`: record the maintenance payload via the configured driver.
    #[allow(clippy::too_many_arguments)]
    async fn run_down(
        retry: Option<u64>,
        refresh: Option<u64>,
        secret: Option<String>,
        with_secret: bool,
        redirect: Option<String>,
        status: u16,
        except: Vec<String>,
        message: Option<String>,
    ) {
        Self::bootstrap_maintenance_driver().await;

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

    /// `up`: clear maintenance state via the configured driver.
    async fn run_up() {
        Self::bootstrap_maintenance_driver().await;

        match maintenance::maintenance_mode().deactivate().await {
            Ok(()) => println!("Application is now live."),
            Err(e) => {
                eprintln!("suprnova: failed to bring the application up: {e}");
                std::process::exit(1);
            }
        }
    }

    /// The cache-backed maintenance driver needs the cache bootstrapped; the
    /// file driver needs nothing. Only boot the cache when it's in use.
    async fn bootstrap_maintenance_driver() {
        if env::var("MAINTENANCE_DRIVER").as_deref() == Ok("cache")
            && let Err(e) = crate::cache::Cache::bootstrap().await
        {
            eprintln!("suprnova: maintenance (cache driver) bootstrap failed: {e}");
            std::process::exit(1);
        }
    }
}

/// Decide whether a `migrate:fresh` may proceed.
///
/// Outside production this is always `Ok` — dropping a local database is
/// routine. In production it demands two *different* kinds of proof:
///
/// 1. `--force`, which proves intent at the moment the command was typed.
/// 2. A typed confirmation on an interactive terminal, which proves a
///    human is present.
///
/// The TTY requirement is the point of the second condition. Without it,
/// `echo production | app migrate:fresh --force` in a deploy script would
/// satisfy the prompt automatically and the confirmation would be just
/// another flag — which is exactly what this guard exists to prevent.
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
            "Confirmation did not match — nothing was dropped and no migrations ran.".to_string(),
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
    /// bootstrap ran `DB::init`. Booting the drivers first — which every
    /// worker subcommand used to do — fails here with "requires DB::init()
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

        Application::<NoMigrator>::boot_worker_process(Some(bootstrap))
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
}

#[cfg(test)]
mod migrate_fresh_gate_tests {
    use super::authorize_migrate_fresh;
    use crate::config::Environment;

    /// Never called — reaching it means the gate asked for a confirmation
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
                "{env} must not be gated — dropping a non-production database is routine"
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

    #[test]
    fn format_schedule_listing_empty_includes_registration_hint() {
        let schedule = build_schedule(None);
        let out = format_schedule_listing(&schedule);
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
        let out = format_schedule_listing(&schedule);
        assert!(out.starts_with("Registered scheduled tasks:\n"));
        assert!(out.contains("nightly-cleanup"));
        assert!(out.contains("[* * * * *]"));
        assert!(out.contains("— Remove stale upload temp files"));
        assert!(out.contains("plain-hourly"));
        assert!(out.contains("[0 * * * *]"));
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
    /// evaluation pipeline as closure-based ones — proves
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
        let listing = format_schedule_listing(&schedule);
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
    /// env var is missing — that would terminate the entire test binary,
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

    /// An empty set is the common case — most shutdowns have no background
    /// work in flight — and must not cost a scheduler timer at all.
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
