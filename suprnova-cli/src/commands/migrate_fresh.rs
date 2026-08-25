//! `suprnova migrate:fresh` - drop every table, then re-run all migrations.
//!
//! This is the only CLI command that destroys data it cannot recover, and it
//! is one shell-history arrow-up away from `migrate`. In production that
//! costs the database, so the command refuses to touch a production
//! environment unless the operator both passes `--force` *and* types the
//! environment name back at an interactive prompt.
//!
//! The two conditions are deliberately different in kind: `--force` proves
//! intent was expressed when the command was written, the typed confirmation
//! proves a human is present when it runs. A pasted command line satisfies
//! the first but not the second, which is exactly the case worth stopping.
//! That is also why a non-TTY refuses instead of falling back to reading
//! piped stdin - `echo production | ... --force` in a deploy script would
//! turn the confirmation back into a flag.

use std::io::IsTerminal;
use std::path::Path;

use crate::commands::interpret_cargo_status;
use crate::ui;

/// Everything the guard needs from the outside world.
///
/// Resolved once in [`run`] and injected, so the decision can be tested
/// without setting environment variables or owning a terminal - and so the
/// tests can observe whether the migrator was reached at all.
struct FreshContext<'a> {
    /// Where the project's migrations live (`src/migrations` in a real run).
    migrations_dir: &'a Path,
    /// The raw `APP_ENV` value, echoed verbatim in the confirmation prompt.
    app_env: &'a str,
    /// Whether `--force` was passed.
    force: bool,
    /// Whether stdin is a terminal, i.e. whether a human can answer.
    stdin_is_tty: bool,
}

/// Is this a production environment?
///
/// Mirrors `suprnova::config::Environment::detect` - including the `prod`
/// alias and the case-insensitive match - because the CLI cannot depend on
/// the framework crate. A guard that disagreed with the app about what
/// "production" means would be worse than no guard.
fn is_production(app_env: &str) -> bool {
    matches!(
        app_env.trim().to_ascii_lowercase().as_str(),
        "production" | "prod"
    )
}

