//! Request-scoped authentication state.
//!
//! Laravel caches the resolved user on the guard *instance* for the
//! duration of a request. Suprnova's guards are container singletons,
//! not per-request objects, so the per-request "who is authenticated
//! right now" slot lives here in a [`tokio::task_local!`] scoped once at
//! the request boundary (`handle_request`), alongside the Inertia flash
//! bag and the SSR-disable flag.
//!
//! The generic current-user slot supports session guards and the static
//! [`crate::Auth`] facade. Bearer credentials also carry explicit provenance
//! in separate fields so a web session or its cached user can never satisfy a
//! [`TokenGuard`](super::TokenGuard).
//!
//! It serves three jobs:
//!
//! 1. **Current user** - the [`Authenticatable`] resolved for this
//!    request. Set by `once`/`once_using_id`/`set_user`, and by a guard's
//!    first `user()` resolution (a per-request cache so repeated lookups
//!    don't re-query the provider - closing a divergence where the old
//!    `Auth::user()` re-queried on every call). `current_user_id` feeds
//!    `Auth::id()` so the static facade sees `once`/`set_user`.
//! 2. **Bearer provenance** - the bearer-authenticated id and optional
//!    resolved user. `BearerTokenMiddleware` sets the id after validating a
//!    token; `TokenGuard` resolves and caches the full user lazily. Both
//!    setters mirror into the generic slots so token-only `Auth::id()` and
//!    `AuthMiddleware` behavior stays unchanged.
//! 3. **Via-remember flag** - whether the current user was
//!    re-authenticated from a remember-me cookie *this request* (set by
//!    `SessionMiddleware`'s hydration path) rather than from an active
//!    session, surfaced through `StatefulGuard::via_remember`.
//!
//! Session guard identities and remember provenance are keyed by the complete
//! guard name. The generic slots remain the compatibility view used by
//! [`crate::Auth`] and are mirrored only from the configured default guard.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use super::authenticatable::Authenticatable;

struct ActiveRememberCarrier {
    guard: String,
    selector: String,
    verified_owner: Option<String>,
}

/// The per-request authentication slot. See the module docs.
#[derive(Default)]
struct AuthRequestState {
    /// Users resolved through session guards, keyed by complete guard name.
    guard_users: HashMap<String, Arc<dyn Authenticatable>>,
    /// Session-guard identifiers known before their user is resolved.
    guard_user_ids: HashMap<String, String>,
    /// Guards whose current identity came from remember-me hydration.
    remembered_guards: HashSet<String>,
    /// The single inbound or most recently queued remember carrier.
    active_remember_carrier: Option<ActiveRememberCarrier>,
    /// The user resolved for this request, if any.
    current_user: Option<Arc<dyn Authenticatable>>,
    /// The authenticated identifier when only the id is known.
    ///
    /// A bearer-token request learns its user id from the token store
    /// before any provider lookup happens, and forcing a lookup there
    /// would put a database round-trip on every request - including
    /// requests that never consult `Auth`. `TokenGuard::user()` already
    /// resolves and caches the full user lazily, so this slot exists to
    /// carry the id until something actually needs the user.
    current_user_id: Option<String>,
    /// The user resolved specifically from bearer-token provenance.
    bearer_user: Option<Arc<dyn Authenticatable>>,
    /// The identifier validated by bearer-token middleware.
    bearer_user_id: Option<String>,
    /// Whether the current user came from a remember-me cookie this
    /// request rather than an active session.
    via_remember: bool,
}

tokio::task_local! {
    // `Arc<Mutex<…>>` rather than a bare cell: the future inside
    // `scope` may move across worker threads at `.await` points (so the
    // value must be `Send + Sync`), and setters mutate it after the
    // scope is installed. The guard is only ever held across synchronous
    // closures - never across an `.await` - so the std mutex is sound.
    static AUTH_STATE: Arc<Mutex<AuthRequestState>>;
}

