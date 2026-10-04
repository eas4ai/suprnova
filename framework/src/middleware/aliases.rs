//! Middleware aliases, named groups, and priority ordering.
//!
//! Mirrors three Laravel kernel surfaces that previously had no Suprnova
//! analogue:
//!
//! - **`middlewareAliases`** - string-keyed lookups so consumers can refer
//!   to `"auth"` / `"throttle"` instead of a fully-qualified type. Laravel
//!   uses the alias map in `Kernel::$middlewareAliases`. A route names one
//!   with `.middleware_named("auth")`, and gives it arguments after a colon,
//!   as in `.middleware_named("throttle:60,1")`.
//! - **`middlewareGroups`** - string-keyed bundles of middleware that
//!   expand at resolution time. Laravel's `web` and `api` groups are the
//!   canonical examples.
//! - **`middlewarePriority`** - an ordered list of middleware types. A
//!   chain is put in the order of the list when it runs, whatever order
//!   its middleware were registered in. The Laravel kernel ships a built-in
//!   priority list ensuring `SubstituteBindings` always runs after
//!   `StartSession`, etc.
//!
//! These three registries are intentionally separate from
//! [`MiddlewareRegistry`] - they're lookup tables, not execution slots.
//! Aliases and groups are read when a route is registered, and the
//! priority list when a chain runs
//! ([`MiddlewareChain::execute`](crate::middleware::MiddlewareChain::execute)).
//! They are also process-global so the bootstrap macros can write into
//! them without having to thread a config object through.

use super::{BoxedMiddleware, Middleware, boxed_as};
use crate::error::FrameworkError;
use std::any::TypeId;
use std::sync::{Arc, OnceLock, RwLock};

/// A factory closure that produces a fresh `BoxedMiddleware`. Used by
/// the alias and group registries because a `Middleware: 'static` trait
/// object can't be cheaply cloned for repeated registrations - we
/// instantiate per registration site via a factory instead.
pub type MiddlewareFactory = std::sync::Arc<dyn Fn() -> BoxedMiddleware + Send + Sync>;

/// A factory that builds a middleware from the arguments a route wrote
/// after the alias name: `["60", "1"]` for `"throttle:60,1"`. Registered
/// with [`register_middleware_alias_with_args`].
pub type MiddlewareArgumentsFactory =
    Arc<dyn Fn(&[&str]) -> Result<BoxedMiddleware, FrameworkError> + Send + Sync>;

/// What an alias name is bound to.
#[derive(Clone)]
enum AliasFactory {
    /// Takes no arguments: `"auth"`.
    Plain(MiddlewareFactory),
    /// Reads the arguments after the colon: `"throttle:60,1"`.
    WithArguments(MiddlewareArgumentsFactory),
}

/// Stored shape of the alias registry - extracted to a `type` alias so
/// the `OnceLock<RwLock<...>>` declaration below doesn't trip
/// `clippy::type_complexity`.
type AliasMap = Vec<(String, AliasFactory)>;

/// Stored shape of the named-group registry. Each entry maps a group
/// name to its ordered list of alias names. Aliased for the same
/// type-complexity reason as [`AliasMap`].
type GroupMap = Vec<(String, Vec<String>)>;

/// Process-global alias registry. `(name, factory)` pairs.
static ALIAS_REGISTRY: OnceLock<RwLock<AliasMap>> = OnceLock::new();

/// Process-global named-group registry. `(group_name, Vec<alias_names>)`.
static GROUP_REGISTRY: OnceLock<RwLock<GroupMap>> = OnceLock::new();

/// Process-global middleware priority list (TypeIds). Order matters: the
/// first TypeId is sorted to the front of the chain.
static PRIORITY_REGISTRY: OnceLock<RwLock<Vec<TypeId>>> = OnceLock::new();

fn alias_lock() -> &'static RwLock<AliasMap> {
    ALIAS_REGISTRY.get_or_init(|| RwLock::new(Vec::new()))
}

fn group_lock() -> &'static RwLock<GroupMap> {
    GROUP_REGISTRY.get_or_init(|| RwLock::new(Vec::new()))
}

fn priority_lock() -> &'static RwLock<Vec<TypeId>> {
    PRIORITY_REGISTRY.get_or_init(|| RwLock::new(Vec::new()))
}

