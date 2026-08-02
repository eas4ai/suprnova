//! First-class localization: Fluent message catalogs, per-request locale
//! detection, translated validation messages, and ICU-backed formatting.
//!
//! See `manual/localization.md`. Feature-gated (`localization`,
//! default-on); with the feature off, validation renders its embedded
//! English fallbacks and nothing here compiles.

mod config;
mod fluent;
mod locale;
mod translator;

pub use config::{Detect, LocalizationConfig};
pub use fluent::FluentTranslator;
pub use locale::{Locale, negotiate};
pub use translator::{CatalogSource, Translator};

use crate::config::Config;
use crate::container::App;
use crate::error::FrameworkError;
use crate::validation::message::TranslateArgs;
use std::collections::HashSet;
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

tokio::task_local! {
    /// Per-request current locale. Bound by `LocaleMiddleware` /
    /// `scope_locale`; interior-mutable (`RwLock`, not a plain `Locale`)
    /// so `Lang::set_locale` can switch it mid-request (Laravel's
    /// `App::setLocale`) without re-scoping the future.
    pub static CURRENT_LOCALE: Arc<RwLock<Locale>>;
}

/// Process-global locale override for non-request contexts (console
/// commands, queue workers) where no future is ever scoped with
/// [`scope_locale`]. Consulted after the task-local, before the
/// configured default.
static GLOBAL_LOCALE: OnceLock<RwLock<Option<Locale>>> = OnceLock::new();

/// Config snapshot captured by the first `Localization::bootstrap()`
/// call. `Lang`'s hot path (`locale`/`has`/`get_with`) reads the default
/// and fallback locale on every call; once bootstrapped, this avoids
/// re-reading `Config`/env on each of those calls. Before bootstrap runs
/// (e.g. tests that bind a `Translator` directly without going through
/// `Localization::bootstrap`), [`resolved_config`] falls back to a fresh
/// `Config::get`/`from_env` read each time — correct, just not memoized.
static LOCALIZATION_CONFIG: OnceLock<LocalizationConfig> = OnceLock::new();

/// Dedup set for `Lang::get`/`Lang::get_with`'s "translation missing in
/// current and fallback locale" warning — logged at most once per key
/// per process, so a hot path repeatedly hitting the same missing key
/// doesn't spam `tracing::warn!`.
fn warned_missing_keys() -> &'static Mutex<HashSet<String>> {
    static WARNED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    WARNED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Resolve the effective `LocalizationConfig`: the bootstrap snapshot if
/// [`Localization::bootstrap`] has run, else `Config::get` /
/// `LocalizationConfig::from_env()`, else the hard `en`/`en` default. A
/// malformed `APP_LOCALE`/`APP_FALLBACK_LOCALE` must not make `Lang`
/// panic or become unusable, so it falls back silently here — callers
/// who invoke `LocalizationConfig::from_env()` directly still get the
/// loud `Err`.
fn resolved_config() -> LocalizationConfig {
    if let Some(c) = LOCALIZATION_CONFIG.get() {
        return c.clone();
    }
    match Config::get::<LocalizationConfig>() {
        Some(c) => c,
        None => LocalizationConfig::from_env().unwrap_or_else(|_| LocalizationConfig {
            default_locale: Locale::fallback_en(),
            fallback_locale: Locale::fallback_en(),
            use_isolating: false,
            detection: Vec::new(),
            session_key: "locale".into(),
            cookie_name: "locale".into(),
        }),
    }
}

