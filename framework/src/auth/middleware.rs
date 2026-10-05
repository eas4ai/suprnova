//! Authentication middleware

use crate::Request;
use crate::http::{HttpResponse, Response};
use crate::middleware::{Middleware, Next};
use async_trait::async_trait;

use super::contract::Credentials;
use super::guard::Auth;

/// Authentication middleware
///
/// Protects routes that require authentication. Unauthenticated requests
/// are either redirected to a login page or receive a 401 response.
///
/// # Example
///
/// ```rust,ignore
/// use suprnova::{AuthMiddleware, group, get};
///
/// // API routes - return 401 for unauthenticated
/// group!("/api")
///     .middleware(AuthMiddleware::new())
///     .routes([...]);
///
/// // Web routes - redirect to login
/// group!("/dashboard")
///     .middleware(AuthMiddleware::redirect_to("/login"))
///     .routes([...]);
/// ```
pub struct AuthMiddleware {
    /// Path to redirect to if not authenticated (None = return 401)
    redirect_to: Option<String>,
    /// Let anonymous requests continue without principal evidence.
    optional: bool,
    /// Named guard to check (None = the sync session-backed default-guard
    /// fast path, or the default guard itself when it is a guard of the
    /// application; `Some(name)` checks that guard via the `AuthManager`).
    guard: Option<String>,
}

impl AuthMiddleware {
    /// Create middleware that returns 401 Unauthorized if not authenticated
    ///
    /// Best for API routes.
    pub fn new() -> Self {
        Self {
            redirect_to: None,
            optional: false,
            guard: None,
        }
    }

    /// Create middleware that records the principal when a user is
    /// authenticated and lets anonymous requests continue without one.
    ///
    /// Use it on routes that serve both signed-in and anonymous visitors, such
    /// as the reserved Live routes of an application whose public-seed
    /// islands accept anonymous actions. Nothing downstream is granted by the
    /// absence: an identity-bound Live mount still refuses a request without
    /// principal evidence, and ordinary handlers see `Auth::check()` as false.
    pub fn optional() -> Self {
        Self {
            redirect_to: None,
            optional: true,
            guard: None,
        }
    }

    /// Create middleware that redirects to a login page if not authenticated
    ///
    /// Best for web routes.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use suprnova::AuthMiddleware;
    /// let _mw = AuthMiddleware::redirect_to("/login");
    /// ```
    pub fn redirect_to(path: impl Into<String>) -> Self {
        Self {
            redirect_to: Some(path.into()),
            optional: false,
            guard: None,
        }
    }

    /// Check a named guard instead of the default. Chainable on `new()` /
    /// `redirect_to(...)`:
    ///
    /// ```rust,no_run
    /// # use suprnova::AuthMiddleware;
    /// let _api = AuthMiddleware::new().for_guard("api");                  // 401 if the api guard is a guest
    /// let _web = AuthMiddleware::redirect_to("/login").for_guard("web");  // otherwise redirect
    /// ```
    ///
    /// Note: a token guard (e.g. `for_guard("api")`) expects the bearer-token
    /// middleware to have run earlier in the chain to populate the request's
    /// auth id; without it the guard always reports unauthenticated.
    ///
    /// A custom guard (registered with `Auth::extend`) decides alone: the
    /// user it resolves is the request's principal, and an error it returns
    /// fails the request rather than letting it through as a guest. For a
    /// guard of `Auth::via_request`, this middleware first runs the resolver
    /// with the request, once per request, and a resolver error fails the
    /// request the same way.
    ///
    /// The route checks after it - `#[authorize]` on the route's handler,
    /// `EnsureEmailVerifiedMiddleware`, `RoleMiddleware` and
    /// `PermissionMiddleware` - ask this guard for the user, so they check
    /// the user this middleware authenticated. When several `AuthMiddleware`
    /// run, the last one that passed the request on names the guard.
    ///
    /// For a Live request, the middleware attests the guard's user as the
    /// principal: the bare id for the default guard, `<guard>:<id>` for any
    /// other guard.
    pub fn for_guard(mut self, name: impl Into<String>) -> Self {
        self.guard = Some(name.into());
        self
    }
}