/// Register a named middleware alias.
///
/// Equivalent to Laravel's
/// `Kernel::$middlewareAliases['auth' => AuthMiddleware::class]`. The
/// alias is the lookup key; the closure produces a fresh boxed
/// middleware on demand. The factory is invoked once per
/// [`resolve_middleware_alias`] / [`resolve_middleware_group`] hit, so
/// per-route registration produces independent instances.
///
/// Registration is **last-wins** for the same name - re-registering an
/// alias swaps the factory rather than panicking, mirroring Laravel's
/// reassignable kernel array. This keeps test setup and hot-reload
/// flows simple.
///
/// # Example
///
/// ```rust,no_run
/// use suprnova::middleware::register_middleware_alias;
/// # use suprnova::{async_trait, Middleware, Next, Request, Response};
/// # struct AuthMiddleware;
/// # #[async_trait]
/// # impl Middleware for AuthMiddleware {
/// #     async fn handle(&self, request: Request, next: Next) -> Response { next(request).await }
/// # }
///
/// register_middleware_alias("auth", || AuthMiddleware);
/// ```
pub fn register_middleware_alias<F, M>(name: &str, factory: F)
where
    F: Fn() -> M + Send + Sync + 'static,
    M: Middleware + 'static,
{
    store_alias(
        name,
        AliasFactory::Plain(Arc::new(move || boxed_as(factory()))),
    );
}

/// Register a named middleware alias that reads arguments.
///
/// A route writes the arguments after the name, separated by commas:
/// `"throttle:60,1"` calls the factory with `["60", "1"]`, and the bare
/// `"throttle"` calls it with none. The factory decides what they mean, and
/// returns an error for arguments it cannot use. The route that named the
/// alias then fails to register, which is at boot and not on a request.
///
/// Registration is last-wins for the same name, as it is for
/// [`register_middleware_alias`], and an alias is one or the other: the
/// later registration replaces the earlier whichever kind it was.
///
/// # Example
///
/// ```rust,no_run
/// use suprnova::middleware::register_middleware_alias_with_args;
/// use suprnova::rate_limit::ThrottleRequestsMiddleware;
///
/// register_middleware_alias_with_args("throttle", ThrottleRequestsMiddleware::from_alias_args);
/// ```
pub fn register_middleware_alias_with_args<F, M>(name: &str, factory: F)
where
    F: Fn(&[&str]) -> Result<M, FrameworkError> + Send + Sync + 'static,
    M: Middleware + 'static,
{
    store_alias(
        name,
        AliasFactory::WithArguments(Arc::new(move |arguments| factory(arguments).map(boxed_as))),
    );
}

fn store_alias(name: &str, factory: AliasFactory) {
    let lock = alias_lock();
    let mut guard = match lock.write() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    if let Some(slot) = guard.iter_mut().find(|(n, _)| n == name) {
        slot.1 = factory;
    } else {
        guard.push((name.to_string(), factory));
    }
}

/// Look up a registered alias and produce a fresh `BoxedMiddleware` from
/// its factory. `name` may carry arguments, as in `"throttle:60,1"`.
/// Returns `None` if no alias with that name was registered, or if the
/// alias did not accept the arguments; [`try_resolve_middleware_alias`]
/// says which.
pub fn resolve_middleware_alias(name: &str) -> Option<BoxedMiddleware> {
    try_resolve_middleware_alias(name).ok()
}

/// [`resolve_middleware_alias`], with the reason when there is no
/// middleware to return: the alias is not registered, it takes no
/// arguments and was given some, or its factory refused the arguments.
pub fn try_resolve_middleware_alias(spec: &str) -> Result<BoxedMiddleware, FrameworkError> {
    let (name, arguments) = split_alias(spec);
    // Cloned out, so the factory runs without the registry lock: a factory
    // may register or resolve an alias of its own.
    let factory = {
        let lock = alias_lock();
        let guard = match lock.read() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        guard
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, factory)| factory.clone())
    };
    match factory {
        None => Err(FrameworkError::internal(format!(
            "middleware alias `{name}` is not registered. Register it with \
             register_middleware_alias before the routes that name it"
        ))),
        Some(AliasFactory::Plain(factory)) if arguments.is_empty() => Ok(factory()),
        Some(AliasFactory::Plain(_)) => Err(FrameworkError::internal(format!(
            "middleware alias `{name}` takes no arguments, and `{spec}` gives it {}. \
             Register it with register_middleware_alias_with_args to read them",
            arguments.len()
        ))),
        Some(AliasFactory::WithArguments(factory)) => factory(&arguments).map_err(|e| {
            FrameworkError::internal(format!("middleware alias `{spec}` was refused: {e}"))
        }),
    }
}

