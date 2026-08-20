//! Session configuration

use std::time::Duration;

use crate::http::CookiePrefix;

/// Session configuration.
///
/// This struct is non-exhaustive so adding a session knob does not become a
/// breaking change; construct it with [`SessionConfig::default`] or
/// [`SessionConfig::from_env`] and assign individual fields.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SessionConfig {
    /// Session lifetime
    pub lifetime: Duration,
    /// Minimum interval between sliding-expiry persistence writes.
    pub touch_interval: Duration,
    /// Interval between supervised expired-session collection runs.
    pub gc_interval: Duration,
    /// Cookie name for the session ID
    pub cookie_name: String,
    /// Cookie path
    pub cookie_path: String,
    /// Cookie domain (e.g. `".example.com"`). Defaults to `None`
    /// (browser scopes the cookie to the request's host). Mirrors
    /// Laravel's `session.domain`.
    pub cookie_domain: Option<String>,
    /// Whether to set Secure flag on cookie (HTTPS only)
    pub cookie_secure: bool,
    /// Whether to set HttpOnly flag on cookie. Always `true` — we
    /// deliberately don't ship a knob to disable HttpOnly on the
    /// session cookie; there is no legitimate modern use case for it
    /// and disabling it forfeits the primary XSS protection.
    pub cookie_http_only: bool,
    /// SameSite attribute for the cookie
    pub cookie_same_site: String,
    /// Emit `Partitioned` (CHIPS) on the cookie. Defaults to `false`.
    /// Mirrors Laravel's `session.partitioned`.
    pub cookie_partitioned: bool,
    /// Cookie-name prefix (`__Host-` / `__Secure-`) applied to the
    /// session and remember-me cookies' **wire** names. The logical
    /// names (`cookie_name`, the remember constant) are what the
    /// encryption binds and what the code configures; the prefix is a
    /// rendering concern. `__Host-` requires `cookie_domain: None` and
    /// `cookie_path: "/"` - enforced at boot by `Config::init` and at
    /// render time by `Cookie::to_header_value`.
    pub cookie_prefix: CookiePrefix,
    /// When `true`, omit `Max-Age` from the session cookie so the
    /// browser drops it when the window closes (a "session cookie"
    /// in the HTTP sense). Defaults to `false`. Mirrors Laravel's
    /// `session.expire_on_close`.
    pub expire_on_close: bool,
    /// Database table name for sessions
    pub table_name: String,
    /// Optional named database connection for the session store.
    /// Defaults to `None` (uses the framework's default `DB::connection()`).
    /// Mirrors Laravel's `session.connection`. Today the
    /// [`crate::session::DatabaseSessionDriver`] reads the default
    /// connection regardless; this field is the wire for a future
    /// `DatabaseSessionDriver::with_connection(name)` ctor.
    pub connection: Option<String>,
    /// Lifetime of a remember-me token (and its cookie). Default 30 days.
    ///
    /// Configured separately from `lifetime` because remember-me is the
    /// "I closed my browser and want to come back next month" path —
    /// the session cookie itself is short-lived (default 2 hours).
    pub remember_lifetime: Duration,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            lifetime: Duration::from_secs(120 * 60), // 2 hours (120 minutes)
            touch_interval: Duration::from_secs(5 * 60),
            gc_interval: Duration::from_secs(60 * 60),
            cookie_name: "suprnova_session".to_string(),
            cookie_path: "/".to_string(),
            cookie_domain: None,
            cookie_secure: true,
            cookie_http_only: true,
            cookie_same_site: "Lax".to_string(),
            cookie_partitioned: false,
            cookie_prefix: CookiePrefix::None,
            expire_on_close: false,
            table_name: "sessions".to_string(),
            connection: None,
            remember_lifetime: Duration::from_secs(30 * 24 * 60 * 60), // 30 days
        }
    }
}

