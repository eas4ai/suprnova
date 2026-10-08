//! The `devtools` section of the Inertia configuration: inertia-laravel's
//! `config('inertia.devtools')`.

use std::path::PathBuf;

/// Settings of the server side of the Inertia DevTools browser extension,
/// set on [`InertiaConfig::devtools`](crate::InertiaConfig::devtools).
///
/// Recording writes every request's headers, bodies and props to disk and
/// serves them to whoever the endpoints admit, which is why it is off
/// outside the `local` environment unless switched on, and why sensitive
/// keys and headers are redacted before an entry is stored. Each field
/// defaults to Laravel's value, read from the same environment variables
/// where Laravel reads one.
///
/// ```rust,no_run
/// use suprnova::{DevToolsConfig, InertiaConfig};
///
/// let config = InertiaConfig::new().devtools(
///     DevToolsConfig::new()
///         .enabled(true)
///         .gate("viewInertiaDevtools")
///         .limit(50),
/// );
/// # let _ = config;
/// ```
#[derive(Debug, Clone)]
pub struct DevToolsConfig {
    /// Whether requests are recorded. `None` records in the `local`
    /// environment only; `Some` decides outright. Default
    /// `INERTIA_DEVTOOLS_ENABLED` when it is set: `true`, `1`, `on` or
    /// `yes` record, `false`, `0`, `off` or `no` do not.
    pub enabled: Option<bool>,
    /// Paths that are never recorded, as `Request::is` reads them: `*`
    /// matches any characters, a leading slash is ignored. Default
    /// `_inertia/devtools*`, the extension's own endpoints, and
    /// `_suprnova/*`, the framework's tooling routes.
    pub except: Vec<String>,
    /// The directory entries are stored in. Default
    /// `storage_path("inertia-devtools")`.
    pub storage_path: PathBuf,
    /// How many hours an entry is kept. Default 24, or
    /// `INERTIA_DEVTOOLS_TTL_HOURS`.
    pub ttl_hours: u64,
    /// How many seconds apart two prunes of expired entries run. `0`
    /// prunes after every request. Default 300, or
    /// `INERTIA_DEVTOOLS_PRUNE_INTERVAL_SECONDS`.
    pub prune_interval_secs: u64,
    /// How many entries one browser tab keeps; older ones are deleted as
    /// new ones arrive. `0` keeps every entry. Default 100, or
    /// `INERTIA_DEVTOOLS_LIMIT`.
    pub limit: usize,
    /// The gate ability that admits a request to the entry endpoints
    /// outside the `local` environment. `None` admits none there. Default
    /// `INERTIA_DEVTOOLS_GATE`.
    pub gate: Option<String>,
    /// Keys whose values are replaced by `[REDACTED]` before an entry is
    /// stored, at any depth of a body or prop value, and as query
    /// parameters of its URLs, compared without case.
    pub redact_keys: Vec<String>,
    /// Headers whose values are replaced by `[REDACTED]` before an entry
    /// is stored, compared without case.
    pub redact_headers: Vec<String>,
}

impl Default for DevToolsConfig {
    fn default() -> Self {
        Self {
            enabled: enabled_from_env(),
            except: ["_inertia/devtools*", "_suprnova/*"]
                .iter()
                .map(|pattern| pattern.to_string())
                .collect(),
            storage_path: crate::app::paths::storage_path("inertia-devtools"),
            ttl_hours: crate::config::env("INERTIA_DEVTOOLS_TTL_HOURS", 24),
            prune_interval_secs: crate::config::env("INERTIA_DEVTOOLS_PRUNE_INTERVAL_SECONDS", 300),
            limit: crate::config::env("INERTIA_DEVTOOLS_LIMIT", 100),
            gate: std::env::var("INERTIA_DEVTOOLS_GATE")
                .ok()
                .map(|gate| gate.trim().to_string())
                .filter(|gate| !gate.is_empty()),
            redact_keys: [
                "password",
                "password_confirmation",
                "current_password",
                "token",
                "_token",
                "access_token",
                "refresh_token",
                "secret",
                "client_secret",
                "api_key",
            ]
            .iter()
            .map(|key| key.to_string())
            .collect(),
            redact_headers: [
                "cookie",
                "set-cookie",
                "authorization",
                "proxy-authorization",
                "x-xsrf-token",
                "x-csrf-token",
            ]
            .iter()
            .map(|header| header.to_string())
            .collect(),
        }
    }
}

