#![cfg(feature = "testing")]

//! Bearer-token auth must work with NO `SessionMiddleware` installed.
//!
//! This is the ordinary shape of a token-only API - it is exactly what
//! `suprnova new x --api` generates, and it never registers
//! `SessionMiddleware`. `BearerTokenMiddleware` used to publish the
//! authenticated id only through `set_auth_user`, which routes through
//! `session_mut` and is a silent no-op when `SESSION_CONTEXT` is not
//! scoped. The id was dropped, `Auth::check()` was always false, and
//! every token-guarded route returned 401 regardless of token validity.
//!
//! Do NOT add `session_scope_for_test` to this file. Installing a
//! session scope makes these tests pass for the wrong reason and
//! restores the blind spot they exist to close.
//!
//! The harness drives a real HTTP request through the request-state scope but
//! deliberately does not install `SessionMiddleware`.

use crate::http_wire::request;
use crate::magnetar_auth;

use std::any::Any;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use std::sync::LazyLock;
use tokio::runtime::Runtime;

use suprnova::http::text;
use suprnova::magnetar_integration::middleware::BearerTokenMiddleware;
use suprnova::testing::TestContainer;
use suprnova::{
    Auth, AuthConfig, AuthManager, AuthMiddleware, Authenticatable, BasicAuthMiddleware,
    FrameworkError, GuardConfig, MiddlewareRegistry, Router, UserProvider, handle_request,
};

/// One tokio runtime shared across every test in this file.
static RT: LazyLock<Runtime> = LazyLock::new(|| Runtime::new().expect("tokio runtime"));

/// One-time Magnetar authentication setup.
static SETUP: LazyLock<()> = LazyLock::new(|| {
    RT.block_on(magnetar_auth::install());
});

/// A minimal router with one guarded route. The handler echoes `Auth::id()`
/// as the response body so tests can assert on the resolved identity, not
/// just the status code.
fn router() -> Router {
    Router::new()
        .get("/protected", |_req| async {
            text(Auth::id().unwrap_or_default())
        })
        .into()
}

/// Spawn a test server with `registry` as the global middleware set,
/// accepting `accepts` connections. Copied from `auth_http_middleware.rs`.
async fn spawn_server(
    router: impl Into<Router>,
    registry: MiddlewareRegistry,
    accepts: usize,
) -> SocketAddr {
    let router = std::sync::Arc::new(router.into());
    let middleware = std::sync::Arc::new(registry);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");

    tokio::spawn(async move {
        for _ in 0..accepts {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let io = TokioIo::new(stream);
            let router = router.clone();
            let middleware = middleware.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req: hyper::Request<Incoming>| {
                    let router = router.clone();
                    let middleware = middleware.clone();
                    async move { Ok::<_, Infallible>(handle_request(router, middleware, req).await) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, svc)
                    .await;
            });
        }
    });

    addr
}

/// The stack under test: `BearerTokenMiddleware` then the sync,
/// session-backed `AuthMiddleware::new()` gate - with no `SessionMiddleware`
/// anywhere in the chain. This is exactly the shape of the `--api` scaffold.
fn token_only_registry() -> MiddlewareRegistry {
    MiddlewareRegistry::new()
        .append(BearerTokenMiddleware)
        .append(AuthMiddleware::new())
}

