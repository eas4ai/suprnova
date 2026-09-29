//! Guards of an application-registered driver: `Auth::extend` registers the
//! factory, `GuardConfig::custom` declares the guard, and `Auth::guard` and
//! `AuthMiddleware::for_guard` resolve it by name.
//!
//! The partner guard reads an API key the way the token guard reads a bearer
//! id: a middleware earlier in the chain binds the key for the request, and
//! the guard reads what was bound. The guards of `Auth::via_request` have
//! that middleware built in: `AuthMiddleware::for_guard` and
//! `GuestMiddleware::for_guard` run their resolver with the request.

use std::any::Any;
use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use hyper::server::conn::http1::Builder;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use reqwest::Client;
use suprnova::http::text;
use suprnova::testing::TestContainer;
use suprnova::{
    Auth, AuthConfig, AuthManager, AuthMiddleware, Authenticatable, Credentials, FrameworkError,
    Guard, GuardConfig, GuestMiddleware, Middleware, MiddlewareRegistry, Next, Request, Response,
    Router, UserProvider, handle_request,
};
use tokio::net::TcpListener;
use tokio::time::timeout;

use crate::env_snapshot::{EnvSnapshot, set_env};

/// The key the partner guard accepts.
const PARTNER_KEY: &str = "partner-key-7f3a";
/// A key whose lookup fails, so the partner guard answers with an error.
const FAILING_KEY: &str = "lookup-fails-9c2e";
/// The key the `second` request guard accepts, from `X-Second-Key`.
const SECOND_KEY: &str = "second-key-41d0";

type GuardResult = Result<Arc<dyn Guard>, FrameworkError>;
type UserResult = Result<Option<Arc<dyn Authenticatable>>, FrameworkError>;

/// The error of a key lookup that failed. It names the key on purpose: the
/// tests prove that a response in production never carries it.
fn lookup_failed(key: &str) -> FrameworkError {
    FrameworkError::internal(format!("partner key store unavailable for key {key}"))
}

tokio::task_local! {
    /// The API key the request carried, bound by `ApiKeyScope` for the rest
    /// of the chain.
    static API_KEY: Option<String>;
}