/// Reads a field of the request-scoped auth state, recording an identity
/// observation before running the read. Every read accessor in this module
/// that reveals an identity or identity-adjacent provenance - a resolved
/// user, an id, a remember flag, a carrier - goes through this single choke
/// point (fix round 3, item 1), so a new accessor built on it inherits the
/// record automatically instead of depending on someone remembering to call
/// `observe_principal_read` at the call site. This is the same "sweep by
/// seam, not by name" technique used to close the write side: enumerating
/// call sites can miss one; enumerating the one seam every read passes
/// through cannot.
///
/// This closed a proven leak: `suprnova::auth_user_id()` (and
/// `guard_auth_user_id`, `SessionGuard`/`TokenGuard`'s accessors, and
/// `session::is_authenticated()`, all built on the functions below) read
/// `current_user_id()` here directly, bypassing `Auth::id()`'s own explicit
/// `observe_principal_read()` call entirely - a bearer-token, `set_user`, or
/// remember-me identity was read with no observation at all, so a render
/// using any of those accessors classified as if it had read nothing
/// private, and could publish one identity's body under a shared key served
/// back to a different identity.
///
/// Returns `None` outside a request scope, matching every accessor's
/// original fallback behavior exactly (`AUTH_STATE.try_with` fails, `.ok()`
/// converts to `None`) - `observe_principal_read` is itself a no-op outside
/// an active [`crate::render_cache::collector::Collector`] scope, so this
/// costs nothing extra on that path either.
fn read_state<R>(f: impl FnOnce(&AuthRequestState) -> R) -> Option<R> {
    crate::render_cache::collector::observe_principal_read();
    AUTH_STATE
        .try_with(|state| f(&state.lock().unwrap_or_else(|e| e.into_inner())))
        .ok()
}

/// Run `fut` with a fresh request-scoped auth state installed.
///
/// Called once per request from `handle_request`, nested next to the
/// Inertia flash-bag and SSR scopes so every middleware and handler
/// downstream can read and write the current user.
pub(crate) async fn scope<F: std::future::Future>(fut: F) -> F::Output {
    AUTH_STATE
        .scope(Arc::new(Mutex::new(AuthRequestState::default())), fut)
        .await
}

/// Set the user resolved for this request.
///
/// No-op when called outside a request scope (e.g. a unit test that did
/// not install one) - the same fail-quiet posture as the session
/// helpers.
pub(crate) fn set_current_user(user: Arc<dyn Authenticatable>) {
    let _ = AUTH_STATE.try_with(|state| {
        state.lock().unwrap_or_else(|e| e.into_inner()).current_user = Some(user);
    });
}

/// The user resolved for this request, if any.
pub(crate) fn current_user() -> Option<Arc<dyn Authenticatable>> {
    let user = read_state(|state| state.current_user.clone()).flatten();
    if let Some(user) = &user {
        crate::render_cache::collector::observe_principal_value(&user.get_auth_identifier());
    }
    user
}

fn is_default_guard(guard: &str) -> bool {
    guard == super::guard::Auth::default_guard_name()
}

/// Cache a user for one named session guard.
pub(crate) fn set_guard_user(guard_name: &str, user: Arc<dyn Authenticatable>) {
    let user_id = user.get_auth_identifier();
    let mirror_generic = is_default_guard(guard_name);
    let _ = AUTH_STATE.try_with(|state| {
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        state
            .guard_users
            .insert(guard_name.to_owned(), user.clone());
        state
            .guard_user_ids
            .insert(guard_name.to_owned(), user_id.clone());
        if mirror_generic {
            state.current_user = Some(user);
            state.current_user_id = Some(user_id);
        }
    });
}

/// Return the user cached for one named session guard.
pub(crate) fn guard_user(guard_name: &str) -> Option<Arc<dyn Authenticatable>> {
    let user = read_state(|state| state.guard_users.get(guard_name).cloned()).flatten();
    if let Some(user) = &user {
        crate::render_cache::collector::observe_principal_value(&user.get_auth_identifier());
    }
    user
}