/// Split `"throttle:60,1"` into the alias name and its arguments. A name
/// with no colon, or with nothing after it, has no arguments.
fn split_alias(spec: &str) -> (&str, Vec<&str>) {
    match spec.split_once(':') {
        Some((name, arguments)) if !arguments.trim().is_empty() => {
            (name.trim(), arguments.split(',').map(str::trim).collect())
        }
        Some((name, _)) => (name.trim(), Vec::new()),
        None => (spec.trim(), Vec::new()),
    }
}

/// Resolve what a route named with `.middleware_named(...)`: a group, which
/// gives every middleware of the group in order, or an alias, with or
/// without arguments, which gives one.
///
/// A group wins over an alias of the same name, as it does inside a group.
pub fn resolve_named_middleware(name: &str) -> Result<Vec<BoxedMiddleware>, FrameworkError> {
    if is_registered_group(name.trim()) {
        return resolve_middleware_group(name.trim())
            .map_err(|e| FrameworkError::internal(e.to_string()));
    }
    try_resolve_middleware_alias(name).map(|middleware| vec![middleware])
}

/// Whether an alias by this name has been registered.
pub fn has_middleware_alias(name: &str) -> bool {
    let lock = alias_lock();
    let guard = match lock.read() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    guard.iter().any(|(n, _)| n == name)
}

/// All currently-registered alias names (snapshot). Order matches
/// registration order. Useful for diagnostic CLI surfaces.
pub fn registered_middleware_aliases() -> Vec<String> {
    let lock = alias_lock();
    let guard = match lock.read() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    guard.iter().map(|(n, _)| n.clone()).collect()
}

/// Remove a registered alias by name. Idempotent - returns `true` if a
/// binding was removed, `false` if no such alias existed. Exposed so
/// tests and hot-reload tooling can teardown cleanly.
pub fn clear_middleware_alias(name: &str) -> bool {
    let lock = alias_lock();
    let mut guard = match lock.write() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    let before = guard.len();
    guard.retain(|(n, _)| n != name);
    before != guard.len()
}

/// Wipe every registered alias. Test-only convenience; the production
/// boot path never needs this.
#[doc(hidden)]
pub fn clear_all_middleware_aliases_for_test() {
    let lock = alias_lock();
    let mut guard = match lock.write() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    guard.clear();
}

/// Define a named middleware group as a list of alias names.
///
/// Mirrors Laravel's `Kernel::$middlewareGroups['web' => [EncryptCookies::class, ...]]`.
/// Each entry in `aliases` must resolve via
/// [`resolve_middleware_alias`] when the group is consulted - calling
/// [`resolve_middleware_group`] on a group whose entries can't be
/// resolved returns an `Err` listing the missing names.
///
/// Registration is last-wins for the same group name. Recursive groups
/// (a group referencing another group) ARE supported via a single
/// pass - see [`resolve_middleware_group`].
pub fn register_middleware_group(name: &str, aliases: impl IntoIterator<Item = String>) {
    let aliases: Vec<String> = aliases.into_iter().collect();
    let lock = group_lock();
    let mut guard = match lock.write() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    let name_owned = name.to_string();
    if let Some(slot) = guard.iter_mut().find(|(n, _)| n == &name_owned) {
        slot.1 = aliases;
    } else {
        guard.push((name_owned, aliases));
    }
}

/// Errors that can surface from group resolution.
#[derive(Debug, PartialEq, Eq)]
pub enum MiddlewareResolveError {
    /// The group itself is not registered.
    UnknownGroup(String),
    /// The group exists but references an alias that wasn't registered.
    UnknownAlias {
        /// Name of the group whose definition references the missing alias.
        group: String,
        /// The alias name that couldn't be resolved.
        missing: String,
    },
    /// The group references another group that doesn't exist.
    UnknownNestedGroup {
        /// Name of the outer group whose definition references the missing group.
        group: String,
        /// The nested group name that couldn't be resolved.
        missing: String,
    },
    /// A nested group references itself (direct or via a chain). Detected
    /// so we don't loop forever on a misconfigured group definition. A group
    /// that two sibling branches both include is not a cycle.
    CycleDetected {
        /// Name of the group at which the cycle was detected.
        group: String,
    },
}

