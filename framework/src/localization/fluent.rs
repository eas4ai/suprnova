//! [`FluentTranslator`], the Fluent-backed [`Translator`] driver.
//!
//! Loads `lang/<locale>/*.ftl` catalogs from disk and builds each
//! locale's served catalog as a fold through `super::merge`'s AST-level
//! merge: the locale's configured fallback parent chain, if any
//! (`LocalizationConfig::parents`, walked recursively), then the
//! framework's embedded `en` validation catalog for `en`/`en-*`
//! locales, then the locale's own app files in filename order. Each
//! step only replaces the ids it defines and leaves everything else
//! untouched — see `super::merge`'s module doc for the override
//! contract. The result is one flattened resource per locale, resolved
//! ahead of time rather than walked key by key at request time,
//! serialized once, and compiled into a single Fluent bundle;
//! `reload()` rebuilds the whole map and swaps it in atomically.

use super::config::LocalizationConfig;
use super::functions;
use super::locale::Locale;
use super::translator::{CatalogSource, Translator};
use crate::error::FrameworkError;
use crate::validation::message::TranslateArgs;
use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use fluent_syntax::ast::Resource as FtlResource;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

/// The framework's embedded English validation catalog. Folded into
/// every `en`/`en-*` locale's catalog ahead of its own app files (see
/// [`catalog_ast`]), so an app file redefining any of these ids
/// overrides it.
const EMBEDDED_EN_VALIDATION: &str = include_str!("catalogs/en/validation.ftl");

/// The concurrent Fluent bundle: `Sync`, so it can live behind a shared
/// reference in a container singleton. The non-concurrent `FluentBundle`
/// is `!Sync` and cannot.
///
/// `pub(crate)` (not private) so `functions::register` — which adds the
/// `DATETIME()` Fluent function to every bundle built below — can name
/// the same type without duplicating it.
pub(crate) type ConcurrentBundle = FluentBundle<Arc<FluentResource>>;

/// One locale's compiled bundle plus the merged source it was built from.
struct LocaleCatalog {
    bundle: ConcurrentBundle,
    source: CatalogSource,
}

/// Fluent-backed [`Translator`].
///
/// `lang/<locale>/*.ftl` on disk is the app's catalog tree: each
/// immediate subdirectory whose name parses as a [`Locale`] contributes
/// one locale. A locale's served catalog is chain-flattened ahead of
/// time: its configured fallback parent (`LocalizationConfig::parents`),
/// the embedded framework catalog for `en`/`en-*`, and its own `*.ftl`
/// files (folded in filename order) are all merged at the AST level
/// into one resource before it is ever queried. Keys are flat — Fluent
/// attribute syntax (`key.attr`) is not resolved by
/// [`Translator::translate`].
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
        f.debug_struct("FluentTranslator")
            .field("dir", &self.dir)
            .field("locales", &locales)
            .finish()
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
    pub fn from_dir(
        dir: impl AsRef<Path>,
        config: &LocalizationConfig,
    ) -> Result<Self, FrameworkError> {
        let dir = dir.as_ref();
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
                Value::Number(_) => FluentValue::try_number(number.as_deref().unwrap_or_default()),
                other => FluentValue::from(other.to_string()),
            };
            fluent_args.set(name.as_str(), fluent_value);
        }

        let mut errors = Vec::new();
        let rendered = catalog
            .bundle
            .format_pattern(pattern, Some(&fluent_args), &mut errors);
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

    /// Delegates to the inherent [`FluentTranslator::reload_if_stale`].
    ///
    /// The inherent method exists so callers holding a concrete
    /// `FluentTranslator` (tests, mainly) can call it without importing
    /// `Translator`. This override exists so callers holding only a
    /// `dyn Translator` (`LocaleMiddleware`, resolved from the
    /// container) reach the same logic. Rust's method resolution always
    /// prefers an inherent method over a trait method on the same
    /// receiver type — so `self.reload_if_stale()` here calls the
    /// inherent method above, not itself; see
    /// `suprnova-macros/src/model/derive_eloquent.rs` for the general
    /// form of this trap, exploited deliberately here instead of being
    /// a bug.
    fn reload_if_stale(&self) -> Result<bool, FrameworkError> {
        self.reload_if_stale()
    }
}

