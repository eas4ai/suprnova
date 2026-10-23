//! Process-wide typed config repository.
//!
//! Stores one instance per `TypeId`; readers reach configs via
//! [`Config::get::<T>()`](crate::Config::get) and writers register them at boot.

use std::any::{Any, TypeId};
use std::collections::{BTreeMap, HashMap};
use std::hash::BuildHasher;
use std::sync::{OnceLock, RwLock, RwLockWriteGuard};

/// Global config repository - stores config instances by type
static CONFIG_REPOSITORY: OnceLock<RwLock<ConfigRepository>> = OnceLock::new();

/// Repository for storing typed configuration structs
pub struct ConfigRepository {
    configs: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl ConfigRepository {
    /// Create a new empty config repository
    pub fn new() -> Self {
        Self {
            configs: HashMap::new(),
        }
    }

    /// Register a config struct in the repository
    pub fn register<T: Any + Send + Sync + 'static>(&mut self, config: T) {
        self.configs.insert(TypeId::of::<T>(), Box::new(config));
    }

    /// Get a config struct by type
    pub fn get<T: Any + Send + Sync + Clone + 'static>(&self) -> Option<T> {
        self.configs
            .get(&TypeId::of::<T>())
            .and_then(|boxed| boxed.downcast_ref::<T>())
            .cloned()
    }

    /// Check if a config type is registered
    pub fn has<T: Any + 'static>(&self) -> bool {
        self.configs.contains_key(&TypeId::of::<T>())
    }

    /// Register `config` only when no value of its type is registered, and
    /// answer whether it did. The application registers first and a crate
    /// registers its defaults after, so the application's value stays.
    pub fn register_default<T: Any + Send + Sync + 'static>(&mut self, config: T) -> bool {
        if self.has::<T>() {
            return false;
        }
        self.register(config);
        true
    }

    /// Merge `defaults` under the registered value of its type, keeping the
    /// registered value's entries, or register `defaults` when none is
    /// registered. See [`MergeConfig`].
    pub fn merge<T: MergeConfig + Any + Send + Sync + 'static>(&mut self, defaults: T) {
        match self
            .configs
            .get_mut(&TypeId::of::<T>())
            .and_then(|boxed| boxed.downcast_mut::<T>())
        {
            Some(registered) => registered.merge_defaults(defaults),
            None => self.register(defaults),
        }
    }
}

/// A configuration value that can take defaults under its own entries.
///
/// [`Config::merge`](crate::Config::merge) uses it so a crate that registers
/// its defaults after the application never overwrites what the application
/// set, as Laravel's `ServiceProvider::mergeConfigFrom` merges a package's
/// file under the application's keys. The merge is shallow, as Laravel's
/// `array_merge` is: a key the registered value holds keeps its whole value.
///
/// Implement it for a configuration struct of your own to choose how its
/// fields merge.
pub trait MergeConfig {
    /// Add to `self` the entries only `defaults` has, keeping every entry
    /// `self` already has.
    fn merge_defaults(&mut self, defaults: Self);
}

impl<V, S: BuildHasher> MergeConfig for HashMap<String, V, S> {
    fn merge_defaults(&mut self, defaults: Self) {
        for (key, value) in defaults {
            self.entry(key).or_insert(value);
        }
    }
}

impl<V> MergeConfig for BTreeMap<String, V> {
    fn merge_defaults(&mut self, defaults: Self) {
        for (key, value) in defaults {
            self.entry(key).or_insert(value);
        }
    }
}

impl MergeConfig for serde_json::Map<String, serde_json::Value> {
    fn merge_defaults(&mut self, defaults: Self) {
        for (key, value) in defaults {
            self.entry(key).or_insert(value);
        }
    }
}

impl Default for ConfigRepository {
    fn default() -> Self {
        Self::new()
    }
}

/// Initialize the global config repository
pub fn init_repository() -> &'static RwLock<ConfigRepository> {
    CONFIG_REPOSITORY.get_or_init(|| RwLock::new(ConfigRepository::new()))
}

/// Register a config in the global repository.
///
/// A poisoned write lock - possible if another thread panicked while
/// holding the lock during boot - is recovered via
/// `PoisonError::into_inner` rather than silently dropping the
/// registration. Silent failure here would mean a custom `DatabaseConfig`
/// or `MailConfig` vanishes from the repository for the rest of the
/// process lifetime; every `Config::get::<T>()` after that returns None,
/// and the framework falls back to defaults invisibly.
pub fn register<T: Any + Send + Sync + 'static>(config: T) {
    write_repository().register(config);
}

/// Register a config in the global repository only when none of its type is
/// registered, answering whether it did. The check and the write happen
/// under one write lock, so two callers never both register.
pub fn register_default<T: Any + Send + Sync + 'static>(config: T) -> bool {
    write_repository().register_default(config)
}

/// Merge `defaults` under the registered config of its type, or register
/// them when none is registered, under one write lock.
pub fn merge<T: MergeConfig + Any + Send + Sync + 'static>(defaults: T) {
    write_repository().merge(defaults);
}

/// The repository's write guard. A poisoned lock is recovered, for the
/// reason [`register`] gives.
fn write_repository() -> RwLockWriteGuard<'static, ConfigRepository> {
    match init_repository().write() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Get a config from the global repository.
///
/// Poisoned read locks recover via `PoisonError::into_inner` so a panic
/// during a prior `register` cannot silently make every subsequent
/// `Config::get::<T>()` return None.
pub fn get<T: Any + Send + Sync + Clone + 'static>() -> Option<T> {
    let repo = CONFIG_REPOSITORY.get()?;
    let guard = match repo.read() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard.get::<T>()
}

/// Check if a config type is registered in the global repository.
///
/// Poisoned read locks recover via `PoisonError::into_inner` for the
/// same reason as [`get`].
pub fn has<T: Any + 'static>() -> bool {
    let Some(repo) = CONFIG_REPOSITORY.get() else {
        return false;
    };
    let guard = match repo.read() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard.has::<T>()
}