impl Default for AuthMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Middleware for AuthMiddleware {
    async fn handle(&self, mut request: Request, next: Next) -> Response {
        // Resolve the user instead of trusting a persisted identifier. A
        // session can outlive its principal (deleted or soft-deleted user),
        // and an ID-presence check would keep authorizing the removed
        // identity. A provider miss clears the stale slot so the next
        // request does not carry it.
        //
        // A named guard's principal is the user that guard resolved, never
        // the default guard's request user or another guard's: the route
        // authenticated this user, and only this user may be attested. A
        // custom guard keeps its identity outside the session, so it has no
        // session slot to clear. A guard of `Auth::via_request` gets its
        // resolver's answer bound first.
        //
        // Without a guard name, a custom default guard is asked the same
        // way: the session fast path below would decide from an identity
        // that guard never reads, and attest a different one.
        let custom_default = match &self.guard {
            Some(_) => None,
            None => Auth::custom_default_guard(),
        };
        let mut guard_principal = None;
        let authenticated = match self.guard.as_deref().or(custom_default.as_deref()) {
            Some(name) => {
                let manager = Auth::manager()?;
                manager.resolve_request_guard(name, &request).await?;
                let guard = manager.guard(name)?;
                match guard.user().await? {
                    Some(user) => {
                        // Every guard but the default session or token guard
                        // has an id space of its own, so its name keeps its
                        // user `7` apart from web user `7`.
                        //
                        // The default guard's user attests its bare
                        // principal, which never reads `<guard>:<id>`: an id
                        // with a `:` gets a leading `:` (see
                        // `Auth::bare_principal`).
                        let id = user.get_auth_identifier();
                        guard_principal = Some(Auth::guard_principal(name, &id));
                        true
                    }
                    None => {
                        if !manager.is_custom_guard(name) && guard.id().await?.is_some() {
                            crate::session::middleware::clear_guard_auth_user(name);
                        }
                        false
                    }
                }
            }
            None => {
                // Fast path: no identity at all, without touching a provider.
                if Auth::id().is_none() {
                    false
                } else {
                    match Auth::user().await {
                        Ok(user) => {
                            if user.is_none() {
                                crate::session::clear_auth_user();
                            }
                            user.is_some()
                        }
                        // Providerless `login_id`-only apps have nothing to
                        // resolve against: keep the ID-presence decision.
                        // Any other resolution failure stays an error.
                        Err(e) if Auth::is_missing_provider(&e) => Auth::check(),
                        Err(e) => return Err(e.into()),
                    }
                }
            }
        };
        if authenticated {
            // Authentication proof belongs to this middleware's successful
            // branch. Merely carrying a session value or Authorization header
            // never mints principal evidence.
            if let Some(principal_id) = guard_principal.or_else(|| {
                crate::auth::request_state::current_user_id()
                    .or_else(Auth::id)
                    .map(|id| Auth::bare_principal(&id))
            }) {
                request.record_live_security_check(
                    crate::live::attestation::SecurityCheck::Principal,
                    Some(principal_id.as_bytes()),
                );
            }
            // User is authenticated, proceed
            crate::auth::request_state::set_route_guard(self.guard.clone());
            return next(request).await;
        }

        // User is not authenticated
        if self.optional {
            // The guest passes on under this guard, so `#[authorize]` finds
            // no user on it rather than asking another guard.
            crate::auth::request_state::set_route_guard(self.guard.clone());
            return next(request).await;
        }
        match &self.redirect_to {
            Some(path) => {
                // For Inertia requests, return 409 with redirect location
                // This tells Inertia to do a full page visit to the login page
                if request.is_inertia() {
                    Err(HttpResponse::text("")
                        .status(409)
                        .header("X-Inertia-Location", path.clone()))
                } else {
                    // Regular redirect for non-Inertia requests
                    Err(HttpResponse::new()
                        .status(302)
                        .header("Location", path.clone()))
                }
            }
            None => {
                // Return 401 Unauthorized
                Err(HttpResponse::json(serde_json::json!({
                    "message": "Unauthenticated."
                }))
                .status(401))
            }
        }
    }
}

/// Guest middleware
///
/// Protects routes that should only be accessible to guests (non-authenticated users).
/// Useful for login and registration pages.
///
/// # Example
///
/// ```rust,ignore
/// use suprnova::{GuestMiddleware, group, get};
///
/// group!("/")
///     .middleware(GuestMiddleware::redirect_to("/dashboard"))
///     .routes([
///         get!("/login", auth::show_login),
///         get!("/register", auth::show_register),
///     ]);
/// ```
pub struct GuestMiddleware {
    /// Path to redirect to if authenticated
    redirect_to: String,
    /// Named guard to check (None = the sync session-backed default-guard
    /// fast path; `Some(name)` checks that guard via the `AuthManager`).
    guard: Option<String>,
}

