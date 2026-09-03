//! Anonymous visitors act on public seeds through the production stack while
//! identity-bound islands keep refusing them: the guard's optional
//! authentication records a principal only when one exists, and the action
//! boundary closes only the absences the mount kind permits.

mod live_dogfood_support;

use std::sync::Arc;

use live_dogfood_support::{
    ActionRequest, DOCUMENT_PATH, PRIVATE_DOCUMENT_PATH, action_request, build_public_router,
    decoded_snapshot, dispatch, fixture, get, private_action_request, production_middleware,
    session_cookie,
};
use serde_json::Value;
use suprnova::StatusCode;
use suprnova::container::testing::TestContainer;
use suprnova::live::testing::prepare_live_router_for_test;

#[tokio::test]
#[serial_test::serial]
async fn an_anonymous_visitor_promotes_and_acts_on_a_public_seed() {
    let _container = TestContainer::fake();
    fixture();
    let router = Arc::new(build_public_router());
    prepare_live_router_for_test(&router).expect("prepare Live runtime");
    let middleware = production_middleware();

    let (status, headers, body) =
        dispatch(router.clone(), middleware.clone(), get(DOCUMENT_PATH)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let cookie = session_cookie(&headers);
    let snapshot = decoded_snapshot(&body);

    let (status, _, body) = dispatch(
        router.clone(),
        middleware.clone(),
        action_request(ActionRequest {
            snapshot,
            cookie: &cookie,
            fetch_site: Some("same-origin"),
            login: None,
            idempotency_key: "QEFCQ0RFRkdISUpLTE1OTw",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let accepted: Value = serde_json::from_slice(&body).expect("accepted action JSON");
    assert_eq!(accepted["outcome"], "accepted");
    assert!(
        accepted["render"]["html"]
            .as_str()
            .is_some_and(|html| html.contains(">1</button>")),
        "the promoted island re-rendered with the incremented count: {accepted}"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn an_identity_bound_island_still_refuses_an_anonymous_action() {
    let _container = TestContainer::fake();
    fixture();
    let router = Arc::new(build_public_router());
    prepare_live_router_for_test(&router).expect("prepare Live runtime");
    let middleware = production_middleware();

    // Sign in on one request, as a login handler would, so the identity-bound
    // render on the next request binds the session that survives the
    // framework's fixation rotation.
    let mut login = get(DOCUMENT_PATH);
    login
        .headers_mut()
        .insert("x-test-login", "user-7".parse().expect("header"));
    let (status, headers, body) = dispatch(router.clone(), middleware.clone(), login).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let signed_in_cookie = session_cookie(&headers);

    let mut private = get(PRIVATE_DOCUMENT_PATH);
    private
        .headers_mut()
        .insert("x-test-login", "user-7".parse().expect("header"));
    private
        .headers_mut()
        .insert("cookie", signed_in_cookie.parse().expect("cookie"));
    let (status, headers, body) = dispatch(router.clone(), middleware.clone(), private).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let owner_cookie = session_cookie(&headers);
    let snapshot = decoded_snapshot(&body);

    // A different, anonymous visitor replays the owner's snapshot.
    let (status, headers, body) =
        dispatch(router.clone(), middleware.clone(), get(DOCUMENT_PATH)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let anonymous_cookie = session_cookie(&headers);
    let (status, _, body) = dispatch(
        router.clone(),
        middleware.clone(),
        private_action_request(ActionRequest {
            snapshot: snapshot.clone(),
            cookie: &anonymous_cookie,
            fetch_site: Some("same-origin"),
            login: None,
            idempotency_key: "QEFCQ0RFRkdISUpLTE1OTw",
        }),
    )
    .await;
    assert!(
        status.is_client_error(),
        "an anonymous action on an identity-bound island is refused: {status} {}",
        String::from_utf8_lossy(&body)
    );

    let (status, _, body) = dispatch(
        router.clone(),
        middleware.clone(),
        private_action_request(ActionRequest {
            snapshot,
            cookie: &owner_cookie,
            fetch_site: Some("same-origin"),
            login: Some("user-7"),
            idempotency_key: "QUFCQ0RFRkdISUpLTE1OTw",
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the same island accepts its signed-in owner: {}",
        String::from_utf8_lossy(&body)
    );
    let accepted: Value = serde_json::from_slice(&body).expect("accepted action JSON");
    assert_eq!(accepted["outcome"], "accepted", "{accepted}");
}
