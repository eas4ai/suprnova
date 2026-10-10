//! Authentication configuration - Laravel's `config/auth.php`.
//!
//! Declares the *wiring*: the default guard, and each named guard's
//! driver + which provider it resolves users through. It is the
//! declarative half a Laravel developer expects.
//!
//! The one Rust-native concession: provider **instances** are registered
//! programmatically ([`crate::Auth::register_provider`]) rather than
//! string-instantiated from the config, because a Suprnova user provider
//! carries a Rust type (e.g. an Eloquent model) that a data-only config
//! cannot name. The config still selects *which* registered provider a
//! guard uses, by name.
//!
//! ```rust,no_run
//! use suprnova::{AuthConfig, GuardConfig, GuardDriver};
//! # fn ex() {
//! // The defaults: a `web` session guard and an `api` token guard, both
//! // backed by the `users` provider. Override the default via AUTH_GUARD.
//! let config = AuthConfig::from_env();
//!
//! // Or build one explicitly:
//! let config = AuthConfig::new("web")
//!     .guard("web", GuardConfig::session("users"))
//!     .guard("admin", GuardConfig::session("admins"))
//!     // A driver the application registers with `Auth::extend("api_key", ...)`.
//!     .guard("partner", GuardConfig::custom("api_key", "partners"));
//! # }
//! ```

use std::collections::HashMap;

/// The kind of guard a named guard entry uses.
///
/// Mirrors Laravel's `'driver'` entry: `'session'`, `'token'`, or a driver
/// the application registers with `Auth::extend`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardDriver {
    /// Session-backed, stateful (login/logout persist). [`crate::SessionGuard`].
    Session,
    /// Bearer-token, stateless (read-only). [`crate::auth::TokenGuard`].
    Token,
    /// A driver the application registers by this name with
    /// [`crate::Auth::extend`], for a guard the framework does not ship (an
    /// API key in a header, a client certificate, a single sign-on cookie).
    /// Declared in code with [`GuardConfig::custom`], so the environment can
    /// never select an application guard by accident.
    Custom(String),
}

impl GuardDriver {
    /// Parse a Laravel-style driver string (`"session"` / `"token"`),
    /// case-insensitively. Unknown values fall back to [`GuardDriver::Session`].
    ///
    /// Never yields [`GuardDriver::Custom`]: a custom driver is declared in
    /// code with [`GuardConfig::custom`], never parsed from a string.
    pub fn from_str_lenient(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "token" => GuardDriver::Token,
            _ => GuardDriver::Session,
        }
    }
}

/// One named guard's configuration: its driver and the name of the
/// provider it resolves users through.
#[derive(Debug, Clone)]
pub struct GuardConfig {
    /// Which guard implementation backs this entry.
    pub driver: GuardDriver,
    /// The name of the registered [`crate::UserProvider`] this guard uses.
    pub provider: String,
}

impl GuardConfig {
    /// A session (stateful) guard backed by `provider`.
    pub fn session(provider: impl Into<String>) -> Self {
        Self {
            driver: GuardDriver::Session,
            provider: provider.into(),
        }
    }

    /// A token (stateless) guard backed by `provider`.
    pub fn token(provider: impl Into<String>) -> Self {
        Self {
            driver: GuardDriver::Token,
            provider: provider.into(),
        }
    }

    /// A guard of the application-registered `driver`, backed by `provider`.
    ///
    /// The driver's factory, registered with [`crate::Auth::extend`], builds
    /// the guard from this entry's name and provider. Resolving the guard
    /// before the factory is registered is an error, never a fallback to a
    /// built-in driver.
    pub fn custom(driver: impl Into<String>, provider: impl Into<String>) -> Self {
        Self {
            driver: GuardDriver::Custom(driver.into()),
            provider: provider.into(),
        }
    }
}

