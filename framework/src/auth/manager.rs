//! The auth manager - Laravel's `AuthManager`.
//!
//! Holds the [`AuthConfig`] wiring plus the registered
//! [`UserProvider`]s and custom guard drivers, and resolves named guards on
//! demand. Lives in the service container
//! (`App::singleton(AuthManager::new(config))`); the static [`crate::Auth`]
//! facade reaches it through `App::get`.
//!
//! Guard instances are built per resolution rather than cached: a
//! Suprnova guard is a cheap value object (name + provider handle), and
//! all per-request state lives in [`crate::auth::request_state`] / the
//! session, never on the instance. Building fresh each call sidesteps
//! cache invalidation and keeps the manager `Clone` + `Send + Sync`
//! without locking guard instances.
//!
//! `AuthManager` is `Clone`; clones share one provider registry and one
//! driver registry (each is `Arc<RwLock<…>>`), so `App::get::<AuthManager>()`
//! returning a clone still sees providers and drivers registered through any
//! other handle.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;

use super::authenticatable::Authenticatable;
use super::config::{AuthConfig, GuardDriver};
use super::contract::{Credentials, Guard, StatefulGuard};
use super::provider::UserProvider;
use super::request_state;
use super::session_guard::SessionGuard;
use super::token_guard::TokenGuard;
use crate::error::FrameworkError;
use crate::http::Request;
use crate::render_cache::collector;

/// What a [`via_request`](AuthManager::via_request) resolver concludes about
/// one request: the user it proves, `None` when it proves nobody, or the
/// error that fails the request.
pub type RequestUserResult = Result<Option<Arc<dyn Authenticatable>>, FrameworkError>;

/// The future a [`via_request`](AuthManager::via_request) resolver returns.
/// It may borrow the request it was given.
pub type RequestUserFuture<'r> = Pin<Box<dyn Future<Output = RequestUserResult> + Send + 'r>>;

/// What a custom driver's factory returns: the guard it built, or why it
/// could not build one.
type GuardResult = Result<Arc<dyn Guard>, FrameworkError>;

/// Builds the guard of one named guard of a custom driver, from the guard's
/// name and the provider its configuration names.
type GuardFactory = dyn Fn(&str, Arc<dyn UserProvider>) -> GuardResult + Send + Sync;

/// Resolves the user of one request for a guard registered with `via_request`.
type RequestResolver = dyn for<'r> Fn(&'r Request) -> RequestUserFuture<'r> + Send + Sync;

/// The prefix of every driver name `via_request` derives. `extend` refuses
/// driver names that start with it, so the two can never collide.
const VIA_REQUEST_PREFIX: &str = "via_request:";

/// Whether `name` can qualify the principal `<guard>:<id>`. Every guard but
/// a default session or token guard attests that principal. A `:` in the
/// name would let two guards attest the same principal, and an empty name
/// would attest `:<id>`, the principal of a default-guard user whose id
/// holds a `:` (see `Auth::bare_principal`).
fn qualifies_a_principal(name: &str) -> bool {
    !name.is_empty() && !name.contains(':')
}

/// The refusal of a guard name that cannot qualify a principal (see
/// [`qualifies_a_principal`]). The text names the rule, never the name.
fn unqualifying_guard_name() -> FrameworkError {
    FrameworkError::internal(
        "The name of a guard other than the default session or token guard cannot \
         be empty and cannot contain ':': the principal it attests is '<guard>:<id>', \
         and an empty name or a ':' in the name would let two users attest the same \
         principal.",
    )
}

/// Resolves named guards from configuration + registered providers.
#[derive(Clone)]
pub struct AuthManager {
    config: Arc<AuthConfig>,
    providers: Arc<RwLock<HashMap<String, Arc<dyn UserProvider>>>>,
    drivers: Arc<RwLock<HashMap<String, Arc<GuardFactory>>>>,
    /// `via_request` resolvers, keyed by the name of the guard they serve.
    resolvers: Arc<RwLock<HashMap<String, Arc<RequestResolver>>>>,
}