impl std::fmt::Display for MiddlewareResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownGroup(name) => write!(f, "unknown middleware group: '{name}'"),
            Self::UnknownAlias { group, missing } => {
                write!(
                    f,
                    "middleware group '{group}' references unknown alias '{missing}'"
                )
            }
            Self::UnknownNestedGroup { group, missing } => {
                write!(
                    f,
                    "middleware group '{group}' references unknown nested group '{missing}'"
                )
            }
            Self::CycleDetected { group } => {
                write!(f, "middleware group '{group}' contains a cyclic reference")
            }
        }
    }
}

impl std::error::Error for MiddlewareResolveError {}

/// Resolve a registered group into a flat list of `BoxedMiddleware`,
/// expanding nested group references along the way.
///
/// Nested groups: an entry in a group's alias list whose name matches a
/// registered group is recursively expanded. Cycle detection prevents
/// infinite recursion on a misconfigured definition; a group reused by
/// several branches (`api = [read, write]`, both including `base`) is
/// not a cycle.
///
/// The list holds each middleware once, at its first occurrence, as
/// Laravel's `Router::uniqueMiddleware` keeps it. Without that, a group
/// reached through two branches put its middleware in the chain twice,
/// and a throttle in it counted every request twice. A middleware is
/// identified by its alias and arguments: `"throttle:60,1"` twice is one
/// throttle, `"throttle:30,1"` beside it is another.
pub fn resolve_middleware_group(
    name: &str,
) -> Result<Vec<BoxedMiddleware>, MiddlewareResolveError> {
    let mut ancestors: Vec<String> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let mut out: Vec<BoxedMiddleware> = Vec::new();
    resolve_group_inner(name, &mut ancestors, &mut seen, &mut out)?;
    Ok(out)
}

/// Expand `name` into `out`.
///
/// `ancestors` holds the groups on the current expansion path only, not
/// every group seen so far: a cycle is a group that reaches one of its own
/// ancestors, while a group reached again through a sibling branch is a
/// diamond and expands normally. Each group pops itself off when its
/// expansion completes. An error abandons the whole resolution, so the
/// early returns leave the path as it was.
///
/// `seen` holds the [`alias_identity`] of every middleware already in
/// `out`, across the whole resolution, so a repeat is skipped before its
/// factory runs.
fn resolve_group_inner(
    name: &str,
    ancestors: &mut Vec<String>,
    seen: &mut Vec<String>,
    out: &mut Vec<BoxedMiddleware>,
) -> Result<(), MiddlewareResolveError> {
    if ancestors.iter().any(|v| v == name) {
        return Err(MiddlewareResolveError::CycleDetected {
            group: name.to_string(),
        });
    }
    ancestors.push(name.to_string());

    let aliases = {
        let lock = group_lock();
        let guard = match lock.read() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        guard
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
    };
    let aliases = aliases.ok_or_else(|| MiddlewareResolveError::UnknownGroup(name.to_string()))?;

    for entry in aliases {
        // Group reference takes precedence over alias lookup so nesting
        // works when both registries share a name (Laravel resolves the
        // same way).
        if is_registered_group(&entry) {
            // Recurse - but pass through any UnknownAlias / nested error.
            resolve_group_inner(&entry, ancestors, seen, out).map_err(|e| match e {
                MiddlewareResolveError::UnknownGroup(missing) => {
                    MiddlewareResolveError::UnknownNestedGroup {
                        group: name.to_string(),
                        missing,
                    }
                }
                other => other,
            })?;
            continue;
        }
        let identity = alias_identity(&entry);
        if seen.contains(&identity) {
            continue;
        }
        let resolved = resolve_middleware_alias(&entry).ok_or_else(|| {
            MiddlewareResolveError::UnknownAlias {
                group: name.to_string(),
                missing: entry.clone(),
            }
        })?;
        seen.push(identity);
        out.push(resolved);
    }
    ancestors.pop();
    Ok(())
}

/// What a group entry is deduplicated by: the alias name and its
/// arguments, normalised the way [`split_alias`] reads them, so
/// `"throttle:60, 1"` and `"throttle:60,1"` are one middleware.
fn alias_identity(spec: &str) -> String {
    let (name, arguments) = split_alias(spec);
    if arguments.is_empty() {
        name.to_string()
    } else {
        format!("{name}:{}", arguments.join(","))
    }
}