/// Record an identifier for one named session guard.
pub(crate) fn set_guard_user_id(guard_name: &str, user_id: impl Into<String>) {
    let user_id = user_id.into();
    let mirror_generic = is_default_guard(guard_name);
    let _ = AUTH_STATE.try_with(|state| {
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        let resolved_id = state
            .guard_users
            .get(guard_name)
            .map(|user| user.get_auth_identifier());
        if resolved_id.as_deref() != Some(user_id.as_str()) {
            state.guard_users.remove(guard_name);
        }
        state
            .guard_user_ids
            .insert(guard_name.to_owned(), user_id.clone());
        if mirror_generic {
            let generic_resolved_id = state
                .current_user
                .as_ref()
                .map(|user| user.get_auth_identifier());
            if generic_resolved_id.as_deref() != Some(user_id.as_str()) {
                state.current_user = None;
            }
            state.current_user_id = Some(user_id);
        }
    });
}

/// Return the identifier known for one named session guard.
pub(crate) fn guard_user_id(guard_name: &str) -> Option<String> {
    let id = read_state(|state| {
        state
            .guard_users
            .get(guard_name)
            .map(|user| user.get_auth_identifier())
            .or_else(|| state.guard_user_ids.get(guard_name).cloned())
    })
    .flatten();
    if let Some(id) = &id {
        crate::render_cache::collector::observe_principal_value(id);
    }
    id
}

/// Clear one named session guard without touching sibling guards.
pub(crate) fn clear_guard_user(guard_name: &str) {
    let clear_generic = is_default_guard(guard_name);
    let _ = AUTH_STATE.try_with(|state| {
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        state.guard_users.remove(guard_name);
        state.guard_user_ids.remove(guard_name);
        state.remembered_guards.remove(guard_name);
        if clear_generic {
            state.current_user = None;
            state.current_user_id = None;
            state.bearer_user = None;
            state.bearer_user_id = None;
            state.via_remember = false;
        }
    });
}

/// Clear every authentication identity and provenance slot in this request.
pub(crate) fn clear_all_authentication() {
    let _ = AUTH_STATE.try_with(|state| {
        *state.lock().unwrap_or_else(|error| error.into_inner()) = AuthRequestState::default();
    });
}

/// Whether one named session guard already resolved a user instance.
pub(crate) fn has_guard_user(guard_name: &str) -> bool {
    read_state(|state| state.guard_users.contains_key(guard_name)).unwrap_or(false)
}

/// Mark whether one guard was hydrated from a remember carrier.
pub(crate) fn set_guard_via_remember(guard_name: &str, value: bool) {
    let mirror_generic = is_default_guard(guard_name);
    let _ = AUTH_STATE.try_with(|state| {
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        if value {
            state.remembered_guards.insert(guard_name.to_owned());
        } else {
            state.remembered_guards.remove(guard_name);
        }
        if mirror_generic {
            state.via_remember = value;
        }
    });
}

/// Whether one guard was hydrated from a remember carrier this request.
pub(crate) fn guard_via_remember(guard_name: &str) -> bool {
    read_state(|state| state.remembered_guards.contains(guard_name)).unwrap_or(false)
}

/// Record the guard and non-secret selector carried by the browser.
pub(crate) fn set_active_remember_carrier(guard_name: &str, selector: &str) {
    let _ = AUTH_STATE.try_with(|state| {
        state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .active_remember_carrier = Some(ActiveRememberCarrier {
            guard: guard_name.to_owned(),
            selector: selector.to_owned(),
            verified_owner: None,
        });
    });
}

/// Record a rotated carrier whose owner was verified before a retryable failure.
pub(crate) fn set_verified_active_remember_carrier(
    guard_name: &str,
    user_id: &str,
    selector: &str,
) {
    let _ = AUTH_STATE.try_with(|state| {
        state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .active_remember_carrier = Some(ActiveRememberCarrier {
            guard: guard_name.to_owned(),
            selector: selector.to_owned(),
            verified_owner: Some(user_id.to_owned()),
        });
    });
}

