use std::sync::{Arc, OnceLock};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::Method;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use suprnova::{
    Crypt, EncryptionKey, HttpResponse, MiddlewareRegistry, Request, Router, handle_request,
};

const LIVE_UPDATE_PATH: &str = "/__live/v1/action";

fn existing_route(path: &str) -> Router {
    Router::new()
        .post(path, |_request: Request| async {
            Ok(HttpResponse::text("application route"))
        })
        .into()
}

fn ensure_crypt() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
}

fn unregistered_update_body() -> Bytes {
    let digest = "ERERERERERERERERERERERERERERERERERERERERERE";
    Bytes::from(
        serde_json::to_vec(&serde_json::json!({
            "base_revision": "0",
            "component": "tests.unregistered",
            "correlation_id": "AAECAwQFBgcICQoLDA0ODw",
            "extensions": {},
            "idempotency_key": "EBESExQVFhcYGRobHB0eHw",
            "model_proposals": {},
            "operations": [{
                "arguments": {},
                "kind": "invoke_action",
                "name": "save"
            }],
            "protocol_version": 1,
            "runtime_contract_version": 1,
            "snapshot": {
                "envelope": {
                    "body": {
                        "build_id": "build-framework-tests",
                        "component": {
                            "contract_digest": digest,
                            "memo_schema_version": 1,
                            "mount_schema_version": 1,
                            "name": "tests.unregistered",
                            "state_schema_version": 1
                        },
                        "expires_at": "9999999999999",
                        "extensions": {},
                        "form": "instance",
                        "instance_id": "ICEiIyQlJicoKSorLC0uLw",
                        "issued_at": "0",
                        "key_id": "00000000000000000000000000000000",
                        "memo": null,
                        "revision": "0",
                        "route": digest,
                        "schema_version": 1,
                        "scope": digest,
                        "slot": "unregistered",
                        "state": {}
                    },
                    "signature": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
                },
                "kind": "instance"
            },
            "snapshot_schema_version": 1
        }))
        .expect("encode update fixture"),
    )
}

async fn dispatch_one(
    router: Router,
    request: hyper::Request<Full<Bytes>>,
) -> (hyper::StatusCode, hyper::HeaderMap, Bytes) {
    ensure_crypt();
    let router = Arc::new(router);
    let middleware = Arc::new(MiddlewareRegistry::new());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept test request");
        let service = service_fn(move |request| {
            let router = Arc::clone(&router);
            let middleware = Arc::clone(&middleware);
            async move {
                Ok::<_, std::convert::Infallible>(handle_request(router, middleware, request).await)
            }
        });
        let _ = hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(stream), service)
            .await;
    });

    let stream = tokio::net::TcpStream::connect(address)
        .await
        .expect("connect test request");
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .expect("HTTP handshake");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let response = sender
        .send_request(request)
        .await
        .expect("send test request");
    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("collect response body")
        .to_bytes();
    (status, headers, body)
}

#[test]
fn live_installation_is_strictly_single_shot_and_routes_method_rejection() {
    let router = Router::new()
        .try_live()
        .expect("first Live installation succeeds");

    assert!(
        router
            .match_route(&Method::POST, LIVE_UPDATE_PATH)
            .is_some()
    );
    assert!(
        router.match_route(&Method::GET, LIVE_UPDATE_PATH).is_some(),
        "non-POST requests must reach the typed 405 mapper"
    );
    assert!(
        router.try_live().is_err(),
        "an identical second installation must fail"
    );
}

#[test]
fn live_namespace_preflight_rejects_literal_dynamic_and_catch_all_overlap() {
    for path in [
        "/__live/v1/action",
        "/__live/custom",
        "/:feature/v1/action",
        "/{feature}/custom",
        "/{*path}",
    ] {
        assert!(
            existing_route(path).try_live().is_err(),
            "route pattern {path:?} overlaps the reserved Live namespace"
        );
    }
}

#[test]
fn failed_live_preflight_does_not_partially_install_the_endpoint() {
    let router = existing_route("/:feature/custom");
    assert!(router.try_live().is_err());

    let clean = Router::new()
        .try_live()
        .expect("a separate clean router remains installable");
    assert!(clean.match_route(&Method::POST, LIVE_UPDATE_PATH).is_some());
}