/// Assertion 1: a request carrying a VALID bearer token, through
/// `BearerTokenMiddleware -> AuthMiddleware::new() -> handler`, with no
/// `SessionMiddleware` installed, must reach the handler - and `Auth::id()`
/// inside the handler must equal the token's user id.
///
/// This is the assertion that fails today: `BearerTokenMiddleware` only
/// published the id through `set_auth_user`, which is a silent no-op
/// without a `SessionMiddleware`-installed session scope, so
/// `AuthMiddleware::new()` always saw a guest and returned 401.
#[test]
fn valid_bearer_token_reaches_handler_without_session_middleware() {
    LazyLock::force(&SETUP);

    RT.block_on(async {
        Auth::password()
            .register("bearer-no-session@example.com", "Bearer1!")
            .await
            .unwrap()
            .created()
            .expect("registration creates a new account");

        let (_user, magnetar_session) = Auth::password()
            .authenticate("bearer-no-session@example.com", "Bearer1!", None, None)
            .await
            .unwrap();

        // Freshly authenticated sessions always carry the plaintext token -
        // `None` is reserved for sessions loaded from storage (hash only).
        let token_str = magnetar_session
            .token
            .as_ref()
            .expect("freshly authenticated session must carry plaintext token")
            .expose_secret()
            .to_string();
        let expected_user_id = magnetar_session.user_id.as_str().to_string();

        let addr = spawn_server(router(), token_only_registry(), 1).await;

        let (status, _headers, body) = request(
            addr,
            "GET",
            "/protected",
            &[("Authorization", &format!("Bearer {token_str}"))],
        )
        .await;

        assert_ne!(
            status, 401,
            "a valid bearer token must not be rejected by AuthMiddleware::new() \
             even with no SessionMiddleware installed"
        );
        assert_eq!(
            body, expected_user_id,
            "Auth::id() inside the handler must equal the token's user id"
        );
    });
}

#[test]
fn valid_bearer_does_not_satisfy_stateful_basic() {
    LazyLock::force(&SETUP);

    RT.block_on(async {
        Auth::password()
            .register("bearer-before-basic@example.com", "BearerBasic1!")
            .await
            .unwrap()
            .created()
            .expect("registration creates a new account");

        let (_user, magnetar_session) = Auth::password()
            .authenticate(
                "bearer-before-basic@example.com",
                "BearerBasic1!",
                None,
                None,
            )
            .await
            .unwrap();

        let token_str = magnetar_session
            .token
            .as_ref()
            .expect("freshly authenticated session must carry plaintext token")
            .expose_secret()
            .to_string();

        let protected_body = "stateful-basic-protected-handler";
        let router: Router = Router::new()
            .get(
                "/protected",
                move |_req| async move { text(protected_body) },
            )
            .into();
        let registry = MiddlewareRegistry::new()
            .append(BearerTokenMiddleware)
            .append(BasicAuthMiddleware::new().realm("Stateful Basic Test"));
        let addr = spawn_server(router, registry, 1).await;

        let (status, headers, body) = request(
            addr,
            "GET",
            "/protected",
            &[("Authorization", &format!("Bearer {token_str}"))],
        )
        .await;

        assert_eq!(status, 401);
        assert_eq!(
            headers.get("www-authenticate").map(String::as_str),
            Some("Basic realm=\"Stateful Basic Test\"")
        );
        assert_ne!(body, protected_body);
    });
}

/// Assertion 2: the same stack with NO `Authorization` header returns 401.
/// Passes before and after the fix - proves the fix did not simply disable
/// the gate.
#[test]
fn missing_authorization_header_returns_401_without_session_middleware() {
    LazyLock::force(&SETUP);

    RT.block_on(async {
        let addr = spawn_server(router(), token_only_registry(), 1).await;

        let (status, _headers, _body) = request(addr, "GET", "/protected", &[]).await;

        assert_eq!(status, 401);
    });
}

/// A user known only by its identifier.
struct Known(String);

