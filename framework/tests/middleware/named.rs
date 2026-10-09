//! Middleware applied by name: `.middleware_named("auth")`, an alias with
//! arguments, and a group, on a route, on a group of routes and on the
//! macro builders. The name is resolved when the route is registered, so
//! an unknown name stops the boot and never a request.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serial_test::serial;
use suprnova::cache::{CacheStore, InMemoryCache};
use suprnova::container::testing::TestContainer;
use suprnova::http::text;
use suprnova::middleware::{
    clear_all_middleware_aliases_for_test, clear_all_middleware_groups_for_test,
    register_middleware_alias, register_middleware_alias_with_args, register_middleware_group,
    resolve_middleware_alias, try_resolve_middleware_alias,
};
use suprnova::rate_limit::ThrottleRequestsMiddleware;
use suprnova::{
    FrameworkError, Middleware, MiddlewareRegistry, Next, Request, Response, Router, handle_request,
};
use suprnova::{get, group};

/// What ran for the request of the test that is running, in order.
static RAN: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Records the label it was built with.
struct Records(String);

#[async_trait]
impl Middleware for Records {
    async fn handle(&self, request: Request, next: Next) -> Response {
        RAN.lock().unwrap().push(self.0.clone());
        next(request).await
    }
}

/// Aliases and groups are process-wide. Each test starts from none and
/// leaves none behind, passed or not.
struct Names;

impl Names {
    fn none() -> Self {
        clear_all_middleware_aliases_for_test();
        clear_all_middleware_groups_for_test();
        RAN.lock().unwrap().clear();
        Self
    }
}

impl Drop for Names {
    fn drop(&mut self) {
        clear_all_middleware_aliases_for_test();
        clear_all_middleware_groups_for_test();
    }
}

fn register_the_usual_names() {
    register_middleware_alias("auth", || Records("auth".into()));
    register_middleware_alias("verified", || Records("verified".into()));
    register_middleware_alias_with_args("role", |arguments| match arguments {
        [] => Err(FrameworkError::internal("role needs the role to require")),
        roles => Ok(Records(format!("role:{}", roles.join("+")))),
    });
    register_middleware_group("members", ["auth".to_string(), "verified".to_string()]);
}

async fn run(router: impl Into<Router>, path: &str) -> Vec<String> {
    let (status, ran) = run_status(router, path).await;
    assert_eq!(status, 200);
    ran
}

/// [`run`], returning the status instead of requiring 200.
async fn run_status(router: impl Into<Router>, path: &str) -> (u16, Vec<String>) {
    let router = Arc::new(router.into());
    let registry = Arc::new(MiddlewareRegistry::new());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind an ephemeral listener");
    let addr: SocketAddr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        let service = service_fn(move |request: hyper::Request<Incoming>| {
            let router = router.clone();
            let registry = registry.clone();
            async move { Ok::<_, Infallible>(handle_request(router, registry, request).await) }
        });
        let _ = hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(stream), service)
            .await;
    });
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(stream))
            .await
            .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let request = hyper::Request::builder()
        .method("GET")
        .uri(path)
        .header("Host", "localhost")
        .header("Content-Length", "0")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(5), sender.send_request(request))
        .await
        .expect("the request timed out")
        .expect("the request failed");
    let status = response.status().as_u16();
    let _ = response.into_body().collect().await.unwrap();
    (status, RAN.lock().unwrap().clone())
}

#[tokio::test]
#[serial]
async fn a_route_names_an_alias_an_alias_with_arguments_and_a_group() {
    let _names = Names::none();
    register_the_usual_names();

    let router = Router::new()
        .get("/reports", |_req: Request| async { text("ok") })
        .middleware_named("members")
        .middleware_named("role:admin,auditor");

    assert_eq!(
        run(router, "/reports").await,
        ["auth", "verified", "role:admin+auditor"]
    );
}

#[tokio::test]
#[serial]
async fn a_group_of_routes_names_its_middleware() {
    let _names = Names::none();
    register_the_usual_names();

    let router = Router::new()
        .group("/admin", |routes| {
            routes.get("/users", |_req: Request| async { text("ok") })
        })
        .middleware_named("auth")
        .middleware_named("role: admin ");

    assert_eq!(run(router, "/admin/users").await, ["auth", "role:admin"]);
}

#[tokio::test]
#[serial]
async fn a_name_that_is_not_registered_fails_when_the_route_is_registered() {
    let _names = Names::none();
    register_the_usual_names();

    let error = Router::new()
        .get("/reports", |_req: Request| async { text("ok") })
        .try_middleware_named("atuh")
        .err()
        .expect("a typo must not become a route without its middleware");

    assert!(
        error
            .to_string()
            .contains("middleware alias `atuh` is not registered"),
        "{error}"
    );
}

#[tokio::test]
#[serial]
#[should_panic(expected = "middleware alias `atuh` is not registered")]
async fn the_infallible_spelling_stops_the_boot() {
    let _names = Names::none();
    let _router = Router::new()
        .get("/reports", |_req: Request| async { text("ok") })
        .middleware_named("atuh");
}

