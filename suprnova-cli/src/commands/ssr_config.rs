//! The configuration the `ssr:*` commands run with, and how they end.
//!
//! The application binary's `ssr:start`, `ssr:stop` and `ssr:check` read the
//! Inertia configuration the application installed. This CLI cannot reach
//! it, so its commands build the same `SsrConfig` from their flags and the
//! `SUPRNOVA_SSR_*` environment and run the same functions,
//! `suprnova::console::ssr` (PAR-061).

use std::future::Future;
use std::time::Duration;

use suprnova::{FrameworkError, SsrConfig};

/// The variable `ssr:start` reads `ensure_runtime_exists` from.
const ENSURE_RUNTIME_EXISTS: &str = "SUPRNOVA_SSR_ENSURE_RUNTIME_EXISTS";

/// What the flags of a command set; `None` falls back to the environment,
/// then to `SsrConfig`'s default.
#[derive(Debug, Default)]
pub(crate) struct Flags {
    /// `--url`, over `SUPRNOVA_SSR_URL`.
    pub url: Option<String>,
    /// `--timeout-ms`, over the default timeout.
    pub timeout: Option<Duration>,
    /// `--bundle`, over `SUPRNOVA_SSR_BUNDLE`.
    pub bundle: Option<String>,
}

/// The configuration for `flags` and the environment.
///
/// SSR is enabled: the developer ran an SSR command. The URL is `--url`,
/// else `SUPRNOVA_SSR_URL`, else `http://127.0.0.1:13714`; the runtime is
/// `SUPRNOVA_SSR_RUNTIME`, else `node` (`--runtime` overrides it when the
/// command runs); the bundle is `--bundle`, else `SUPRNOVA_SSR_BUNDLE`,
/// else none, which leaves the conventional paths to the bundle detection.
///
/// # Errors
///
/// When `SUPRNOVA_SSR_ENSURE_RUNTIME_EXISTS` is neither true nor false.
pub(crate) fn config(flags: Flags) -> Result<SsrConfig, String> {
    let defaults = SsrConfig::default();
    let var = |name: &str| std::env::var(name).ok();
    Ok(SsrConfig {
        enabled: true,
        url: flags
            .url
            .or_else(|| var("SUPRNOVA_SSR_URL"))
            .unwrap_or(defaults.url.clone()),
        timeout: flags.timeout.unwrap_or(defaults.timeout),
        runtime: var("SUPRNOVA_SSR_RUNTIME").unwrap_or(defaults.runtime.clone()),
        bundle_path: flags
            .bundle
            .or_else(|| var("SUPRNOVA_SSR_BUNDLE"))
            .map(Into::into),
        ensure_runtime_exists: flag(ENSURE_RUNTIME_EXISTS, var(ENSURE_RUNTIME_EXISTS).as_deref())?,
        ..defaults
    })
}

/// A yes-or-no variable: the framework's truthy words are yes, their
/// opposites and an empty or unset variable are no. Anything else is an
/// error rather than a guess, since it decides whether `ssr:start` checks
/// the runtime.
fn flag(name: &str, value: Option<&str>) -> Result<bool, String> {
    match value.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
        None | Some("" | "0" | "false" | "no" | "off") => Ok(false),
        Some("1" | "true" | "yes" | "on") => Ok(true),
        Some(_) => Err(format!(
            "{name} must be true or false, not \"{}\".",
            value.unwrap_or_default()
        )),
    }
}

/// Build the configuration, run `command` with it to the end, and exit with
/// the status it returns. A configuration that cannot be built, or a
/// command that could not run, exits with 1 after printing why.
pub(crate) fn run<F, Fut>(flags: Flags, command: F) -> !
where
    F: FnOnce(SsrConfig) -> Fut,
    Fut: Future<Output = Result<i32, FrameworkError>>,
{
    let config = match config(flags) {
        Ok(config) => config,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("Unable to start the async runtime: {e}");
            std::process::exit(1);
        }
    };
    let status = match runtime.block_on(command(config)) {
        Ok(status) => status,
        Err(e) => {
            eprintln!("{}", e.message());
            1
        }
    };
    std::process::exit(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_win_over_the_defaults() {
        let config = config(Flags {
            url: Some("http://ssr.internal:4000".into()),
            timeout: Some(Duration::from_millis(250)),
            bundle: Some("build/ssr.js".into()),
        })
        .expect("a configuration");

        assert!(config.enabled, "the developer ran an SSR command");
        assert_eq!(config.url, "http://ssr.internal:4000");
        assert_eq!(config.timeout, Duration::from_millis(250));
        assert_eq!(config.bundle_path, Some("build/ssr.js".into()));
    }

    #[test]
    fn the_defaults_fill_what_neither_flags_nor_environment_set() {
        // Read only when the developer's shell sets none of them.
        let set = [
            "SUPRNOVA_SSR_URL",
            "SUPRNOVA_SSR_RUNTIME",
            "SUPRNOVA_SSR_BUNDLE",
        ]
        .iter()
        .any(|name| std::env::var_os(name).is_some());
        if set || std::env::var_os(ENSURE_RUNTIME_EXISTS).is_some() {
            return;
        }
        let config = config(Flags::default()).expect("a configuration");

        assert_eq!(config.url, "http://127.0.0.1:13714");
        assert_eq!(config.runtime, "node");
        assert_eq!(config.bundle_path, None, "the conventional paths decide");
        assert!(!config.ensure_runtime_exists);
    }

    #[test]
    fn a_flag_reads_the_framework_words_and_refuses_the_rest() {
        for (value, expected) in [
            (None, false),
            (Some(""), false),
            (Some("0"), false),
            (Some("FALSE"), false),
            (Some("no"), false),
            (Some("off"), false),
            (Some("1"), true),
            (Some("true"), true),
            (Some(" Yes "), true),
            (Some("on"), true),
        ] {
            assert_eq!(flag("X", value), Ok(expected), "{value:?}");
        }
        let error = flag("X", Some("maybe")).expect_err("not a yes or a no");
        assert!(error.contains("X") && error.contains("maybe"), "{error}");
    }
}
