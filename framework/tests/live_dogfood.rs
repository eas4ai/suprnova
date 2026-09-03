//! Live through the production middleware stack: real sessions, origin-verified
//! CSRF, an authenticated principal, and tenant and rate-limit facts attached to
//! the reserved routes with `Router::try_live_with`.

mod live_dogfood_support;

use std::sync::Arc;

use bytes::Bytes;
use http_body_util::Full;
use live_dogfood_support::{
    ActionRequest, DOCUMENT_PATH, SUBSCRIPTION_PATH, action_request, build_router,
    decoded_snapshot, dispatch, fixture, get, production_middleware, session_cookie,
};
use serde_json::Value;
use suprnova::StatusCode;
use suprnova::container::testing::TestContainer;
use suprnova::live::testing::prepare_live_router_for_test;

#[tokio::test]
#[serial_test::serial]
async fn a_signed_in_user_runs_an_action_through_the_production_stack() {
    let _container = TestContainer::fake();
    fixture();
    let router = Arc::new(build_router());
    prepare_live_router_for_test(&router).expect("prepare Live runtime");
    let middleware = production_middleware();

    let (status, headers, body) =
        dispatch(router.clone(), middleware.clone(), get(DOCUMENT_PATH)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let html = std::str::from_utf8(&body).expect("document UTF-8");
    assert!(
        html.contains("<h1>Dogfood</h1>"),
        "ordinary SSR content renders"
    );
    assert!(
        html.contains("id=\"suprnova-live-config\""),
        "bootstrap configuration is emitted"
    );
    assert!(
        html.contains("data-suprnova-live-island"),
        "the island mounts"
    );
    let cookie = session_cookie(&headers);
    let snapshot = decoded_snapshot(&body);

    let (status, headers, body) = dispatch(
        router.clone(),
        middleware.clone(),
        action_request(ActionRequest {
            snapshot: snapshot.clone(),
            cookie: &cookie,
            fetch_site: Some("same-origin"),
            login: Some("user-7"),
            idempotency_key: "QEFCQ0RFRkdISUpLTE1OTw",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(headers["cache-control"], "no-store");
    let accepted: Value = serde_json::from_slice(&body).expect("accepted action JSON");
    assert_eq!(accepted["outcome"], "accepted");
    assert!(
        accepted["render"]["html"]
            .as_str()
            .is_some_and(|html| html.contains(">1</button>")),
        "the island re-rendered with the incremented count: {accepted}"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn the_guard_and_the_configured_csrf_proof_fail_closed() {
    let _container = TestContainer::fake();
    fixture();
    let router = Arc::new(build_router());
    prepare_live_router_for_test(&router).expect("prepare Live runtime");
    let middleware = production_middleware();
    let (status, headers, body) =
        dispatch(router.clone(), middleware.clone(), get(DOCUMENT_PATH)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let cookie = session_cookie(&headers);
    let snapshot = decoded_snapshot(&body);

    // No principal: the guard's AuthMiddleware answers before any engine work.
    let (status, _, _) = dispatch(
        router.clone(),
        middleware.clone(),
        action_request(ActionRequest {
            snapshot: snapshot.clone(),
            cookie: &cookie,
            fetch_site: Some("same-origin"),
            login: None,
            idempotency_key: "QEFCQ0RFRkdISUpLTE1OTw",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // A cross-site or header-less request falls back to token validation,
    // which the shipped runtime cannot satisfy, so the state change is refused.
    for fetch_site in [Some("cross-site"), None] {
        let (status, _, _) = dispatch(
            router.clone(),
            middleware.clone(),
            action_request(ActionRequest {
                snapshot: snapshot.clone(),
                cookie: &cookie,
                fetch_site,
                login: Some("user-7"),
                idempotency_key: "QEFCQ0RFRkdISUpLTE1OTw",
            }),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::from_u16(419).expect("419"),
            "{fetch_site:?}"
        );
    }

    // The guard covers the asynchronous control routes too.
    let subscription = hyper::Request::builder()
        .method(hyper::Method::POST)
        .uri(SUBSCRIPTION_PATH)
        .header("host", "127.0.0.1")
        .header("content-type", "application/json")
        .header("x-suprnova-live", "async-v1")
        .header("sec-fetch-site", "same-origin")
        .header("cookie", &cookie)
        .body(Full::new(Bytes::from_static(b"{}")))
        .expect("build subscription request");
    let (status, _, _) = dispatch(router.clone(), middleware.clone(), subscription).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Assets stay public: no session, principal, or origin proof is needed.
    let config: Value = {
        let html = std::str::from_utf8(&body).expect("html");
        let start = html
            .find("<script id=\"suprnova-live-config\"")
            .expect("config element");
        let open = html[start..].find('>').expect("config open") + start + 1;
        let close = html[open..].find("</script>").expect("config close") + open;
        serde_json::from_str(&html[open..close]).expect("config JSON")
    };
    let asset = format!(
        "/__live/v1/assets/{}/suprnova-live.esm.js",
        config["asset_identity"].as_str().expect("asset identity")
    );
    let (status, headers, _) = dispatch(router, middleware, get(&asset)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers["cache-control"],
        "public, max-age=31536000, immutable"
    );
}
