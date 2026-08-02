//! [`FluentTranslator`], the Fluent-backed [`Translator`] driver.
//!
//! Loads `lang/<locale>/*.ftl` catalogs from disk, merges each locale's
//! files into one bundle (framework-embedded text first, app files
//! sorted by filename and free to override any id), and serves
//! translations from an in-memory map that `reload()` swaps atomically.

use super::config::LocalizationConfig;
use super::locale::Locale;
use super::translator::{CatalogSource, Translator};
use crate::error::FrameworkError;
use crate::validation::message::TranslateArgs;
use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

/// The framework's embedded English validation catalog. Loaded into
/// every `en`/`en-*` bundle before app resources, so an app redefining
/// any of these ids overrides it (loaded via `add_resource_overriding`).
const EMBEDDED_EN_VALIDATION: &str = include_str!("catalogs/en/validation.ftl");

/// The concurrent Fluent bundle: `Sync`, so it can live behind a shared
/// reference in a container singleton. The non-concurrent `FluentBundle`
/// is `!Sync` and cannot.
type ConcurrentBundle = FluentBundle<Arc<FluentResource>>;

/// One locale's compiled bundle plus the merged source it was built from.
struct LocaleCatalog {
    bundle: ConcurrentBundle,
    source: CatalogSource,
}

/// Fluent-backed [`Translator`].
///
/// `lang/<locale>/*.ftl` on disk is the app's catalog tree: each
/// immediate subdirectory whose name parses as a [`Locale`] contributes
/// one locale, and every `*.ftl` file inside it is merged into that
/// locale's bundle in filename order. Keys are flat — Fluent attribute
/// syntax (`key.attr`) is not resolved by [`Translator::translate`].
pub struct FluentTranslator {
    dir: PathBuf,
    config: LocalizationConfig,
    inner: RwLock<HashMap<Locale, LocaleCatalog>>,
    latest_mtime: RwLock<SystemTime>,
}

// `fluent_bundle::concurrent::FluentBundle` doesn't implement `Debug`, so
// the derive is written by hand — the catalog dir and loaded locales are
// the useful bits, not the compiled bundle internals.
impl std::fmt::Debug for FluentTranslator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let locales: Vec<String> = self
            .inner
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .map(Locale::as_str)
            .collect();
        f.debug_struct("FluentTranslator").field("dir", &self.dir).field("locales", &locales).finish()
    }
}

impl FluentTranslator {
    /// Load every locale under `dir`, merged with the framework's
    /// embedded catalogs.
    ///
    /// A missing `dir` is not an error — the app still boots with the
    /// embedded-only `en` catalog. A subdirectory whose name doesn't
    /// parse as a [`Locale`] is skipped with a `tracing::warn!`. A
    /// malformed `.ftl` file fails loudly, naming the offending file.
    pub fn from_dir(dir: &Path, config: &LocalizationConfig) -> Result<Self, FrameworkError> {
        let inner = load_all(dir, config)?;
        let latest = latest_mtime(dir);
        Ok(Self {
            dir: dir.to_path_buf(),
            config: config.clone(),
            inner: RwLock::new(inner),
            latest_mtime: RwLock::new(latest),
        })
    }

    /// Re-read catalogs from disk if any `.ftl` file under the catalog
    /// directory changed since the last load (construction or the last
    /// `reload`). Returns whether a reload actually happened. Intended
    /// for a dev-mode watcher; production deployments call `reload()`
    /// explicitly (e.g. on a deploy hook) instead of polling mtimes.
    pub fn reload_if_stale(&self) -> Result<bool, FrameworkError> {
        let current = latest_mtime(&self.dir);
        let is_stale = {
            let latest = self.latest_mtime.read().unwrap_or_else(|e| e.into_inner());
            current > *latest
        };
        if !is_stale {
            return Ok(false);
        }
        self.reload()?;
        Ok(true)
    }
}