#[test]
#[serial]
fn an_alias_says_why_it_has_no_middleware_to_give() {
    let _names = Names::none();
    register_the_usual_names();

    let given_arguments = try_resolve_middleware_alias("auth:admin")
        .err()
        .expect("`auth` reads no arguments");
    assert!(
        given_arguments
            .to_string()
            .contains("`auth` takes no arguments"),
        "{given_arguments}"
    );

    let refused = try_resolve_middleware_alias("role")
        .err()
        .expect("`role` needs one");
    assert!(
        refused
            .to_string()
            .contains("role needs the role to require"),
        "the factory's own reason must reach the caller: {refused}"
    );

    assert!(resolve_middleware_alias("auth").is_some());
    assert!(resolve_middleware_alias("role:admin").is_some());
    assert!(resolve_middleware_alias("auth:admin").is_none());
    assert!(
        resolve_middleware_alias("role:").is_none(),
        "a colon with nothing after it gives no arguments"
    );
}

#[test]
#[serial]
fn a_group_may_list_an_alias_with_arguments() {
    let _names = Names::none();
    register_the_usual_names();
    register_middleware_group("admins", ["members".to_string(), "role:admin".to_string()]);

    let resolved = suprnova::middleware::resolve_named_middleware("admins").unwrap();

    assert_eq!(resolved.len(), 3, "auth, verified and role:admin");
}

/// A group that two sibling groups both include is a diamond. Its
/// middleware must appear once in the expanded chain, keeping the first
/// occurrence, as Laravel's `uniqueMiddleware` does: a throttle in the
/// shared group would otherwise count each request twice, and a limit of
/// one would refuse the very first request.
#[tokio::test]
#[serial]
async fn a_throttle_in_a_shared_group_counts_one_hit_per_request() {
    let _names = Names::none();
    let _cache = TestContainer::fake();
    TestContainer::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));
    register_the_usual_names();
    register_middleware_alias_with_args("throttle", ThrottleRequestsMiddleware::from_alias_args);
    register_middleware_group("base", ["throttle:1,1".to_string()]);
    register_middleware_group("read", ["base".to_string(), "auth".to_string()]);
    register_middleware_group(
        "write",
        [
            "base".to_string(),
            "verified".to_string(),
            "auth".to_string(),
        ],
    );
    register_middleware_group("api", ["read".to_string(), "write".to_string()]);

    let router = || {
        Router::new()
            .get("/reports", |_req: Request| async { text("ok") })
            .middleware_named("api")
    };

    let (status, ran) = run_status(router(), "/reports").await;
    assert_eq!(status, 200, "a limit of one admits the first request");
    assert_eq!(
        ran,
        ["auth", "verified"],
        "each middleware runs once, in the order it was first named"
    );

    RAN.lock().unwrap().clear();
    let (status, _) = run_status(router(), "/reports").await;
    assert_eq!(
        status, 429,
        "the one throttle still counts the request it saw"
    );
}

/// A route keeps each named middleware once, at its first occurrence,
/// across the groups and the aliases it names, as Laravel's
/// `uniqueMiddleware` does across group and route middleware. An alias
/// is identified with its arguments as they are parsed, so `role: admin`
/// is `role:admin` and `role:auditor` is another middleware.
#[tokio::test]
#[serial]
async fn a_route_runs_each_named_middleware_once() {
    let _names = Names::none();
    register_the_usual_names();

    let router = Router::new()
        .get("/reports", |_req: Request| async { text("ok") })
        .middleware_named("members")
        .middleware_named("auth")
        .middleware_named("verified")
        .middleware_named("role:admin")
        .middleware_named("role: admin")
        .middleware_named("role:auditor");

    assert_eq!(
        run(router, "/reports").await,
        ["auth", "verified", "role:admin", "role:auditor"]
    );
}

/// The same holds for a group of routes, and for a route of the
/// `routes!` macros that names an alias its group already brings.
#[tokio::test]
#[serial]
async fn a_group_of_routes_runs_each_named_middleware_once() {
    let _names = Names::none();
    register_the_usual_names();

    let router = Router::new()
        .group("/admin", |routes| {
            routes.get("/users", |_req: Request| async { text("ok") })
        })
        .middleware_named("members")
        .middleware_named("auth");
    assert_eq!(run(router, "/admin/users").await, ["auth", "verified"]);

    RAN.lock().unwrap().clear();
    let router = group!("/staff", {
        get!("/users", |_req: Request| async { text("ok") }).middleware_named("verified"),
    })
    .middleware_named("members")
    .register(Router::new());
    assert_eq!(run(router, "/staff/users").await, ["auth", "verified"]);
}

/// A throttle a route names both itself and through a group counts one
/// hit per request: a limit of one admits the first request.
#[tokio::test]
#[serial]
async fn a_throttle_a_route_names_twice_counts_one_hit_per_request() {
    let _names = Names::none();
    let _cache = TestContainer::fake();
    TestContainer::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));
    register_middleware_alias_with_args("throttle", ThrottleRequestsMiddleware::from_alias_args);
    register_middleware_group("limited", ["throttle:1,1".to_string()]);

    let router = || {
        Router::new()
            .get("/reports", |_req: Request| async { text("ok") })
            .middleware_named("limited")
            .middleware_named("throttle:1, 1")
    };

    let (status, _) = run_status(router(), "/reports").await;
    assert_eq!(status, 200, "a limit of one admits the first request");
    let (status, _) = run_status(router(), "/reports").await;
    assert_eq!(
        status, 429,
        "the one throttle still counts the request it saw"
    );
}
