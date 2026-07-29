//! P2-01 — the split between liveness and readiness, and the optional
//! secret that closes readiness off.
//!
//! Two separate problems shared one endpoint. `/_suprnova/health?db=true`
//! ran a database round trip for any anonymous caller, and it decided to
//! do so by testing `query.contains("db=true")` — a substring match over
//! the whole query string, so `?nodb=true` ran the probe too. (The parsing
//! half is pinned by unit tests in `server.rs`; this file covers the
//! behaviour end to end, through a real socket.)
//!
//! The constraint that shapes the fix: `/_suprnova/health` and its
//! `?db=true` form are named in six `manual/` chapters, in the generated
//! Docker `HEALTHCHECK`, in the Railway `healthcheckPath`, and in the
//! DigitalOcean app spec. That path's behaviour is a published contract.
//! So readiness stays public by default and the original path keeps
//! answering exactly as documented; operators who want readiness closed
//! set `SERVER_HEALTH_READINESS_TOKEN` and get a 404 for everyone else.
//!
//! 404 rather than 401 is the point of the exercise — a 401 advertises
//! that something is there. And the 404 is not hand-built: a rejected
//! readiness probe falls through to normal routing and gets the router's
//! own not-found response, so it cannot drift into being distinguishable.
//! `a_gated_readiness_probe_is_indistinguishable_from_an_unrouted_path`
//! asserts that byte for byte.

use serde_json::Value;
use serial_test::serial;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;

use suprnova::config::{AppConfig, Config, Environment, ServerConfig};
use suprnova::{MiddlewareRegistry, Router, handle_request};

const TOKEN_HEADER: &str = "X-Suprnova-Health-Token";
const TOKEN: &str = "correct-horse-battery-staple";