impl Translator for FluentTranslator {
    fn translate(
        &self,
        locale: &Locale,
        key: &str,
        args: &TranslateArgs,
    ) -> Result<String, FrameworkError> {
        let map = self.inner.read().unwrap_or_else(|e| e.into_inner());
        let catalog = map.get(locale).ok_or_else(|| {
            FrameworkError::param(format!("no catalog loaded for locale `{locale}`"))
        })?;
        let message = catalog.bundle.get_message(key).ok_or_else(|| {
            FrameworkError::param(format!("`{key}` is not defined in the `{locale}` catalog"))
        })?;
        let pattern = message.value().ok_or_else(|| {
            FrameworkError::param(format!("`{key}` in the `{locale}` catalog has no value"))
        })?;

        // JSON numbers are round-tripped through their string form so
        // Fluent's NUMBER() sees the same digits the caller supplied.
        // The owned strings must outlive `fluent_args`, which only
        // borrows — so they're collected up front, parallel to `args`.
        let number_text: Vec<Option<String>> = args
            .values()
            .map(|v| match v {
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .collect();

        let mut fluent_args = FluentArgs::with_capacity(args.len());
        for ((name, value), number) in args.iter().zip(number_text.iter()) {
            let fluent_value = match value {
                Value::String(s) => FluentValue::from(s.as_str()),
                Value::Number(_) => {
                    FluentValue::try_number(number.as_deref().unwrap_or_default())
                }
                other => FluentValue::from(other.to_string()),
            };
            fluent_args.set(name.as_str(), fluent_value);
        }

        let mut errors = Vec::new();
        let rendered = catalog.bundle.format_pattern(pattern, Some(&fluent_args), &mut errors);
        if !errors.is_empty() {
            return Err(FrameworkError::param(format!(
                "translating `{key}` for locale `{locale}` failed: {errors:?}"
            )));
        }
        Ok(rendered.into_owned())
    }

    fn has(&self, locale: &Locale, key: &str) -> bool {
        let map = self.inner.read().unwrap_or_else(|e| e.into_inner());
        map.get(locale).is_some_and(|c| c.bundle.has_message(key))
    }

    fn available_locales(&self) -> Vec<Locale> {
        let map = self.inner.read().unwrap_or_else(|e| e.into_inner());
        map.keys().cloned().collect()
    }

    fn catalog(&self, locale: &Locale) -> Option<CatalogSource> {
        let map = self.inner.read().unwrap_or_else(|e| e.into_inner());
        map.get(locale).map(|c| c.source.clone())
    }

    fn reload(&self) -> Result<(), FrameworkError> {
        let rebuilt = load_all(&self.dir, &self.config)?;
        let mtime = latest_mtime(&self.dir);
        {
            let mut guard = self.inner.write().unwrap_or_else(|e| e.into_inner());
            *guard = rebuilt;
        }
        {
            let mut guard = self.latest_mtime.write().unwrap_or_else(|e| e.into_inner());
            *guard = mtime;
        }
        Ok(())
    }
}

/// Load and compile every locale under `dir` into a fresh map. `en`
/// always exists in the result, even with an empty/missing `dir`,
/// because the embedded framework catalog alone must let a fresh app
/// boot.
fn load_all(
    dir: &Path,
    config: &LocalizationConfig,
) -> Result<HashMap<Locale, LocaleCatalog>, FrameworkError> {
    let mut files_by_locale: HashMap<Locale, Vec<(String, String)>> = HashMap::new();
    files_by_locale.entry(Locale::parse("en")?).or_default();

    if dir.is_dir() {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .map_err(|e| FrameworkError::param(format!("lang dir `{}`: {e}", dir.display())))?
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_dir())
            .collect();
        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            let locale = match Locale::parse(&name_str) {
                Ok(locale) => locale,
                Err(_) => {
                    tracing::warn!(
                        "lang/{name_str}: directory name is not a valid BCP-47 locale, skipping"
                    );
                    continue;
                }
            };

            let mut ftl_files: Vec<PathBuf> = fs::read_dir(entry.path())
                .map_err(|e| FrameworkError::param(format!("lang/{locale}: {e}")))?
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|ext| ext.to_str()) == Some("ftl"))
                .collect();
            ftl_files.sort();

            let bucket = files_by_locale.entry(locale.clone()).or_default();
            for path in ftl_files {
                let filename = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let text = fs::read_to_string(&path).map_err(|e| {
                    FrameworkError::param(format!("lang/{locale}/{filename}: {e}"))
                })?;
                bucket.push((filename, text));
            }
        }
    }

    let mut compiled = HashMap::with_capacity(files_by_locale.len());
    for (locale, files) in files_by_locale {
        let catalog = build_locale_catalog(&locale, &files, config)?;
        compiled.insert(locale, catalog);
    }
    Ok(compiled)
}

/// Compile one locale's bundle: embedded framework text first (for
/// `en`/`en-*`), then each app file in filename order, each one
/// overriding ids from what came before it.
fn build_locale_catalog(
    locale: &Locale,
    files: &[(String, String)],
    config: &LocalizationConfig,
) -> Result<LocaleCatalog, FrameworkError> {
    let mut bundle: ConcurrentBundle = FluentBundle::new_concurrent(vec![locale.as_langid().clone()]);
    bundle.set_use_isolating(config.use_isolating);
    bundle.add_builtins().map_err(|e| {
        FrameworkError::param(format!("lang/{locale}: failed to register Fluent builtins: {e}"))
    })?;

    let mut merged = String::new();

    if locale.language() == "en" {
        merged.push_str(EMBEDDED_EN_VALIDATION);
        merged.push('\n');
        let resource = FluentResource::try_new(EMBEDDED_EN_VALIDATION.to_string()).map_err(
            |(_, errors)| {
                FrameworkError::param(format!(
                    "lang/{locale}/<embedded validation.ftl>: {errors:?}"
                ))
            },
        )?;
        bundle.add_resource_overriding(Arc::new(resource));
    }

    for (filename, text) in files {
        merged.push_str(text);
        merged.push('\n');
        let resource = FluentResource::try_new(text.clone()).map_err(|(_, errors)| {
            FrameworkError::param(format!("lang/{locale}/{filename}: {errors:?}"))
        })?;
        bundle.add_resource_overriding(Arc::new(resource));
    }

    let hash = crate::hashing::sha256_hex(&merged).chars().take(32).collect();
    Ok(LocaleCatalog { bundle, source: CatalogSource { text: Arc::from(merged.as_str()), hash } })
}

/// The latest modification time of any `.ftl` file directly under a
/// locale subdirectory of `dir`. `SystemTime::UNIX_EPOCH` when `dir`
/// doesn't exist or holds nothing — IO errors while probing are not
/// fatal here, they just mean "assume nothing changed" for the
/// hot-reload heuristic; `reload()` itself surfaces real IO failures.
fn latest_mtime(dir: &Path) -> SystemTime {
    let mut latest = SystemTime::UNIX_EPOCH;
    let Ok(locale_dirs) = fs::read_dir(dir) else {
        return latest;
    };
    for locale_dir in locale_dirs.flatten() {
        let path = locale_dir.path();
        if !path.is_dir() {
            continue;
        }
        let Ok(files) = fs::read_dir(&path) else {
            continue;
        };
        for file in files.flatten() {
            if let Ok(modified) = file.metadata().and_then(|m| m.modified()) {
                if modified > latest {
                    latest = modified;
                }
            }
        }
    }
    latest
}