/// A user identified by a fixed string.
struct Named(&'static str);

impl Authenticatable for Named {
    fn get_auth_identifier(&self) -> String {
        self.0.to_string()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

fn named(id: &'static str) -> Arc<dyn Authenticatable> {
    Arc::new(Named(id))
}

/// Knows the partner `api-7` and the web user `7`.
struct Directory;

#[async_trait]
impl UserProvider for Directory {
    async fn retrieve_by_id(
        &self,
        id: &str,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        Ok(match id {
            "api-7" => Some(named("api-7")),
            "7" => Some(named("7")),
            _ => None,
        })
    }
}

/// Accepts the request whose API key is the partner's, refuses any other,
/// and fails when the key lookup fails.
struct ApiKeyGuard {
    provider: Arc<dyn UserProvider>,
}

#[async_trait]
impl Guard for ApiKeyGuard {
    async fn user(&self) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        let key = API_KEY.try_with(Clone::clone).ok().flatten();
        match key.as_deref() {
            Some(PARTNER_KEY) => self.provider.retrieve_by_id("api-7").await,
            Some(FAILING_KEY) => Err(lookup_failed(FAILING_KEY)),
            _ => Ok(None),
        }
    }

    async fn id(&self) -> Result<Option<String>, FrameworkError> {
        Ok(self.user().await?.map(|user| user.get_auth_identifier()))
    }

    async fn validate(&self, _credentials: &Credentials) -> Result<bool, FrameworkError> {
        Ok(false)
    }

    async fn set_user(&self, _user: Arc<dyn Authenticatable>) {}

    async fn has_user(&self) -> bool {
        false
    }
}

fn api_key_guard(_name: &str, provider: Arc<dyn UserProvider>) -> GuardResult {
    Ok(Arc::new(ApiKeyGuard { provider }) as Arc<dyn Guard>)
}

fn failing_guard(_name: &str, _provider: Arc<dyn UserProvider>) -> GuardResult {
    Err(FrameworkError::internal("partner guard misconfigured"))
}

/// Binds the request's `X-Api-Key` for the partner guard.
struct ApiKeyScope;

#[async_trait]
impl Middleware for ApiKeyScope {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let key = request.header("x-api-key").map(str::to_owned);
        API_KEY.scope(key, next(request)).await
    }
}

/// Signs the web guard in for this request.
struct WebUserScope;

#[async_trait]
impl Middleware for WebUserScope {
    async fn handle(&self, request: Request, next: Next) -> Response {
        Auth::set_user(named("7"));
        next(request).await
    }
}

/// Installs a container-scoped manager: the default `web` and `api` guards,
/// `partner` of the `api_key` driver, `broken` of a driver whose factory
/// fails, and `orphan` of a driver nobody registered.
fn install_manager() {
    let config = AuthConfig::new("web")
        .guard("partner", GuardConfig::custom("api_key", "partners"))
        .guard("broken", GuardConfig::custom("failing", "partners"))
        .guard("orphan", GuardConfig::custom("unregistered", "partners"));
    TestContainer::singleton(AuthManager::new(config));
    Auth::register_provider("users", Arc::new(Directory)).unwrap();
    Auth::register_provider("partners", Arc::new(Directory)).unwrap();
    Auth::extend("api_key", api_key_guard).unwrap();
    Auth::extend("failing", failing_guard).unwrap();
}

/// Reports the user each guard sees for this request.
async fn whoami(_request: Request) -> Response {
    let partner = Auth::guard("partner")?.user().await?;
    let web = Auth::guard("web")?.user().await?;
    let partner = partner.map(|user| user.get_auth_identifier());
    let web = web.map(|user| user.get_auth_identifier());
    text(format!("partner={partner:?} web={web:?}"))
}

/// What a request guard's resolver concludes from the key a request
/// carried: the partner `id` for the `accepted` key, an error for the
/// failing key, and nobody otherwise.
fn partner_for(key: Option<String>, accepted: &str, id: &'static str) -> UserResult {
    match key.as_deref() {
        Some(FAILING_KEY) => Err(lookup_failed(FAILING_KEY)),
        Some(key) if key == accepted => Ok(Some(named(id))),
        _ => Ok(None),
    }
}

/// Installs the request guards: `keyed` finds partner `api-7` from
/// `X-Api-Key` and counts its resolver's calls in `calls`, and `second`
/// finds partner `api-9` from `X-Second-Key`.
fn install_request_guards(calls: &Arc<AtomicUsize>) {
    let keyed = GuardConfig::custom(AuthManager::via_request_driver("keyed"), "partners");
    let second = GuardConfig::custom(AuthManager::via_request_driver("second"), "partners");
    let config = AuthConfig::new("web")
        .guard("keyed", keyed)
        .guard("second", second);
    TestContainer::singleton(AuthManager::new(config));
    Auth::register_provider("users", Arc::new(Directory)).unwrap();
    Auth::register_provider("partners", Arc::new(Directory)).unwrap();
    let counted = calls.clone();
    Auth::via_request("keyed", move |request| {
        counted.fetch_add(1, Ordering::SeqCst);
        let key = request.header("x-api-key").map(str::to_owned);
        Box::pin(async move { partner_for(key, PARTNER_KEY, "api-7") })
    })
    .unwrap();
    Auth::via_request("second", |request| {
        let key = request.header("x-second-key").map(str::to_owned);
        Box::pin(async move { partner_for(key, SECOND_KEY, "api-9") })
    })
    .unwrap();
}

/// Reports the user each request guard sees for this request.
async fn request_guards(_request: Request) -> Response {
    let keyed = Auth::guard("keyed")?.user().await?;
    let second = Auth::guard("second")?.user().await?;
    let keyed = keyed.map(|user| user.get_auth_identifier());
    let second = second.map(|user| user.get_auth_identifier());
    text(format!("keyed={keyed:?} second={second:?}"))
}

/// The auth check of the `keyed` request guard.
fn keyed_chain() -> MiddlewareRegistry {
    MiddlewareRegistry::new().append(AuthMiddleware::new().for_guard("keyed"))
}

/// The key binding, then the auth check of `guard`.
fn partner_chain(guard: &str) -> MiddlewareRegistry {
    MiddlewareRegistry::new()
        .append(ApiKeyScope)
        .append(AuthMiddleware::new().for_guard(guard))
}

/// Serves one request to `path` through `registry`, and returns its status
/// and body.
async fn serve(
    registry: MiddlewareRegistry,
    path: &str,
    headers: &[(&str, &str)],
) -> (u16, String) {
    let router: Router = Router::new()
        .get("/protected", |_request| async { text("reached") })
        .get("/whoami", whoami)
        .get("/request-guards", request_guards)
        .into();
    let router = Arc::new(router);
    let middleware = Arc::new(registry);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = TestContainer::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let service = service_fn(move |request| {
            let router = router.clone();
            let middleware = middleware.clone();
            async move { Ok::<_, Infallible>(handle_request(router, middleware, request).await) }
        });
        Builder::new()
            .serve_connection(TokioIo::new(stream), service)
            .await
            .unwrap();
    });
    let mut outgoing = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
        .get(format!("http://{address}{path}"))
        .header("Connection", "close");
    for (name, value) in headers {
        outgoing = outgoing.header(*name, *value);
    }
    let response = outgoing.send().await.unwrap();
    let status = response.status().as_u16();
    let body = response.text().await.unwrap();
    timeout(Duration::from_secs(5), server)
        .await
        .expect("server timeout")
        .unwrap();
    (status, body)
}

