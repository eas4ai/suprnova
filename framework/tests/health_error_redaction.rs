//! CI-05 — `/_suprnova/health?db=true` must not hand a raw driver error
//! to an unauthenticated caller.
//!
//! The health endpoint is unauthenticated by design: it exists for
//! k8s-style `livenessProbe` / `readinessProbe` configurations, so it
//! cannot sit behind auth and it short-circuits before the middleware
//! chain. That makes it the one 5xx path in the framework that anybody can
//! reach, and until this landed it embedded the database driver's error
//! text verbatim in the response body.
//!
//! Driver errors are not neutral strings. They name hosts, ports, database
//! and schema names, and server versions; sqlx's configuration errors can
//! carry the connection URL. Handing those to whoever asks, precisely when
//! the system is already degraded, is the definition of an information
//! leak — and the endpoint's 503 is exactly the moment an attacker is most
//! interested.
//!
//! `http/response.rs` and `resources/errors.rs` already gate their 5xx
//! detail on `status >= 500 && Config::is_debug()`. The health endpoint
//! predated that convention and never adopted it. These tests pin both
//! halves of the fix: the detail is present in debug, absent otherwise,
//! and the machine-readable shape a dashboard parses is unchanged either
//! way.

use serde_json::Value;
use serial_test::serial;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;

use suprnova::config::{AppConfig, Config, Environment};
use suprnova::{MiddlewareRegistry, Router, handle_request};

/// Route through the real `handle_request`, so this exercises the actual
/// short-circuit branch rather than a re-implementation of it.
async fn spawn_server(accepts: usize) -> SocketAddr {
    let router = Arc::new(Router::new());
    let middleware = Arc::new(MiddlewareRegistry::new());

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

async fn get(addr: SocketAddr, path: &str) -> (u16, Value) {
    let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
        .await
        .expect("handshake");
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let req = hyper::Request::builder()
        .method("GET")
        .uri(path)
        .header("Host", "localhost")
        .body(http_body_util::Empty::<bytes::Bytes>::new())
        .expect("request");

    let res = sender.send_request(req).await.expect("send");
    let status = res.status().as_u16();
    let bytes = res.into_body().collect().await.expect("body").to_bytes();
    let json: Value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("health body must be JSON: {e}; body={bytes:?}"));
    (status, json)
}

fn install_app_config(env: Environment, debug: bool) {
    Config::register(
        AppConfig::builder()
            .name("health-error-redaction-test")
            .environment(env)
            .debug(debug)
            .url("http://localhost:0")
            .build(),
    );
}

/// The headline case. Production-shaped config, database not initialized,
/// so the probe fails — and the caller learns only that it failed.
#[tokio::test]
#[serial]
async fn a_degraded_health_probe_hides_the_driver_error_outside_debug() {
    install_app_config(Environment::Production, false);
    let addr = spawn_server(1).await;

    let (status, body) = get(addr, "/_suprnova/health?db=true").await;

    assert_eq!(
        status, 503,
        "a failing database probe must still report 503 so k8s restarts the pod"
    );
    assert_eq!(
        body["status"], "degraded",
        "the machine-readable status must be unchanged by redaction — \
         dashboards parse this field"
    );
    assert_eq!(
        body["database"], "error",
        "and so must the coarse `database` field"
    );
    assert!(
        body.get("database_error").is_none(),
        "the raw driver error must NOT reach an unauthenticated caller in \
         production; it names hosts, ports, schemas and versions, and sqlx \
         configuration errors can carry the connection URL. Got: {body}"
    );
}

/// The other half: an operator running locally still gets the detail, or
/// the redaction would just be a debugging regression.
#[tokio::test]
#[serial]
async fn a_degraded_health_probe_keeps_the_driver_error_in_debug() {
    install_app_config(Environment::Local, true);
    let addr = spawn_server(1).await;

    let (status, body) = get(addr, "/_suprnova/health?db=true").await;

    assert_eq!(status, 503);
    assert_eq!(body["status"], "degraded");
    assert!(
        body.get("database_error").is_some(),
        "debug builds must keep the detail — redacting it everywhere would \
         trade a leak for an undiagnosable outage. Got: {body}"
    );
}

/// The default probe touches no database and must stay a plain 200, in
/// both configurations. Guards against the redaction accidentally
/// widening into "always degraded".
#[tokio::test]
#[serial]
async fn the_default_health_probe_is_unaffected() {
    for (env, debug) in [(Environment::Production, false), (Environment::Local, true)] {
        install_app_config(env, debug);
        let addr = spawn_server(1).await;

        let (status, body) = get(addr, "/_suprnova/health").await;

        assert_eq!(status, 200, "a probe without `db=true` must be a plain 200");
        assert_eq!(body["status"], "ok");
        assert!(
            body.get("database").is_none() && body.get("database_error").is_none(),
            "a probe that did not ask about the database must not report on it: {body}"
        );
    }
}
