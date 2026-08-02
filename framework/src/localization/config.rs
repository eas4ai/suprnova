//! Localization configuration — env-driven with a programmatic builder,
//! same shape as `SessionConfig` / `CacheConfig`.

use super::locale::Locale;
use crate::error::FrameworkError;

/// One source the locale middleware consults, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detect {
    /// The session key (default name `locale`).
    Session,
    /// The cookie (default name `locale`).
    Cookie,
    /// `Accept-Language`, negotiated against the available catalogs.
    Header,
}

/// Configuration for the localization subsystem.
#[derive(Debug, Clone)]
pub struct LocalizationConfig {
    /// Locale used when detection finds nothing. Env: `APP_LOCALE`.
    pub default_locale: Locale,
    /// Locale consulted when a key is missing from the current locale's
    /// catalog. Env: `APP_FALLBACK_LOCALE`.
    pub fallback_locale: Locale,
    /// Whether Fluent wraps interpolations in Unicode isolation marks
    /// (U+2068/U+2069). Off by default — see the manual's divergence
    /// note; turn on when shipping RTL locales.
    pub use_isolating: bool,
    /// Detection order; first hit wins.
    pub detection: Vec<Detect>,
    /// Session key holding a locale override.
    pub session_key: String,
    /// Cookie name holding a locale override.
    pub cookie_name: String,
}

impl LocalizationConfig {
    /// Build from `APP_LOCALE` / `APP_FALLBACK_LOCALE`, defaulting both
    /// to `en`. Fails loudly on a malformed locale value.
    pub fn from_env() -> Result<Self, FrameworkError> {
        let default_locale =
            Locale::parse(&std::env::var("APP_LOCALE").unwrap_or_else(|_| "en".into()))?;
        let fallback_locale =
            Locale::parse(&std::env::var("APP_FALLBACK_LOCALE").unwrap_or_else(|_| "en".into()))?;
        Ok(Self {
            default_locale,
            fallback_locale,
            use_isolating: false,
            detection: vec![Detect::Session, Detect::Cookie, Detect::Header],
            session_key: "locale".into(),
            cookie_name: "locale".into(),
        })
    }

    /// Override the default locale.
    pub fn default_locale(mut self, locale: Locale) -> Self {
        self.default_locale = locale;
        self
    }

    /// Override the fallback locale.
    pub fn fallback_locale(mut self, locale: Locale) -> Self {
        self.fallback_locale = locale;
        self
    }

    /// Turn Unicode isolation marks on/off (default off).
    pub fn use_isolating(mut self, on: bool) -> Self {
        self.use_isolating = on;
        self
    }

    /// Replace the detection chain.
    pub fn detection(mut self, order: Vec<Detect>) -> Self {
        self.detection = order;
        self
    }

    /// Rename the session key consulted for a locale override.
    pub fn session_key(mut self, key: impl Into<String>) -> Self {
        self.session_key = key.into();
        self
    }

    /// Rename the cookie consulted for a locale override.
    pub fn cookie_name(mut self, name: impl Into<String>) -> Self {
        self.cookie_name = name.into();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_overrides_env_defaults() {
        // No env manipulation: construct the default directly to stay
        // parallel-safe, then exercise the builder.
        let cfg = LocalizationConfig {
            default_locale: Locale::parse("en").unwrap(),
            fallback_locale: Locale::parse("en").unwrap(),
            use_isolating: false,
            detection: vec![Detect::Session, Detect::Cookie, Detect::Header],
            session_key: "locale".into(),
            cookie_name: "locale".into(),
        }
        .default_locale(Locale::parse("es").unwrap())
        .use_isolating(true)
        .detection(vec![Detect::Header]);
        assert_eq!(cfg.default_locale.as_str(), "es");
        assert!(cfg.use_isolating);
        assert_eq!(cfg.detection, vec![Detect::Header]);
    }
}
