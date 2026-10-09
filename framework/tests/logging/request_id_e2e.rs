//! End-to-end request-id tests that drive the real `handle_request`
//! path (router + middleware registry + hyper request), as opposed to
//! the `logging.rs` tests which drive `chain.execute()` directly.
//!
//! Driving `handle_request` is what makes the panic-boundary and
//! built-in-endpoint behaviours testable: a handler panic is caught in
//! `execute_chain_safely`, and the health endpoint short-circuits
//! before the normal routing path - neither is reachable through a
//! bare `chain.execute()`.

use crate::http_wire::request;

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;

use suprnova::http::text;
use suprnova::{MiddlewareRegistry, Router, handle_request};
use tracing_test::traced_test;

/// Spawn a test server that routes through the real `handle_request`.
async fn spawn_server(
    router: impl Into<Router>,
    registry: MiddlewareRegistry,
    accepts: usize,
) -> SocketAddr {
    let router = Arc::new(router.into());
    let middleware = Arc::new(registry);

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

/// A router whose `/boom` handler panics, `/logs` handler emits a
/// `tracing` event, plus a healthy `/ok` route.
fn router() -> Router {
    Router::new()
        .get("/ok", |_req| async { text("ok") })
        .get("/boom", |_req| async move {
            panic!("intentional handler panic for the request-id echo test");
            #[allow(unreachable_code)]
            text("unreachable")
        })
        .get("/logs", |_req| async {
            // Deliberately does NOT mention any request id - if the id
            // shows up in this event's captured output, it can only have
            // come from the surrounding `request` span context.
            tracing::info!(target: "span_probe", "handler executed");
            text("logged")
        })
        .into()
}

/// A panicking handler is caught by `execute_chain_safely` and converted
/// to a 500 OUTSIDE the `RequestIdMiddleware` scope (the unwind tore the
/// scope down). The synthesized 500 must still echo the inbound
/// `X-Request-Id` so an operator can correlate the client-visible error
/// with the structured panic log. Before the single-source-id fix the
/// header was absent on this path.
#[tokio::test]
async fn panic_response_still_echoes_inbound_request_id() {
    let addr = spawn_server(router(), MiddlewareRegistry::new(), 1).await;

    let (status, headers, _body) = request(
        addr,
        "GET",
        "/boom",
        &[("X-Request-Id", "panic-echo-correlation-id-0001")],
    )
    .await;

    assert_eq!(status, 500, "a panicking handler must surface as a 500");
    assert_eq!(
        headers.get("x-request-id").map(String::as_str),
        Some("panic-echo-correlation-id-0001"),
        "the synthesized panic 500 must echo the inbound X-Request-Id"
    );
}

/// A panic without an inbound id must still carry SOME echoed
/// `X-Request-Id` (a fresh one), so every panic 500 is correlatable.
#[tokio::test]
async fn panic_response_echoes_a_fresh_request_id_when_none_supplied() {
    let addr = spawn_server(router(), MiddlewareRegistry::new(), 1).await;

    let (status, headers, _body) = request(addr, "GET", "/boom", &[]).await;

    assert_eq!(status, 500);
    let echoed = headers
        .get("x-request-id")
        .expect("panic 500 must carry a fresh X-Request-Id even with no inbound id");
    // Fresh ids are lowercase hyphenated UUID v4 (36 chars, 4 dashes).
    assert_eq!(echoed.len(), 36, "fresh id should be a UUID v4");
    assert_eq!(echoed.chars().filter(|c| *c == '-').count(), 4);
}

/// The HIGH fix: `RequestIdMiddleware` enters a `request` span carrying
/// `request_id`, so a downstream handler's `tracing` event inherits the
/// id as span context even though the event itself never mentions it.
/// `logs_contain` matches against the formatted output, which includes
/// the span-field prefix - so a hit proves the id propagated via the
/// span, not via the event. Without `.instrument(span)` the id would be
/// absent from the handler's log line.
#[tokio::test]
#[traced_test]
async fn downstream_events_inherit_request_id_via_request_span() {
    let addr = spawn_server(router(), MiddlewareRegistry::new(), 1).await;

    let (status, _headers, _body) = request(
        addr,
        "GET",
        "/logs",
        &[("X-Request-Id", "span-context-probe-id-4242")],
    )
    .await;
    assert_eq!(status, 200);

    assert!(
        logs_contain("span-context-probe-id-4242"),
        "the /logs handler event must carry request_id from the request span context"
    );
}

/// The built-in `/_suprnova/health` endpoint short-circuits before the
/// middleware chain, but must still honor the `X-Request-Id` contract so
/// liveness probes stay correlatable with logs.
#[tokio::test]
async fn health_endpoint_echoes_inbound_request_id() {
    let addr = spawn_server(router(), MiddlewareRegistry::new(), 1).await;

    let (status, headers, _body) = request(
        addr,
        "GET",
        "/_suprnova/health",
        &[("X-Request-Id", "health-probe-id-9001")],
    )
    .await;

    assert_eq!(status, 200);
    assert_eq!(
        headers.get("x-request-id").map(String::as_str),
        Some("health-probe-id-9001"),
        "the health endpoint must echo the inbound X-Request-Id"
    );
}

/// With no inbound id, the health endpoint still mints and echoes a fresh
/// one (UUID v4), matching the routed-path contract.
#[tokio::test]
async fn health_endpoint_echoes_a_fresh_request_id_when_none_supplied() {
    let addr = spawn_server(router(), MiddlewareRegistry::new(), 1).await;

    let (status, headers, _body) = request(addr, "GET", "/_suprnova/health", &[]).await;

    assert_eq!(status, 200);
    let echoed = headers
        .get("x-request-id")
        .expect("health endpoint must carry a fresh X-Request-Id");
    assert_eq!(echoed.len(), 36, "fresh id should be a UUID v4");
    assert_eq!(echoed.chars().filter(|c| *c == '-').count(), 4);
}

/// An unrouted path with no registered fallback hits the static 404, which
/// still runs the RequestId + global middleware chain - so the 404 must
/// carry `X-Request-Id`, keeping even not-found traffic correlatable.
#[tokio::test]
async fn default_404_echoes_request_id() {
    let addr = spawn_server(router(), MiddlewareRegistry::new(), 1).await;

    let (status, headers, _body) = request(
        addr,
        "GET",
        "/no-such-route",
        &[("X-Request-Id", "notfound-probe-id-7")],
    )
    .await;

    assert_eq!(status, 404);
    assert_eq!(
        headers.get("x-request-id").map(String::as_str),
        Some("notfound-probe-id-7"),
        "the static 404 must echo X-Request-Id"
    );
}

#[derive(serde::Serialize, serde::Deserialize)]
struct QueuedByARequest;

#[suprnova::async_trait]
impl suprnova::Job for QueuedByARequest {
    fn job_name() -> &'static str {
        "request_id_e2e::QueuedByARequest"
    }
    async fn handle(self) -> Result<(), suprnova::FrameworkError> {
        Ok(())
    }
}