/// Return the verified owner and selector for one retryable rotated carrier.
pub(crate) fn verified_active_remember_carrier_for_guard(
    guard_name: &str,
) -> Option<(String, String)> {
    read_state(|state| {
        state
            .active_remember_carrier
            .as_ref()
            .filter(|carrier| carrier.guard == guard_name)
            .and_then(|carrier| {
                carrier
                    .verified_owner
                    .as_ref()
                    .map(|owner| (owner.clone(), carrier.selector.clone()))
            })
    })
    .flatten()
}

/// Return the active carrier's guard and non-secret selector.
pub(crate) fn active_remember_carrier() -> Option<(String, String)> {
    read_state(|state| {
        state
            .active_remember_carrier
            .as_ref()
            .map(|carrier| (carrier.guard.clone(), carrier.selector.clone()))
    })
    .flatten()
}

/// Return the active carrier selector when it belongs to one guard.
pub(crate) fn active_remember_selector_for_guard(guard_name: &str) -> Option<String> {
    read_state(|state| {
        state
            .active_remember_carrier
            .as_ref()
            .filter(|carrier| carrier.guard == guard_name)
            .map(|carrier| carrier.selector.clone())
    })
    .flatten()
}

/// Forget the active carrier only when both its guard and selector match.
pub(crate) fn take_active_remember_carrier(guard_name: &str, selector: &str) -> bool {
    // Read-and-clear, not a pure read, so it cannot go through `read_state`
    // (which only hands out `&AuthRequestState`); instrumented explicitly
    // here instead - still the same seam, `AUTH_STATE` itself.
    crate::render_cache::collector::observe_principal_read();
    AUTH_STATE
        .try_with(|state| {
            let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
            let matches = state
                .active_remember_carrier
                .as_ref()
                .is_some_and(|carrier| carrier.guard == guard_name && carrier.selector == selector);
            if matches {
                state.active_remember_carrier = None;
            }
            matches
        })
        .unwrap_or(false)
}

/// Forget whichever inbound or outbound remember carrier is active.
pub(crate) fn clear_active_remember_carrier() {
    let _ = AUTH_STATE.try_with(|state| {
        state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .active_remember_carrier = None;
    });
}

/// Forget the active carrier when it belongs to one guard.
pub(crate) fn clear_active_remember_carrier_for_guard(guard_name: &str) {
    let _ = AUTH_STATE.try_with(|state| {
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        if state
            .active_remember_carrier
            .as_ref()
            .is_some_and(|carrier| carrier.guard == guard_name)
        {
            state.active_remember_carrier = None;
        }
    });
}

/// Record the authenticated identifier when the full user has not been
/// resolved yet.
///
/// Set by `BearerTokenMiddleware` after it validates the token. A later
/// `set_current_user` takes precedence - see `current_user_id`.
///
/// No-op outside a request scope, matching `set_current_user`.
///
/// Gated the same way its only caller is: `magnetar_integration` needs a
/// database backend, so without one this is genuinely dead and warns under
/// `--no-default-features`. The `test` arm keeps the unit tests below
/// reachable in every profile.
#[cfg(any(
    test,
    feature = "database-sqlite",
    feature = "database-postgres",
    feature = "database-mysql"
))]
pub(crate) fn set_bearer_user_id(id: impl Into<String>) {
    let id = id.into();
    let _ = AUTH_STATE.try_with(|state| {
        let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
        if guard.bearer_user_id.as_deref() != Some(id.as_str()) {
            let current_user_is_bearer = match (&guard.current_user, &guard.bearer_user) {
                (Some(current_user), Some(bearer_user)) => Arc::ptr_eq(current_user, bearer_user),
                _ => false,
            };
            if current_user_is_bearer {
                guard.current_user = None;
            }
            guard.bearer_user = None;
        }
        guard.bearer_user_id = Some(id.clone());
        guard.current_user_id = Some(id);
    });
}

/// The identifier validated from a bearer token, if any.
pub(crate) fn bearer_user_id() -> Option<String> {
    let id = read_state(|state| state.bearer_user_id.clone()).flatten();
    if let Some(id) = &id {
        crate::render_cache::collector::observe_principal_value(id);
    }
    id
}