fn is_registered_group(name: &str) -> bool {
    let lock = group_lock();
    let guard = match lock.read() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    guard.iter().any(|(n, _)| n == name)
}

/// Whether a group by this name has been registered.
pub fn has_middleware_group(name: &str) -> bool {
    is_registered_group(name)
}

/// All currently-registered group names (snapshot). Order matches
/// registration order.
pub fn registered_middleware_groups() -> Vec<String> {
    let lock = group_lock();
    let guard = match lock.read() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    guard.iter().map(|(n, _)| n.clone()).collect()
}

/// Remove a registered group by name. Returns `true` if a binding was
/// removed.
pub fn clear_middleware_group(name: &str) -> bool {
    let lock = group_lock();
    let mut guard = match lock.write() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    let before = guard.len();
    guard.retain(|(n, _)| n != name);
    before != guard.len()
}

/// Wipe every registered group. Test-only convenience.
#[doc(hidden)]
pub fn clear_all_middleware_groups_for_test() {
    let lock = group_lock();
    let mut guard = match lock.write() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    guard.clear();
}

/// Prepend a middleware type to the priority list, in front of every type
/// the list holds. Laravel's `prependToMiddlewarePriority`.
///
/// See [`append_middleware_priority`] for what the list does to a chain.
pub fn prepend_middleware_priority<M: Middleware + 'static>() {
    let tid = TypeId::of::<M>();
    let lock = priority_lock();
    let mut guard = match lock.write() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    if !guard.contains(&tid) {
        guard.insert(0, tid);
    }
}

/// Append a middleware type to the priority list. Laravel's
/// `appendToMiddlewarePriority`.
///
/// The list gives order-dependent middleware a safe order when global,
/// group and route registrations interleave: the session before
/// authentication, authentication before the bindings.
///
/// ```rust,no_run
/// # use suprnova::middleware::append_middleware_priority;
/// # use suprnova::{async_trait, Middleware, Next, Request, Response};
/// # struct SessionMiddleware;
/// # #[async_trait]
/// # impl Middleware for SessionMiddleware {
/// #     async fn handle(&self, request: Request, next: Next) -> Response { next(request).await }
/// # }
/// # struct AuthMiddleware;
/// # #[async_trait]
/// # impl Middleware for AuthMiddleware {
/// #     async fn handle(&self, request: Request, next: Next) -> Response { next(request).await }
/// # }
/// // The session runs before authentication on every route, whichever
/// // of the two was registered first.
/// append_middleware_priority::<SessionMiddleware>();
/// append_middleware_priority::<AuthMiddleware>();
/// ```
///
/// # What moves
///
/// When a chain runs, a middleware the list names is moved in front of any
/// middleware that the list places after it and that stands before it in
/// the chain. Nothing else moves. A middleware the list does not name keeps
/// its place behind the middleware it was registered after, so a middleware
/// registered after `AuthMiddleware` still runs after it.
///
/// The list sees the middleware registered by type: `.middleware(M)` on a
/// route or a group, `global_middleware!`, an alias. A middleware boxed by
/// hand and added with `.middleware_boxed(...)` has no type the list could
/// name, and it keeps its place.
pub fn append_middleware_priority<M: Middleware + 'static>() {
    let tid = TypeId::of::<M>();
    let lock = priority_lock();
    let mut guard = match lock.write() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    if !guard.contains(&tid) {
        guard.push(tid);
    }
}

/// Read the current priority list as a snapshot of TypeIds, first in the
/// list first.
pub fn middleware_priority() -> Vec<TypeId> {
    let lock = priority_lock();
    let guard = match lock.read() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    guard.clone()
}

