//! Service auto-registration for suprnova framework
//!
//! This module provides automatic service registration via macros:
//! - `#[service(ConcreteType)]` - auto-register trait bindings
//! - `#[derive(Injectable)]` - auto-register concrete types as singletons
//!
//! # Example - Trait binding
//!
//! ```rust,ignore
//! use suprnova::service;
//!
//! // Auto-register: dyn CacheStore → RedisCache
//! #[service(RedisCache)]
//! pub trait CacheStore: Send + Sync + 'static {
//!     fn get(&self, key: &str) -> Option<String>;
//!     fn set(&self, key: &str, value: &str);
//! }
//!
//! pub struct RedisCache;
//! impl Default for RedisCache {
//!     fn default() -> Self { Self }
//! }
//! impl CacheStore for RedisCache { ... }
//! ```
//!
//! # Example - Concrete singleton
//!
//! ```rust,no_run
//! use suprnova::{injectable, App};
//!
//! #[injectable]
//! pub struct AppState {
//!     pub counter: u32,
//! }
//!
//! # fn ex() {
//! // Resolve via:
//! let state: AppState = App::get().unwrap();
//! # }
//! ```
//!
//! # Boot order and dependency resolution
//!
//! Service bindings (`#[service(...)]`) install a default-constructed concrete
//! impl for a trait; they never reach into the container themselves, so they
//! can register in any order with no inter-dependencies. They are processed
//! first as a single pass.
//!
//! Singletons (`#[injectable]`) may declare dependencies via `#[inject]`,
//! pulling them out of the container as they construct. Inventory iteration
//! order is implementation-defined, so a naive single pass could try to
//! resolve a dependency before the producer was registered and report a
//! spurious "missing service" failure. To survive that, [`bootstrap`] runs the
//! singleton entries in a fixed-point loop: every iteration tries every
//! still-pending entry once, removes the ones that succeeded, and stops when
//! either the pending set is empty (success) or a full pass made zero progress
//! (genuine missing or cyclic dependency - returned as a structured error
//! naming the failing entry).

use crate::config::Config;
use crate::error::FrameworkError;

/// Entry for inventory-collected service bindings (trait → impl)
///
/// Used internally by the `#[service(ConcreteType)]` macro to register
/// service bindings at compile time. The register function returns
/// `Result<(), String>` so a registration that depends on the container
/// (e.g. via `App::resolve`) can report a missing dependency instead of
/// panicking.
pub struct ServiceBindingEntry {
    /// Function to register the service binding. Returns `Ok(())` on success
    /// or `Err(reason)` if the binding cannot be installed yet (e.g. a
    /// transitive `App::resolve` failed).
    pub register: fn() -> Result<(), String>,
    /// Service name for debugging/logging
    pub name: &'static str,
}

/// Entry for inventory-collected service bindings chosen by the
/// environment: `#[service(impl = Real, bind(Fake, env = ["testing"]))]`.
///
/// A type of its own beside [`ServiceBindingEntry`], whose fields
/// applications may build as a struct literal, so adding fields to it
/// would break them. At boot, the first choice with an environment
/// pattern that matches [`Config::environment`] registers, and the
/// fallback (the `impl` type) when none matches, as Laravel's `#[Bind]`
/// attribute with `environments` picks a concrete type.
pub struct EnvironmentServiceBindingEntry {
    /// Service name for debugging/logging
    pub name: &'static str,
    /// The `bind(...)` entries, in the order written.
    pub choices: &'static [EnvironmentBinding],
    /// Registers the `impl` type when no choice matches; `None` when the
    /// service names no `impl`, which then stays unbound.
    pub fallback: Option<fn() -> Result<(), String>>,
}

/// One `bind(Concrete, env = [...])` entry of an
/// [`EnvironmentServiceBindingEntry`].
pub struct EnvironmentBinding {
    /// The environment patterns, `*` matching any run of characters.
    pub environments: &'static [&'static str],
    /// Registers the entry's concrete type.
    pub register: fn() -> Result<(), String>,
}

/// Entry for inventory-collected singleton registrations (concrete types)
///
/// Used internally by the `#[derive(Injectable)]` macro to register
/// concrete singletons at compile time. The register function returns
/// `Result<(), String>` so a singleton with `#[inject]` fields can report
/// a missing dependency rather than panicking; the bootstrap loop retries
/// failed entries until they all succeed or progress stalls.
pub struct SingletonEntry {
    /// Function to register the singleton. Returns `Ok(())` on success or
    /// `Err(reason)` if a dependency wasn't registered yet - the bootstrap
    /// loop will retry on later iterations.
    pub register: fn() -> Result<(), String>,
    /// Type name for debugging/logging
    pub name: &'static str,
}

// Inventory collection for auto-registered service bindings
inventory::collect!(ServiceBindingEntry);

// Inventory collection for auto-registered singletons
inventory::collect!(SingletonEntry);

// Inventory collection for service bindings chosen by the environment
inventory::collect!(EnvironmentServiceBindingEntry);

