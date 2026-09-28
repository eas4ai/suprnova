//! [`LocaleMiddleware`] - the per-request locale detection chain.

use super::config::{Detect, LocalizationConfig};
use super::locale::{Locale, negotiate};
use super::translator::Translator;
use crate::config::Environment;
use crate::container::App;
use crate::error::FrameworkError;
use crate::http::{Request, Response};
use crate::middleware::{Middleware, Next};
use async_trait::async_trait;

/// Detects the request's locale and scopes it for the rest of the
/// request via [`scope_locale`](super::scope_locale), so `Lang::get` /
/// the `__!` macro resolve against it anywhere downstream.
///
/// Detection runs the configured [`LocalizationConfig::detection`]
/// chain in order - `Session` then `Cookie` then `Header` by default -
/// and returns the first candidate that both parses as a [`Locale`]
/// *and* has a loaded catalog (per [`Translator::available_locales`]).
/// A candidate that fails either check (a garbage cookie, a locale with
/// no catalog) is skipped silently rather than erroring - a bad client
/// value must never turn into a 500. When nothing in the chain hits,
/// the request falls back to [`LocalizationConfig::default_locale`].
///
/// If no `dyn Translator` is bound in the container, the middleware is
/// a no-op pass-through: `Lang` still works (it falls back to the
/// configured default locale on its own), it just isn't scoped to a
/// per-request detected value.
///
/// ```rust,no_run
/// use suprnova::{LocaleMiddleware, LocalizationConfig};
///
/// let middleware = LocaleMiddleware::new(LocalizationConfig::from_env().unwrap());
/// ```
pub struct LocaleMiddleware {
    config: LocalizationConfig,
}

impl LocaleMiddleware {
    /// Build from an explicit config - the programmatic path for apps
    /// that construct [`LocalizationConfig`] via its builder rather
    /// than environment variables.
    pub fn new(config: LocalizationConfig) -> Self {
        Self { config }
    }

    /// Build from `APP_LOCALE` / `APP_FALLBACK_LOCALE` and the default
    /// detection chain, same as [`LocalizationConfig::from_env`]. Fails
    /// loudly on a malformed env value rather than silently degrading -
    /// callers that want the silent-fallback behavior should use
    /// [`LocalizationConfig::from_env`]'s already-resolved config with
    /// [`LocaleMiddleware::new`] instead.
    pub fn from_env() -> Result<Self, FrameworkError> {
        Ok(Self::new(LocalizationConfig::from_env()?))
    }

    /// Run the detection chain against `request`. `translator` supplies
    /// `available_locales()`, the set every `Session`/`Cookie` candidate
    /// is validated against and every `Header` candidate is negotiated
    /// against.
    fn detect(&self, request: &Request, translator: &dyn Translator) -> Locale {
        let available = translator.available_locales();
        for source in &self.config.detection {
            let hit = match source {
                Detect::Session => crate::session::session()
                    .and_then(|session| session.get::<String>(&self.config.session_key))
                    .and_then(|raw| Locale::parse(&raw).ok())
                    .filter(|locale| available.contains(locale)),
                Detect::Cookie => request
                    .cookie(&self.config.cookie_name)
                    .and_then(|raw| Locale::parse(&raw).ok())
                    .filter(|locale| available.contains(locale)),
                Detect::Header => request
                    .header("Accept-Language")
                    .and_then(|header| negotiate(header, &available)),
            };
            if let Some(locale) = hit {
                return locale;
            }
        }
        self.config.default_locale.clone()
    }
}

#[async_trait]
impl Middleware for LocaleMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        if let Ok(translator) = App::resolve_make::<dyn Translator>() {
            // Dev-mode hot reload: pick up catalog edits without a
            // restart. Best-effort - a reload failure (e.g. a
            // momentarily malformed `.ftl` mid-save) must never turn a
            // page request into a 500, so the error is dropped.
            if matches!(
                Environment::detect(),
                Environment::Local | Environment::Development
            ) {
                let _ = translator.reload_if_stale();
            }
            let locale = self.detect(&request, translator.as_ref());
            return crate::localization::scope_locale(locale, next(request)).await;
        }
        // No translator bound: pass through unscoped rather than fail
        // the request. `Lang` degrades gracefully on its own (falls
        // back to the configured default), so this is a safe no-op.
        next(request).await
    }
}