#[tokio::test]
async fn custom_guard_resolves_through_auth_guard() {
    TestContainer::scope(async {
        install_manager();
        let guard = Auth::guard("partner").unwrap();
        // No key is bound outside a request: the partner guard sees a guest.
        assert!(guard.user().await.unwrap().is_none());
        let partner = API_KEY
            .scope(Some(PARTNER_KEY.to_string()), guard.user())
            .await
            .unwrap()
            .expect("the partner key resolves the partner");
        assert_eq!(partner.get_auth_identifier(), "api-7");
    })
    .await;
}

#[tokio::test]
async fn factory_receives_the_guard_name_and_its_configured_provider() {
    TestContainer::scope(async {
        install_manager();
        let partners: Arc<dyn UserProvider> = Arc::new(Directory);
        Auth::register_provider("partners", partners.clone()).unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let recorded = calls.clone();
        Auth::extend("api_key", move |name, provider| {
            let mut seen = recorded.lock().unwrap();
            seen.push((name.to_string(), provider.clone()));
            api_key_guard(name, provider)
        })
        .unwrap();

        assert!(Auth::guard("partner").is_ok());

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "partner");
        assert!(Arc::ptr_eq(&calls[0].1, &partners));
    })
    .await;
}

#[tokio::test]
async fn unregistered_driver_is_an_error_naming_guard_driver_and_remedy() {
    TestContainer::scope(async {
        install_manager();
        let err = Auth::guard("orphan").err().expect("no factory");
        let message = err.to_string();
        assert!(message.contains("'orphan'"), "got: {message}");
        assert!(message.contains("'unregistered'"), "got: {message}");
        assert!(message.contains("Auth::extend"), "got: {message}");
    })
    .await;
}

#[tokio::test]
async fn a_colon_in_a_guard_name_is_refused_and_the_name_is_not_echoed() {
    TestContainer::scope(async {
        let config = AuthConfig::new("web")
            .guard("a:b", GuardConfig::custom("api_key", "partners"))
            .guard("a", GuardConfig::custom("api_key", "partners"));
        TestContainer::singleton(AuthManager::new(config));
        Auth::register_provider("users", Arc::new(Directory)).unwrap();
        Auth::register_provider("partners", Arc::new(Directory)).unwrap();
        Auth::extend("api_key", api_key_guard).unwrap();

        let declared = Auth::guard("a:b")
            .err()
            .expect("the declaration is refused");
        let message = declared.to_string();
        assert!(message.contains("cannot contain ':'"), "got: {message}");
        assert!(!message.contains("a:b"), "got: {message}");
        assert!(Auth::guard("a").is_ok());

        let registered = Auth::via_request("a:b", |_request| Box::pin(async { Ok(None) }))
            .expect_err("the registration is refused");
        let message = registered.to_string();
        assert!(message.contains("cannot contain ':'"), "got: {message}");
        assert!(!message.contains("a:b"), "got: {message}");
    })
    .await;
}

#[tokio::test]
async fn auth_middleware_admits_the_request_the_custom_guard_accepts() {
    TestContainer::scope(async {
        install_manager();
        let headers = [("X-Api-Key", PARTNER_KEY)];
        let (status, body) = serve(partner_chain("partner"), "/protected", &headers).await;
        assert_eq!(status, 200);
        assert_eq!(body, "reached");
    })
    .await;
}

#[tokio::test]
async fn auth_middleware_answers_401_when_the_custom_guard_refuses() {
    TestContainer::scope(async {
        install_manager();
        let headers = [("X-Api-Key", "not-a-partner-key")];
        let (status, body) = serve(partner_chain("partner"), "/protected", &headers).await;
        assert_eq!(status, 401);
        assert_ne!(body, "reached");

        let (status, body) = serve(partner_chain("partner"), "/protected", &[]).await;
        assert_eq!(status, 401);
        assert_ne!(body, "reached");
    })
    .await;
}