/// Register all service bindings from inventory.
///
/// Services have no inter-service dependencies (each just installs a
/// `Default::default()` concrete impl), so a single pass is sufficient.
/// A service with `bind(...)` entries registers the first entry whose
/// environment patterns match [`Config::environment`], or its `impl` when
/// none matches. Any error is wrapped into a `FrameworkError::internal`
/// naming the failing entry and returned immediately.
pub fn register_service_bindings() -> Result<(), FrameworkError> {
    let failed = |name: &str, reason: String| {
        FrameworkError::internal(format!("service `{name}` failed to register: {reason}"))
    };
    for entry in inventory::iter::<ServiceBindingEntry> {
        (entry.register)().map_err(|reason| failed(entry.name, reason))?;
    }
    let environment = Config::environment().to_string();
    for entry in inventory::iter::<EnvironmentServiceBindingEntry> {
        let chosen = entry
            .choices
            .iter()
            .find(|choice| {
                choice
                    .environments
                    .iter()
                    .any(|pattern| environment_matches(pattern, &environment))
            })
            .map(|choice| choice.register)
            .or(entry.fallback);
        if let Some(register) = chosen {
            register().map_err(|reason| failed(entry.name, reason))?;
        }
    }
    Ok(())
}

/// Whether `environment` matches `pattern`, where `*` matches any run of
/// characters, the empty run included, and every other character matches
/// itself, case included, as Laravel's `Str::is` matches an environment
/// name.
fn environment_matches(pattern: &str, environment: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = environment.chars().collect();
    let (mut p, mut t) = (0, 0);
    // The last `*` seen, and the text position it was tried against.
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, t));
            p += 1;
        } else if p < pattern.len() && pattern[p] == text[t] {
            p += 1;
            t += 1;
        } else if let Some((star_at, tried)) = star {
            // Let the `*` take one more character and try again.
            p = star_at + 1;
            t = tried + 1;
            star = Some((star_at, tried + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|c| *c == '*')
}

/// Register all singleton entries from inventory.
///
/// Singletons can declare `#[inject]` dependencies on other singletons.
/// Inventory order is implementation-defined, so we run a fixed-point loop:
/// each iteration tries every pending entry; entries that succeed drop out
/// of the pending set; the loop stops when either the set empties (success)
/// or a full pass makes no progress (return Err naming the most recently
/// failing entry - its `reason` typically already says which transitive
/// dependency couldn't be resolved).
pub fn register_singletons() -> Result<(), FrameworkError> {
    // Snapshot inventory into an owned vec so we can drain it across
    // multiple passes without reborrowing the iterator each round.
    let mut pending: Vec<&'static SingletonEntry> =
        inventory::iter::<SingletonEntry>.into_iter().collect();

    if pending.is_empty() {
        return Ok(());
    }

    loop {
        let before = pending.len();
        let mut next_pending: Vec<&'static SingletonEntry> = Vec::with_capacity(before);
        let mut iteration_failures: Vec<(&'static str, String)> = Vec::new();

        for entry in pending.drain(..) {
            match (entry.register)() {
                Ok(()) => {
                    // Registered (or already present via if_absent) - done.
                }
                Err(reason) => {
                    iteration_failures.push((entry.name, reason));
                    next_pending.push(entry);
                }
            }
        }

        if next_pending.is_empty() {
            // All entries succeeded this iteration - done.
            return Ok(());
        }

        if next_pending.len() == before {
            // No progress this iteration: the remaining entries cannot be
            // registered because their dependencies are missing or form a
            // cycle. Report the first failure from this iteration; its
            // `reason` text typically names the unresolved type.
            let (name, reason) = iteration_failures
                .into_iter()
                .next()
                .unwrap_or_else(|| ("<unknown>", "no progress in singleton boot loop".into()));
            return Err(FrameworkError::internal(format!(
                "singleton `{name}` could not be booted: {reason} \
                 (no progress across the remaining {} entries - check for \
                 a missing #[injectable] type or a cyclic dependency)",
                next_pending.len()
            )));
        }

        // Made progress; drop the iteration's failure list and try again.
        // The next iteration will repopulate it for any entries that still
        // can't resolve.
        let _ = iteration_failures;
        pending = next_pending;
    }
}

/// Full bootstrap sequence for services.
///
/// Called automatically by `Server::from_config()`. Services first, then the
/// fixed-point loop for singletons. Any failure is returned as a structured
/// `FrameworkError::internal` naming the failing entry - `Server::from_config`
/// propagates it as the boot error.
pub fn bootstrap() -> Result<(), FrameworkError> {
    register_service_bindings()?;
    register_singletons()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::environment_matches;

    #[test]
    fn a_star_matches_any_run_and_other_characters_match_themselves() {
        assert!(environment_matches("testing", "testing"));
        assert!(!environment_matches("testing", "Testing"));
        assert!(environment_matches("*", "production"));
        assert!(environment_matches("*", ""));
        assert!(environment_matches("stag*", "staging"));
        assert!(environment_matches("stag*", "stag"));
        assert!(environment_matches("*-eu", "qa-eu"));
        assert!(environment_matches("qa-*-1", "qa-eu-west-1"));
        assert!(environment_matches("a*b*c", "aXbYbZc"));
        assert!(!environment_matches("local", "localhost"));
        assert!(!environment_matches("stag*", "prestaging"));
        assert!(!environment_matches("a*b", "acd"));
    }
}
