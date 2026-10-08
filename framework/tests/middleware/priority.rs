//! Middleware priority: the priority list orders the chain, whatever order
//! the middleware were registered in.
//!
//! The failure that matters is an order-dependent pair run the wrong way
//! round: authentication before the session is loaded, authorization before
//! the model is bound. Each test registers the pair in the wrong order and
//! drives a real request through `handle_request`.

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
use suprnova::http::text;
use suprnova::middleware::{
    append_middleware_priority, clear_middleware_priority_for_test, into_boxed,
    prepend_middleware_priority,
};
use suprnova::{Middleware, MiddlewareRegistry, Next, Request, Response, Router, handle_request};

/// The middleware that ran for the request of the test that is running, in
/// the order they ran.
static RAN: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

macro_rules! recording_middleware {
    ($name:ident, $label:literal) => {
        struct $name;

        #[async_trait]
        impl Middleware for $name {
            async fn handle(&self, request: Request, next: Next) -> Response {
                RAN.lock().unwrap().push($label);
                next(request).await
            }
        }
    };
}

recording_middleware!(SessionMiddleware, "session");
recording_middleware!(AuthMiddleware, "auth");
recording_middleware!(BindingsMiddleware, "bindings");
recording_middleware!(AuditMiddleware, "audit");

/// An empty priority list for the length of one test. The list is
/// process-wide, so it is emptied again when the test ends, passed or not.
struct PriorityList;

impl PriorityList {
    fn empty() -> Self {
        clear_middleware_priority_for_test();
        RAN.lock().unwrap().clear();
        Self
    }
}

impl Drop for PriorityList {
    fn drop(&mut self) {
        clear_middleware_priority_for_test();
    }
}

/// Serve one request for `path` through `handle_request` and give back the
/// middleware that ran, in order.
async fn run(
    router: impl Into<Router>,
    registry: MiddlewareRegistry,
    path: &str,
) -> Vec<&'static str> {
    let addr = spawn_server(router.into(), registry).await;
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
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        status.as_u16(),
        200,
        "the request did not reach the handler: {}",
        String::from_utf8_lossy(&body)
    );
    RAN.lock().unwrap().clone()
}

async fn spawn_server(router: Router, registry: MiddlewareRegistry) -> SocketAddr {
    let router = Arc::new(router);
    let registry = Arc::new(registry);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind an ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");
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
    addr
}

#[tokio::test]
#[serial]
async fn the_list_puts_the_session_before_authentication_whatever_was_registered_first() {
    let _list = PriorityList::empty();
    append_middleware_priority::<SessionMiddleware>();
    append_middleware_priority::<AuthMiddleware>();

    let router = Router::new()
        .get("/account", |_req: Request| async { text("ok") })
        .middleware(AuthMiddleware)
        .middleware(SessionMiddleware);

    assert_eq!(
        run(router, MiddlewareRegistry::new(), "/account").await,
        ["session", "auth"]
    );
}

#[tokio::test]
#[serial]
async fn an_empty_list_leaves_the_order_of_registration() {
    let _list = PriorityList::empty();

    let router = Router::new()
        .get("/account", |_req: Request| async { text("ok") })
        .middleware(AuthMiddleware)
        .middleware(SessionMiddleware);

    assert_eq!(
        run(router, MiddlewareRegistry::new(), "/account").await,
        ["auth", "session"]
    );
}

#[tokio::test]
#[serial]
async fn a_middleware_the_list_does_not_name_keeps_its_place_behind_the_one_it_followed() {
    let _list = PriorityList::empty();
    append_middleware_priority::<SessionMiddleware>();
    append_middleware_priority::<AuthMiddleware>();

    // `audit` was registered after `auth`, and may read what `auth` found.
    // Moving `session` forward must not put `audit` in front of `auth`.
    let router = Router::new()
        .get("/account", |_req: Request| async { text("ok") })
        .middleware(AuthMiddleware)
        .middleware(AuditMiddleware)
        .middleware(SessionMiddleware);

    assert_eq!(
        run(router, MiddlewareRegistry::new(), "/account").await,
        ["session", "auth", "audit"]
    );
}

#[tokio::test]
#[serial]
async fn the_list_orders_global_and_route_middleware_together() {
    let _list = PriorityList::empty();
    append_middleware_priority::<SessionMiddleware>();
    append_middleware_priority::<AuthMiddleware>();
    append_middleware_priority::<BindingsMiddleware>();

    // Authentication is global, and the session and the bindings are on the
    // route, registered the wrong way round.
    let registry = MiddlewareRegistry::new().append(AuthMiddleware);
    let router = Router::new()
        .get("/account", |_req: Request| async { text("ok") })
        .middleware(BindingsMiddleware)
        .middleware(SessionMiddleware);

    assert_eq!(
        run(router, registry, "/account").await,
        ["session", "auth", "bindings"]
    );
}

#[tokio::test]
#[serial]
async fn prepending_to_the_list_outranks_what_the_list_held() {
    let _list = PriorityList::empty();
    append_middleware_priority::<AuthMiddleware>();
    prepend_middleware_priority::<SessionMiddleware>();

    let router = Router::new()
        .get("/account", |_req: Request| async { text("ok") })
        .middleware(AuthMiddleware)
        .middleware(SessionMiddleware);

    assert_eq!(
        run(router, MiddlewareRegistry::new(), "/account").await,
        ["session", "auth"]
    );
}

#[tokio::test]
#[serial]
async fn a_middleware_boxed_by_hand_has_no_type_and_keeps_its_place() {
    let _list = PriorityList::empty();
    append_middleware_priority::<SessionMiddleware>();
    append_middleware_priority::<AuthMiddleware>();

    // `into_boxed` does not register the type. The list cannot name what
    // it cannot see, so the box stays where it was put.
    let router = Router::new()
        .get("/account", |_req: Request| async { text("ok") })
        .middleware_boxed(into_boxed(AuthMiddleware))
        .middleware(SessionMiddleware);

    assert_eq!(
        run(router, MiddlewareRegistry::new(), "/account").await,
        ["auth", "session"]
    );
}
