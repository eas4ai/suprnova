//! First-class localization: Fluent message catalogs, per-request locale
//! detection, translated validation messages, and ICU-backed formatting.
//!
//! See `manual/localization.md`. Feature-gated (`localization`,
//! default-on); with the feature off, validation renders its embedded
//! English fallbacks and nothing here compiles.

mod config;
mod fluent;
mod format;
mod functions;
mod locale;
mod merge;
mod middleware;
mod translator;

pub use config::{Detect, LocalizationConfig};
pub use fluent::FluentTranslator;
pub use format::{DateStyle, ListStyle, RelativeUnit, TimeStyle};
pub use locale::{Locale, negotiate};
pub use middleware::LocaleMiddleware;
pub use translator::{CatalogSource, Translator};

use crate::config::Config;
use crate::container::App;
use crate::error::FrameworkError;
use crate::inertia::{InertiaRequestExt, InertiaSharedData, Prop};
use crate::validation::message::TranslateArgs;
use async_trait::async_trait;
use chrono::NaiveDateTime;
use indexmap::IndexMap;
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
            parents: Default::default(),
        }),
    }
}

/// Locales consulted *after* `current` when resolving a key: `current`'s
/// configured fallback parents ([`LocalizationConfig::parents`]), walked
/// transitively (`pt-PT` -> `pt-BR` -> ...), followed by
/// `config.fallback_locale` unless it already appears earlier in that
/// walk (a parent chain that terminates at the fallback, or that names
/// the fallback directly, must not list it twice).
///
/// Guarded against cycles with a `visited` set pre-seeded with `current`:
/// `LocalizationConfig::parents` is `pub`, so a caller can hand-build a
/// cyclic map bypassing `parse_parents`'s rejection (`config.rs`'s
/// `parse_parents_rejects_cycles` documents the same concern for the
/// env-parsed path). Revisiting an already-seen locale stops the walk
/// there rather than looping forever — the chain is simply truncated to
/// whatever it collected before the repeat, then the fallback is
/// appended per the dedup rule above.
pub(crate) fn fallback_chain(current: &Locale, config: &LocalizationConfig) -> Vec<Locale> {
    let mut chain = Vec::new();
    // `visited` borrows from `current` and `config`, which both outlive
    // this body — so the walk clones only what it actually returns.
    let mut visited: HashSet<&Locale> = HashSet::new();
    visited.insert(current);

    let mut cursor = current;
    while let Some(parent) = config.parents.get(cursor) {
        if !visited.insert(parent) {
            break;
        }
        chain.push(parent.clone());
        cursor = parent;
    }

    if !visited.contains(&config.fallback_locale) {
        chain.push(config.fallback_locale.clone());
    }

    chain
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

        let translator = FluentTranslator::from_dir(crate::app::paths::lang_path(""), &config)?;
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

    /// Translate `key` for the current locale, walking its fallback
    /// chain on a miss — the current locale's configured parents
    /// ([`LocalizationConfig::parents`]), transitively, then the global
    /// fallback locale — and finally falling back to the key itself if
    /// nothing in the chain resolves it. Never panics, never errs.
    /// Equivalent to `__!(key)`. Logs a `tracing::warn!` (once per key
    /// per process) when the key resolves nowhere in the chain.
    pub fn get(key: &str) -> String {
        Self::get_with(key, TranslateArgs::new())
    }

    /// [`Lang::get`] with interpolation arguments. Equivalent to
    /// `__!(key, name: value, ...)`.
    pub fn get_with(key: &str, args: TranslateArgs) -> String {
        match Self::try_get_with(key, args) {
            Ok(s) => s,
            Err(_) => {
                let mut warned = warned_missing_keys()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
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
    /// key missing from the current locale and its entire fallback chain
    /// (parents, transitively, then the global fallback), instead of
    /// silently returning the key.
    pub fn try_get(key: &str) -> Result<String, FrameworkError> {
        Self::try_get_with(key, TranslateArgs::new())
    }

    /// [`Lang::try_get`] with interpolation arguments. Tries the current
    /// locale first, then walks the fallback chain: the current locale's
    /// configured parents ([`LocalizationConfig::parents`]), transitively,
    /// then the global `fallback_locale`. Returns the first hit; errs
    /// (with the last locale's error) only once every step has missed.
    ///
    /// This walk runs regardless of which `Translator` is bound, even
    /// though [`FluentTranslator`] already serves chain-flattened
    /// catalogs (each locale's served catalog is pre-merged with its
    /// parent chain — see `localization/fluent.rs`). That is deliberate
    /// double-cover, not redundancy to trim: the flattening is a
    /// `FluentTranslator`-specific optimization, but the `Translator`
    /// trait itself makes no such promise, so a custom driver that never
    /// flattens (a database-backed translator, a remote service) still
    /// needs the facade to walk the chain on its behalf, or chains would
    /// silently stop working for any driver but the default one. For
    /// `FluentTranslator` specifically, the earlier chain steps here are
    /// redundant-but-harmless in practice — the current locale's already-
    /// flattened catalog contains the parents' keys, so this loop
    /// short-circuits on the first `translate` call — but that is an
    /// implementation detail of one driver, not a property of the trait.
    /// Do not "optimize" this walk away in favor of relying on
    /// `FluentTranslator`'s flattening; that would silently break chains
    /// for every other driver.
    pub fn try_get_with(key: &str, args: TranslateArgs) -> Result<String, FrameworkError> {
        let translator = App::resolve_make::<dyn Translator>()?;
        let current = Self::locale();
        let mut last_err = match translator.translate(&current, key, &args) {
            Ok(s) => return Ok(s),
            Err(e) => e,
        };

        let config = resolved_config();
        for locale in fallback_chain(&current, &config) {
            match translator.translate(&locale, key, &args) {
                Ok(s) => return Ok(s),
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }

    /// Whether `key` resolves for the current locale or anywhere in its
    /// fallback chain — i.e. whether [`Lang::get`] would return a real
    /// translation rather than the bare key. Chain-aware: a key defined
    /// only in a configured parent locale, or only in the global
    /// fallback catalog, still counts, because `Lang::get`/`try_get`
    /// would still translate it via that step of the fallback chain.
    /// Returns `false` (never panics) when no `Translator` is bound.
    pub fn has(key: &str) -> bool {
        let Ok(translator) = App::resolve_make::<dyn Translator>() else {
            return false;
        };
        let current = Self::locale();
        if translator.has(&current, key) {
            return true;
        }
        let config = resolved_config();
        fallback_chain(&current, &config)
            .into_iter()
            .any(|locale| translator.has(&locale, key))
    }

    /// Locales with a loaded catalog. Empty if no `Translator` is bound.
    pub fn available_locales() -> Vec<Locale> {
        App::resolve_make::<dyn Translator>()
            .map(|t| t.available_locales())
            .unwrap_or_default()
    }

    /// Locale-aware number formatting via ICU4X, e.g. `1234567.89` renders
    /// as `1,234,567.89` in `en-US` and `1.234.567,89` in `de-DE`. Never
    /// panics — any ICU failure logs a `tracing::warn!` and falls back to
    /// `format!("{n}")`.
    pub fn number(n: f64) -> String {
        Self::try_number(n).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Lang::number: ICU formatting failed, falling back to plain rendering");
            format!("{n}")
        })
    }

    /// [`Lang::number`], but `Err` on an ICU formatting failure instead of
    /// falling back to plain rendering.
    pub fn try_number(n: f64) -> Result<String, FrameworkError> {
        format::try_number(&Self::locale(), n)
    }

    /// Locale-aware currency formatting. `iso_code` is a 3-letter ISO
    /// 4217 code (`"USD"`, `"EUR"`, ...), case-insensitive. Never panics
    /// — any ICU failure (including an invalid `iso_code`) logs a
    /// `tracing::warn!` and falls back to `format!("{iso_code} {amount}")`.
    pub fn currency(amount: f64, iso_code: &str) -> String {
        Self::try_currency(amount, iso_code).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Lang::currency: ICU formatting failed, falling back to plain rendering");
            format!("{iso_code} {amount}")
        })
    }

    /// [`Lang::currency`], but `Err` on an ICU formatting failure instead
    /// of falling back to plain rendering.
    pub fn try_currency(amount: f64, iso_code: &str) -> Result<String, FrameworkError> {
        format::try_currency(&Self::locale(), amount, iso_code)
    }

    /// Locale-aware date formatting. See [`DateStyle`]. Never panics —
    /// any ICU failure logs a `tracing::warn!` and falls back to the
    /// plain ISO-8601 date (`dt.date()`'s `Display`).
    pub fn date(dt: &NaiveDateTime, style: DateStyle) -> String {
        Self::try_date(dt, style).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Lang::date: ICU formatting failed, falling back to plain rendering");
            dt.date().to_string()
        })
    }

    /// [`Lang::date`], but `Err` on an ICU formatting failure instead of
    /// falling back to plain rendering.
    pub fn try_date(dt: &NaiveDateTime, style: DateStyle) -> Result<String, FrameworkError> {
        format::try_date(&Self::locale(), dt, style)
    }

    /// Locale-aware time-of-day formatting. See [`TimeStyle`]. Never
    /// panics — any ICU failure logs a `tracing::warn!` and falls back to
    /// the plain 24-hour time (`dt.time()`'s `Display`).
    pub fn time(dt: &NaiveDateTime, style: TimeStyle) -> String {
        Self::try_time(dt, style).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Lang::time: ICU formatting failed, falling back to plain rendering");
            dt.time().to_string()
        })
    }

    /// [`Lang::time`], but `Err` on an ICU formatting failure instead of
    /// falling back to plain rendering.
    pub fn try_time(dt: &NaiveDateTime, style: TimeStyle) -> Result<String, FrameworkError> {
        format::try_time(&Self::locale(), dt, style)
    }

    /// Locale-aware combined date + time formatting. See [`DateStyle`]
    /// and [`TimeStyle`]. Never panics — any ICU failure logs a
    /// `tracing::warn!` and falls back to `dt`'s plain `Display`.
    pub fn datetime(dt: &NaiveDateTime, date: DateStyle, time: TimeStyle) -> String {
        Self::try_datetime(dt, date, time).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Lang::datetime: ICU formatting failed, falling back to plain rendering");
            dt.to_string()
        })
    }

    /// [`Lang::datetime`], but `Err` on an ICU formatting failure instead
    /// of falling back to plain rendering.
    pub fn try_datetime(
        dt: &NaiveDateTime,
        date: DateStyle,
        time: TimeStyle,
    ) -> Result<String, FrameworkError> {
        format::try_datetime(&Self::locale(), dt, date, time)
    }

    /// Locale-aware list formatting. See [`ListStyle`]. Never panics —
    /// any ICU failure logs a `tracing::warn!` and falls back to a plain
    /// comma join (`items.join(", ")`).
    pub fn list(items: &[&str], style: ListStyle) -> String {
        Self::try_list(items, style).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Lang::list: ICU formatting failed, falling back to plain rendering");
            items.join(", ")
        })
    }

    /// [`Lang::list`], but `Err` on an ICU formatting failure instead of
    /// falling back to plain rendering.
    pub fn try_list(items: &[&str], style: ListStyle) -> Result<String, FrameworkError> {
        format::try_list(&Self::locale(), items, style)
    }

    /// Locale-aware relative time formatting, e.g. `-3` with
    /// [`RelativeUnit::Day`] renders as `"3 days ago"` in `en`. Never
    /// panics — any ICU failure logs a `tracing::warn!` and falls back to
    /// `format!("{amount} {unit:?}")`.
    pub fn relative(amount: i64, unit: RelativeUnit) -> String {
        Self::try_relative(amount, unit).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Lang::relative: ICU formatting failed, falling back to plain rendering");
            format!("{amount} {unit:?}")
        })
    }

    /// [`Lang::relative`], but `Err` on an ICU formatting failure instead
    /// of falling back to plain rendering.
    pub fn try_relative(amount: i64, unit: RelativeUnit) -> Result<String, FrameworkError> {
        format::try_relative(&Self::locale(), amount, unit)
    }
}

