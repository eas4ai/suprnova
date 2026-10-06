//! Route binding (docs/spec/route-binding.md, BIND-001 to BIND-015).
//!
//! Every test is named after the requirement it proves, lowercase id first.
//! Requests go through `handle_request` on a loopback socket, so a route
//! parameter really comes from a matched path and the router's startup
//! checks run the way they run for a server.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;

use suprnova::testing::TestContainer;
use suprnova::{MiddlewareRegistry, Router, handle_request};

pub mod contract;
pub mod custom;
pub mod enums;
pub mod handler_form;
pub mod keys;
pub mod manual;
pub mod missing;
pub mod regressions;
pub mod resources;
pub mod scoped;
pub mod startup;
pub mod trashed;
pub mod urls;

/// Serve `router` through `handle_request` on a loopback socket. The
/// connections run inside the test's container scope, so a database the
/// test installed is the one the handlers read.
pub async fn serve(router: Router) -> SocketAddr {
    serve_with(router, MiddlewareRegistry::new()).await
}

/// [`serve`] with global middleware.
pub async fn serve_with(router: Router, registry: MiddlewareRegistry) -> SocketAddr {
    let router = Arc::new(router);
    let registry = Arc::new(registry);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a loopback listener");
    let addr = listener.local_addr().expect("the listener's address");
    TestContainer::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let router = router.clone();
            let registry = registry.clone();
            TestContainer::spawn(async move {
                let svc = service_fn(move |req: hyper::Request<Incoming>| {
                    let router = router.clone();
                    let registry = registry.clone();
                    async move { Ok::<_, Infallible>(handle_request(router, registry, req).await) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), svc)
                    .await;
            });
        }
    });
    addr
}

/// Send one request with optional headers and a JSON body, and return the
/// status and the body.
pub async fn send(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    json: Option<&str>,
) -> (u16, String) {
    let stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connect to the test server");
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(stream))
            .await
            .expect("handshake");
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let body = json.unwrap_or("");
    let mut builder = hyper::Request::builder()
        .method(method)
        .uri(path)
        .header("Host", "localhost")
        .header("Content-Length", body.len().to_string());
    if json.is_some() {
        builder = builder.header("Content-Type", "application/json");
    }
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let request = builder
        .body(Full::new(Bytes::from(body.to_owned())))
        .expect("a request");
    let response = tokio::time::timeout(Duration::from_secs(10), sender.send_request(request))
        .await
        .expect("the request timed out")
        .expect("send the request");
    let status = response.status().as_u16();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("read the body")
        .to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// `GET path`, through the shared raw HTTP client.
pub async fn get(addr: SocketAddr, path: &str) -> (u16, String) {
    let (status, _headers, body) = crate::http_wire::request(addr, "GET", path, &[]).await;
    (status, body)
}

/// The `message` of a JSON error body.
pub fn message(body: &str) -> String {
    let value: serde_json::Value =
        serde_json::from_str(body).unwrap_or_else(|_| panic!("a JSON error body: {body}"));
    value["message"]
        .as_str()
        .unwrap_or_else(|| panic!("a message in {body}"))
        .to_owned()
}

/// The startup refusal of `router`'s binding checks, as text.
pub fn refusal(router: &Router) -> String {
    router
        .prepare_bindings()
        .expect_err("the startup checks must refuse this router")
        .to_string()
}

/// Run `sql` statements against the test database.
pub async fn run_sql(db: &suprnova::testing::TestDatabase, statements: &[&str]) {
    for statement in statements {
        db.execute_unprepared(statement)
            .await
            .unwrap_or_else(|e| panic!("{statement}: {e}"));
    }
}