impl AuthManager {
    /// Create a manager for the given configuration with no providers yet
    /// registered.
    pub fn new(config: AuthConfig) -> Self {
        Self {
            config: Arc::new(config),
            providers: Arc::new(RwLock::new(HashMap::new())),
            drivers: Arc::new(RwLock::new(HashMap::new())),
            resolvers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// The configuration this manager resolves against.
    pub fn config(&self) -> &AuthConfig {
        &self.config
    }

    /// The default guard's name (from [`AuthConfig::default_guard`]).
    pub fn default_guard_name(&self) -> &str {
        &self.config.default_guard
    }

    /// Register a [`UserProvider`] under `name`. Guards reference providers
    /// by this name in their [`crate::GuardConfig`].
    ///
    /// Idempotent-by-replacement: registering the same name twice keeps the
    /// last provider.
    pub fn register_provider(&self, name: impl Into<String>, provider: Arc<dyn UserProvider>) {
        // Recover-in-place on poison: a poisoned providers registry must not
        // take auth resolution down for every subsequent request.
        let mut map = self.providers.write().unwrap_or_else(|e| e.into_inner());
        map.insert(name.into(), provider);
    }

    /// Register the factory of a custom guard driver under `driver`.
    /// Mirrors Laravel's `Auth::extend($driver, $callback)`.
    ///
    /// Every guard declared with [`crate::GuardConfig::custom`] naming this
    /// driver resolves through `factory`, called on each resolution with the
    /// guard's name and the provider its configuration names. Registering
    /// the same driver twice keeps the last factory. The built-in `session`
    /// and `token` drivers are never replaced: a factory serves only guards
    /// declared custom.
    ///
    /// A factory error fails the resolution, and so the request that asked
    /// for the guard; it is never read as a guest. A guard that reads a
    /// credential from the request does so the way the token guard does: a
    /// middleware earlier in the chain validates the credential and records
    /// what it proved in request-scoped state that the guard reads.
    /// [`via_request`](Self::via_request) is that pattern built in.
    ///
    /// # Errors
    ///
    /// Refuses a driver name that starts with `via_request:`: that prefix
    /// belongs to the drivers [`via_request`](Self::via_request) derives.
    pub fn extend<F>(&self, driver: impl Into<String>, factory: F) -> Result<(), FrameworkError>
    where
        F: Fn(&str, Arc<dyn UserProvider>) -> Result<Arc<dyn Guard>, FrameworkError>
            + Send
            + Sync
            + 'static,
    {
        let driver = driver.into();
        if driver.starts_with(VIA_REQUEST_PREFIX) {
            return Err(FrameworkError::internal(format!(
                "The guard driver name '{driver}' is reserved: names that start with \
                 '{VIA_REQUEST_PREFIX}' belong to Auth::via_request."
            )));
        }
        let factory: Arc<GuardFactory> = Arc::new(factory);
        // Recover-in-place on poison, for the same reason as the provider
        // registry.
        let mut map = self.drivers.write().unwrap_or_else(|e| e.into_inner());
        map.insert(driver, factory);
        Ok(())
    }

    /// Register `resolver` as the way the guard `name` authenticates a
    /// request. Mirrors Laravel's `Auth::viaRequest`. The guard reports a
    /// user only after `AuthMiddleware::for_guard(name)` or
    /// `GuestMiddleware::for_guard(name)` ran the resolver for the request
    /// (or `AuthMiddleware::new()`, when it is the default guard), as the
    /// token guard needs `BearerTokenMiddleware`: resolved anywhere else,
    /// such as a handler behind no such middleware, it reports no user.
    ///
    /// This declares nothing. Declare the guard in the [`AuthConfig`] with
    /// [`crate::GuardConfig::custom`], under the driver
    /// [`via_request_driver`](Self::via_request_driver) derives from its name.
    /// That driver serves only the guard it was derived from. Its name starts
    /// with `via_request:`, a prefix [`extend`](Self::extend) refuses, so it
    /// never collides with a driver the application registers.
    ///
    /// The middleware runs the resolver at most once per guard name in one
    /// request, and binds what it returns under the guard's name; the guard
    /// reads only that binding, never another guard's user. A resolver error
    /// fails the request, under `AuthMiddleware::optional()` too: it is never
    /// read as a guest. The guard's `validate` goes through the provider its
    /// configuration names, as the token guard's does. Registering a name
    /// twice keeps the last resolver.
    ///
    /// See [`crate::Auth::via_request`] for a worked example.
    ///
    /// # Errors
    ///
    /// Refuses a guard name that is empty or contains `:`, and registers
    /// nothing: the principal a guard of the application attests is
    /// `<guard>:<id>`.
    pub fn via_request<F>(&self, name: impl Into<String>, resolver: F) -> Result<(), FrameworkError>
    where
        F: for<'r> Fn(&'r Request) -> RequestUserFuture<'r> + Send + Sync + 'static,
    {
        let name = name.into();
        if !qualifies_a_principal(&name) {
            return Err(unqualifying_guard_name());
        }
        let resolver: Arc<RequestResolver> = Arc::new(resolver);
        // Recover-in-place on poison, for the same reason as the provider
        // registry.
        let mut map = self.resolvers.write().unwrap_or_else(|e| e.into_inner());
        map.insert(name, resolver);
        Ok(())
    }

    /// The driver name [`via_request`](Self::via_request) derives for the
    /// guard `name`: `via_request:` followed by the guard name. Declare the
    /// guard with `GuardConfig::custom(AuthManager::via_request_driver(name), provider)`.
    pub fn via_request_driver(name: &str) -> String {
        format!("{VIA_REQUEST_PREFIX}{name}")
    }

    /// The resolver of `name`, when the guard is declared with the driver
    /// [`via_request`](Self::via_request) derived for it and one is
    /// registered.
    fn resolver(&self, name: &str) -> Option<Arc<RequestResolver>> {
        let config = self.config.guard_config(name)?;
        let GuardDriver::Custom(driver) = &config.driver else {
            return None;
        };
        if driver.strip_prefix(VIA_REQUEST_PREFIX) != Some(name) {
            return None;
        }
        let map = self.resolvers.read().unwrap_or_else(|e| e.into_inner());
        map.get(name).cloned()
    }

    /// Run the `via_request` resolver of the guard `name` for `request`, and
    /// bind what it returns under the guard's name, unless it already ran in
    /// this request. Does nothing for a guard of any other driver.
    ///
    /// Every middleware that takes a guard name calls this before it asks
    /// the guard. The resolver is cloned out of the registry, so no lock is
    /// held while it runs.
    pub(crate) async fn resolve_request_guard(
        &self,
        name: &str,
        request: &Request,
    ) -> Result<(), FrameworkError> {
        let Some(resolver) = self.resolver(name) else {
            return Ok(());
        };
        if request_state::request_guard_resolved(name) {
            return Ok(());
        }
        let user = resolver(request).await?;
        request_state::set_request_guard_user(name, user);
        Ok(())
    }

    /// Build the guard `name`, declared with the `via_request` driver
    /// `driver`.
    fn request_guard(
        &self,
        name: &str,
        driver: &str,
        provider: Arc<dyn UserProvider>,
    ) -> GuardResult {
        if driver.strip_prefix(VIA_REQUEST_PREFIX) != Some(name) {
            return Err(FrameworkError::internal(format!(
                "Auth guard '{name}' uses the driver '{driver}', which Auth::via_request \
                 derives for another guard. Declare it with \
                 GuardConfig::custom(AuthManager::via_request_driver(\"{name}\"), ...)."
            )));
        }
        if self.resolver(name).is_none() {
            return Err(FrameworkError::internal(format!(
                "Auth guard '{name}' uses the driver '{driver}', but no resolver is \
                 registered for it. Register one with: \
                 Auth::via_request(\"{name}\", |request| ...)"
            )));
        }
        let guard = RequestGuard {
            name: name.to_owned(),
            provider,
        };
        Ok(Arc::new(guard) as Arc<dyn Guard>)
    }

    /// Look up the factory registered for the custom `driver` of `guard`.
    ///
    /// The factory is cloned out so the registry lock is released before
    /// the factory runs: a factory that registers a driver would otherwise
    /// wait on this lock forever.
    fn factory(&self, guard: &str, driver: &str) -> Result<Arc<GuardFactory>, FrameworkError> {
        let map = self.drivers.read().unwrap_or_else(|e| e.into_inner());
        map.get(driver).cloned().ok_or_else(|| {
            FrameworkError::internal(format!(
                "Auth guard '{guard}' uses the custom driver '{driver}', but no factory is \
                 registered for that driver. Register one with: \
                 Auth::extend(\"{driver}\", |name, provider| ...)"
            ))
        })
    }

    /// Whether `name` is declared with a custom driver.
    ///
    /// Middleware takes the principal of such a guard from the guard itself,
    /// never from the session slot of its name or the default guard's
    /// request user, which belong to other guards.
    pub(crate) fn is_custom_guard(&self, name: &str) -> bool {
        let Some(config) = self.config.guard_config(name) else {
            return false;
        };
        matches!(config.driver, GuardDriver::Custom(_))
    }

    /// The provider that the configuration of the guard `name` names.
    ///
    /// A check that runs for the route's guard, such as the verified gate,
    /// asks this provider about the route's user, never the default guard's
    /// provider.
    pub(crate) fn guard_provider(
        &self,
        name: &str,
    ) -> Result<Arc<dyn UserProvider>, FrameworkError> {
        let config = self.guard_config(name)?;
        self.provider(&config.provider)
    }

    /// Look up a registered provider by name.
    fn provider(&self, name: &str) -> Result<Arc<dyn UserProvider>, FrameworkError> {
        let map = self.providers.read().unwrap_or_else(|e| e.into_inner());
        map.get(name).cloned().ok_or_else(|| {
            FrameworkError::internal(format!(
                "No UserProvider registered under '{name}'. Register one with: \
                 Auth::register_provider(\"{name}\", Arc::new(YourProvider))"
            ))
        })
    }

    /// Resolve a guard by name as the read-only [`Guard`] contract.
    ///
    /// Works for every driver (session, token, and custom). For
    /// login/logout/attempt, use [`stateful_guard`](Self::stateful_guard).
    ///
    /// A custom driver without a registered factory, or a `via_request`
    /// driver without a resolver, is an error naming the guard and the
    /// driver, never a fallback to a built-in driver.
    pub fn guard(&self, name: &str) -> Result<Arc<dyn Guard>, FrameworkError> {
        let config = self.guard_config(name)?;
        let provider = self.provider(&config.provider)?;
        Ok(match &config.driver {
            GuardDriver::Session => Arc::new(SessionGuard::named(name, provider)) as Arc<dyn Guard>,
            GuardDriver::Token => Arc::new(TokenGuard::named(name, provider)) as Arc<dyn Guard>,
            GuardDriver::Custom(driver) => {
                let inner = if driver.starts_with(VIA_REQUEST_PREFIX) {
                    self.request_guard(name, driver, provider)?
                } else {
                    let factory = self.factory(name, driver)?;
                    factory(name, provider)?
                };
                Arc::new(ObservedGuard {
                    name: name.to_owned(),
                    inner,
                }) as Arc<dyn Guard>
            }
        })
    }

    /// Resolve a guard by name as a [`StatefulGuard`] (login/logout/attempt).
    ///
    /// Errors if the named guard's driver is stateless (a token guard):
    /// stateless API auth has no login, and surfacing that as an error
    /// rather than a silent `None` tells the caller exactly why. A custom
    /// guard is read-only through the manager, so it errors the same way.
    pub fn stateful_guard(&self, name: &str) -> Result<Arc<dyn StatefulGuard>, FrameworkError> {
        let config = self.guard_config(name)?;
        let provider = self.provider(&config.provider)?;
        match &config.driver {
            GuardDriver::Session => {
                Ok(Arc::new(SessionGuard::named(name, provider)) as Arc<dyn StatefulGuard>)
            }
            GuardDriver::Token => Err(FrameworkError::internal(format!(
                "Guard '{name}' is a token guard (stateless): it has no login/logout/attempt. \
                 Use Auth::guard(\"{name}\") for read-only access."
            ))),
            GuardDriver::Custom(driver) => Err(FrameworkError::internal(format!(
                "Guard '{name}' uses the custom driver '{driver}': a custom guard is \
                 read-only through the manager, with no login/logout/attempt. \
                 Use Auth::guard(\"{name}\") for read-only access."
            ))),
        }
    }

    /// Resolve the default guard as the read-only [`Guard`] contract.
    pub fn default_guard(&self) -> Result<Arc<dyn Guard>, FrameworkError> {
        self.guard(&self.config.default_guard)
    }

    /// Resolve the [`UserProvider`] backing the default guard.
    ///
    /// Selects the provider the same way [`default_guard`](Self::default_guard)
    /// does - by the default guard's configured provider name - but hands back
    /// the bare provider rather than a guard wrapper. The auth-flow facades
    /// ([`crate::auth_flows::EmailVerification`],
    /// [`crate::auth_flows::PasswordReset`]) use this to reach the
    /// `retrieve_by_email` / `mark_email_verified` / `set_password` surface
    /// without going through a guard's request-scoped user cache.
    pub fn default_provider(&self) -> Result<Arc<dyn UserProvider>, FrameworkError> {
        let config = self.guard_config(&self.config.default_guard)?;
        self.provider(&config.provider)
    }

    /// Resolve the default guard as a [`StatefulGuard`].
    pub fn default_stateful_guard(&self) -> Result<Arc<dyn StatefulGuard>, FrameworkError> {
        self.stateful_guard(&self.config.default_guard)
    }

    /// The configuration of the guard `name`.
    ///
    /// Every resolution reads it here, so this is where a guard whose name
    /// is empty or contains `:` is refused, before anything is built or
    /// attested under it: a guard of the application, or any guard but the
    /// default guard, attests `<guard>:<id>`.
    fn guard_config(&self, name: &str) -> Result<super::config::GuardConfig, FrameworkError> {
        let config = self.config.guard_config(name).cloned().ok_or_else(|| {
            FrameworkError::internal(format!(
                "Auth guard '{name}' is not defined. Define it in your AuthConfig \
                 (e.g. AuthConfig::new(\"web\").guard(\"{name}\", GuardConfig::session(\"users\")))."
            ))
        })?;
        let attests_its_name =
            matches!(config.driver, GuardDriver::Custom(_)) || name != self.config.default_guard;
        if attests_its_name && !qualifies_a_principal(name) {
            return Err(unqualifying_guard_name());
        }
        Ok(config)
    }
}

/// A guard an application factory built, as the manager hands it out.
///
/// Records every identity the guard reveals for the render cache, as the
/// built-in guards record theirs through the request-scoped auth state: a
/// render that read the principal is then keyed by it, so a body built for
/// one identity is never served to another. The identity is recorded under
/// the guard's name (see [`Auth::guard_principal`](super::guard::Auth::guard_principal)),
/// so this guard's user `7` never matches a key built from web user `7`.
/// The guard decides everything else; this only observes its answers.
struct ObservedGuard {
    /// The guard's name, which qualifies every identity it reveals.
    name: String,
    /// The guard the application's factory returned.
    inner: Arc<dyn Guard>,
}

impl ObservedGuard {
    /// Records whose identity stands behind an authenticated answer that did
    /// not carry it, so a render that branches on the answer alone is keyed
    /// by that identity. A guard that cannot name it keeps the render out of
    /// the render cache instead.
    async fn observe_authenticated(&self) {
        if !collector::is_active() {
            return;
        }
        match self.inner.id().await {
            Ok(Some(id)) => self.observe(&id),
            _ => collector::observe_unobservable_read(),
        }
    }