/// Translate a Fluent message key for the current locale — the
/// ergonomic entry point to [`Lang::get`] / [`Lang::get_with`].
/// `suprnova::__!("key")` calls `Lang::get`; `suprnova::__!("key", name:
/// value, ...)` builds a `TranslateArgs` from the named arguments and
/// calls `Lang::get_with`. Named after Laravel's `__()` helper — `_`
/// alone is Rust's ignore pattern, so it can't be a macro name.
///
/// Named-argument values are converted through `$crate::serde_json::Value`
/// (the framework's re-export of `serde_json` at the crate root), not a
/// bare `::serde_json::Value` path — so callers never need `serde_json`
/// as a direct dependency of their own; only `suprnova` does.
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
        $( args.insert(stringify!($name).to_string(), $crate::serde_json::Value::from($value)); )+
        $crate::Lang::get_with($key, args)
    }};
}

/// Inertia shared-data provider for the active locale — the `lang` prop
/// that tells the frontend which language is in effect and where to
/// fetch its Fluent catalog.
///
/// Register it once, alongside an app's other Inertia shares:
///
/// ```rust,no_run
/// # use std::sync::Arc;
/// # use suprnova::{App, LocaleShare};
/// # fn ex() {
/// App::register_inertia_shared(Arc::new(LocaleShare));
/// # }
/// ```
///
/// Every Inertia response then carries a `lang` prop shaped like:
///
/// ```json
/// "lang": {
///   "locale": "es",
///   "fallback": "en",
///   "catalog": { "url": "/_suprnova/lang/es.ftl?v=<hash>", "hash": "<hash>" }
/// }
/// ```
///
/// `locale` is [`Lang::locale`] — the per-request locale bound by
/// `LocaleMiddleware` / [`scope_locale`]. `fallback` is the configured
/// [`LocalizationConfig::fallback_locale`], resolved the same way
/// `Lang`'s hot path does (the bootstrap snapshot if one exists, else a
/// fresh env/config read). `catalog.url` names the
/// `/_suprnova/lang/<locale>.ftl` endpoint with a `?v=<hash>`
/// cache-buster matching [`CatalogSource::hash`], so the frontend can
/// request it as immutably cacheable once it already has the hash (from
/// this share, or from a prior fetch's `ETag`).
///
/// `catalog` is JSON `null` — never an error — when no [`Translator`] is
/// bound, or the active locale has no loaded catalog: a page must never
/// fail to render for want of a translation source.
pub struct LocaleShare;