#[tokio::test]
async fn auth_middleware_fails_the_request_when_the_custom_guard_errors() {
    let _env = crate::env_lock::lock_env_async().await;
    let _debug = EnvSnapshot::capture(&["APP_DEBUG"]);
    TestContainer::scope(async {
        install_manager();
        let headers = [("X-Api-Key", FAILING_KEY)];

        // In debug mode the guard's error text, key included, reaches the
        // body, so the production check below is able to fail.
        set_env("APP_DEBUG", Some("true"));
        let (status, body) = serve(partner_chain("partner"), "/protected", &headers).await;
        assert_eq!(status, 500);
        assert!(body.contains(FAILING_KEY), "debug body: {body}");

        // In production the body carries no error text.
        set_env("APP_DEBUG", Some("false"));
        let (status, body) = serve(partner_chain("partner"), "/protected", &headers).await;
        assert_eq!(status, 500);
        assert_ne!(body, "reached");
        assert!(!body.contains(FAILING_KEY), "production body: {body}");

        // An optional check lets a guest through, but never a failed guard.
        let registry = MiddlewareRegistry::new()
            .append(ApiKeyScope)
            .append(AuthMiddleware::optional().for_guard("partner"));
        let (status, body) = serve(registry, "/protected", &headers).await;
        assert_eq!(status, 500);
        assert_ne!(body, "reached");
    })
    .await;
}

#[tokio::test]
async fn auth_middleware_fails_the_request_when_the_guard_cannot_be_built() {
    TestContainer::scope(async {
        install_manager();
        let headers = [("X-Api-Key", PARTNER_KEY)];
        let (status, body) = serve(partner_chain("broken"), "/protected", &headers).await;
        assert_eq!(status, 500);
        assert_ne!(body, "reached");

        let (status, body) = serve(partner_chain("orphan"), "/protected", &headers).await;
        assert_eq!(status, 500);
        assert_ne!(body, "reached");
    })
    .await;
}