impl SessionConfig {
    /// Create a new session config with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Load session configuration from environment variables.
    ///
    /// Environment variables:
    /// - `SESSION_LIFETIME`: Session lifetime in minutes (default: 120)
    /// - `SESSION_TOUCH_INTERVAL`: Minimum sliding-expiry write interval in
    ///   seconds (default: 300)
    /// - `SESSION_GC_INTERVAL`: Expired-session collection interval in
    ///   seconds (default: 3600)
    /// - `SESSION_COOKIE`: Cookie name (default: `suprnova_session`)
    /// - `SESSION_SECURE`: Set `Secure` flag (default: `true`)
    /// - `SESSION_PATH`: Cookie path (default: `/`)
    /// - `SESSION_DOMAIN`: Cookie domain (default: unset)
    /// - `SESSION_SAME_SITE`: SameSite attribute (default: `Lax`)
    /// - `SESSION_PARTITIONED`: Emit `Partitioned` / CHIPS (default: `false`)
    /// - `SESSION_COOKIE_PREFIX`: `__Host-` / `__Secure-` / unset (default: unset)
    /// - `SESSION_EXPIRE_ON_CLOSE`: Drop `Max-Age` so the browser
    ///   forgets the cookie on close (default: `false`)
    /// - `SESSION_CONNECTION`: Named DB connection for the session
    ///   store (default: unset)
    /// - `REMEMBER_LIFETIME`: Remember-me token/cookie lifetime in
    ///   minutes (default: `43200` = 30 days)
    pub fn from_env() -> Self {
        fn bool_env(name: &str, default: bool) -> bool {
            crate::env_optional(name)
                .map(|s: String| {
                    let l = s.to_lowercase();
                    l == "true" || l == "1" || l == "yes"
                })
                .unwrap_or(default)
        }

        let lifetime_minutes: u64 = crate::env_optional("SESSION_LIFETIME")
            .and_then(|s: String| s.parse().ok())
            .unwrap_or(120);
        let touch_interval_seconds: u64 = crate::env_optional("SESSION_TOUCH_INTERVAL")
            .and_then(|s: String| s.parse().ok())
            .unwrap_or(5 * 60);
        let gc_interval_seconds: u64 = crate::env_optional("SESSION_GC_INTERVAL")
            .and_then(|s: String| s.parse().ok())
            .unwrap_or(60 * 60);

        let cookie_secure = bool_env("SESSION_SECURE", true);
        let cookie_partitioned = bool_env("SESSION_PARTITIONED", false);
        // Invalid values fall back to None here because from_env is
        // infallible and per-request; Config::init re-parses the same
        // variable at boot and fails loudly, so a typo cannot survive
        // to production silently.
        let cookie_prefix = crate::env_optional("SESSION_COOKIE_PREFIX")
            .and_then(|s: String| CookiePrefix::parse(&s))
            .unwrap_or_default();
        let expire_on_close = bool_env("SESSION_EXPIRE_ON_CLOSE", false);

        let remember_lifetime_minutes: u64 = crate::env_optional("REMEMBER_LIFETIME")
            .and_then(|s: String| s.parse().ok())
            .unwrap_or(30 * 24 * 60); // 30 days

        Self {
            lifetime: Duration::from_secs(lifetime_minutes * 60),
            touch_interval: Duration::from_secs(touch_interval_seconds.max(1)),
            gc_interval: Duration::from_secs(gc_interval_seconds.max(1)),
            cookie_name: crate::env_optional("SESSION_COOKIE")
                .unwrap_or_else(|| "suprnova_session".to_string()),
            cookie_path: crate::env_optional("SESSION_PATH").unwrap_or_else(|| "/".to_string()),
            cookie_domain: crate::env_optional("SESSION_DOMAIN"),
            cookie_secure,
            cookie_http_only: true, // Always true for security
            cookie_same_site: crate::env_optional("SESSION_SAME_SITE")
                .unwrap_or_else(|| "Lax".to_string()),
            cookie_partitioned,
            cookie_prefix,
            expire_on_close,
            table_name: "sessions".to_string(),
            connection: crate::env_optional("SESSION_CONNECTION"),
            remember_lifetime: Duration::from_secs(remember_lifetime_minutes * 60),
        }
    }

    /// Set the session lifetime
    pub fn lifetime(mut self, duration: Duration) -> Self {
        self.lifetime = duration;
        self
    }

    /// Set the minimum interval between sliding-expiry writes.
    pub fn touch_interval(mut self, duration: Duration) -> Self {
        self.touch_interval = duration.max(Duration::from_secs(1));
        self
    }

    /// Set the interval between supervised expired-session collection runs.
    pub fn gc_interval(mut self, duration: Duration) -> Self {
        self.gc_interval = duration.max(Duration::from_secs(1));
        self
    }

    /// Set the cookie name
    pub fn cookie_name(mut self, name: impl Into<String>) -> Self {
        self.cookie_name = name.into();
        self
    }

    /// Set whether the cookie should be secure (HTTPS only)
    pub fn secure(mut self, secure: bool) -> Self {
        self.cookie_secure = secure;
        self
    }

    /// Set the remember-me token/cookie lifetime.
    pub fn remember_lifetime(mut self, duration: Duration) -> Self {
        self.remember_lifetime = duration;
        self
    }

    /// Set the cookie domain (e.g. `".example.com"`).
    pub fn domain(mut self, domain: impl Into<String>) -> Self {
        self.cookie_domain = Some(domain.into());
        self
    }

    /// Mark the cookie as Partitioned (CHIPS).
    pub fn partitioned(mut self, value: bool) -> Self {
        self.cookie_partitioned = value;
        self
    }

    /// Drop `Max-Age` so the browser forgets the cookie when the
    /// window closes.
    pub fn expire_on_close(mut self, value: bool) -> Self {
        self.expire_on_close = value;
        self
    }

    /// Set the named DB connection used by the session store.
    pub fn connection(mut self, name: impl Into<String>) -> Self {
        self.connection = Some(name.into());
        self
    }
}