/// Decide whether `migrate:fresh` may drop the database.
///
/// Returns `Ok(())` only when the run is allowed. Every refusal path returns
/// before `read_confirmation` can be reached where a prompt would be
/// meaningless, so a caller cannot mistake "we asked and they said no" for
/// "we never asked".
fn authorize(
    ctx: &FreshContext<'_>,
    read_confirmation: &mut dyn FnMut() -> Result<String, String>,
) -> Result<(), String> {
    if !is_production(ctx.app_env) {
        return Ok(());
    }

    if !ctx.force {
        return Err(format!(
            "Refusing to run migrate:fresh with APP_ENV={}: it drops every table \
             in the database and the data is not recoverable.\n  If you are \
             certain, re-run it as `suprnova migrate:fresh --force` from an \
             interactive terminal and type the environment name when asked.",
            ctx.app_env
        ));
    }

    if !ctx.stdin_is_tty {
        return Err(format!(
            "Refusing to run migrate:fresh with APP_ENV={}: --force alone is not \
             enough, it also needs a typed confirmation, and stdin is not a \
             terminal.\n  Run it from an interactive shell. Piping the answer in \
             would make the confirmation just another flag, which is the thing \
             this guard exists to prevent.",
            ctx.app_env
        ));
    }

    let expected = ctx.app_env.trim();
    ui::warning(&format!(
        "About to DROP ALL TABLES in the {expected} database. This cannot be undone."
    ));
    ui::info(&format!(
        "Type `{expected}` to confirm, or anything else to abort:"
    ));

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

/// Hand the drop-and-migrate over to the project's own binary.
fn spawn_migrator() -> Result<(), String> {
    let status = crate::commands::cargo_run(&["migrate:fresh"]).status();

    interpret_cargo_status(status, "migrate:fresh", false)
}

pub fn run(force: bool) {
    // Load `.env` so APP_ENV resolves to what the app itself would see.
    let _ = dotenvy::dotenv();
    let app_env = std::env::var("APP_ENV").unwrap_or_else(|_| "local".to_string());

    let ctx = FreshContext {
        migrations_dir: Path::new("src/migrations"),
        app_env: &app_env,
        force,
        stdin_is_tty: std::io::stdin().is_terminal(),
    };

    let result = run_inner(&ctx, &mut read_confirmation_from_stdin, &mut spawn_migrator);

    if let Err(e) = result {
        ui::error(&e);
        std::process::exit(1);
    }
}

fn run_inner(
    ctx: &FreshContext<'_>,
    read_confirmation: &mut dyn FnMut() -> Result<String, String>,
    migrator: &mut dyn FnMut() -> Result<(), String>,
) -> Result<(), String> {
    if !ctx.migrations_dir.exists() {
        ui::hint("Run 'suprnova make:migration <name>' to create your first migration.");
        return Err(format!(
            "No migrations directory found at {}",
            ctx.migrations_dir.display()
        ));
    }

    authorize(ctx, read_confirmation)?;

    ui::warning("Dropping all tables and re-running migrations...");
    ui::warning("This will delete all data in your database!");
    ui::br();

    migrator()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// A migrator that records whether it was ever reached.
    ///
    /// This is the seam the guard is really asserted against: a refusal that
    /// still spawned the migrator would print a reassuring message while the
    /// tables went away regardless.
    struct Spy {
        migrator_calls: Cell<usize>,
        confirm_calls: Cell<usize>,
    }

    impl Spy {
        fn new() -> Self {
            Self {
                migrator_calls: Cell::new(0),
                confirm_calls: Cell::new(0),
            }
        }
    }

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("src/migrations")).expect("mkdir migrations");
        dir
    }

    fn drive(
        dir: &std::path::Path,
        app_env: &str,
        force: bool,
        stdin_is_tty: bool,
        answer: Option<&str>,
        spy: &Spy,
    ) -> Result<(), String> {
        let migrations_dir = dir.join("src/migrations");
        let ctx = FreshContext {
            migrations_dir: &migrations_dir,
            app_env,
            force,
            stdin_is_tty,
        };
        let answer = answer.map(|a| a.to_string());
        let mut confirm = || {
            spy.confirm_calls.set(spy.confirm_calls.get() + 1);
            answer.clone().ok_or_else(|| "stdin closed".to_string())
        };
        let mut migrator = || {
            spy.migrator_calls.set(spy.migrator_calls.get() + 1);
            Ok(())
        };
        run_inner(&ctx, &mut confirm, &mut migrator)
    }

    #[test]
    fn non_production_runs_without_a_prompt() {
        let dir = project();
        let spy = Spy::new();
        drive(dir.path(), "local", false, false, None, &spy).expect("local must not be gated");
        assert_eq!(spy.migrator_calls.get(), 1, "the migrator must run");
        assert_eq!(spy.confirm_calls.get(), 0, "no prompt outside production");
    }

    #[test]
    fn production_without_force_refuses_and_never_reaches_the_migrator() {
        let dir = project();
        let spy = Spy::new();
        let err = drive(
            dir.path(),
            "production",
            false,
            true,
            Some("production"),
            &spy,
        )
        .expect_err("production without --force must refuse");
        assert!(
            err.contains("--force"),
            "the refusal should say what is missing; got: {err}"
        );
        assert_eq!(
            spy.migrator_calls.get(),
            0,
            "the migrator MUST NOT be spawned on an unconfirmed production path"
        );
        assert_eq!(
            spy.confirm_calls.get(),
            0,
            "no prompt should be offered without --force"
        );
    }

    #[test]
    fn production_with_force_but_no_tty_refuses_and_never_auto_confirms() {
        let dir = project();
        let spy = Spy::new();
        let err = drive(
            dir.path(),
            "production",
            true,
            false,
            Some("production"),
            &spy,
        )
        .expect_err("a non-interactive production run must refuse");
        assert!(
            err.contains("terminal"),
            "the refusal should explain the TTY requirement; got: {err}"
        );
        assert_eq!(
            spy.migrator_calls.get(),
            0,
            "the migrator MUST NOT be spawned without a typed confirmation"
        );
        assert_eq!(
            spy.confirm_calls.get(),
            0,
            "a non-TTY answer must never be read, let alone accepted"
        );
    }

    #[test]
    fn production_with_force_and_matching_confirmation_proceeds() {
        let dir = project();
        let spy = Spy::new();
        drive(
            dir.path(),
            "production",
            true,
            true,
            Some("production\n"),
            &spy,
        )
        .expect("--force plus a matching confirmation must proceed");
        assert_eq!(spy.confirm_calls.get(), 1, "the human must be asked once");
        assert_eq!(spy.migrator_calls.get(), 1, "then the migrator runs");
    }

    #[test]
    fn production_with_force_and_wrong_confirmation_refuses() {
        let dir = project();
        let spy = Spy::new();
        let err = drive(dir.path(), "production", true, true, Some("yes\n"), &spy)
            .expect_err("a mismatched confirmation must refuse");
        assert!(
            err.contains("nothing was dropped"),
            "the refusal should say the database is untouched; got: {err}"
        );
        assert_eq!(
            spy.migrator_calls.get(),
            0,
            "a wrong answer MUST NOT reach the migrator"
        );
    }

    #[test]
    fn production_with_force_and_unreadable_stdin_refuses() {
        let dir = project();
        let spy = Spy::new();
        let err = drive(dir.path(), "production", true, true, None, &spy)
            .expect_err("an unreadable confirmation must refuse");
        assert!(
            err.contains("stdin closed"),
            "the read failure should surface; got: {err}"
        );
        assert_eq!(spy.migrator_calls.get(), 0, "no migrator on a failed read");
    }

    #[test]
    fn the_prod_alias_and_odd_casing_are_still_production() {
        for env in ["prod", "PRODUCTION", " Production "] {
            let dir = project();
            let spy = Spy::new();
            assert!(
                drive(dir.path(), env, false, true, None, &spy).is_err(),
                "APP_ENV={env} must be gated"
            );
            assert_eq!(
                spy.migrator_calls.get(),
                0,
                "APP_ENV={env} must not reach the migrator"
            );
        }
    }

    #[test]
    fn a_missing_migrations_directory_stops_before_the_guard() {
        let dir = tempfile::tempdir().expect("tempdir");
        let spy = Spy::new();
        let err = drive(dir.path(), "local", false, false, None, &spy)
            .expect_err("no migrations directory must error");
        assert!(err.contains("No migrations directory"), "got: {err}");
        assert_eq!(
            spy.migrator_calls.get(),
            0,
            "nothing to migrate, nothing run"
        );
    }
}