#[async_trait]
impl InertiaSharedData for LocaleShare {
    async fn share(
        &self,
        _req: &dyn InertiaRequestExt,
    ) -> Result<IndexMap<String, Prop>, FrameworkError> {
        let locale = Lang::locale();
        let fallback = resolved_config().fallback_locale;
        let catalog = App::resolve_make::<dyn Translator>()
            .ok()
            .and_then(|translator| translator.catalog(&locale))
            .map(|source| {
                serde_json::json!({
                    "url": format!("/_suprnova/lang/{}.ftl?v={}", locale.as_str(), source.hash),
                    "hash": source.hash,
                })
            });

        let mut shared = IndexMap::new();
        shared.insert(
            "lang".to_string(),
            Prop::eager(serde_json::json!({
                "locale": locale.as_str(),
                "fallback": fallback.as_str(),
                "catalog": catalog,
            })),
        );
        Ok(shared)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn locale(s: &str) -> Locale {
        Locale::parse(s).unwrap()
    }

    /// A minimal `LocalizationConfig` for `fallback_chain` unit tests —
    /// only `parents` and `fallback_locale` matter to it; the rest are
    /// unused-but-required fields, same defaults `config.rs`'s own
    /// `#[cfg(test)]` helpers use.
    fn config_with(parents: &[(&str, &str)], fallback: &str) -> LocalizationConfig {
        let mut map = HashMap::new();
        for (child, parent) in parents {
            map.insert(locale(child), locale(parent));
        }
        LocalizationConfig {
            default_locale: locale("en"),
            fallback_locale: locale(fallback),
            use_isolating: false,
            detection: Vec::new(),
            session_key: "locale".into(),
            cookie_name: "locale".into(),
            parents: map,
        }
    }

    #[test]
    fn empty_parents_is_just_the_fallback() {
        let cfg = config_with(&[], "en");
        assert_eq!(fallback_chain(&locale("pt-PT"), &cfg), vec![locale("en")]);
    }

    #[test]
    fn a_parent_that_equals_the_fallback_is_not_duplicated() {
        let cfg = config_with(&[("pt-PT", "en")], "en");
        assert_eq!(
            fallback_chain(&locale("pt-PT"), &cfg),
            vec![locale("en")],
            "the fallback must appear once, not once as a parent and again as the fallback step"
        );
    }

    #[test]
    fn the_chain_walks_transitively_then_appends_the_fallback() {
        let cfg = config_with(&[("de-CH", "de-AT"), ("de-AT", "de")], "en");
        assert_eq!(
            fallback_chain(&locale("de-CH"), &cfg),
            vec![locale("de-AT"), locale("de"), locale("en")]
        );
    }

    #[test]
    fn a_hand_built_cycle_terminates_the_walk() {
        // Bypasses `parse_parents`'s cycle rejection by building the map
        // directly — `LocalizationConfig::parents` is `pub`, so
        // `fallback_chain` must defend itself regardless of how a cyclic
        // map was constructed, the same contract `parents_cycle` in
        // `config.rs` documents for `from_dir`'s defense.
        let cfg = config_with(&[("pt-PT", "pt-BR"), ("pt-BR", "pt-PT")], "en");
        assert_eq!(
            fallback_chain(&locale("pt-PT"), &cfg),
            vec![locale("pt-BR"), locale("en")],
            "the walk must stop the moment it revisits a locale, not loop forever"
        );
    }

    #[test]
    fn a_self_referential_parent_terminates_the_walk() {
        let cfg = config_with(&[("pt-PT", "pt-PT")], "en");
        assert_eq!(
            fallback_chain(&locale("pt-PT"), &cfg),
            vec![locale("en")],
            "a locale configured as its own parent must not loop; it degrades to just \
             the global fallback"
        );
    }
}
