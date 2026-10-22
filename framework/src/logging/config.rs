//! Configuration for tracing/log output. Read from environment so
//! consumers can change verbosity without recompiling.

use std::env;

use super::channel::LogLevel;
use crate::error::FrameworkError;

/// Output format for log lines.
#[derive(Debug, Clone, Copy)]
pub enum LogFormat {
    /// Human-friendly multi-line output. Default for dev.
    Pretty,
    /// One-JSON-object-per-line. Default for production / log aggregators.
    Json,
}

/// Logging configuration.
#[derive(Debug, Clone)]
pub struct LogConfig {
    /// `tracing-subscriber` env-filter directive
    /// (e.g. `"info"`, `"debug,hyper=warn,sqlx=info"`).
    pub level: String,
    /// Output format.
    pub format: LogFormat,
}

impl LogConfig {
    /// Read from `LOG_LEVEL` (default `"info"`) and `LOG_FORMAT`
    /// (`"pretty"` | `"json"`).
    ///
    /// `LOG_LEVEL` is the `tracing` filter directive, which may use the
    /// PSR-3 names Laravel's do (`warning`, `notice`, `critical`, `alert`,
    /// `emergency`): the filter reads each as its nearest `tracing` level.
    /// Its bare level is also the lowest level each built-in file or stream
    /// channel keeps.
    ///
    /// When `LOG_FORMAT` is unset, the default is environment-aware:
    /// `json` in production (the log-aggregator-friendly format
    /// [`LogFormat::Json`] documents as the production default) and
    /// `pretty` everywhere else for human-readable local/dev output. An
    /// explicit `LOG_FORMAT` always wins over this default. The
    /// environment is detected from `APP_ENV` via
    /// [`Environment::detect`](crate::config::Environment::detect).
    pub fn from_env() -> Self {
        let level = env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
        let format = match env::var("LOG_FORMAT").as_deref() {
            Ok("json") => LogFormat::Json,
            Ok("pretty") => LogFormat::Pretty,
            _ => {
                if crate::config::Environment::detect().is_production() {
                    LogFormat::Json
                } else {
                    LogFormat::Pretty
                }
            }
        };
        Self { level, format }
    }
}

impl Default for LogConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

/// The bare level of a `LOG_LEVEL` directive: its last part that names no
/// target, `info` in `info,sqlx=warn`, or `None` when every part names one.
pub(crate) fn bare_level(directive: &str) -> Option<&str> {
    directive
        .split(',')
        .map(str::trim)
        .rfind(|part| !part.is_empty() && !part.contains('=') && !part.contains('['))
}

/// The lowest level the built-in file and stream channels keep: the bare
/// level of `LOG_LEVEL`, or `debug` when it names none, as Laravel's
/// `config/logging.php` passes `env('LOG_LEVEL', 'debug')` to each channel.
///
/// # Errors
///
/// When the bare level is no level [`LogLevel::parse`] reads, as Laravel's
/// `level()` refuses one.
pub(crate) fn channel_level() -> Result<LogLevel, FrameworkError> {
    let Ok(directive) = env::var("LOG_LEVEL") else {
        return Ok(LogLevel::Debug);
    };
    match bare_level(&directive) {
        Some(name) => LogLevel::parse(name).map_err(|error| {
            FrameworkError::internal(format!("LOG_LEVEL is '{directive}': {error}"))
        }),
        None => Ok(LogLevel::Debug),
    }
}