/// `INERTIA_DEVTOOLS_ENABLED`, read the way Laravel's `env()` and a boolean
/// cast read it: unset, empty or `null` leaves the decision to the
/// environment.
fn enabled_from_env() -> Option<bool> {
    let raw = std::env::var("INERTIA_DEVTOOLS_ENABLED").ok()?;
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "null" | "(null)" => None,
        "true" | "(true)" | "1" | "on" | "yes" => Some(true),
        "false" | "(false)" | "0" | "off" | "no" => Some(false),
        _ => {
            tracing::warn!(
                env_var = "INERTIA_DEVTOOLS_ENABLED",
                raw_value = %raw,
                "environment variable is not a boolean; leaving DevTools to the environment"
            );
            None
        }
    }
}

impl DevToolsConfig {
    /// The defaults, with the `INERTIA_DEVTOOLS_*` environment variables
    /// applied.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record requests (`true`) or never (`false`), whatever the
    /// environment.
    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = Some(on);
        self
    }

    /// Replace the paths that are never recorded; see
    /// [`except`](Self::except).
    pub fn except<I, S>(mut self, patterns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.except = patterns.into_iter().map(Into::into).collect();
        self
    }

    /// Store entries in `path`.
    pub fn storage_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.storage_path = path.into();
        self
    }

    /// Keep entries for `hours` hours.
    pub fn ttl_hours(mut self, hours: u64) -> Self {
        self.ttl_hours = hours;
        self
    }

    /// Prune expired entries at most once every `seconds` seconds; `0`
    /// prunes after every request.
    pub fn prune_interval_secs(mut self, seconds: u64) -> Self {
        self.prune_interval_secs = seconds;
        self
    }

    /// Keep the newest `limit` entries of each browser tab; `0` keeps
    /// every entry.
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Admit a request to the entry endpoints outside the `local`
    /// environment when the gate ability `ability` allows its user; see
    /// [`gate`](Self::gate).
    pub fn gate(mut self, ability: impl Into<String>) -> Self {
        self.gate = Some(ability.into());
        self
    }

    /// Replace the keys whose values are redacted.
    pub fn redact_keys<I, S>(mut self, keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.redact_keys = keys.into_iter().map(Into::into).collect();
        self
    }

    /// Replace the headers whose values are redacted.
    pub fn redact_headers<I, S>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.redact_headers = headers.into_iter().map(Into::into).collect();
        self
    }

    /// Whether requests are recorded: [`enabled`](Self::enabled) when it
    /// is set, else whether the application runs in the `local`
    /// environment, Laravel's `DevTools::enabled`.
    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or_else(|| {
            crate::config::Config::environment() == crate::config::Environment::Local
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indt_the_defaults_are_laravels() {
        let config = DevToolsConfig {
            enabled: None,
            ..DevToolsConfig::default()
        };
        assert_eq!(config.except, vec!["_inertia/devtools*", "_suprnova/*"]);
        assert!(config.storage_path.ends_with("inertia-devtools"));
        assert!(config.redact_keys.contains(&"password".to_string()));
        assert_eq!(config.redact_keys.len(), 10);
        assert_eq!(config.redact_headers.len(), 6);
        assert!(DevToolsConfig::new().enabled(true).is_enabled());
        assert!(!DevToolsConfig::new().enabled(false).is_enabled());
    }
}