/// Cache a user resolved specifically through bearer authentication.
///
/// The generic mirrors preserve token-only [`crate::Auth`] facade behavior
/// without allowing generic web identity to flow back into `TokenGuard`.
pub(crate) fn set_bearer_user(user: Arc<dyn Authenticatable>) {
    let id = user.get_auth_identifier();
    let _ = AUTH_STATE.try_with(|state| {
        let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
        guard.current_user = Some(user.clone());
        guard.current_user_id = Some(id.clone());
        guard.bearer_user = Some(user);
        guard.bearer_user_id = Some(id);
    });
}

/// The user resolved specifically through bearer authentication, if any.
pub(crate) fn bearer_user() -> Option<Arc<dyn Authenticatable>> {
    let user = read_state(|state| state.bearer_user.clone()).flatten();
    if let Some(user) = &user {
        crate::render_cache::collector::observe_principal_value(&user.get_auth_identifier());
    }
    user
}

/// Whether a bearer user has already been resolved for this request.
pub(crate) fn has_bearer_user() -> bool {
    read_state(|state| state.bearer_user.is_some()).unwrap_or(false)
}

/// The current request user's identifier, if one is known.
///
/// A fully-resolved [`Authenticatable`] wins over the id-only slot: when
/// both are present the resolved user is the more authoritative value
/// (it came from the provider, not from a token payload).
pub(crate) fn current_user_id() -> Option<String> {
    let id = read_state(|state| {
        state
            .current_user
            .as_ref()
            .map(|user| user.get_auth_identifier())
            .or_else(|| state.current_user_id.clone())
    })
    .flatten();
    if let Some(id) = &id {
        crate::render_cache::collector::observe_principal_value(id);
    }
    id
}

/// Clear the resolved request user (used by `logout`).
pub(crate) fn clear_current_user() {
    let _ = AUTH_STATE.try_with(|state| {
        let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
        guard.current_user = None;
        guard.current_user_id = None;
        guard.bearer_user = None;
        guard.bearer_user_id = None;
    });
}

/// Whether a user instance has been resolved for this request - without
/// triggering provider resolution. Backs [`Guard::has_user`](super::Guard::has_user).
pub(crate) fn has_current_user() -> bool {
    read_state(|state| state.current_user.is_some()).unwrap_or(false)
}

/// Whether the current user came from a remember-me cookie this request.
/// Backs [`StatefulGuard::via_remember`](super::StatefulGuard::via_remember).
pub(crate) fn via_remember() -> bool {
    read_state(|state| state.via_remember).unwrap_or(false)
}

