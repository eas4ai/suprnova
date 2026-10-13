//! Integration test for `ResourceRoutes::authorize_resource` - Laravel's
//! `authorizeResource` parity.
//!
//! Resource routes are ungated unless every controller body remembers to call
//! `Gate::authorize`; a single forgotten `destroy` ships an ungated delete.
//! `authorize_resource::<User, Resource>()` closes that gap by attaching the
//! conventional ability check to every generated route as per-route
//! middleware. This drives the wiring end-to-end through `handle_request` over
//! a loopback socket (the established middleware test pattern - a synthetic
//! `hyper::body::Incoming` can't be built directly), proving that:
//!
//! - a denied ability short-circuits with `403` before the handler runs,
//! - a granted ability reaches the handler,
//! - an unauthenticated request fails closed,
//! - the action→ability mapping (index/show→view, store→create,
//!   update→update + PATCH alongside PUT, destroy→delete) holds.

use std::any::Any;
use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;

use suprnova::http::text;
use suprnova::testing::TestContainer;
use suprnova::{
    Auth, AuthConfig, AuthManager, AuthMiddleware, Authenticatable, FrameworkError, Gate,
    GuardConfig, Middleware, MiddlewareRegistry, Next, Request, ResourceController, Response,
    Router, UserProvider, handle_request,
};

/// A user with explicit per-ability grants, so a single server can serve
/// "allowed" and "denied" assertions by switching which user the login
/// middleware sets.
#[derive(Clone)]
struct TestUser {
    can_view: bool,
    can_create: bool,
    can_update: bool,
    can_delete: bool,
}