/// Route through the real `handle_request` so these exercise the actual
/// short-circuit branch — and, for the gated cases, the actual fall-through
/// into routing — rather than a re-implementation of either.
async fn spawn_server() -> SocketAddr {
    let router = Arc::new(Router::new());
    let middleware = Arc::new(MiddlewareRegistry::new());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");

    tokio::spawn(async move {
        loop {
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

struct Reply {
    status: u16,
    content_type: String,
    body: String,
}

impl Reply {
    /// The JSON body, for the cases that return one. Panics with the raw
    /// body when it is not JSON, which is itself a useful failure: a 404
    /// arriving where a health response was expected shows up as its own
    /// message rather than a parse error.
    fn json(&self) -> Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|e| panic!("expected a JSON body, got {:?} ({e})", self.body))
    }
}

async fn get(addr: SocketAddr, path: &str, headers: &[(&str, &str)]) -> Reply {
    let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
        .await
        .expect("handshake");
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let mut builder = hyper::Request::builder()
        .method("GET")
        .uri(path)
        .header("Host", "localhost");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let req = builder
        .body(http_body_util::Empty::<bytes::Bytes>::new())
        .expect("request");

    let res = sender.send_request(req).await.expect("send");
    let status = res.status().as_u16();
    let content_type = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = res.into_body().collect().await.expect("body").to_bytes();
    Reply {
        status,
        content_type,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

/// Production-shaped, so a leaked `database_error` would be visible as a
/// failure of the CI-05 redaction rather than hidden by a debug build.
fn install_config(readiness_token: Option<&str>) {
    Config::register(
        AppConfig::builder()
            .name("health-readiness-gate-test")
            .environment(Environment::Production)
            .debug(false)
            .url("http://localhost:0")
            .build(),
    );
    let builder = ServerConfig::builder();
    let builder = match readiness_token {
        Some(t) => builder.health_readiness_token(t),
        None => builder,
    };
    Config::register(builder.build());
}

// ---------------------------------------------------------------------
// The default: readiness is public, because six deployment guides say so.
// ---------------------------------------------------------------------

/// The published contract, pinned. If this test ever needs changing, every
/// deployment guide and generated `HEALTHCHECK` needs changing with it.
#[tokio::test]
#[serial]
async fn the_documented_endpoint_keeps_answering_exactly_as_documented() {
    install_config(None);
    let addr = spawn_server().await;

    let plain = get(addr, "/_suprnova/health", &[]).await;
    assert_eq!(plain.status, 200);
    assert_eq!(plain.json()["status"], "ok");
    assert!(
        plain.json().get("database").is_none(),
        "a probe that did not ask about the database must not report on it"
    );

    // No database is initialized in this test process, so `?db=true`
    // reaching the probe is exactly what a 503 proves.
    let probed = get(addr, "/_suprnova/health?db=true", &[]).await;
    assert_eq!(
        probed.status, 503,
        "`?db=true` must still run the probe with no token configured — \
         the Docker HEALTHCHECK, the Railway healthcheckPath and the \
         DigitalOcean app spec all depend on it"
    );
    assert_eq!(probed.json()["database"], "error");
}

/// The substring bug, observed through a socket rather than a unit test:
/// a probe that says "no database" gets no database.
#[tokio::test]
#[serial]
async fn a_query_key_merely_ending_in_db_does_not_trigger_the_probe() {
    install_config(None);
    let addr = spawn_server().await;

    for query in ["nodb=true", "notdb=true", "other=1&notdb=true"] {
        let reply = get(addr, &format!("/_suprnova/health?{query}"), &[]).await;
        assert_eq!(
            reply.status, 200,
            "`?{query}` ran a database probe and reported degraded; it \
             matched the old `query.contains(\"db=true\")` substring test"
        );
        assert!(
            reply.json().get("database").is_none(),
            "`?{query}` must leave the database untouched, so there is \
             nothing to report about it: {}",
            reply.body
        );
    }
}

// ---------------------------------------------------------------------
// The split.
// ---------------------------------------------------------------------

/// Liveness answers while the database is down. This is the whole reason
/// the split exists: a liveness failure restarts the pod, so wiring a
/// database probe into it turns a database blip into a rolling restart of
/// every replica — at the worst possible moment.
#[tokio::test]
#[serial]
async fn liveness_answers_200_while_the_database_is_unreachable() {
    install_config(None);
    let addr = spawn_server().await;

    let reply = get(addr, "/_suprnova/health/live", &[]).await;

    assert_eq!(reply.status, 200);
    assert_eq!(reply.json()["status"], "ok");
    assert!(
        reply.json().get("database").is_none(),
        "liveness must touch nothing: {}",
        reply.body
    );
}

/// And it stays liveness even when asked to probe.
#[tokio::test]
#[serial]
async fn liveness_ignores_a_db_query_parameter() {
    install_config(None);
    let addr = spawn_server().await;

    let reply = get(addr, "/_suprnova/health/live?db=true", &[]).await;

    assert_eq!(
        reply.status, 200,
        "`/live` must not be talked into a dependency probe by a query \
         parameter — its contract is that it touches nothing"
    );
    assert!(reply.json().get("database").is_none());
}

#[tokio::test]
#[serial]
async fn readiness_probes_the_database_without_being_asked_to() {
    install_config(None);
    let addr = spawn_server().await;

    let reply = get(addr, "/_suprnova/health/ready", &[]).await;

    assert_eq!(
        reply.status, 503,
        "`/ready` probes dependencies by definition; no query parameter \
         should be needed to get the behaviour the path names"
    );
    assert_eq!(reply.json()["database"], "error");
    assert_eq!(reply.json()["status"], "degraded");
    assert!(
        reply.json().get("database_error").is_none(),
        "CI-05: the raw driver error must not reach an unauthenticated \
         caller in production. Got: {}",
        reply.body
    );
}

// ---------------------------------------------------------------------
// The gate.
// ---------------------------------------------------------------------

/// The core assertion of the gate: not merely "closed" but *invisible*,
/// and provably so — compared byte for byte against a path that genuinely
/// does not exist. A hand-built 404 would pass a status check and still
/// leak through a different body or content type.
#[tokio::test]
#[serial]
async fn a_gated_readiness_probe_is_indistinguishable_from_an_unrouted_path() {
    install_config(Some(TOKEN));
    let addr = spawn_server().await;

    let unrouted = get(addr, "/_suprnova/no-such-path", &[]).await;
    assert_eq!(unrouted.status, 404, "sanity: the control really is a 404");

    for path in ["/_suprnova/health/ready", "/_suprnova/health?db=true"] {
        let gated = get(addr, path, &[]).await;

        assert_eq!(
            gated.status, unrouted.status,
            "`{path}` must answer with the same status as a path that does \
             not exist; 401 would advertise that something is there"
        );
        assert_eq!(
            gated.body, unrouted.body,
            "`{path}` must answer with the same body as a path that does \
             not exist"
        );
        assert_eq!(
            gated.content_type, unrouted.content_type,
            "`{path}` must answer with the same content type as a path \
             that does not exist"
        );
    }
}

#[tokio::test]
#[serial]
async fn the_configured_token_admits_the_probe() {
    install_config(Some(TOKEN));
    let addr = spawn_server().await;

    for path in ["/_suprnova/health/ready", "/_suprnova/health?db=true"] {
        let reply = get(addr, path, &[(TOKEN_HEADER, TOKEN)]).await;

        assert_eq!(
            reply.status, 503,
            "with the right token `{path}` must reach the probe and report \
             the real state of the database, not 404"
        );
        assert_eq!(reply.json()["database"], "error");
    }
}

/// A wrong token must be as invisible as no token. Includes a value that
/// is a strict *prefix* of the real one: the comparison is constant-time
/// (`subtle::ConstantTimeEq`), and a length-only or short-circuiting
/// comparison is what makes a pollable endpoint into an oracle.
#[tokio::test]
#[serial]
async fn a_wrong_token_is_refused_including_a_prefix_of_the_right_one() {
    install_config(Some(TOKEN));
    let addr = spawn_server().await;

    let wrong = [
        "",
        "wrong",
        "correct-horse-battery-stapl",  // one char short
        "correct-horse-battery-staplf", // same length, last char differs
        "correct-horse-battery-staple-extra",
        "CORRECT-HORSE-BATTERY-STAPLE", // case must matter
    ];

    for candidate in wrong {
        let reply = get(
            addr,
            "/_suprnova/health/ready",
            &[(TOKEN_HEADER, candidate)],
        )
        .await;

        assert_eq!(
            reply.status, 404,
            "token {candidate:?} must not open the readiness probe"
        );
    }
}

/// Closing readiness must not cost you liveness. If it did, operators
/// would have to put the secret in every k8s manifest, and the ones who
/// did not would silently lose their restart-on-hang signal.
#[tokio::test]
#[serial]
async fn a_configured_token_leaves_liveness_public() {
    install_config(Some(TOKEN));
    let addr = spawn_server().await;

    for path in ["/_suprnova/health", "/_suprnova/health/live"] {
        let reply = get(addr, path, &[]).await;

        assert_eq!(
            reply.status, 200,
            "`{path}` touches no dependency, so there is nothing for the \
             readiness secret to protect and no reason to demand it"
        );
        assert_eq!(reply.json()["status"], "ok");
    }
}
