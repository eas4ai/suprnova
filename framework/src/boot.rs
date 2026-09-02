//! Pre-runtime boot steps.
//!
//! Everything here must run while the process is still single-threaded.
//!
//! Loading `.env` writes to the process environment, and `set_var` is
//! sound only when no other thread can be reading it. `#[tokio::main]`
//! constructs the runtime around the whole of `main`, so its worker
//! threads already exist before the first statement executes - and any
//! of them may call `getenv` indirectly through DNS resolution, time
//! formatting, or a C dependency. The window is small and the corruption
//! is silent, which is the worst combination to debug.
//!
//! [`crate::main`] exists to close that window: it loads the environment
//! first, *then* builds the runtime. This module holds the half that
//! macro calls, plus the guard that stops the old shape from silently
//! coming back.

use crate::config::{Config, Environment};
use crate::error::FrameworkError;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Set by [`load_env`] once the environment has been loaded from a
/// single-threaded context.
static ENV_LOADED_PRE_RUNTIME: AtomicBool = AtomicBool::new(false);

/// Load `.env` and register the framework's typed config.
///
/// Refuses when a Tokio runtime is already running, because at that
/// point the mutation this performs is exactly the unsound one the
/// module doc describes. Returning an error rather than proceeding is
/// deliberate: the caller can still be fixed, and a silent success here
/// is a data race that shows up somewhere else entirely.
///
/// # Errors
///
/// Returns [`FrameworkError`] when called from inside a Tokio runtime,
/// when a discovered `.env` file cannot be read or parsed, or when a
/// typed framework knob fails to parse.
pub fn load_env() -> Result<Environment, FrameworkError> {
    if tokio::runtime::Handle::try_current().is_ok() {
        return Err(FrameworkError::internal(
            "suprnova::boot::load_env() was called from inside a Tokio runtime. \
             Loading .env mutates the process environment, which is only sound \
             while the process is single-threaded. Use #[suprnova::main] instead \
             of #[tokio::main] so the environment loads before the runtime is built.",
        ));
    }

    let env = Config::init(Path::new("."))?;
    ENV_LOADED_PRE_RUNTIME.store(true, Ordering::Release);
    Ok(env)
}

/// [`load_env`], but reports the failure to the operator and exits.
///
/// This is what [`crate::main`] expands to. A boot that cannot read its
/// configuration has nothing useful to do next, and an operator reading
/// stderr is better served by one clear line than by a panic backtrace
/// through a proc-macro expansion.
pub fn load_env_or_exit() -> Environment {
    match load_env() {
        Ok(env) => env,
        Err(e) => {
            eprintln!("framework configuration init failed: {e}");
            std::process::exit(1);
        }
    }
}

/// Validate and install the process-wide encryption key ring.
///
/// Generated applications call this immediately before their real application
/// bootstrap. Keeping it out of [`load_env_or_exit`] lets console help,
/// version, and parse-error paths load enough configuration to render output
/// without requiring an `APP_KEY` for bootstrap work they never perform.
///
/// This helper reports validation failures to stderr and exits because an
/// application bootstrap cannot safely continue without Crypt.
pub fn initialize_crypt_or_exit() {
    let environment = Config::get::<crate::config::AppConfig>()
        .map(|config| config.environment)
        .unwrap_or_else(Environment::detect);
    if let Err(error) = crate::crypto::initialize_from_environment(&environment) {
        eprintln!("framework encryption init failed: {error}");
        std::process::exit(1);
    }
}

/// Whether [`load_env`] has run from a single-threaded context.
pub fn env_loaded_pre_runtime() -> bool {
    ENV_LOADED_PRE_RUNTIME.load(Ordering::Acquire)
}

/// The guard [`crate::Application::run`] applies before doing anything.
///
/// Split out as a pure function of the flag so the refusal can be tested
/// without a process exit or a global mutation.
pub(crate) fn boot_precondition(loaded_pre_runtime: bool) -> Result<(), String> {
    if loaded_pre_runtime {
        return Ok(());
    }
    Err(
        "the environment was never loaded before the Tokio runtime started.\n\
         \n\
         Replace #[tokio::main] with #[suprnova::main] on your `main` function:\n\
         \n\
             #[suprnova::main]\n\
             async fn main() {\n\
                 Application::new()\n\
                     // ...\n\
                     .run()\n\
                     .await;\n\
             }\n\
         \n\
         Loading .env mutates the process environment, which is only sound while \
         the process is single-threaded. #[suprnova::main] loads it before \
         building the runtime; #[tokio::main] cannot."
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loaded_environment_satisfies_the_precondition() {
        assert!(boot_precondition(true).is_ok());
    }

    /// The message is the entire value of this guard - someone hits it
    /// exactly once, while confused, and needs the fix in front of them.
    #[test]
    fn an_unloaded_environment_is_refused_with_the_fix_in_the_message() {
        let err = boot_precondition(false).expect_err("must refuse");
        assert!(
            err.contains("#[suprnova::main]"),
            "the message must name the attribute that fixes it; got: {err}"
        );
        assert!(
            err.contains("#[tokio::main]"),
            "the message must name the attribute being replaced; got: {err}"
        );
        assert!(
            err.contains("single-threaded"),
            "the message must say why, or it reads as an arbitrary rule; got: {err}"
        );
    }

    /// `load_env` mutates a global, so this asserts the refusal path
    /// only - the one that must never set the flag.
    #[tokio::test]
    async fn load_env_refuses_inside_a_runtime() {
        let err = load_env().expect_err("must refuse inside a runtime");
        let msg = err.to_string();
        assert!(
            msg.contains("#[suprnova::main]"),
            "the refusal must point at the fix; got: {msg}"
        );
        assert!(
            !env_loaded_pre_runtime(),
            "a refused load must not mark the environment as soundly loaded"
        );
    }
}