    /// Records `id` as this guard's principal.
    fn observe(&self, id: &str) {
        if collector::is_active() {
            collector::observe_principal_value(&super::guard::Auth::guard_principal(
                &self.name, id,
            ));
        }
    }
}

#[async_trait]
impl Guard for ObservedGuard {
    async fn user(&self) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        collector::observe_principal_read();
        let user = self.inner.user().await?;
        if let Some(user) = &user {
            self.observe(&user.get_auth_identifier());
        }
        Ok(user)
    }

    async fn id(&self) -> Result<Option<String>, FrameworkError> {
        collector::observe_principal_read();
        let id = self.inner.id().await?;
        if let Some(id) = &id {
            self.observe(id);
        }
        Ok(id)
    }

    async fn validate(&self, credentials: &Credentials) -> Result<bool, FrameworkError> {
        self.inner.validate(credentials).await
    }

    async fn set_user(&self, user: Arc<dyn Authenticatable>) {
        self.inner.set_user(user).await;
    }

    async fn has_user(&self) -> bool {
        collector::observe_principal_read();
        let has_user = self.inner.has_user().await;
        if has_user {
            self.observe_authenticated().await;
        }
        has_user
    }

    async fn check(&self) -> Result<bool, FrameworkError> {
        collector::observe_principal_read();
        let authenticated = self.inner.check().await?;
        if authenticated {
            self.observe_authenticated().await;
        }
        Ok(authenticated)
    }