#[tokio::test]
async fn non_post_requests_use_the_engine_owned_405_response() {
    let request = hyper::Request::builder()
        .method(Method::GET)
        .uri(LIVE_UPDATE_PATH)
        .body(Full::new(Bytes::new()))
        .expect("build request");
    let (status, headers, body) = dispatch_one(
        Router::new().try_live().expect("install Live routes"),
        request,
    )
    .await;

    assert_eq!(status, hyper::StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(headers.get("allow").expect("Allow header"), "POST");
    assert_eq!(
        headers.get("cache-control").expect("Cache-Control header"),
        "no-store"
    );
    assert_eq!(
        headers
            .get("x-content-type-options")
            .expect("X-Content-Type-Options header"),
        "nosniff"
    );
    assert_eq!(
        headers
            .get("referrer-policy")
            .expect("Referrer-Policy header"),
        "no-referrer"
    );
    assert_eq!(
        headers
            .get("content-security-policy")
            .expect("Content-Security-Policy header"),
        "default-src 'none'; frame-ancestors 'none'"
    );
    assert_eq!(headers.get("content-length").expect("Content-Length"), "0");
    assert!(body.is_empty());
}

#[tokio::test]
async fn invalid_live_media_and_charset_use_closed_engine_mappings() {
    for (content_type, expected) in [
        ("application/json; charset=utf-8; version=1", 415),
        (
            "application/vnd.suprnova.live+json; charset=latin1; version=1",
            415,
        ),
        (
            "application/vnd.suprnova.live+json; charset=utf-8; version=99",
            400,
        ),
    ] {
        let request = hyper::Request::builder()
            .method(Method::POST)
            .uri(LIVE_UPDATE_PATH)
            .header("content-type", content_type)
            .body(Full::new(Bytes::from_static(b"{}")))
            .expect("build request");
        let (status, headers, body) = dispatch_one(
            Router::new().try_live().expect("install Live routes"),
            request,
        )
        .await;

        assert_eq!(status.as_u16(), expected, "content type {content_type:?}");
        assert_eq!(
            headers.get("cache-control").expect("Cache-Control header"),
            "no-store"
        );
        assert_eq!(headers.get("content-length").expect("Content-Length"), "0");
        assert!(body.is_empty());
    }
}

#[tokio::test]
async fn oversized_live_body_is_rejected_before_protocol_or_component_work() {
    let request = hyper::Request::builder()
        .method(Method::POST)
        .uri(LIVE_UPDATE_PATH)
        .header(
            "content-type",
            "application/vnd.suprnova.live+json; charset=utf-8; version=1",
        )
        .body(Full::new(Bytes::from(vec![b'x'; 1024 * 1024 + 1])))
        .expect("build request");
    let (status, headers, body) = dispatch_one(
        Router::new().try_live().expect("install Live routes"),
        request,
    )
    .await;

    assert_eq!(status, hyper::StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        headers.get("cache-control").expect("Cache-Control header"),
        "no-store"
    );
    assert_eq!(headers.get("content-length").expect("Content-Length"), "0");
    assert!(body.is_empty());
}

#[tokio::test]
async fn malformed_live_protocol_is_rejected_before_context_or_component_work() {
    let request = hyper::Request::builder()
        .method(Method::POST)
        .uri(LIVE_UPDATE_PATH)
        .header(
            "content-type",
            "application/vnd.suprnova.live+json; charset=utf-8; version=1",
        )
        .body(Full::new(Bytes::from_static(b"{}")))
        .expect("build request");
    let (status, headers, body) = dispatch_one(
        Router::new().try_live().expect("install Live routes"),
        request,
    )
    .await;

    assert_eq!(status, hyper::StatusCode::BAD_REQUEST);
    assert_eq!(
        headers.get("cache-control").expect("Cache-Control header"),
        "no-store"
    );
    assert_eq!(headers.get("content-length").expect("Content-Length"), "0");
    assert!(body.is_empty());
}

#[tokio::test]
async fn unregistered_mount_selection_is_concealed_before_snapshot_or_component_work() {
    let request = hyper::Request::builder()
        .method(Method::POST)
        .uri(LIVE_UPDATE_PATH)
        .header(
            "content-type",
            "application/vnd.suprnova.live+json; charset=utf-8; version=1",
        )
        .body(Full::new(unregistered_update_body()))
        .expect("build request");
    let (status, headers, body) = dispatch_one(
        Router::new().try_live().expect("install Live routes"),
        request,
    )
    .await;

    assert_eq!(status, hyper::StatusCode::NOT_FOUND);
    assert_eq!(
        headers.get("cache-control").expect("Cache-Control header"),
        "no-store"
    );
    assert_eq!(headers.get("content-length").expect("Content-Length"), "0");
    assert!(body.is_empty());
}
