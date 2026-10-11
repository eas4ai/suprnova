//! Where a [`FluentTranslator`](super::FluentTranslator) reads its
//! catalogs: the application's `lang/` directory and the directories and
//! package namespaces added beside it, as Laravel's `FileLoader` reads its
//! paths, JSON paths and namespace hints.
//!
//! A namespaced key `namespace::key` is written `namespace__key` in a
//! catalog; [`TranslationSources`] says why.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use crate::error::FrameworkError;

/// What a namespaced key's `::` becomes in a Fluent id.
const ENCODED_SEPARATOR: &str = "__";

/// Refuse a namespace that cannot name a package's catalogs safely: empty,
/// holding `/`, `\`, `..` or a NUL byte, the test Laravel's loader applies
/// to a locale or a group before it builds a path from one, or anything
/// else that is not a letter followed by letters, digits and `-`, which a
/// Fluent id needs.
pub(crate) fn validate_namespace(namespace: &str) -> Result<(), FrameworkError> {
    let unsafe_segment = namespace.is_empty()
        || namespace.contains("..")
        || namespace.contains('/')
        || namespace.contains('\\')
        || namespace.contains('\0');
    let mut chars = namespace.chars();
    let identifier = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '-');
    if unsafe_segment || !identifier {
        return Err(FrameworkError::param(format!(
            "the translation namespace {namespace:?} is refused: a namespace is a letter followed by letters, digits and `-`, and never empty or holding `/`, `\\`, `..` or a NUL byte"
        )));
    }
    Ok(())
}

/// The Fluent id of `id` in `namespace`: `courier__bye`.
pub(crate) fn namespaced_id(namespace: &str, id: &str) -> String {
    format!("{namespace}{ENCODED_SEPARATOR}{id}")
}

/// The Fluent id a translation key is looked up by: `namespace::key` is
/// encoded as `namespace__key`, and any other key is its own id.
pub(crate) fn catalog_id(key: &str) -> Cow<'_, str> {
    match key.split_once("::") {
        Some((namespace, id)) if validate_namespace(namespace).is_ok() => {
            Cow::Owned(namespaced_id(namespace, id))
        }
        _ => Cow::Borrowed(key),
    }
}

/// One source added beside the application's catalog directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TranslationSource {
    /// A directory merged after the application's, from
    /// [`Lang::add_path`](super::Lang::add_path).
    Path(PathBuf),
    /// A directory merged before the application's, from
    /// [`Lang::add_fallback_path`](super::Lang::add_fallback_path).
    FallbackPath(PathBuf),
    /// A package's directory, read as `namespace::key`, from
    /// [`Lang::add_namespace`](super::Lang::add_namespace).
    Namespace {
        /// The namespace keys are read under.
        namespace: String,
        /// The package's catalog directory.
        dir: PathBuf,
    },
}

/// Every catalog directory a translator reads, in the shape of Laravel's
/// `FileLoader`. [`Lang::loader`](super::Lang::loader) answers the set the
/// framework's translator is built from;
/// [`FluentTranslator::from_sources`](super::FluentTranslator::from_sources)
/// reads one.
///
/// # Merge order
///
/// Each directory holds one subdirectory per locale with `*.ftl` files,
/// as `lang/` does. A locale's catalog merges, lowest priority first: the
/// fallback paths in the order they were added, the application's
/// directory, then the added paths in order, a later one's message winning.
/// Each namespace's catalogs are merged on top under their encoded ids,
/// overridden by the files under `vendor/<namespace>/<locale>/` of the
/// application's directory and of each added path, in that order.
///
/// # Namespaced keys
///
/// A Fluent message id cannot hold `::`, so a namespaced key
/// `namespace::key` is written `namespace__key` in a catalog: the message
/// `bye` of the namespace `courier` is `courier__bye`, and its term `-brand`
/// is `-courier__brand`. The translator writes them so when it loads a
/// namespace, reads `courier::bye` as `courier__bye`, and serves them so
/// in the catalog the browser fetches, whose client encodes a key the same
/// way. A namespace is a letter followed by letters, digits and `-`, so the
/// first `__` of an encoded id always ends the namespace. Ids of the form
/// `<namespace>__<key>` are reserved for namespaces: an application message
/// with such an id collides with the namespace's.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranslationSources {
    dir: PathBuf,
    paths: Vec<PathBuf>,
    fallback_paths: Vec<PathBuf>,
    namespaces: Vec<(String, PathBuf)>,
}