/// Wipe the priority list. Test-only convenience.
#[doc(hidden)]
pub fn clear_middleware_priority_for_test() {
    let lock = priority_lock();
    let mut guard = match lock.write() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    guard.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{Request, Response};
    use crate::middleware::{Middleware, Next};
    use async_trait::async_trait;
    use std::sync::Mutex;

    /// Tests touch the process-global registries, so they all share
    /// this serial group to keep snapshot assertions reproducible.
    static SERIAL_TEST_LOCK: Mutex<()> = Mutex::new(());

    struct AuthMw;
    #[async_trait]
    impl Middleware for AuthMw {
        async fn handle(&self, request: Request, next: Next) -> Response {
            next(request).await
        }
    }

    struct ThrottleMw;
    #[async_trait]
    impl Middleware for ThrottleMw {
        async fn handle(&self, request: Request, next: Next) -> Response {
            next(request).await
        }
    }

    struct CorsMw;
    #[async_trait]
    impl Middleware for CorsMw {
        async fn handle(&self, request: Request, next: Next) -> Response {
            next(request).await
        }
    }

    fn reset_all() {
        clear_all_middleware_aliases_for_test();
        clear_all_middleware_groups_for_test();
        clear_middleware_priority_for_test();
    }

    #[test]
    fn aliases_register_resolve_and_clear() {
        let _guard = SERIAL_TEST_LOCK.lock().unwrap();
        reset_all();

        assert!(!has_middleware_alias("auth"));
        register_middleware_alias("auth", || AuthMw);
        assert!(has_middleware_alias("auth"));
        assert!(resolve_middleware_alias("auth").is_some());
        assert!(resolve_middleware_alias("missing").is_none());
        assert_eq!(registered_middleware_aliases(), vec!["auth".to_string()]);

        assert!(clear_middleware_alias("auth"));
        assert!(!has_middleware_alias("auth"));
        assert!(!clear_middleware_alias("auth"));
    }

    #[test]
    fn aliases_re_registration_is_last_wins() {
        let _guard = SERIAL_TEST_LOCK.lock().unwrap();
        reset_all();

        register_middleware_alias("auth", || AuthMw);
        register_middleware_alias("auth", || ThrottleMw);
        // Still one alias under the same name.
        assert_eq!(registered_middleware_aliases().len(), 1);
        // Resolution succeeds - the second registration won.
        assert!(resolve_middleware_alias("auth").is_some());
    }

    #[test]
    fn group_expands_to_underlying_aliases() {
        let _guard = SERIAL_TEST_LOCK.lock().unwrap();
        reset_all();

        register_middleware_alias("auth", || AuthMw);
        register_middleware_alias("throttle", || ThrottleMw);
        register_middleware_group("api", ["auth".to_string(), "throttle".to_string()]);
        let mws = resolve_middleware_group("api").expect("api group resolves");
        assert_eq!(mws.len(), 2);
    }

    #[test]
    fn group_with_missing_alias_errors_with_name() {
        let _guard = SERIAL_TEST_LOCK.lock().unwrap();
        reset_all();

        register_middleware_alias("auth", || AuthMw);
        register_middleware_group("api", ["auth".to_string(), "throttle".to_string()]);
        let err = resolve_middleware_group("api")
            .err()
            .expect("resolve must err");
        match err {
            MiddlewareResolveError::UnknownAlias { group, missing } => {
                assert_eq!(group, "api");
                assert_eq!(missing, "throttle");
            }
            other => panic!("expected UnknownAlias, got {other:?}"),
        }
    }

    #[test]
    fn group_unknown_returns_unknown_group_error() {
        let _guard = SERIAL_TEST_LOCK.lock().unwrap();
        reset_all();

        let err = resolve_middleware_group("ghost")
            .err()
            .expect("resolve must err");
        assert_eq!(
            err,
            MiddlewareResolveError::UnknownGroup("ghost".to_string())
        );
    }

    #[test]
    fn group_supports_nested_groups() {
        let _guard = SERIAL_TEST_LOCK.lock().unwrap();
        reset_all();

        register_middleware_alias("auth", || AuthMw);
        register_middleware_alias("throttle", || ThrottleMw);
        register_middleware_alias("cors", || CorsMw);
        register_middleware_group("base", ["auth".to_string()]);
        register_middleware_group(
            "api",
            [
                "base".to_string(),
                "throttle".to_string(),
                "cors".to_string(),
            ],
        );

        let mws = resolve_middleware_group("api").expect("api resolves");
        assert_eq!(mws.len(), 3);
    }

    /// A group reused by two sibling branches is a diamond, not a cycle.
    /// Cycle detection must reject only a group that reaches one of its
    /// own ancestors.
    #[test]
    fn a_nested_group_reused_by_sibling_branches_is_not_a_cycle() {
        let _guard = SERIAL_TEST_LOCK.lock().unwrap();
        reset_all();

        register_middleware_alias("auth", || AuthMw);
        register_middleware_alias("throttle", || ThrottleMw);
        register_middleware_alias("cors", || CorsMw);
        register_middleware_group("base", ["auth".to_string()]);
        register_middleware_group("read", ["base".to_string(), "throttle".to_string()]);
        register_middleware_group("write", ["base".to_string(), "cors".to_string()]);
        register_middleware_group("api", ["read".to_string(), "write".to_string()]);

        let mws = resolve_middleware_group("api").expect("a diamond of groups must resolve");
        assert_eq!(
            mws.len(),
            3,
            "base's auth appears once, where it was first reached: auth, throttle, cors"
        );

        // Listing the same group, or the same alias, twice in one group is
        // the same shape.
        register_middleware_group(
            "twice",
            ["base".to_string(), "base".to_string(), "auth".to_string()],
        );
        let mws = resolve_middleware_group("twice").expect("a repeated group must resolve");
        assert_eq!(mws.len(), 1);

        // A real cycle below the root is still refused.
        register_middleware_group("loop_a", ["loop_b".to_string()]);
        register_middleware_group("loop_b", ["base".to_string(), "loop_c".to_string()]);
        register_middleware_group("loop_c", ["loop_b".to_string()]);
        match resolve_middleware_group("loop_a") {
            Err(MiddlewareResolveError::CycleDetected { group }) => assert_eq!(group, "loop_b"),
            other => panic!("expected CycleDetected for loop_b, got {:?}", other.err()),
        }
    }

    /// Duplicates are found by alias and arguments, as Laravel compares
    /// the full `name:args` string: the same throttle spelled with other
    /// spacing is one middleware, a throttle with other limits is another.
    #[test]
    fn a_group_keeps_one_of_each_alias_and_argument_list() {
        let _guard = SERIAL_TEST_LOCK.lock().unwrap();
        reset_all();

        register_middleware_alias_with_args("limit", |_arguments: &[&str]| {
            Ok::<_, FrameworkError>(ThrottleMw)
        });
        register_middleware_group(
            "limited",
            [
                "limit:60,1".to_string(),
                "limit: 60, 1".to_string(),
                "limit:30,1".to_string(),
            ],
        );

        let mws = resolve_middleware_group("limited").expect("limited resolves");
        assert_eq!(mws.len(), 2, "limit:60,1 once, and limit:30,1");
    }

    #[test]
    fn group_cycle_detected() {
        let _guard = SERIAL_TEST_LOCK.lock().unwrap();
        reset_all();

        register_middleware_group("a", ["b".to_string()]);
        register_middleware_group("b", ["a".to_string()]);
        let err = resolve_middleware_group("a")
            .err()
            .expect("resolve must err");
        match err {
            MiddlewareResolveError::CycleDetected { group } => {
                assert!(group == "a" || group == "b");
            }
            other => panic!("expected CycleDetected, got {other:?}"),
        }
    }

    #[test]
    fn group_lookup_introspection() {
        let _guard = SERIAL_TEST_LOCK.lock().unwrap();
        reset_all();

        assert!(!has_middleware_group("web"));
        register_middleware_group("web", Vec::<String>::new());
        assert!(has_middleware_group("web"));
        assert_eq!(registered_middleware_groups(), vec!["web".to_string()]);
        assert!(clear_middleware_group("web"));
        assert!(!has_middleware_group("web"));
    }

    #[test]
    fn priority_appends_unique() {
        let _guard = SERIAL_TEST_LOCK.lock().unwrap();
        reset_all();

        append_middleware_priority::<AuthMw>();
        append_middleware_priority::<ThrottleMw>();
        // Duplicate append is a no-op.
        append_middleware_priority::<AuthMw>();

        let snapshot = middleware_priority();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0], TypeId::of::<AuthMw>());
        assert_eq!(snapshot[1], TypeId::of::<ThrottleMw>());
    }

    #[test]
    fn priority_prepend_lifts_to_front() {
        let _guard = SERIAL_TEST_LOCK.lock().unwrap();
        reset_all();

        append_middleware_priority::<AuthMw>();
        prepend_middleware_priority::<ThrottleMw>();
        let snapshot = middleware_priority();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0], TypeId::of::<ThrottleMw>());
        assert_eq!(snapshot[1], TypeId::of::<AuthMw>());
    }
}