/// The application's authentication configuration.
///
/// Holds the default guard name and the set of named guards. Provider
/// instances are registered separately on the [`crate::auth::AuthManager`].
#[derive(Debug, Clone)]
pub struct AuthConfig {
    /// The guard used by the bare `Auth` facade and by `Auth::guard` /
    /// `Auth::stateful_guard` when no name is given.
    pub default_guard: String,
    /// Named guards, keyed by guard name (e.g. `"web"`, `"api"`).
    pub guards: HashMap<String, GuardConfig>,
}

impl Default for AuthConfig {
    /// The conventional default: a `web` session guard and an `api` token
    /// guard, both backed by a `users` provider, defaulting to `web`.
    fn default() -> Self {
        let mut guards = HashMap::new();
        guards.insert("web".to_string(), GuardConfig::session("users"));
        guards.insert("api".to_string(), GuardConfig::token("users"));
        Self {
            default_guard: "web".to_string(),
            guards,
        }
    }
}

impl AuthConfig {
    /// Start from the conventional defaults with an explicit default guard.
    pub fn new(default_guard: impl Into<String>) -> Self {
        Self {
            default_guard: default_guard.into(),
            ..Self::default()
        }
    }

    /// Load the configuration from the environment.
    ///
    /// Only the **default guard** is env-driven (`AUTH_GUARD`, default
    /// `web`) - mirroring Laravel, where guards live in code and only the
    /// default is environment-selectable. Apps that need additional or
    /// renamed guards build an [`AuthConfig`] explicitly.
    pub fn from_env() -> Self {
        let default_guard = crate::env_optional("AUTH_GUARD").unwrap_or_else(|| "web".to_string());
        Self::new(default_guard)
    }

    /// Add or replace a named guard (builder style).
    pub fn guard(mut self, name: impl Into<String>, config: GuardConfig) -> Self {
        self.guards.insert(name.into(), config);
        self
    }

    /// Look up a named guard's configuration.
    pub fn guard_config(&self, name: &str) -> Option<&GuardConfig> {
        self.guards.get(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_web_session_and_api_token() {
        let config = AuthConfig::default();
        assert_eq!(config.default_guard, "web");
        assert_eq!(
            config.guard_config("web").unwrap().driver,
            GuardDriver::Session
        );
        assert_eq!(config.guard_config("web").unwrap().provider, "users");
        assert_eq!(
            config.guard_config("api").unwrap().driver,
            GuardDriver::Token
        );
        assert!(config.guard_config("missing").is_none());
    }

    #[test]
    fn builder_adds_named_guards() {
        let config = AuthConfig::new("admin").guard("admin", GuardConfig::session("admins"));
        assert_eq!(config.default_guard, "admin");
        assert_eq!(
            config.guard_config("admin").unwrap().provider,
            "admins".to_string()
        );
        // Defaults are still present unless overridden.
        assert!(config.guard_config("web").is_some());
    }

    #[test]
    fn driver_parse_is_lenient() {
        assert_eq!(GuardDriver::from_str_lenient("token"), GuardDriver::Token);
        assert_eq!(GuardDriver::from_str_lenient("TOKEN"), GuardDriver::Token);
        assert_eq!(
            GuardDriver::from_str_lenient("session"),
            GuardDriver::Session
        );
        assert_eq!(GuardDriver::from_str_lenient("weird"), GuardDriver::Session);
    }

    #[test]
    fn custom_guard_names_its_driver_and_provider() {
        let entry = GuardConfig::custom("api_key", "partners");
        let config = AuthConfig::new("web").guard("partner", entry);
        let partner = config.guard_config("partner").unwrap();
        assert_eq!(partner.driver, GuardDriver::Custom("api_key".to_string()));
        assert_eq!(partner.provider, "partners");
    }

    #[test]
    fn driver_parse_never_yields_a_custom_driver() {
        assert_eq!(
            GuardDriver::from_str_lenient("api_key"),
            GuardDriver::Session
        );
    }
}
