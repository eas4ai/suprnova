//! The translation seam: one trait, drivers behind it.

use super::locale::Locale;
use crate::error::FrameworkError;
use crate::validation::message::TranslateArgs;
use std::sync::Arc;

/// A locale's merged catalog source and its content hash - consumed by
/// the `/_suprnova/lang/<locale>.ftl` endpoint and the Inertia share.
#[derive(Debug, Clone)]
pub struct CatalogSource {
    /// The serialized, chain-flattened catalog: the locale's fallback
    /// parent chain, the embedded framework catalog (for `en`/`en-*`),
    /// and its own app files, folded into one AST-merged resource and
    /// re-serialized.
    pub text: Arc<str>,
    /// Hex content hash; doubles as the ETag and cache-buster.
    pub hash: String,
}

/// A message translator. One driver ships ([`FluentTranslator`](super::FluentTranslator));
/// the trait is the extension seam for alternative backends.
pub trait Translator: Send + Sync {
    /// Translate `key` for `locale`. Missing key or locale is an `Err` -
    /// the *fallback chain* (current → configured parents
    /// (`APP_LOCALE_PARENTS`) → fallback → key) belongs to the `Lang`
    /// facade, not to drivers.
    fn translate(
        &self,
        locale: &Locale,
        key: &str,
        args: &TranslateArgs,
    ) -> Result<String, FrameworkError>;

    /// Whether `locale`'s catalog defines `key`.
    fn has(&self, locale: &Locale, key: &str) -> bool;

    /// Locales with a loaded catalog.
    fn available_locales(&self) -> Vec<Locale>;

    /// The merged catalog for `locale`, if loaded. Custom drivers should
    /// return chain-complete text - the framework's own driver serves
    /// catalogs already flattened across the locale's parent chain; a
    /// driver that hands back only the locale's own keys leaves the
    /// browser unable to resolve any key the served locale inherits rather
    /// than declares.
    fn catalog(&self, locale: &Locale) -> Option<CatalogSource>;

    /// Re-read catalogs from disk (dev hot-reload).
    fn reload(&self) -> Result<(), FrameworkError>;

    /// Re-read catalogs from disk only if they changed since the last
    /// load - the dev-mode hot-reload hook `LocaleMiddleware` calls on
    /// every request in `Local`/`Development`. Defaults to `Ok(false)`
    /// (never stale, nothing reloaded) so a custom driver only needs to
    /// implement staleness detection if it wants the hook to do
    /// anything; a driver with no on-disk source (e.g. one backed by a
    /// database or a remote service) can leave the default in place.
    fn reload_if_stale(&self) -> Result<bool, FrameworkError> {
        Ok(false)
    }
}