    async fn guest(&self) -> Result<bool, FrameworkError> {
        collector::observe_principal_read();
        let guest = self.inner.guest().await?;
        if !guest {
            self.observe_authenticated().await;
        }
        Ok(guest)
    }
}

/// The guard [`AuthManager::via_request`] registers.
///
/// Reads only the user its resolver bound for its own name in this request,
/// the way the token guard reads only what `BearerTokenMiddleware` bound.
struct RequestGuard {
    /// The guard's name, which keys its request-scoped binding.
    name: String,
    /// The provider of the guard's configuration, for `validate`.
    provider: Arc<dyn UserProvider>,
}

#[async_trait]
impl Guard for RequestGuard {
    async fn user(&self) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        Ok(request_state::request_guard_user(&self.name))
    }

    async fn id(&self) -> Result<Option<String>, FrameworkError> {
        let user = request_state::request_guard_user(&self.name);
        Ok(user.map(|user| user.get_auth_identifier()))
    }

    async fn validate(&self, credentials: &Credentials) -> Result<bool, FrameworkError> {
        let creds = credentials.as_value();
        match self.provider.retrieve_by_credentials(&creds).await? {
            Some(user) => self.provider.validate_credentials(&*user, &creds).await,
            None => Ok(false),
        }
    }

    async fn set_user(&self, user: Arc<dyn Authenticatable>) {
        request_state::set_request_guard_user(&self.name, Some(user));
    }

    async fn has_user(&self) -> bool {
        request_state::request_guard_user(&self.name).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Authenticatable;
    use crate::auth::config::GuardConfig;
    use crate::auth::contract::Credentials;
    use async_trait::async_trait;

    struct FakeProvider;
    #[async_trait]
    impl UserProvider for FakeProvider {
        async fn retrieve_by_id(
            &self,
            _id: &str,
        ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
            Ok(None)
        }
    }

    fn manager_with_users() -> AuthManager {
        let m = AuthManager::new(AuthConfig::default());
        m.register_provider("users", Arc::new(FakeProvider));
        m
    }

    #[test]
    fn resolves_session_guard_as_both_contracts() {
        let m = manager_with_users();
        assert!(m.guard("web").is_ok());
        assert!(m.stateful_guard("web").is_ok());
        assert_eq!(m.default_guard_name(), "web");
        assert!(m.default_guard().is_ok());
        assert!(m.default_stateful_guard().is_ok());
    }

    #[test]
    fn token_guard_resolves_as_guard_but_not_stateful() {
        let m = manager_with_users();
        assert!(m.guard("api").is_ok());
        let err = m
            .stateful_guard("api")
            .err()
            .expect("expected a 'token guard is stateless' error");
        assert!(
            err.to_string().contains("token guard"),
            "expected a 'token guard is stateless' message, got: {err}"
        );
    }

    #[test]
    fn unknown_guard_is_an_error() {
        let m = manager_with_users();
        let err = m
            .guard("ghost")
            .err()
            .expect("expected unknown-guard error");
        assert!(err.to_string().contains("not defined"));
    }

    #[test]
    fn missing_provider_is_an_error_with_remediation() {
        // No provider registered for the default "users" name.
        let m = AuthManager::new(AuthConfig::default());
        let err = m
            .guard("web")
            .err()
            .expect("expected missing-provider error");
        assert!(
            err.to_string()
                .contains("No UserProvider registered under 'users'")
        );
        assert!(err.to_string().contains("Auth::register_provider"));
    }

    #[test]
    fn register_provider_shared_across_clones() {
        let m = AuthManager::new(AuthConfig::default());
        let clone = m.clone();
        // Register through the clone; original must see it (shared registry).
        clone.register_provider("users", Arc::new(FakeProvider));
        assert!(m.guard("web").is_ok());
    }

    // A guard resolved from the manager is the real contract object.
    #[tokio::test]
    async fn resolved_guard_validate_routes_through_provider() {
        let m = manager_with_users();
        let g = m.guard("web").unwrap();
        // FakeProvider returns None for retrieve_by_credentials (trait
        // default), so validate is false - proves the wiring resolves.
        assert!(
            !g.validate(&Credentials::password("a@b.com", "x"))
                .await
                .unwrap()
        );
    }

    /// A user identified by a fixed string.
    struct PartnerUser(&'static str);

    impl Authenticatable for PartnerUser {
        fn get_auth_identifier(&self) -> String {
            self.0.to_string()
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn into_arc_any(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync> {
            self
        }
    }

    fn partner_user(id: &'static str) -> Arc<dyn Authenticatable> {
        Arc::new(PartnerUser(id))
    }

    /// A custom guard with a fixed answer: authenticated as the id it holds,
    /// or a guest when it holds none.
    struct FixedGuard(Option<&'static str>);

    #[async_trait]
    impl Guard for FixedGuard {
        async fn user(&self) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
            Ok(self.0.map(partner_user))
        }

        async fn id(&self) -> Result<Option<String>, FrameworkError> {
            Ok(self.0.map(str::to_string))
        }

        async fn validate(&self, _credentials: &Credentials) -> Result<bool, FrameworkError> {
            Ok(false)
        }

        async fn set_user(&self, _user: Arc<dyn Authenticatable>) {}

        async fn has_user(&self) -> bool {
            self.0.is_some()
        }
    }

    /// A custom guard that answers "authenticated" and names nobody.
    struct NamelessGuard;

    #[async_trait]
    impl Guard for NamelessGuard {
        async fn user(&self) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
            Ok(None)
        }

        async fn id(&self) -> Result<Option<String>, FrameworkError> {
            Ok(None)
        }

        async fn validate(&self, _credentials: &Credentials) -> Result<bool, FrameworkError> {
            Ok(false)
        }

        async fn set_user(&self, _user: Arc<dyn Authenticatable>) {}

        async fn has_user(&self) -> bool {
            true
        }

        async fn check(&self) -> Result<bool, FrameworkError> {
            Ok(true)
        }
    }

    fn partner_factory(_name: &str, _provider: Arc<dyn UserProvider>) -> GuardResult {
        Ok(Arc::new(FixedGuard(Some("api-7"))) as Arc<dyn Guard>)
    }

    fn guest_factory(_name: &str, _provider: Arc<dyn UserProvider>) -> GuardResult {
        Ok(Arc::new(FixedGuard(None)) as Arc<dyn Guard>)
    }

    fn nameless_factory(_name: &str, _provider: Arc<dyn UserProvider>) -> GuardResult {
        Ok(Arc::new(NamelessGuard) as Arc<dyn Guard>)
    }

    fn failing_factory(_name: &str, _provider: Arc<dyn UserProvider>) -> GuardResult {
        Err(FrameworkError::internal("partner key store unavailable"))
    }

    /// The default guards plus `partner`, a guard of the custom `api_key`
    /// driver backed by the `partners` provider. No factory is registered.
    fn manager_with_partner() -> AuthManager {
        let entry = GuardConfig::custom("api_key", "partners");
        let m = AuthManager::new(AuthConfig::new("web").guard("partner", entry));
        m.register_provider("users", Arc::new(FakeProvider));
        m.register_provider("partners", Arc::new(FakeProvider));
        m
    }

    #[test]
    fn custom_driver_builds_through_its_factory_with_name_and_provider() {
        let partners: Arc<dyn UserProvider> = Arc::new(FakeProvider);
        let entry = GuardConfig::custom("api_key", "partners");
        let m = AuthManager::new(AuthConfig::new("web").guard("partner", entry));
        m.register_provider("partners", partners.clone());
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = calls.clone();
        m.extend("api_key", move |name, provider| {
            let mut seen = recorded.lock().unwrap();
            seen.push((name.to_string(), provider));
            Ok(Arc::new(FixedGuard(Some("api-7"))) as Arc<dyn Guard>)
        })
        .unwrap();

        assert!(m.guard("partner").is_ok());

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "partner");
        assert!(Arc::ptr_eq(&calls[0].1, &partners));
    }

    #[test]
    fn unregistered_custom_driver_is_an_error_naming_guard_and_driver() {
        let m = manager_with_partner();
        let err = m
            .guard("partner")
            .err()
            .expect("expected a missing-driver error");
        let message = err.to_string();
        assert!(message.contains("'partner'"), "got: {message}");
        assert!(message.contains("'api_key'"), "got: {message}");
        assert!(message.contains("Auth::extend"), "got: {message}");
    }

    #[test]
    fn custom_guard_is_read_only_through_the_manager() {
        let m = manager_with_partner();
        m.extend("api_key", guest_factory).unwrap();
        assert!(m.guard("partner").is_ok());
        let err = m
            .stateful_guard("partner")
            .err()
            .expect("expected a read-only error");
        assert!(err.to_string().contains("read-only"), "got: {err}");
    }

    #[test]
    fn factory_error_fails_the_resolution() {
        let m = manager_with_partner();
        m.extend("api_key", failing_factory).unwrap();
        let err = m
            .guard("partner")
            .err()
            .expect("expected the factory's error");
        assert!(
            err.to_string().contains("key store unavailable"),
            "got: {err}"
        );
    }

    #[test]
    fn extend_is_shared_across_clones() {
        let m = manager_with_partner();
        // Register through a clone; the original must see it.
        m.clone().extend("api_key", partner_factory).unwrap();
        assert!(m.guard("partner").is_ok());
    }

    #[test]
    fn extend_never_replaces_a_built_in_driver() {
        let m = manager_with_partner();
        m.extend("session", failing_factory).unwrap();
        m.extend("token", failing_factory).unwrap();
        assert!(m.guard("web").is_ok());
        assert!(m.stateful_guard("web").is_ok());
        assert!(m.guard("api").is_ok());
    }

    #[tokio::test]
    async fn custom_guard_identity_is_recorded_for_the_render_cache() {
        let m = manager_with_partner();
        m.extend("api_key", partner_factory).unwrap();
        let guard = m.guard("partner").unwrap();
        collector::Collector::scope(async {
            assert!(guard.check().await.unwrap());
            let report = collector::current_report().expect("in scope");
            assert!(
                report
                    .gate
                    .context
                    .principal_material
                    .contains("partner:api-7")
            );
            assert!(!report.context.overflowed);
        })
        .await;
    }

    #[tokio::test]
    async fn nameless_authenticated_answer_keeps_the_render_out_of_the_cache() {
        let m = manager_with_partner();
        m.extend("api_key", nameless_factory).unwrap();
        let guard = m.guard("partner").unwrap();
        collector::Collector::scope(async {
            assert!(guard.check().await.unwrap());
            let report = collector::current_report().expect("in scope");
            assert!(report.gate.context.principal_material.is_empty());
            assert!(report.context.overflowed);
        })
        .await;
    }

    /// A resolver that proves nobody.
    fn no_one(_request: &Request) -> RequestUserFuture<'_> {
        Box::pin(async { Ok(None) })
    }

    /// The default guards plus `keyed`, declared under the driver
    /// `via_request` derives for it, and `stray`, declared under that same
    /// driver although its name differs. No resolver is registered.
    fn manager_with_request_guards() -> AuthManager {
        let keyed = GuardConfig::custom(AuthManager::via_request_driver("keyed"), "partners");
        let stray = GuardConfig::custom(AuthManager::via_request_driver("keyed"), "partners");
        let config = AuthConfig::new("web")
            .guard("keyed", keyed)
            .guard("stray", stray);
        let m = AuthManager::new(config);
        m.register_provider("users", Arc::new(FakeProvider));
        m.register_provider("partners", Arc::new(FakeProvider));
        m
    }

    #[test]
    fn via_request_driver_is_the_prefixed_guard_name() {
        assert_eq!(
            AuthManager::via_request_driver("partner"),
            "via_request:partner"
        );
    }

    #[test]
    fn extend_refuses_the_via_request_namespace() {
        let m = manager_with_partner();
        let err = m
            .extend("via_request:partner", partner_factory)
            .expect_err("expected a reserved-name error");
        assert!(err.to_string().contains("reserved"), "got: {err}");
    }

    #[test]
    fn a_colon_in_a_custom_guard_name_is_refused_when_the_guard_is_resolved() {
        let entry = GuardConfig::custom("api_key", "partners");
        let m = AuthManager::new(AuthConfig::new("web").guard("a:b", entry));
        m.register_provider("partners", Arc::new(FakeProvider));
        m.extend("api_key", guest_factory).unwrap();
        let message = m
            .guard("a:b")
            .err()
            .expect("expected a refusal")
            .to_string();
        assert!(message.contains("cannot contain ':'"), "got: {message}");
        assert!(
            !message.contains("a:b"),
            "the name is not echoed: {message}"
        );
        assert!(m.stateful_guard("a:b").is_err());
    }

    // A guard other than the default attests `<guard>:<id>`, whatever its
    // driver, so a `:` in its name is refused too. The default session or
    // token guard attests the bare id, so its name may hold one.
    #[test]
    fn a_colon_is_refused_in_every_guard_name_but_the_default_built_in_guard() {
        let config = AuthConfig::new("a:b")
            .guard("a:b", GuardConfig::session("users"))
            .guard("c:d", GuardConfig::session("users"))
            .guard("e:f", GuardConfig::token("users"));
        let m = AuthManager::new(config);
        m.register_provider("users", Arc::new(FakeProvider));
        assert!(m.guard("a:b").is_ok());
        assert!(m.stateful_guard("a:b").is_ok());
        for refused in ["c:d", "e:f"] {
            let message = m
                .guard(refused)
                .err()
                .expect("expected a refusal")
                .to_string();
            assert!(message.contains("cannot contain ':'"), "got: {message}");
            assert!(
                !message.contains(refused),
                "the name is not echoed: {message}"
            );
        }
    }

    // A guard that attests `<guard>:<id>` under an empty name would attest
    // `:<id>`, the principal a default-guard user whose id holds a `:` stands
    // for. The empty name is refused with the `:`, for every guard that
    // attests its name; the default session or token guard keeps any name.
    #[test]
    fn an_empty_name_is_refused_in_every_guard_name_but_the_default_built_in_guard() {
        let entry = GuardConfig::custom("api_key", "partners");
        let config = AuthConfig::new("web")
            .guard("", GuardConfig::session("users"))
            .guard("partner", entry);
        let m = AuthManager::new(config);
        m.register_provider("users", Arc::new(FakeProvider));
        let message = m.guard("").err().expect("expected a refusal").to_string();
        assert!(message.contains("cannot be empty"), "got: {message}");
        assert!(m.stateful_guard("").is_err());

        let default_empty =
            AuthManager::new(AuthConfig::new("").guard("", GuardConfig::session("users")));
        default_empty.register_provider("users", Arc::new(FakeProvider));
        assert!(default_empty.guard("").is_ok());

        let resolvers = manager_with_request_guards();
        let message = resolvers
            .via_request("", no_one)
            .expect_err("expected a refusal")
            .to_string();
        assert!(message.contains("cannot be empty"), "got: {message}");
    }

    #[test]
    fn via_request_refuses_a_colon_in_the_guard_name() {
        let m = manager_with_request_guards();
        let message = m
            .via_request("a:b", no_one)
            .expect_err("expected a refusal")
            .to_string();
        assert!(message.contains("cannot contain ':'"), "got: {message}");
        assert!(
            !message.contains("a:b"),
            "the name is not echoed: {message}"
        );
    }

    #[test]
    fn request_guard_needs_its_resolver() {
        let m = manager_with_request_guards();
        let err = m
            .guard("keyed")
            .err()
            .expect("expected a missing-resolver error");
        assert!(err.to_string().contains("via_request"), "got: {err}");

        m.via_request("keyed", no_one).unwrap();
        assert!(m.guard("keyed").is_ok());
        // Read-only through the manager, as every custom guard is.
        assert!(m.stateful_guard("keyed").is_err());
    }

    #[test]
    fn request_driver_serves_only_the_guard_it_was_derived_from() {
        let m = manager_with_request_guards();
        m.via_request("keyed", no_one).unwrap();
        m.via_request("stray", no_one).unwrap();
        let err = m
            .guard("stray")
            .err()
            .expect("expected a foreign-driver error");
        assert!(err.to_string().contains("another guard"), "got: {err}");
    }

    #[tokio::test]
    async fn request_guard_reads_only_its_own_binding() {
        let m = manager_with_request_guards();
        m.via_request("keyed", no_one).unwrap();
        let keyed = m.guard("keyed").unwrap();
        request_state::scope(async {
            assert!(keyed.user().await.unwrap().is_none());
            request_state::set_request_guard_user("other", Some(partner_user("api-9")));
            assert!(keyed.user().await.unwrap().is_none());
            assert!(!keyed.has_user().await);

            request_state::set_request_guard_user("keyed", Some(partner_user("api-7")));
            let user = keyed.user().await.unwrap().expect("keyed is bound");
            assert_eq!(user.get_auth_identifier(), "api-7");
            assert_eq!(keyed.id().await.unwrap().as_deref(), Some("api-7"));
            assert!(keyed.has_user().await);
        })
        .await;
    }

    // `Request::for_test` exists only with the `testing` feature, which the
    // minimal profile checked by scripts/check-feature-matrix.sh leaves off.
    #[cfg(feature = "testing")]
    #[tokio::test]
    async fn resolver_runs_once_per_guard_in_one_request() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let m = manager_with_request_guards();
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        m.via_request("keyed", move |_request| {
            counted.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(Some(partner_user("api-7"))) })
        })
        .unwrap();
        let request = Request::for_test("GET", "/");
        request_state::scope(async {
            m.resolve_request_guard("keyed", &request).await.unwrap();
            m.resolve_request_guard("keyed", &request).await.unwrap();
            // A guard of any other driver runs no resolver.
            m.resolve_request_guard("web", &request).await.unwrap();
            let keyed = m.guard("keyed").unwrap();
            assert!(keyed.check().await.unwrap());
        })
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