impl Authenticatable for TestUser {
    fn get_auth_identifier(&self) -> String {
        "1".to_string()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

/// Resource marker type - the Gate discriminates on this type, the way
/// Laravel discriminates on the model class.
#[derive(Default)]
struct Post;

/// The user a header value names: `admin` has every ability, `viewer` may
/// only view, and anything else is nobody.
fn user_named(name: &str) -> Option<Arc<dyn Authenticatable>> {
    match name {
        "admin" => Some(Arc::new(TestUser {
            can_view: true,
            can_create: true,
            can_update: true,
            can_delete: true,
        })),
        "viewer" => Some(Arc::new(TestUser {
            can_view: true,
            can_create: false,
            can_update: false,
            can_delete: false,
        })),
        _ => None,
    }
}

/// Global middleware that authenticates the request on the default guard as
/// the user the `X-Test-User` header names, or leaves it unauthenticated. The
/// authorize middleware then resolves that user as a `TestUser`.
struct LoginAs;

#[async_trait::async_trait]
impl Middleware for LoginAs {
    async fn handle(&self, request: Request, next: Next) -> Response {
        if let Some(user) = request.header("X-Test-User").and_then(user_named) {
            Auth::set_user(user);
        }
        next(request).await
    }
}

/// A guard other than the default. Its resolver finds the user the
/// `X-Partner` header names.
const PARTNER: &str = "partner";

/// The provider the guards are declared with. The guards under test never
/// ask it for a user.
struct NoUsers;

#[async_trait::async_trait]
impl UserProvider for NoUsers {
    async fn retrieve_by_id(
        &self,
        _id: &str,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        Ok(None)
    }
}

/// Installs a container-scoped manager: the default `web` session guard,
/// which `LoginAs` signs in, and the `partner` request guard.
fn install_partner_guard() {
    let driver = AuthManager::via_request_driver(PARTNER);
    let config = AuthConfig::new("web").guard(PARTNER, GuardConfig::custom(driver, "users"));
    TestContainer::singleton(AuthManager::new(config));
    Auth::register_provider("users", Arc::new(NoUsers)).unwrap();
    Auth::via_request(PARTNER, |request| {
        let user = request.header("X-Partner").and_then(user_named);
        Box::pin(async move { Ok(user) })
    })
    .unwrap();
}

struct PostsCtl;
impl ResourceController for PostsCtl {
    fn index(&self, _req: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        Box::pin(async { text("index") })
    }
    fn show(&self, _req: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        Box::pin(async { text("show") })
    }
    fn store(&self, _req: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        Box::pin(async { text("store") })
    }
    fn update(&self, _req: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        Box::pin(async { text("update") })
    }
    fn destroy(&self, _req: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        Box::pin(async { text("destroy") })
    }
}

fn register_gates() {
    // Abilities keyed on (ability, TestUser, Post). The `(U, R)` pair is
    // unique to this test binary, so there is no cross-test bleed in the
    // process-global gate registry.
    Gate::define::<TestUser, Post>("view", |u, _p| u.can_view);
    Gate::define::<TestUser, Post>("create", |u, _p| u.can_create);
    Gate::define::<TestUser, Post>("update", |u, _p| u.can_update);
    Gate::define::<TestUser, Post>("delete", |u, _p| u.can_delete);
}

fn build_router() -> Router {
    Router::new()
        .resource("posts", PostsCtl)
        .unnamed()
        .authorize_resource::<TestUser, Post>()
        .into()
}

async fn spawn_server(accepts: usize) -> SocketAddr {
    spawn_server_with(accepts, MiddlewareRegistry::new().append(LoginAs)).await
}

/// Serves the router through `registry` for `accepts` connections. Inside a
/// `TestContainer::scope`, the server sees that scope's container.
async fn spawn_server_with(accepts: usize, registry: MiddlewareRegistry) -> SocketAddr {
    let router = Arc::new(build_router());
    let registry = Arc::new(registry);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");

    TestContainer::spawn(async move {
        for _ in 0..accepts {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let io = TokioIo::new(stream);
            let router = router.clone();
            let registry = registry.clone();
            TestContainer::spawn(async move {
                let svc = service_fn(move |req: hyper::Request<Incoming>| {
                    let router = router.clone();
                    let registry = registry.clone();
                    async move { Ok::<_, Infallible>(handle_request(router, registry, req).await) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, svc)
                    .await;
            });
        }
    });

    addr
}

async fn request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
) -> (u16, String) {
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(io)
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let mut builder = hyper::Request::builder()
        .method(method)
        .uri(path)
        .header("Host", "localhost")
        .header("Content-Length", "0");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let req = builder.body(Full::new(Bytes::new())).unwrap();

    let resp = tokio::time::timeout(Duration::from_secs(5), sender.send_request(req))
        .await
        .expect("send_request timeout")
        .expect("hyper send_request");

    let (parts, body) = resp.into_parts();
    let status = parts.status.as_u16();
    let bytes = body.collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

#[tokio::test]
async fn authorize_resource_denies_destroy_without_grant() {
    register_gates();
    // One server handles every assertion below (loopback, sequential).
    let addr = spawn_server(8).await;

    // A viewer may view (index/show) but NOT create/update/destroy.
    let (status, body) = request(addr, "GET", "/posts", &[("X-Test-User", "viewer")]).await;
    assert_eq!(status, 200, "viewer may index");
    assert_eq!(body, "index");

    let (status, _) = request(addr, "GET", "/posts/42", &[("X-Test-User", "viewer")]).await;
    assert_eq!(status, 200, "viewer may show");

    // The headline case: a forgotten controller-side check would let this
    // through. authorize_resource gates it - DELETE must be 403 for a viewer.
    let (status, body) = request(addr, "DELETE", "/posts/42", &[("X-Test-User", "viewer")]).await;
    assert_eq!(
        status, 403,
        "viewer must NOT be able to destroy: body={body}"
    );
    assert_ne!(body, "destroy", "the destroy handler must never run");

    // Store + update are likewise denied for a viewer.
    let (status, _) = request(addr, "POST", "/posts", &[("X-Test-User", "viewer")]).await;
    assert_eq!(status, 403, "viewer must NOT be able to store");

    let (status, _) = request(addr, "PUT", "/posts/42", &[("X-Test-User", "viewer")]).await;
    assert_eq!(status, 403, "viewer must NOT be able to update via PUT");

    // PATCH shares the update action and must be gated the same as PUT -
    // never an ungated bypass.
    let (status, _) = request(addr, "PATCH", "/posts/42", &[("X-Test-User", "viewer")]).await;
    assert_eq!(status, 403, "viewer must NOT be able to update via PATCH");
}

#[tokio::test]
async fn authorize_resource_allows_granted_actions() {
    register_gates();
    let addr = spawn_server(5).await;

    let (status, body) = request(addr, "POST", "/posts", &[("X-Test-User", "admin")]).await;
    assert_eq!(status, 200, "admin may store");
    assert_eq!(body, "store");

    let (status, body) = request(addr, "PUT", "/posts/7", &[("X-Test-User", "admin")]).await;
    assert_eq!(status, 200, "admin may update via PUT");
    assert_eq!(body, "update");

    let (status, body) = request(addr, "PATCH", "/posts/7", &[("X-Test-User", "admin")]).await;
    assert_eq!(status, 200, "admin may update via PATCH");
    assert_eq!(body, "update");

    let (status, body) = request(addr, "DELETE", "/posts/7", &[("X-Test-User", "admin")]).await;
    assert_eq!(status, 200, "admin may destroy");
    assert_eq!(body, "destroy");
}

#[tokio::test]
async fn authorize_resource_fails_closed_when_unauthenticated() {
    register_gates();
    let addr = spawn_server(2).await;

    // No X-Test-User header → no authenticated user → fail closed.
    let (status, body) = request(addr, "GET", "/posts", &[]).await;
    assert_ne!(
        status, 200,
        "unauthenticated request must not reach the handler"
    );
    assert_ne!(
        body, "index",
        "the index handler must never run unauthenticated"
    );

    let (status, _) = request(addr, "DELETE", "/posts/1", &[]).await;
    assert_ne!(
        status, 200,
        "unauthenticated destroy must fail closed, never 200"
    );
}

#[tokio::test]
async fn authorize_resource_checks_the_user_of_the_route_guard() {
    TestContainer::scope(async {
        register_gates();
        install_partner_guard();
        let guarded = MiddlewareRegistry::new()
            .append(LoginAs)
            .append(AuthMiddleware::new().for_guard(PARTNER));
        let addr = spawn_server_with(2, guarded).await;

        // A user of the route's guard is authorized as usual.
        let (status, body) = request(addr, "DELETE", "/posts/7", &[("X-Partner", "admin")]).await;
        assert_eq!((status, body.as_str()), (200, "destroy"));

        // The route's user decides, not the default guard's.
        let headers = [("X-Test-User", "admin"), ("X-Partner", "viewer")];
        let (status, body) = request(addr, "DELETE", "/posts/7", &headers).await;
        assert_eq!(status, 403, "body: {body}");

        // With no user on the route's guard, the default guard's user never
        // stands in: the request fails closed with the middleware's 403.
        let optional = MiddlewareRegistry::new()
            .append(LoginAs)
            .append(AuthMiddleware::optional().for_guard(PARTNER));
        let addr = spawn_server_with(1, optional).await;
        let (status, body) = request(addr, "DELETE", "/posts/7", &[("X-Test-User", "admin")]).await;
        assert_eq!(status, 403, "body: {body}");
        assert_ne!(body, "destroy", "the destroy handler must never run");
    })
    .await;
}