/// `directive` with each PSR-3 level name the `tracing` filter does not know
/// replaced by its nearest `tracing` level: `warning` by `warn`, `notice` by
/// `info`, and `critical`, `alert` and `emergency` by `error`, so
/// `LOG_LEVEL=warning` keeps errors and warnings as it does in Laravel.
pub(crate) fn tracing_directive(directive: &str) -> String {
    fn nearest(level: &str) -> Option<&'static str> {
        match level.trim().to_ascii_lowercase().as_str() {
            "warning" => Some("warn"),
            "notice" => Some("info"),
            "critical" | "alert" | "emergency" => Some("error"),
            _ => None,
        }
    }
    directive
        .split(',')
        .map(|part| match part.rsplit_once('=') {
            Some((target, level)) => match nearest(level) {
                Some(level) => format!("{target}={level}"),
                None => part.to_owned(),
            },
            None => nearest(part).map_or_else(|| part.to_owned(), str::to_owned),
        })
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // tests in this module touch the global env, so they need to run sequentially.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn from_env_defaults_to_info_pretty() {
        let _g = ENV_LOCK.lock().unwrap();
        // SAFETY: ENV_LOCK serializes env access within this module
        unsafe {
            std::env::remove_var("LOG_LEVEL");
            std::env::remove_var("LOG_FORMAT");
            // The format default is environment-aware: name a development
            // environment, since an unset APP_ENV is production, whose
            // default is JSON.
            std::env::set_var("APP_ENV", "local");
        }
        let cfg = LogConfig::from_env();
        assert_eq!(cfg.level, "info");
        assert!(matches!(cfg.format, LogFormat::Pretty));
        unsafe {
            std::env::remove_var("APP_ENV");
        }
    }

    #[test]
    fn from_env_defaults_to_json_with_app_env_unset() {
        let _g = ENV_LOCK.lock().unwrap();
        // SAFETY: ENV_LOCK serializes env access within this module
        unsafe {
            std::env::remove_var("LOG_FORMAT");
            std::env::remove_var("APP_ENV");
        }
        assert!(
            matches!(LogConfig::from_env().format, LogFormat::Json),
            "an unset APP_ENV is production"
        );
    }

    #[test]
    fn the_bare_level_is_the_part_without_a_target() {
        assert_eq!(bare_level("info,sqlx=warn"), Some("info"));
        assert_eq!(bare_level("sqlx=warn, warning "), Some("warning"));
        assert_eq!(bare_level("sqlx=warn"), None);
        assert_eq!(bare_level("app[request{id=1}]=debug"), None);
        assert_eq!(bare_level(""), None);
    }

    #[test]
    fn psr_names_become_tracing_levels_in_the_filter() {
        assert_eq!(tracing_directive("warning"), "warn");
        assert_eq!(tracing_directive("notice,sqlx=critical"), "info,sqlx=error");
        assert_eq!(tracing_directive("Emergency"), "error");
        assert_eq!(tracing_directive("info,hyper=warn"), "info,hyper=warn");
        assert_eq!(tracing_directive("app[span{a=b}]"), "app[span{a=b}]");
    }

    #[test]
    fn from_env_defaults_to_json_in_production() {
        let _g = ENV_LOCK.lock().unwrap();
        // SAFETY: ENV_LOCK serializes env access within this module
        unsafe {
            std::env::remove_var("LOG_FORMAT");
            std::env::set_var("APP_ENV", "production");
        }
        let cfg = LogConfig::from_env();
        assert!(
            matches!(cfg.format, LogFormat::Json),
            "production must default to JSON logs when LOG_FORMAT is unset"
        );
        unsafe {
            std::env::remove_var("APP_ENV");
        }
    }

    #[test]
    fn explicit_log_format_wins_over_production_default() {
        let _g = ENV_LOCK.lock().unwrap();
        // SAFETY: ENV_LOCK serializes env access within this module
        unsafe {
            std::env::set_var("APP_ENV", "production");
            std::env::set_var("LOG_FORMAT", "pretty");
        }
        let cfg = LogConfig::from_env();
        assert!(
            matches!(cfg.format, LogFormat::Pretty),
            "an explicit LOG_FORMAT must override the production JSON default"
        );
        unsafe {
            std::env::remove_var("APP_ENV");
            std::env::remove_var("LOG_FORMAT");
        }
    }

    #[test]
    fn from_env_reads_overrides() {
        let _g = ENV_LOCK.lock().unwrap();
        // SAFETY: ENV_LOCK serializes env access within this module
        unsafe {
            std::env::set_var("LOG_LEVEL", "debug,hyper=warn");
            std::env::set_var("LOG_FORMAT", "json");
        }
        let cfg = LogConfig::from_env();
        assert_eq!(cfg.level, "debug,hyper=warn");
        assert!(matches!(cfg.format, LogFormat::Json));
        // Cleanup so other tests see a fresh env
        unsafe {
            std::env::remove_var("LOG_LEVEL");
            std::env::remove_var("LOG_FORMAT");
        }
    }
}