impl Authenticatable for Known {
    fn get_auth_identifier(&self) -> String {
        self.0.clone()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

/// The provider of the `api` token guard: it knows every user the Magnetar
/// engine authenticates.
struct EveryUser;

#[async_trait]
impl UserProvider for EveryUser {
    async fn retrieve_by_id(
        &self,
        id: &str,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        Ok(Some(Arc::new(Known(id.to_owned()))))
    }
}

/// The provider of the `admin_api` token guard: it knows nobody.
struct NoAdmins;

#[async_trait]
impl UserProvider for NoAdmins {
    async fn retrieve_by_id(
        &self,
        _id: &str,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        Ok(None)
    }
}

/// Installs a container-scoped manager with two token guards over two
/// providers: `api` over `users`, and `admin_api` over `admins`.
fn install_two_token_guards() {
    let config = AuthConfig::new("web")
        .guard("api", GuardConfig::token("users"))
        .guard("admin_api", GuardConfig::token("admins"));
    TestContainer::singleton(AuthManager::new(config));
    Auth::register_provider("users", Arc::new(EveryUser)).unwrap();
    Auth::register_provider("admins", Arc::new(NoAdmins)).unwrap();
}

/// Serves one request to `/protected` through `registry` and returns its
/// status. The server task inherits the caller's container scope, so the
/// middleware resolves the guards the test installed.
async fn serve_scoped(registry: MiddlewareRegistry, headers: &[(&str, &str)]) -> u16 {
    let router: Router = Router::new()
        .get("/protected", |_req| async { text("reached") })
        .into();
    let router = Arc::new(router);
    let middleware = Arc::new(registry);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");
    let server = TestContainer::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        let svc = service_fn(move |req: hyper::Request<Incoming>| {
            let router = router.clone();
            let middleware = middleware.clone();
            async move { Ok::<_, Infallible>(handle_request(router, middleware, req).await) }
        });
        let _ = hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(stream), svc)
            .await;
    });
    let (status, _headers, _body) = request(addr, "GET", "/protected", headers).await;
    server.await.expect("server task");
    status
}

/// IDENTITY-001: every token guard reads its user through its own provider.
///
/// The `api` guard resolves the bearer user first, as a global optional
/// check would. The route then checks `admin_api`, whose provider knows
/// nobody. A request-wide bearer cache shared by every token guard answered
/// `admin_api` with the `api` guard's user, so the route let the token in as
/// an admin.
#[test]
fn a_second_token_guard_never_answers_with_the_first_guards_user() {
    LazyLock::force(&SETUP);

    RT.block_on(TestContainer::scope(async {
        install_two_token_guards();
        let registration = Auth::password()
            .register("two-token-guards@example.com", "TwoGuards1!")
            .await
            .unwrap();
        assert!(
            matches!(registration, suprnova::Registration::Created(_)),
            "a fresh address registers a new account"
        );
        let (_user, magnetar_session) = Auth::password()
            .authenticate("two-token-guards@example.com", "TwoGuards1!", None, None)
            .await
            .unwrap();
        let token = magnetar_session
            .token
            .as_ref()
            .expect("freshly authenticated session must carry plaintext token")
            .expose_secret()
            .to_string();
        let bearer = format!("Bearer {token}");
        let headers = [("Authorization", bearer.as_str())];

        // Control: the `api` guard's provider knows the token's user.
        let api_only = MiddlewareRegistry::new()
            .append(BearerTokenMiddleware)
            .append(AuthMiddleware::new().for_guard("api"));
        assert_eq!(serve_scoped(api_only, &headers).await, 200);

        let api_then_admin = MiddlewareRegistry::new()
            .append(BearerTokenMiddleware)
            .append(AuthMiddleware::optional().for_guard("api"))
            .append(AuthMiddleware::new().for_guard("admin_api"));
        assert_eq!(
            serve_scoped(api_then_admin, &headers).await,
            401,
            "the admin_api guard's provider knows no such user, so the route must refuse \
             the token even after the api guard resolved it"
        );

        let admin_only = MiddlewareRegistry::new()
            .append(BearerTokenMiddleware)
            .append(AuthMiddleware::new().for_guard("admin_api"));
        assert_eq!(serve_scoped(admin_only, &headers).await, 401);
    }));
}

/// Assertion 3: the same stack with a syntactically valid but unknown
/// bearer token returns 401. Passes before and after the fix - proves the
/// fix did not simply disable the gate.
#[test]
fn unknown_bearer_token_returns_401_without_session_middleware() {
    LazyLock::force(&SETUP);

    RT.block_on(async {
        let addr = spawn_server(router(), token_only_registry(), 1).await;

        let (status, _headers, _body) = request(
            addr,
            "GET",
            "/protected",
            &[(
                "Authorization",
                "Bearer syntactically_valid_but_unknown_xyz",
            )],
        )
        .await;

        assert_eq!(status, 401);
    });
}