/// Load and compile every locale under `dir` into a fresh map. `en`
/// always exists in the result, even with an empty/missing `dir`,
/// because the embedded framework catalog alone must let a fresh app
/// boot. Every locale named as a fallback child in `config.parents`
/// also exists in the result even without its own directory — it
/// inherits everything from its parent chain instead.
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
                let filename = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let text = fs::read_to_string(&path)
                    .map_err(|e| FrameworkError::param(format!("lang/{locale}/{filename}: {e}")))?;
                bucket.push((filename, text));
            }
        }
    }

    // Every configured fallback child gets a catalog even when its own
    // directory doesn't exist on disk — `catalog_ast` below still
    // resolves it by walking its parent chain.
    for child in config.parents.keys() {
        files_by_locale.entry(child.clone()).or_default();
    }

    if let Some(cycle) = super::config::parents_cycle(&config.parents) {
        let path = cycle
            .iter()
            .map(|l| format!("`{}`", l.as_str()))
            .collect::<Vec<_>>()
            .join(" -> ");
        return Err(FrameworkError::param(format!(
            "locale fallback parents contain a cycle: {path}"
        )));
    }

    // A parent named in `config.parents` that has neither catalog files
    // of its own nor a parent of its own contributes nothing to the
    // locale(s) that name it — surfaced once per dangling parent, not
    // once per child that references it, since several children can
    // share the same missing ancestor.
    let mut warned_parents: HashSet<Locale> = HashSet::new();
    for parent in config.parents.values() {
        if warned_parents.contains(parent) {
            continue;
        }
        let has_files = files_by_locale.contains_key(parent);
        let has_own_parent = config.parents.contains_key(parent);
        if !has_files && !has_own_parent {
            tracing::warn!(
                "lang: fallback parent `{parent}` is configured but has no catalog \
                 directory and no parent of its own — it contributes nothing to the \
                 locale(s) that name it as a fallback"
            );
            warned_parents.insert(parent.clone());
        }
    }

    let mut memo: HashMap<Locale, FtlResource<String>> = HashMap::new();
    let mut compiled = HashMap::with_capacity(files_by_locale.len());
    for locale in files_by_locale.keys() {
        let ast = catalog_ast(locale, &files_by_locale, config, &mut memo)?;
        let catalog = build_locale_catalog(locale, &ast, config)?;
        compiled.insert(locale.clone(), catalog);
    }
    Ok(compiled)
}

/// Fold `locale`'s catalog into one AST: its configured fallback parent
/// chain first (recursively, via `config.parents`), then the
/// framework's embedded `en` validation catalog for `en`/`en-*`
/// locales, then `locale`'s own files in filename order — each step
/// merged over what came before via `super::merge::merge`. Memoized per
/// `load_all` call so a parent chain shared by several children (or
/// revisited deeper in the same chain) is only walked once. Recursion
/// always terminates: `load_all` runs `super::config::parents_cycle`
/// over `config.parents` before ever calling this, so there is no cycle
/// left to loop on.
fn catalog_ast(
    locale: &Locale,
    files_by_locale: &HashMap<Locale, Vec<(String, String)>>,
    config: &LocalizationConfig,
    memo: &mut HashMap<Locale, FtlResource<String>>,
) -> Result<FtlResource<String>, FrameworkError> {
    if let Some(done) = memo.get(locale) {
        return Ok(done.clone());
    }
    let mut ast = match config.parents.get(locale) {
        Some(parent) => catalog_ast(parent, files_by_locale, config, memo)?,
        None => super::merge::empty(),
    };
    if locale.language() == "en" {
        let embedded =
            super::merge::parse_strict(EMBEDDED_EN_VALIDATION, "embedded validation.ftl")?;
        ast = super::merge::merge(&ast, &embedded);
    }
    for (filename, text) in files_by_locale
        .get(locale)
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        let file_ast = super::merge::parse_strict(text, &format!("lang/{locale}/{filename}"))?;
        ast = super::merge::merge(&ast, &file_ast);
    }
    memo.insert(locale.clone(), ast.clone());
    Ok(ast)
}

/// Compile one locale's bundle from its already-flattened AST (see
/// [`catalog_ast`]): serialize it once and register that as the
/// bundle's only resource.
fn build_locale_catalog(
    locale: &Locale,
    ast: &FtlResource<String>,
    config: &LocalizationConfig,
) -> Result<LocaleCatalog, FrameworkError> {
    let mut bundle: ConcurrentBundle =
        FluentBundle::new_concurrent(vec![locale.as_langid().clone()]);
    bundle.set_use_isolating(config.use_isolating);
    bundle.add_builtins().map_err(|e| {
        FrameworkError::param(format!(
            "lang/{locale}: failed to register Fluent builtins: {e}"
        ))
    })?;
    // `add_builtins()` covers `NUMBER()`; `DATETIME()` is the framework's
    // own ICU4X-backed addition (see `functions.rs`).
    functions::register(&mut bundle)?;

    let serialized = super::merge::serialize(ast);
    // Every entry in `ast` already passed through `parse_strict` as an
    // individual file (or the embedded catalog), so a failure to
    // re-parse the serialized, merged result is an internal invariant
    // failure of the merge/serialize round trip — not a user-facing
    // malformed-file error — and must never panic.
    let resource = FluentResource::try_new(serialized.clone()).map_err(|(_, errors)| {
        FrameworkError::param(format!(
            "lang/{locale}: internal error re-parsing the flattened catalog: {errors:?}"
        ))
    })?;
    bundle.add_resource_overriding(Arc::new(resource));

    let hash = crate::hashing::sha256_hex(&serialized)
        .chars()
        .take(32)
        .collect();
    Ok(LocaleCatalog {
        bundle,
        source: CatalogSource {
            text: Arc::from(serialized.as_str()),
            hash,
        },
    })
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