impl GuestMiddleware {
    /// Create middleware that redirects authenticated users
    ///
    /// # Arguments
    ///
    /// * `redirect_to` - Path to redirect authenticated users to
    pub fn redirect_to(path: impl Into<String>) -> Self {
        Self {
            redirect_to: path.into(),
            guard: None,
        }
    }

    /// Alias for `redirect_to` with a default path
    pub fn new() -> Self {
        Self::redirect_to("/")
    }

    /// Check a named guard instead of the default (chainable on
    /// `redirect_to(...)` / `new()`).
    ///
    /// For a guard of `Auth::via_request`, this first runs the resolver with
    /// the request, once per request; a resolver error fails the request.
    pub fn for_guard(mut self, name: impl Into<String>) -> Self {
        self.guard = Some(name.into());
        self
    }
}

impl Default for GuestMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Middleware for GuestMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let is_guest = match &self.guard {
            Some(name) => {
                let manager = Auth::manager()?;
                manager.resolve_request_guard(name, &request).await?;
                manager.guard(name)?.guest().await?
            }
            None => Auth::guest(),
        };
        if is_guest {
            // User is a guest, proceed
            return next(request).await;
        }

        // User is authenticated, redirect them away
        if request.is_inertia() {
            // For Inertia requests, return 409 with redirect location
            Err(HttpResponse::text("")
                .status(409)
                .header("X-Inertia-Location", &self.redirect_to))
        } else {
            // Regular redirect for non-Inertia requests
            Err(HttpResponse::new()
                .status(302)
                .header("Location", &self.redirect_to))
        }
    }
}

/// HTTP Basic authentication middleware.
///
/// Authenticates requests from the `Authorization: Basic <base64(user:pass)>`
/// header against a guard - mirroring Laravel's `Auth::basic` / `onceBasic`.
/// The decoded username is matched against the `field` credential (default
/// `"email"`); the password is verified by the guard's provider.
///
/// Every failure mode - a missing or malformed header, or credentials that do
/// not resolve a user - returns `401 Unauthorized` with a
/// `WWW-Authenticate: Basic realm="..."` challenge so a browser/client can
/// prompt for (new) credentials.
///
/// ```rust,ignore
/// use suprnova::{BasicAuthMiddleware, group};
///
/// // Stateful - logs the user into the session on success (Auth::basic):
/// group!("/admin").middleware(BasicAuthMiddleware::new()).routes([...]);
///
/// // Stateless - authenticates for this request only (Auth::onceBasic):
/// group!("/api").middleware(BasicAuthMiddleware::once()).routes([...]);
/// ```
pub struct BasicAuthMiddleware {
    /// Credential field the decoded username is matched against (default `email`).
    field: String,
    /// Realm advertised in the `WWW-Authenticate` challenge.
    realm: String,
    /// Named guard to authenticate against (None = the default guard).
    guard: Option<String>,
    /// When true, authenticate for this request only (`once`); when false,
    /// persist to the session (`attempt`).
    stateless: bool,
}

impl BasicAuthMiddleware {
    fn build(stateless: bool) -> Self {
        Self {
            field: "email".to_string(),
            // Default realm: the app name when set, else a neutral fallback.
            realm: std::env::var("APP_NAME").unwrap_or_else(|_| "Restricted".to_string()),
            guard: None,
            stateless,
        }
    }

    /// Stateful HTTP Basic auth against the default guard - logs the user into
    /// the session on success. Mirrors Laravel's `Auth::basic()`.
    ///
    /// Persisting the login requires `SessionMiddleware` earlier in the chain
    /// (the session write is a no-op without it); [`once`](Self::once) has no
    /// such dependency.
    pub fn new() -> Self {
        Self::build(false)
    }

    /// Stateless HTTP Basic auth against the default guard - authenticates for
    /// the current request only (no session). Mirrors Laravel's `Auth::onceBasic()`.
    pub fn once() -> Self {
        Self::build(true)
    }

    /// Set the credential field the decoded username is matched against
    /// (default `"email"`).
    pub fn field(mut self, field: impl Into<String>) -> Self {
        self.field = field.into();
        self
    }