fn global_locale() -> Option<Locale> {
    GLOBAL_LOCALE
        .get()?
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

fn set_global_locale(locale: Locale) {
    let lock = GLOBAL_LOCALE.get_or_init(|| RwLock::new(None));
    *lock.write().unwrap_or_else(|e| e.into_inner()) = Some(locale);
}

/// Scope a current locale around a future. Mirrors `scope_include_set`
/// (`data/include_set.rs`): installs a task-local [`Locale`] for the
/// duration of `f`, so nested `Lang::locale()` / `Lang::get()` /
/// `__!` calls see it without threading it through every signature.
/// `LocaleMiddleware` uses this to bind the per-request locale; tests
/// and console commands can use it directly.
///
/// ```rust,no_run
/// # use suprnova::{Lang, Locale, scope_locale};
/// # async fn ex() {
/// scope_locale(Locale::parse("es").unwrap(), async {
///     assert_eq!(Lang::locale().as_str(), "es");
/// })
/// .await;
/// # }
/// ```
pub async fn scope_locale<F: Future>(locale: Locale, f: F) -> F::Output {
    CURRENT_LOCALE.scope(Arc::new(RwLock::new(locale)), f).await
}

/// Bootstraps the localization subsystem: builds a `FluentTranslator`
/// from the application's `lang/` directory and binds it as `dyn
/// Translator` in the container. Called from `Server::run` and the
/// non-server subcommand bootstraps, beside `Cache::bootstrap`.
pub struct Localization;

impl Localization {
    /// Bind the default `FluentTranslator`, unless a `dyn Translator` is
    /// already bound — an app-level `bootstrap_fn` override (a custom
    /// driver, a pre-seeded translator for tests) always wins, the same
    /// "respect app overrides" contract `Cache::bootstrap` follows.
    ///
    /// A missing `lang/` directory is not an error: `FluentTranslator`
    /// still boots successfully with only the embedded framework
    /// catalogs (English).
    pub(crate) async fn bootstrap() -> Result<(), FrameworkError> {
        if App::resolve_make::<dyn Translator>().is_ok() {
            return Ok(());
        }

        let config = match Config::get::<LocalizationConfig>() {
            Some(c) => c,
            None => LocalizationConfig::from_env()?,
        };

        let translator = FluentTranslator::from_dir(&crate::app::paths::lang_path(""), &config)?;
        App::bind::<dyn Translator>(Arc::new(translator));
        // Best-effort: if another task raced us and already set this,
        // keep theirs — both snapshots came from the same config source.
        let _ = LOCALIZATION_CONFIG.set(config);
        Ok(())
    }
}

/// Laravel-style translation facade. `Lang::get(key)` (or the `__!`
/// macro) resolves a Fluent message key against the current locale,
/// falls back to the configured fallback locale on a miss, and finally
/// falls back to the key itself so a missing translation never crashes
/// a page — `Lang::try_get`/`try_get_with` are the `Result`-returning
/// siblings for callers that want to detect the miss instead.
pub struct Lang;

impl Lang {
    /// The locale in effect for the current call. Resolution order: the
    /// task-local locale ([`scope_locale`] / `LocaleMiddleware`), the
    /// process-global override (`Lang::set_locale` outside a scope),
    /// then the configured default locale. Never panics — a malformed
    /// env default falls back to the hard-coded `en`.
    pub fn locale() -> Locale {
        if let Ok(l) =
            CURRENT_LOCALE.try_with(|lock| lock.read().unwrap_or_else(|e| e.into_inner()).clone())
        {
            return l;
        }
        if let Some(l) = global_locale() {
            return l;
        }
        resolved_config().default_locale
    }

    /// Set the current locale. Inside a [`scope_locale`]d future (a
    /// request, typically), this mutates the task-local slot so the
    /// switch is visible for the rest of that scope — Laravel's
    /// `App::setLocale` called mid-request. Outside a scope (console
    /// commands, queue workers), it sets the process-global override
    /// consulted by [`Lang::locale`].
    pub fn set_locale(locale: Locale) {
        let scoped = CURRENT_LOCALE.try_with(|lock| {
            *lock.write().unwrap_or_else(|e| e.into_inner()) = locale.clone();
        });
        if scoped.is_err() {
            set_global_locale(locale);
        }
    }

    /// Translate `key` for the current locale, falling back to the
    /// fallback locale and finally to the key itself — never panics,
    /// never errs. Equivalent to `__!(key)`. Logs a `tracing::warn!`
    /// (once per key per process) when the key resolves nowhere.
    pub fn get(key: &str) -> String {
        Self::get_with(key, TranslateArgs::new())
    }

    /// [`Lang::get`] with interpolation arguments. Equivalent to
    /// `__!(key, name: value, ...)`.
    pub fn get_with(key: &str, args: TranslateArgs) -> String {
        match Self::try_get_with(key, args) {
            Ok(s) => s,
            Err(_) => {
                let mut warned = warned_missing_keys().lock().unwrap_or_else(|e| e.into_inner());
                if warned.insert(key.to_string()) {
                    tracing::warn!(
                        key,
                        "Lang: translation key missing in current and fallback locale; returning the key as-is"
                    );
                }
                key.to_string()
            }
        }
    }

    /// [`Lang::get`], but `Err` on a missing `Translator` binding or a
    /// key missing from both the current and fallback locale, instead
    /// of silently returning the key.
    pub fn try_get(key: &str) -> Result<String, FrameworkError> {
        Self::try_get_with(key, TranslateArgs::new())
    }

    /// [`Lang::try_get`] with interpolation arguments. Tries the current
    /// locale first; on a miss, retries the fallback locale; only errs
    /// once both have missed.
    pub fn try_get_with(key: &str, args: TranslateArgs) -> Result<String, FrameworkError> {
        let translator = App::resolve_make::<dyn Translator>()?;
        let current = Self::locale();
        match translator.translate(&current, key, &args) {
            Ok(s) => Ok(s),
            Err(_) => {
                let fallback = resolved_config().fallback_locale;
                translator.translate(&fallback, key, &args)
            }
        }
    }

    /// Whether `key` resolves for the current locale *or* its fallback —
    /// i.e. whether [`Lang::get`] would return a real translation rather
    /// than the bare key. Chain-aware: a key defined only in the
    /// fallback catalog still counts, because `Lang::get`/`try_get`
    /// would still translate it via the fallback step. Returns `false`
    /// (never panics) when no `Translator` is bound.
    pub fn has(key: &str) -> bool {
        let Ok(translator) = App::resolve_make::<dyn Translator>() else {
            return false;
        };
        let current = Self::locale();
        if translator.has(&current, key) {
            return true;
        }
        let fallback = resolved_config().fallback_locale;
        translator.has(&fallback, key)
    }

    /// Locales with a loaded catalog. Empty if no `Translator` is bound.
    pub fn available_locales() -> Vec<Locale> {
        App::resolve_make::<dyn Translator>()
            .map(|t| t.available_locales())
            .unwrap_or_default()
    }
}

/// Translate a Fluent message key for the current locale — the
/// ergonomic entry point to [`Lang::get`] / [`Lang::get_with`].
/// `suprnova::__!("key")` calls `Lang::get`; `suprnova::__!("key", name:
/// value, ...)` builds a `TranslateArgs` from the named arguments and
/// calls `Lang::get_with`. Named after Laravel's `__()` helper — `_`
/// alone is Rust's ignore pattern, so it can't be a macro name.
///
/// ```rust,no_run
/// # use suprnova::__;
/// # fn ex() -> String {
/// __!("welcome")
/// # }
/// # fn ex_args() -> String {
/// __!("welcome", name: "Ada")
/// # }
/// ```
#[macro_export]
macro_rules! __ {
    ($key:expr) => {
        $crate::Lang::get($key)
    };
    ($key:expr, $($name:ident : $value:expr),+ $(,)?) => {{
        let mut args = $crate::TranslateArgs::new();
        $( args.insert(stringify!($name).to_string(), ::serde_json::Value::from($value)); )+
        $crate::Lang::get_with($key, args)
    }};
}
