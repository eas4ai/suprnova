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
use std::collections::{BTreeMap, HashMap};
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
    /// Path → mtime for every `.ftl` file under a locale directory, as of
    /// the last load/reload. A full inventory rather than a running
    /// maximum, so a *deleted* file (which can only hold or lower a max,
    /// never raise it) is still detected — see `mtime_snapshot`.
    snapshot: RwLock<BTreeMap<PathBuf, SystemTime>>,
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
        let snapshot = mtime_snapshot(dir);
        Ok(Self {
            dir: dir.to_path_buf(),
            config: config.clone(),
            inner: RwLock::new(inner),
            snapshot: RwLock::new(snapshot),
        })
    }

    /// Re-read catalogs from disk if the set of `.ftl` files under the
    /// catalog directory changed since the last load (construction or
    /// the last `reload`/`reload_if_stale`) — a file added, a file
    /// removed, or an existing file's mtime changed (including a whole
    /// locale directory appearing or disappearing, since that changes
    /// which files exist). Returns whether a reload actually happened.
    /// Intended for a dev-mode watcher; production deployments call
    /// `reload()` explicitly (e.g. on a deploy hook) instead of polling.
    pub fn reload_if_stale(&self) -> Result<bool, FrameworkError> {
        let current = mtime_snapshot(&self.dir);
        let is_stale = {
            let stored = self.snapshot.read().unwrap_or_else(|e| e.into_inner());
            current != *stored
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
        let snapshot = mtime_snapshot(&self.dir);
        {
            let mut guard = self.inner.write().unwrap_or_else(|e| e.into_inner());
            *guard = rebuilt;
        }
        {
            let mut guard = self.snapshot.write().unwrap_or_else(|e| e.into_inner());
            *guard = snapshot;
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

/// A path → mtime inventory of every `.ftl` file directly under a locale
/// subdirectory of `dir`, used by `reload_if_stale` to detect *any*
/// change to the catalog tree — not just an mtime increasing, which a
/// deleted file can never do (removing a file can only hold or lower a
/// running maximum, never raise it, so a plain "latest mtime" watermark
/// silently misses deletions). Comparing two snapshots for equality
/// catches an added file, a removed file, an edited file, and a whole
/// locale directory appearing or disappearing (its files' entries appear
/// or vanish along with it). An empty snapshot (`dir` missing, or a
/// locale directory holding zero `.ftl` files) is indistinguishable from
/// "nothing here" — which is also exactly what it contributes to a
/// compiled catalog. IO errors while probing are not fatal here, they
/// just mean "assume nothing changed" for this heuristic; `reload()`
/// itself surfaces real IO failures.
fn mtime_snapshot(dir: &Path) -> BTreeMap<PathBuf, SystemTime> {
    let mut files = BTreeMap::new();
    let Ok(locale_dirs) = fs::read_dir(dir) else {
        return files;
    };
    for locale_dir in locale_dirs.flatten() {
        let path = locale_dir.path();
        if !path.is_dir() {
            continue;
        }
        let Ok(entries) = fs::read_dir(&path) else {
            continue;
        };
        for file in entries.flatten() {
            let file_path = file.path();
            if file_path.extension().and_then(|ext| ext.to_str()) != Some("ftl") {
                continue;
            }
            if let Ok(modified) = file.metadata().and_then(|m| m.modified()) {
                files.insert(file_path, modified);
            }
        }
    }
    files
}