    /// Set the realm advertised in the `WWW-Authenticate` challenge (default:
    /// the `APP_NAME` env var, else `"Restricted"`).
    pub fn realm(mut self, realm: impl Into<String>) -> Self {
        self.realm = realm.into();
        self
    }

    /// Authenticate against a named guard instead of the default.
    pub fn for_guard(mut self, name: impl Into<String>) -> Self {
        self.guard = Some(name.into());
        self
    }

    /// Decode `Authorization: Basic <base64(user:password)>` into credentials
    /// keyed by `field` + `password`. `None` when the header is absent, not the
    /// `Basic` scheme, not valid base64/UTF-8, or missing the `:` separator.
    fn decode(&self, request: &Request) -> Option<Credentials> {
        use base64::Engine as _;

        let header = request.header("authorization")?;
        let (scheme, encoded) = header.split_once(' ')?;
        if !scheme.eq_ignore_ascii_case("Basic") {
            return None;
        }
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded.trim())
            .ok()?;
        let decoded = String::from_utf8(decoded).ok()?;
        let (user, password) = decoded.split_once(':')?;
        Some(
            Credentials::new()
                .insert(self.field.clone(), user.to_string())
                .insert("password", password.to_string()),
        )
    }

    /// The `401 Unauthorized` challenge response.
    fn challenge(&self) -> HttpResponse {
        HttpResponse::json(serde_json::json!({ "message": "Invalid credentials." }))
            .status(401)
            .header(
                "WWW-Authenticate",
                format!("Basic realm=\"{}\"", quote_realm(&self.realm)),
            )
    }
}

/// Escape the realm string for inclusion inside the `quoted-string`
/// production of an HTTP `WWW-Authenticate` header (RFC 7230 §3.2.6 /
/// RFC 7617 §2). The realm originates from the operator's `APP_NAME`
/// env var, so we don't trust it to be `"`/`\`-clean - a hostname with
/// a stray `"` would otherwise smuggle the closing delimiter and
/// terminate the auth-scheme parameters early, which some user agents
/// silently misinterpret as "no realm".
///
/// Strategy:
/// - Backslash-escape `\` and `"` (the two reserved characters inside
///   a quoted-string).
/// - Drop control characters (< 0x20 except HTAB, plus DEL) entirely.
///   They're not valid `qdtext`; rather than reject the request, drop
///   them so the worst case is "realm renders with one fewer
///   character" instead of "header is invalid".
fn quote_realm(realm: &str) -> String {
    let mut out = String::with_capacity(realm.len());
    for ch in realm.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\t' => out.push(ch),
            c if (c as u32) < 0x20 || (c as u32) == 0x7F => {}
            c => out.push(c),
        }
    }
    out
}

impl Default for BasicAuthMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Middleware for BasicAuthMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        // Stateful basic short-circuits on an already-authenticated session,
        // matching Laravel's `basic()`. Stateless `once` always re-reads the
        // header.
        if !self.stateless {
            let already = match &self.guard {
                Some(name) => {
                    // Do not trust a stale session slot for an absent or
                    // stateless guard.
                    Auth::stateful_guard(name)?;
                    crate::session::middleware::persisted_guard_auth_user_id(name).is_some()
                }
                None => crate::session::middleware::persisted_guard_auth_user_id(
                    &Auth::default_guard_name(),
                )
                .is_some(),
            };
            if already {
                return next(request).await;
            }
        }

        let credentials = match self.decode(&request) {
            Some(c) => c,
            None => return Err(self.challenge()),
        };

        let authenticated = match (&self.guard, self.stateless) {
            (Some(name), true) => Auth::stateful_guard(name)?.once(&credentials).await?,
            (Some(name), false) => Auth::stateful_guard(name)?
                .attempt(&credentials, false)
                .await?
                .is_some(),
            (None, true) => Auth::once(&credentials).await?,
            (None, false) => Auth::attempt(&credentials, false).await?.is_some(),
        };

        if authenticated {
            next(request).await
        } else {
            Err(self.challenge())
        }
    }
}

#[cfg(test)]
mod realm_quoting_tests {
    use super::quote_realm;

    #[test]
    fn ascii_alnum_realm_passes_through() {
        assert_eq!(quote_realm("My App"), "My App");
    }

    #[test]
    fn embedded_double_quote_is_backslash_escaped() {
        // Operator with a hostname like `Acme "Internal" Tools`.
        // Without escaping, the inner `"` would terminate the
        // quoted-string and confuse user agents that strictly parse
        // RFC 7230 §3.2.6.
        assert_eq!(quote_realm("Acme \"Internal\""), "Acme \\\"Internal\\\"");
    }