impl TranslationSources {
    /// Sources holding the application's catalog directory alone.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            paths: Vec::new(),
            fallback_paths: Vec::new(),
            namespaces: Vec::new(),
        }
    }

    /// Add a directory merged after the application's and every path
    /// added before it, as Laravel's `addPath` does.
    pub fn path(mut self, dir: impl Into<PathBuf>) -> Self {
        self.paths.push(dir.into());
        self
    }

    /// Add a directory merged before the application's, so the
    /// application's message wins, as Laravel merges its JSON paths.
    pub fn fallback_path(mut self, dir: impl Into<PathBuf>) -> Self {
        self.fallback_paths.push(dir.into());
        self
    }

    /// Read `dir`'s catalogs as `namespace::key`, as Laravel's
    /// `addNamespace` does. Naming a namespace again replaces its
    /// directory.
    ///
    /// # Errors
    ///
    /// When the namespace is empty, holds `/`, `\`, `..` or a NUL byte, or
    /// is otherwise not a letter followed by letters, digits and `-`.
    pub fn namespace(
        mut self,
        namespace: &str,
        dir: impl Into<PathBuf>,
    ) -> Result<Self, FrameworkError> {
        validate_namespace(namespace)?;
        self.set_namespace(namespace.to_owned(), dir.into());
        Ok(self)
    }

    /// The application's catalog directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The directories merged after the application's, in order.
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// The directories merged before the application's, in order.
    pub fn fallback_paths(&self) -> &[PathBuf] {
        &self.fallback_paths
    }

    /// The namespaces and their directories, in the order they were first
    /// added.
    pub fn namespaces(&self) -> &[(String, PathBuf)] {
        &self.namespaces
    }

    /// These sources with `source` added. A namespace in `source` was
    /// validated when the source was made.
    pub(crate) fn with(mut self, source: &TranslationSource) -> Self {
        match source {
            TranslationSource::Path(dir) => self.paths.push(dir.clone()),
            TranslationSource::FallbackPath(dir) => self.fallback_paths.push(dir.clone()),
            TranslationSource::Namespace { namespace, dir } => {
                self.set_namespace(namespace.clone(), dir.clone());
            }
        }
        self
    }

    /// Every directory whose files make up a catalog, for the staleness
    /// check: the application's, the added paths, the fallback paths, each
    /// namespace's, and each namespace's `vendor` overrides.
    pub(crate) fn watched_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = vec![self.dir.clone()];
        dirs.extend(self.paths.iter().cloned());
        dirs.extend(self.fallback_paths.iter().cloned());
        for (namespace, dir) in &self.namespaces {
            dirs.push(dir.clone());
            dirs.extend(self.vendor_dirs(namespace));
        }
        dirs
    }

    /// The directories that override `namespace`, lowest priority first:
    /// `vendor/<namespace>` of the application's directory, then of each
    /// added path, as Laravel's `loadNamespaceOverrides` walks its paths.
    pub(crate) fn vendor_dirs(&self, namespace: &str) -> Vec<PathBuf> {
        std::iter::once(&self.dir)
            .chain(self.paths.iter())
            .map(|root| root.join("vendor").join(namespace))
            .collect()
    }

    fn set_namespace(&mut self, namespace: String, dir: PathBuf) {
        match self
            .namespaces
            .iter_mut()
            .find(|(name, _)| *name == namespace)
        {
            Some(entry) => entry.1 = dir,
            None => self.namespaces.push((namespace, dir)),
        }
    }
}