/// A job queued while a request is served carries the request's id, so the
/// job and the log lines it writes can be traced to the request. The request
/// middleware puts the id in the `Context`, and the push snapshots the
/// `Context` into the envelope.
#[tokio::test]
#[serial_test::serial]
async fn a_job_queued_by_a_request_carries_the_requests_id() {
    use suprnova::queue::{MemoryQueueDriver, Queue, QueueDriver};

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    let router: Router = Router::new()
        .get("/queue", |_req| async {
            Queue::push(QueuedByARequest).await?;
            text("queued")
        })
        .into();
    let addr = spawn_server(router, MiddlewareRegistry::new(), 1).await;

    let (status, headers, _body) = request(
        addr,
        "GET",
        "/queue",
        &[("X-Request-Id", "queued-by-request-0001")],
    )
    .await;

    assert_eq!(status, 200);
    assert_eq!(
        headers.get("x-request-id").map(String::as_str),
        Some("queued-by-request-0001")
    );
    let envelope = driver
        .pop(Duration::from_secs(5))
        .await
        .unwrap()
        .expect("the handler queued a job")
        .envelope;
    let context = envelope
        .context
        .expect("a request always has its id to carry");
    assert_eq!(
        context.data.get("_request_id"),
        Some(&serde_json::json!("queued-by-request-0001"))
    );
}