#[tokio::test]
async fn custom_and_session_guards_do_not_share_a_cached_user() {
    TestContainer::scope(async {
        install_manager();
        let headers = [("X-Api-Key", PARTNER_KEY)];

        // The web user alone does not satisfy the partner guard.
        let registry = MiddlewareRegistry::new()
            .append(WebUserScope)
            .append(ApiKeyScope)
            .append(AuthMiddleware::new().for_guard("partner"));
        let (status, _) = serve(registry, "/protected", &[]).await;
        assert_eq!(status, 401);

        // The partner alone is not the web user.
        let (status, body) = serve(partner_chain("partner"), "/whoami", &headers).await;
        assert_eq!(status, 200);
        assert_eq!(body, r#"partner=Some("api-7") web=None"#);

        // Both signed in: each guard still reports its own user.
        let registry = MiddlewareRegistry::new()
            .append(WebUserScope)
            .append(ApiKeyScope)
            .append(AuthMiddleware::new().for_guard("partner"));
        let (status, body) = serve(registry, "/whoami", &headers).await;
        assert_eq!(status, 200);
        assert_eq!(body, r#"partner=Some("api-7") web=Some("7")"#);
    })
    .await;
}

#[tokio::test]
async fn request_guard_admits_the_right_key_and_the_handler_sees_its_user() {
    TestContainer::scope(async {
        let calls = Arc::new(AtomicUsize::new(0));
        install_request_guards(&calls);
        let headers = [("X-Api-Key", PARTNER_KEY)];
        let (status, body) = serve(keyed_chain(), "/request-guards", &headers).await;
        assert_eq!(status, 200);
        assert_eq!(body, r#"keyed=Some("api-7") second=None"#);
    })
    .await;
}

#[tokio::test]
async fn request_guard_answers_401_without_the_right_key() {
    TestContainer::scope(async {
        let calls = Arc::new(AtomicUsize::new(0));
        install_request_guards(&calls);
        let (status, body) = serve(keyed_chain(), "/request-guards", &[]).await;
        assert_eq!(status, 401);
        assert!(!body.starts_with("keyed="));

        let headers = [("X-Api-Key", "not-a-partner-key")];
        let (status, body) = serve(keyed_chain(), "/request-guards", &headers).await;
        assert_eq!(status, 401);
        assert!(!body.starts_with("keyed="));
    })
    .await;
}

#[tokio::test]
async fn request_guard_resolver_error_fails_the_request() {
    let _env = crate::env_lock::lock_env_async().await;
    let _debug = EnvSnapshot::capture(&["APP_DEBUG"]);
    TestContainer::scope(async {
        let calls = Arc::new(AtomicUsize::new(0));
        install_request_guards(&calls);
        let headers = [("X-Api-Key", FAILING_KEY)];

        // In debug mode the resolver's error text, key included, reaches the
        // body, so the production check below is able to fail.
        set_env("APP_DEBUG", Some("true"));
        let (status, body) = serve(keyed_chain(), "/request-guards", &headers).await;
        assert_eq!(status, 500);
        assert!(body.contains(FAILING_KEY), "debug body: {body}");

        // In production the body carries no error text.
        set_env("APP_DEBUG", Some("false"));
        let (status, body) = serve(keyed_chain(), "/request-guards", &headers).await;
        assert_eq!(status, 500);
        assert!(!body.contains(FAILING_KEY), "production body: {body}");

        // An optional check lets a guest through, but never a failed resolver.
        let optional = AuthMiddleware::optional().for_guard("keyed");
        let registry = MiddlewareRegistry::new().append(optional);
        let (status, body) = serve(registry, "/request-guards", &headers).await;
        assert_eq!(status, 500);
        assert!(!body.starts_with("keyed="));
    })
    .await;
}

#[tokio::test]
async fn request_guard_resolver_runs_once_per_request() {
    TestContainer::scope(async {
        let calls = Arc::new(AtomicUsize::new(0));
        install_request_guards(&calls);

        // Two checks and the handler ask for the user the resolver found.
        let registry = MiddlewareRegistry::new()
            .append(AuthMiddleware::optional().for_guard("keyed"))
            .append(AuthMiddleware::new().for_guard("keyed"));
        let headers = [("X-Api-Key", PARTNER_KEY)];
        let (status, body) = serve(registry, "/request-guards", &headers).await;
        assert_eq!(status, 200);
        assert_eq!(body, r#"keyed=Some("api-7") second=None"#);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // A resolver that found nobody is not asked again either.
        let registry = MiddlewareRegistry::new()
            .append(AuthMiddleware::optional().for_guard("keyed"))
            .append(GuestMiddleware::new().for_guard("keyed"));
        let (status, body) = serve(registry, "/request-guards", &[]).await;
        assert_eq!(status, 200);
        assert_eq!(body, "keyed=None second=None");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    })
    .await;
}

#[tokio::test]
async fn request_guards_do_not_see_each_others_user() {
    TestContainer::scope(async {
        let calls = Arc::new(AtomicUsize::new(0));
        install_request_guards(&calls);

        // Both resolvers run; only the first finds its partner.
        let registry = MiddlewareRegistry::new()
            .append(GuestMiddleware::new().for_guard("second"))
            .append(AuthMiddleware::new().for_guard("keyed"));
        let headers = [("X-Api-Key", PARTNER_KEY)];
        let (status, body) = serve(registry, "/request-guards", &headers).await;
        assert_eq!(status, 200);
        assert_eq!(body, r#"keyed=Some("api-7") second=None"#);

        // The other way round.
        let registry = MiddlewareRegistry::new()
            .append(GuestMiddleware::new().for_guard("keyed"))
            .append(AuthMiddleware::new().for_guard("second"));
        let headers = [("X-Second-Key", SECOND_KEY)];
        let (status, body) = serve(registry, "/request-guards", &headers).await;
        assert_eq!(status, 200);
        assert_eq!(body, r#"keyed=None second=Some("api-9")"#);
    })
    .await;
}

#[tokio::test]
async fn request_guard_reports_no_user_outside_its_middleware() {
    TestContainer::scope(async {
        let calls = Arc::new(AtomicUsize::new(0));
        install_request_guards(&calls);
        let guard = Auth::guard("keyed").unwrap();
        assert!(guard.user().await.unwrap().is_none());

        // Inside a request whose chain never runs the resolver, too.
        let headers = [("X-Api-Key", PARTNER_KEY)];
        let registry = MiddlewareRegistry::new();
        let (status, body) = serve(registry, "/request-guards", &headers).await;
        assert_eq!(status, 200);
        assert_eq!(body, "keyed=None second=None");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    })
    .await;
}