    #[test]
    fn embedded_backslash_is_doubled() {
        assert_eq!(quote_realm("path\\to"), "path\\\\to");
    }

    #[test]
    fn control_characters_are_dropped() {
        // CR/LF are the dangerous ones - a CRLF in the realm would
        // smuggle a header injection if it survived to the wire.
        assert_eq!(quote_realm("Bad\r\nRealm"), "BadRealm");
        // TAB is preserved (it's the one allowed control character
        // inside qdtext per RFC 7230 §3.2.6).
        assert_eq!(quote_realm("Tab\there"), "Tab\there");
    }
}

// `Request::for_test` exists only with the `testing` feature, which the
// minimal profile checked by scripts/check-feature-matrix.sh leaves off.
#[cfg(all(test, feature = "testing"))]
mod custom_guard_principal_tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::auth::{AuthConfig, AuthManager, Authenticatable, Guard, GuardConfig, UserProvider};
    use crate::container::testing::TestContainer;
    use crate::error::FrameworkError;
    use crate::live::attestation::SecurityCheck;
    use crate::live::testing::{LiveTestOperation, prepare_live_request_for_test};

    /// A user identified by a fixed string.
    struct Named(&'static str);

    impl Authenticatable for Named {
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

    /// Resolves nobody: the guards under test never ask it.
    struct NoUsers;

    #[async_trait]
    impl UserProvider for NoUsers {
        async fn retrieve_by_id(
            &self,
            _id: &str,
        ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
            Ok(None)
        }
    }

    /// Accepts every request as the partner it holds.
    struct PartnerGuard(&'static str);

    #[async_trait]
    impl Guard for PartnerGuard {
        async fn user(&self) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
            Ok(Some(Arc::new(Named(self.0)) as Arc<dyn Authenticatable>))
        }

        async fn id(&self) -> Result<Option<String>, FrameworkError> {
            Ok(Some(self.0.to_string()))
        }

        async fn validate(&self, _credentials: &Credentials) -> Result<bool, FrameworkError> {
            Ok(false)
        }

        async fn set_user(&self, _user: Arc<dyn Authenticatable>) {}

        async fn has_user(&self) -> bool {
            true
        }
    }

    type GuardResult = Result<Arc<dyn Guard>, FrameworkError>;
    type Factory = fn(&str, Arc<dyn UserProvider>) -> GuardResult;

    fn api_7(_name: &str, _provider: Arc<dyn UserProvider>) -> GuardResult {
        Ok(Arc::new(PartnerGuard("api-7")) as Arc<dyn Guard>)
    }

    fn b_colon_c(_name: &str, _provider: Arc<dyn UserProvider>) -> GuardResult {
        Ok(Arc::new(PartnerGuard("b:c")) as Arc<dyn Guard>)
    }

    fn user_7(_name: &str, _provider: Arc<dyn UserProvider>) -> GuardResult {
        Ok(Arc::new(PartnerGuard("7")) as Arc<dyn Guard>)
    }

    /// Installs a manager whose guard `partner`, of the custom driver
    /// `api_key`, is built by `factory`; `default_guard` names the default.
    fn install_partner(default_guard: &str, factory: Factory) {
        install_guard("partner", default_guard, factory);
    }

    /// Installs a manager whose guard `name`, of the custom driver
    /// `api_key`, is built by `factory`; `default_guard` names the default.
    fn install_guard(name: &str, default_guard: &str, factory: Factory) {
        let entry = GuardConfig::custom("api_key", "partners");
        let config = AuthConfig::new(default_guard).guard(name, entry);
        TestContainer::singleton(AuthManager::new(config));
        Auth::register_provider("users", Arc::new(NoUsers)).unwrap();
        Auth::register_provider("partners", Arc::new(NoUsers)).unwrap();
        Auth::extend("api_key", factory).unwrap();
    }

    /// A request of a Live route, so the middleware's principal evidence is
    /// recorded on it.
    fn live_request() -> Request {
        let request = Request::for_test("GET", "/live").with_route_pattern("/live");
        prepare_live_request_for_test(request, LiveTestOperation::Action)
    }

    fn principal_fingerprint(request: &Request) -> Option<[u8; 32]> {
        let attestation = request.live_security_attestation();
        let fact = attestation.fact(request.live_request_identity(), SecurityCheck::Principal);
        fact.and_then(|fact| fact.fingerprint)
    }

    /// The principal evidence a request carries once `principal` is attested.
    fn attested(principal: &str) -> Option<[u8; 32]> {
        let mut request = live_request();
        let principal = Some(principal.as_bytes());
        let recorded = request.record_live_security_check(SecurityCheck::Principal, principal);
        assert!(recorded);
        principal_fingerprint(&request)
    }

    /// Runs `middleware` on a Live request, after `Auth::set_user` signs in
    /// `generic_user` when one is given, and returns the principal evidence
    /// the middleware attested: `None` when it refused the request.
    async fn attested_through(
        middleware: AuthMiddleware,
        generic_user: Option<&'static str>,
    ) -> Option<[u8; 32]> {
        let seen = Arc::new(Mutex::new(None));
        let recorded = seen.clone();
        let next: Next = Arc::new(move |request| {
            *recorded.lock().unwrap() = principal_fingerprint(&request);
            Box::pin(async { Ok(HttpResponse::text("reached")) })
        });
        crate::auth::request_state::scope(async {
            if let Some(id) = generic_user {
                Auth::set_user(Arc::new(Named(id)));
            }
            // A refusal is an error response and never reaches `next`.
            let _ = middleware.handle(live_request(), next).await;
        })
        .await;
        *seen.lock().unwrap()
    }

    /// Installs a manager with the session guards `web` (the default) and
    /// `admin`, over two providers.
    fn install_session_guards() {
        let config = AuthConfig::new("web").guard("admin", GuardConfig::session("admins"));
        TestContainer::singleton(AuthManager::new(config));
        Auth::register_provider("users", Arc::new(NoUsers)).unwrap();
        Auth::register_provider("admins", Arc::new(NoUsers)).unwrap();
    }

    /// Runs `middleware` on a Live request after signing in each
    /// `(guard, id)` of `signed_in` on its guard, and returns the principal
    /// evidence the middleware attested: `None` when it refused the request
    /// or attested nothing.
    async fn attested_with_guards(
        middleware: AuthMiddleware,
        signed_in: &[(&'static str, &'static str)],
    ) -> Option<[u8; 32]> {
        let seen = Arc::new(Mutex::new(None));
        let recorded = seen.clone();
        let next: Next = Arc::new(move |request| {
            *recorded.lock().unwrap() = principal_fingerprint(&request);
            Box::pin(async { Ok(HttpResponse::text("reached")) })
        });
        crate::auth::request_state::scope(async {
            for &(guard, id) in signed_in {
                let user: Arc<dyn Authenticatable> = Arc::new(Named(id));
                Auth::guard(guard).unwrap().set_user(user).await;
            }
            let _ = middleware.handle(live_request(), next).await;
        })
        .await;
        *seen.lock().unwrap()
    }

    // IDENTITY-002: a route behind a second session guard attests the user
    // that guard authenticated, under the guard's name. It never attests the
    // default guard's user, and it attests its own user when the default
    // guard has none.
    #[tokio::test]
    async fn a_second_session_guard_attests_its_own_user_under_its_name() {
        let _scope = TestContainer::fake();
        install_session_guards();
        let admin = || AuthMiddleware::new().for_guard("admin");
        assert!(attested("admin:9").is_some());

        let both = attested_with_guards(admin(), &[("web", "7"), ("admin", "9")]).await;
        assert_eq!(both, attested("admin:9"));
        assert_ne!(both, attested("7"));

        let admin_alone = attested_with_guards(admin(), &[("admin", "9")]).await;
        assert_eq!(admin_alone, attested("admin:9"));

        // The default guard, named or not, keeps the bare id.
        let web = AuthMiddleware::new().for_guard("web");
        let named_default = attested_with_guards(web, &[("web", "7"), ("admin", "9")]).await;
        assert_eq!(named_default, attested("7"));
    }

    // A default-guard user whose id reads like another guard's principal is
    // not that guard's user. Web user `admin:9` and admin user `9` are two
    // principals, whichever route attests them, and Live gates, Pusher and
    // the route's memberships, which read `Auth::route_principal`, see two
    // different strings. A default id without `:` keeps its bare value.
    #[tokio::test]
    async fn a_default_id_shaped_like_a_qualified_principal_is_a_principal_of_its_own() {
        let _scope = TestContainer::fake();
        install_session_guards();
        let admin =
            attested_with_guards(AuthMiddleware::new().for_guard("admin"), &[("admin", "9")]).await;
        assert_eq!(admin, attested("admin:9"));

        let named_default = attested_with_guards(
            AuthMiddleware::new().for_guard("web"),
            &[("web", "admin:9")],
        )
        .await;
        assert!(named_default.is_some());
        assert_ne!(named_default, admin);
        let unnamed_default =
            attested_with_guards(AuthMiddleware::new(), &[("web", "admin:9")]).await;
        assert!(unnamed_default.is_some());
        assert_ne!(unnamed_default, admin);
        let generic_user = attested_through(AuthMiddleware::new(), Some("admin:9")).await;
        assert!(generic_user.is_some());
        assert_ne!(generic_user, admin);

        let bare = attested_with_guards(AuthMiddleware::new(), &[("web", "7")]).await;
        assert_eq!(bare, attested("7"));

        let route_principal =
            |guard: Option<&'static str>, signed_in: (&'static str, &'static str)| async move {
                crate::auth::request_state::scope(async move {
                    let user: Arc<dyn Authenticatable> = Arc::new(Named(signed_in.1));
                    Auth::guard(signed_in.0).unwrap().set_user(user).await;
                    crate::auth::request_state::set_route_guard(guard.map(str::to_owned));
                    Auth::route_principal().await.unwrap()
                })
                .await
            };
        let admin_principal = route_principal(Some("admin"), ("admin", "9")).await;
        assert_eq!(admin_principal.as_deref(), Some("admin:9"));
        let web_principal = route_principal(None, ("web", "admin:9")).await;
        assert!(web_principal.is_some());
        assert_ne!(web_principal, admin_principal);
        assert_eq!(
            route_principal(None, ("web", "7")).await.as_deref(),
            Some("7")
        );
    }

    // The default guard's user sits in the request's generic slot. A route
    // behind a custom guard must attest the user that guard resolved, not
    // that one.
    #[tokio::test]
    async fn custom_guard_attests_the_user_it_resolved() {
        let _scope = TestContainer::fake();
        install_partner("web", api_7);
        let check = AuthMiddleware::new().for_guard("partner");
        let evidence = attested_through(check, Some("7")).await;
        assert!(attested("partner:api-7").is_some());
        assert_eq!(evidence, attested("partner:api-7"));
        assert_ne!(evidence, attested("7"));
    }

    // Id `7` of a guard of the application and web user `7` are two
    // principals.
    #[tokio::test]
    async fn custom_principal_carries_the_guard_name() {
        let _scope = TestContainer::fake();
        install_partner("web", user_7);
        let partner_check = AuthMiddleware::new().for_guard("partner");
        let partner = attested_through(partner_check, Some("7")).await;
        let web_check = AuthMiddleware::new().for_guard("web");
        let web = attested_through(web_check, Some("7")).await;
        assert_eq!(partner, attested("partner:7"));
        assert_eq!(web, attested("7"));
        assert_ne!(partner, web);
    }

    // A guard `a` whose user id is `b:c` attests `a:b:c`, and a guard whose
    // own name has a `:` is refused before it can attest anything.
    #[tokio::test]
    async fn a_colon_in_an_id_is_attested_and_a_colon_in_a_guard_name_is_refused() {
        let _scope = TestContainer::fake();
        install_guard("a", "web", b_colon_c);
        let check = AuthMiddleware::new().for_guard("a");
        let evidence = attested_through(check, None).await;
        assert!(attested("a:b:c").is_some());
        assert_eq!(evidence, attested("a:b:c"));

        install_guard("a:b", "web", user_7);
        let refused = AuthMiddleware::new().for_guard("a:b");
        assert_eq!(attested_through(refused, None).await, None);
    }

    // Without a guard name, a custom default guard decides, and the principal
    // is the user it returned, never the generic user.
    #[tokio::test]
    async fn unnamed_check_asks_and_attests_a_custom_default_guard() {
        let _scope = TestContainer::fake();
        install_partner("partner", api_7);
        let expected = attested("partner:api-7");
        let with_generic_user = attested_through(AuthMiddleware::new(), Some("7")).await;
        let alone = attested_through(AuthMiddleware::new(), None).await;
        assert!(expected.is_some());
        assert_eq!(with_generic_user, expected);
        assert_eq!(alone, expected);
    }
}