/// Test-only: run `fut` with a fresh request-scoped auth state.
///
/// Mirrors the per-request scope `handle_request` installs at runtime,
/// for unit/integration tests that drive guards without booting a
/// server. Crates outside the framework should not need this.
#[doc(hidden)]
pub async fn request_state_scope_for_test<F: std::future::Future>(fut: F) -> F::Output {
    scope(fut).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::Any;

    #[derive(Clone)]
    struct TestUser {
        id: String,
    }

    impl Authenticatable for TestUser {
        fn get_auth_identifier(&self) -> String {
            self.id.clone()
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
            self
        }
    }

    #[tokio::test]
    async fn current_user_round_trips_within_scope() {
        scope(async {
            assert!(current_user().is_none());
            assert!(!has_current_user());
            assert_eq!(current_user_id(), None);

            set_current_user(Arc::new(TestUser { id: "42".into() }));
            assert!(has_current_user());
            assert_eq!(current_user_id(), Some("42".to_string()));

            clear_current_user();
            assert!(!has_current_user());
            assert_eq!(current_user_id(), None);
        })
        .await;
    }

    #[tokio::test]
    async fn named_session_guard_state_is_independent() {
        scope(async {
            set_guard_user("web", Arc::new(TestUser { id: "7".into() }));
            set_guard_user("admin", Arc::new(TestUser { id: "9".into() }));
            set_guard_via_remember("admin", true);

            assert_eq!(guard_user_id("web").as_deref(), Some("7"));
            assert_eq!(guard_user_id("admin").as_deref(), Some("9"));
            assert_eq!(current_user_id().as_deref(), Some("7"));
            assert!(!guard_via_remember("web"));
            assert!(guard_via_remember("admin"));
            assert!(!via_remember());

            clear_guard_user("admin");
            assert_eq!(guard_user_id("admin"), None);
            assert_eq!(guard_user_id("web").as_deref(), Some("7"));
            assert_eq!(current_user_id().as_deref(), Some("7"));
        })
        .await;
    }

    #[tokio::test]
    async fn set_bearer_user_id_mirrors_into_generic_identity() {
        request_state_scope_for_test(async {
            assert_eq!(current_user_id(), None);
            assert_eq!(bearer_user_id(), None);

            set_bearer_user_id("usr_only_id");
            assert_eq!(current_user_id(), Some("usr_only_id".to_string()));
            assert_eq!(bearer_user_id(), Some("usr_only_id".to_string()));
        })
        .await;
    }

    #[tokio::test]
    async fn set_bearer_user_mirrors_user_and_identifier() {
        request_state_scope_for_test(async {
            set_bearer_user(Arc::new(TestUser {
                id: "bearer-7".into(),
            }));

            assert_eq!(current_user_id(), Some("bearer-7".to_string()));
            assert_eq!(bearer_user_id(), Some("bearer-7".to_string()));
            assert_eq!(
                bearer_user()
                    .expect("bearer user is cached")
                    .get_auth_identifier(),
                "bearer-7"
            );
            assert!(has_bearer_user());
        })
        .await;
    }

    #[tokio::test]
    async fn generic_resolved_user_takes_precedence_in_generic_identity() {
        request_state_scope_for_test(async {
            // The id-only slot is set first, as `BearerTokenMiddleware` would
            // do before any provider lookup.
            set_bearer_user_id("id-only-99");
            assert_eq!(current_user_id(), Some("id-only-99".to_string()));

            // A later generic user resolution, such as a web guard lookup,
            // still wins in the generic facade view. Bearer-specific readers
            // continue to use the separate provenance fields.
            set_current_user(Arc::new(TestUser {
                id: "resolved-7".into(),
            }));
            assert_eq!(current_user_id(), Some("resolved-7".to_string()));
        })
        .await;
    }

    #[tokio::test]
    async fn clear_current_user_clears_bearer_provenance() {
        request_state_scope_for_test(async {
            set_bearer_user(Arc::new(TestUser {
                id: "usr_to_clear".into(),
            }));
            assert_eq!(current_user_id(), Some("usr_to_clear".to_string()));
            assert_eq!(bearer_user_id(), Some("usr_to_clear".to_string()));
            assert!(has_bearer_user());

            clear_current_user();

            // Logout must not leave either the generic mirrors or the
            // bearer-specific provenance authenticated.
            assert_eq!(current_user_id(), None);
            assert_eq!(bearer_user_id(), None);
            assert!(bearer_user().is_none());
            assert!(!has_bearer_user());
        })
        .await;
    }

    #[tokio::test]
    async fn via_remember_round_trips_within_scope() {
        scope(async {
            assert!(!via_remember());
            set_guard_via_remember("web", true);
            assert!(via_remember());
        })
        .await;
    }

    #[tokio::test]
    async fn helpers_are_inert_outside_a_scope() {
        // No scope installed: getters fall back to None/false and
        // setters silently no-op rather than panic.
        assert!(current_user().is_none());
        assert_eq!(current_user_id(), None);
        assert!(bearer_user().is_none());
        assert_eq!(bearer_user_id(), None);
        assert!(!has_current_user());
        assert!(!has_bearer_user());
        assert!(!via_remember());
        set_current_user(Arc::new(TestUser { id: "1".into() }));
        set_bearer_user(Arc::new(TestUser { id: "2".into() }));
        set_bearer_user_id("3");
        set_guard_via_remember("web", true);
        assert!(current_user().is_none());
        assert!(bearer_user().is_none());
        assert!(!via_remember());
    }
}
