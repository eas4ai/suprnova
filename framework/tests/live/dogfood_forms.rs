//! LIVE-031 through the production middleware stack: a model proposal its
//! field cannot decode answers a validation error on that field, and the
//! action it accompanies does not run.
// Its own test binary. These tests bind the process-global Live runtime
// and mount catalog, and a second binding in one process is rejected, so
// this file may not share a binary with the counter-only dogfood tests.
#[path = "../support/env_lock.rs"]
mod env_lock;
#[path = "../support/live_dogfood_support/mod.rs"]
mod live_dogfood_support;

use std::sync::Arc;

use live_dogfood_support::{
    ActionRequest, FORM_DOCUMENT_PATH, build_form_router, decoded_snapshot, dispatch,
    form_action_request, form_fixture, get, production_middleware, session_cookie,
};
use serde_json::{Value, json};
use suprnova::StatusCode;
use suprnova::container::testing::TestContainer;
use suprnova::live::testing::prepare_live_router_for_test;

/// LIVE-031: a proposal its field cannot decode (null for a `u64`, a boolean or
/// a string where a list or a number belongs) answers a validation error on
/// that field, and the action does not run; a decodable proposal saves.
#[tokio::test]
#[serial_test::serial]
async fn live_031_an_undecodable_proposal_is_a_field_error_and_the_action_does_not_run() {
    let _container = TestContainer::fake();
    form_fixture();
    let router = Arc::new(build_form_router());
    prepare_live_router_for_test(&router).expect("prepare Live runtime");
    let middleware = production_middleware();

    let cases = [
        (json!({"seats": null}), "seats", "QUFBQUFBQUFBQUFBQUFBQQ"),
        (json!({"topics": false}), "topics", "QkJCQkJCQkJCQkJCQkJCQg"),
        (json!({"seats": "many"}), "seats", "Q0NDQ0NDQ0NDQ0NDQ0NDQw"),
        (
            json!({"seats": 2, "topics": "releases"}),
            "topics",
            "RERERERERERERERERERERA",
        ),
    ];
    for (proposals, field, idempotency_key) in cases {
        let (status, headers, body) =
            dispatch(router.clone(), middleware.clone(), get(FORM_DOCUMENT_PATH)).await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        let cookie = session_cookie(&headers);
        let snapshot = decoded_snapshot(&body);
        let (status, _, body) = dispatch(
            router.clone(),
            middleware.clone(),
            form_action_request(
                ActionRequest {
                    snapshot,
                    cookie: &cookie,
                    fetch_site: Some("same-origin"),
                    login: Some("user-7"),
                    idempotency_key,
                },
                proposals.clone(),
                true,
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "{proposals}: {}",
            String::from_utf8_lossy(&body)
        );
        let answer: Value = serde_json::from_slice(&body).expect("action JSON");
        assert!(
            answer["validation"].get(field).is_some(),
            "{proposals} carries a validation error on {field}: {answer}"
        );
        assert!(
            answer["render"]["html"]
                .as_str()
                .is_some_and(|html| html.contains("<p id=\"saves\">0</p>")),
            "{proposals} did not run save: {answer}"
        );
    }

    let (_, headers, body) =
        dispatch(router.clone(), middleware.clone(), get(FORM_DOCUMENT_PATH)).await;
    let cookie = session_cookie(&headers);
    let snapshot = decoded_snapshot(&body);
    let (status, _, body) = dispatch(
        router.clone(),
        middleware.clone(),
        form_action_request(
            ActionRequest {
                snapshot,
                cookie: &cookie,
                fetch_site: Some("same-origin"),
                login: Some("user-7"),
                idempotency_key: "RUVFRUVFRUVFRUVFRUVFRQ",
            },
            json!({"seats": 3, "topics": ["releases", "security"]}),
            true,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let answer: Value = serde_json::from_slice(&body).expect("action JSON");
    assert!(
        answer["validation"]
            .as_object()
            .is_none_or(|entries| entries.is_empty()),
        "a decodable proposal carries no validation error: {answer}"
    );
    let html = answer["render"]["html"].as_str().expect("render html");
    assert!(html.contains("<p id=\"saves\">1</p>"), "{answer}");
    assert!(html.contains("value=\"3\""), "{answer}");

    // A model sync with no action, on the instance the save promoted, answers
    // the same field error.
    let snapshot = answer["snapshot"].clone();
    let (status, _, body) = dispatch(
        router.clone(),
        middleware.clone(),
        form_action_request(
            ActionRequest {
                snapshot,
                cookie: &cookie,
                fetch_site: Some("same-origin"),
                login: Some("user-7"),
                idempotency_key: "RkZGRkZGRkZGRkZGRkZGRg",
            },
            json!({"seats": null}),
            false,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let answer: Value = serde_json::from_slice(&body).expect("sync JSON");
    assert!(
        answer["validation"].get("seats").is_some(),
        "a model sync carries the field error: {answer}"
    );

    // An action on that instance, with a refused proposal beside it, answers
    // the field error and leaves the save count where the first save put it.
    let snapshot = answer["snapshot"].clone();
    let (status, _, body) = dispatch(
        router,
        middleware,
        form_action_request(
            ActionRequest {
                snapshot,
                cookie: &cookie,
                fetch_site: Some("same-origin"),
                login: Some("user-7"),
                idempotency_key: "R0dHR0dHR0dHR0dHR0dHRw",
            },
            json!({"seats": null}),
            true,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let answer: Value = serde_json::from_slice(&body).expect("instanced action JSON");
    assert!(
        answer["validation"].get("seats").is_some(),
        "an instanced action carries the field error: {answer}"
    );
    assert!(
        answer["render"]["html"]
            .as_str()
            .is_some_and(|html| html.contains("<p id=\"saves\">1</p>")),
        "the instanced save did not run: {answer}"
    );
}

/// The browser runtime sends an immediate `live:model` edit on a public seed
/// as a model sync with no action. That first request promotes the seed for
/// the visitor, applies the proposal, and answers the promoted instance; a
/// save on that instance then runs with the synchronized value.
#[tokio::test]
#[serial_test::serial]
async fn a_model_sync_promotes_a_public_seed() {
    let _container = TestContainer::fake();
    form_fixture();
    let router = Arc::new(build_form_router());
    prepare_live_router_for_test(&router).expect("prepare Live runtime");
    let middleware = production_middleware();

    let (status, headers, body) =
        dispatch(router.clone(), middleware.clone(), get(FORM_DOCUMENT_PATH)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let cookie = session_cookie(&headers);
    let seed = decoded_snapshot(&body);

    let (status, _, body) = dispatch(
        router.clone(),
        middleware.clone(),
        form_action_request(
            ActionRequest {
                snapshot: seed,
                cookie: &cookie,
                fetch_site: Some("same-origin"),
                login: Some("user-7"),
                idempotency_key: "SEhISEhISEhISEhISEhISA",
            },
            json!({"seats": 4}),
            false,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let answer: Value = serde_json::from_slice(&body).expect("sync JSON");
    assert_eq!(answer["outcome"], "accepted", "{answer}");
    let html = answer["render"]["html"].as_str().expect("render html");
    assert!(
        html.contains("value=\"4\""),
        "the proposal was applied: {answer}"
    );
    assert!(
        html.contains("<p id=\"saves\">0</p>"),
        "no action ran: {answer}"
    );
    assert!(
        !answer["snapshot"]["body"]["revision"].is_null(),
        "the answer carries the promoted instance: {answer}"
    );

    let (status, _, body) = dispatch(
        router.clone(),
        middleware.clone(),
        form_action_request(
            ActionRequest {
                snapshot: answer["snapshot"].clone(),
                cookie: &cookie,
                fetch_site: Some("same-origin"),
                login: Some("user-7"),
                idempotency_key: "SUlJSUlJSUlJSUlJSUlJSQ",
            },
            json!({}),
            true,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let answer: Value = serde_json::from_slice(&body).expect("save JSON");
    let html = answer["render"]["html"].as_str().expect("render html");
    assert!(html.contains("<p id=\"saves\">1</p>"), "{answer}");
    assert!(
        html.contains("value=\"4\""),
        "the instance kept the synchronized seats: {answer}"
    );
}

/// A first model sync whose proposal its field cannot decode promotes the
/// seed and answers the field error, as the same sync does on an instance.
#[tokio::test]
#[serial_test::serial]
async fn an_undecodable_first_model_sync_answers_the_field_error() {
    let _container = TestContainer::fake();
    form_fixture();
    let router = Arc::new(build_form_router());
    prepare_live_router_for_test(&router).expect("prepare Live runtime");
    let middleware = production_middleware();

    let (status, headers, body) =
        dispatch(router.clone(), middleware.clone(), get(FORM_DOCUMENT_PATH)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let cookie = session_cookie(&headers);
    let seed = decoded_snapshot(&body);

    let (status, _, body) = dispatch(
        router,
        middleware,
        form_action_request(
            ActionRequest {
                snapshot: seed,
                cookie: &cookie,
                fetch_site: Some("same-origin"),
                login: Some("user-7"),
                idempotency_key: "SkpKSkpKSkpKSkpKSkpKSg",
            },
            json!({"seats": null}),
            false,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let answer: Value = serde_json::from_slice(&body).expect("sync JSON");
    assert!(
        answer["validation"].get("seats").is_some(),
        "the first model sync carries the field error: {answer}"
    );
}

/// The same first model sync sent twice, with one seed, nonce and
/// idempotency key, never mints a second instance.
#[tokio::test]
#[serial_test::serial]
async fn a_replayed_first_model_sync_does_not_mint_a_second_instance() {
    let _container = TestContainer::fake();
    form_fixture();
    let router = Arc::new(build_form_router());
    prepare_live_router_for_test(&router).expect("prepare Live runtime");
    let middleware = production_middleware();

    let (status, headers, body) =
        dispatch(router.clone(), middleware.clone(), get(FORM_DOCUMENT_PATH)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let cookie = session_cookie(&headers);
    let seed = decoded_snapshot(&body);
    let sync = || {
        form_action_request(
            ActionRequest {
                snapshot: seed.clone(),
                cookie: &cookie,
                fetch_site: Some("same-origin"),
                login: Some("user-7"),
                idempotency_key: "S0tLS0tLS0tLS0tLS0tLSw",
            },
            json!({"seats": 4}),
            false,
        )
    };

    let (status, _, body) = dispatch(router.clone(), middleware.clone(), sync()).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let first: Value = serde_json::from_slice(&body).expect("sync JSON");
    let instance = first["snapshot"]["body"]["instance_id"].clone();
    assert!(instance.is_string(), "{first}");

    let (status, _, body) = dispatch(router, middleware, sync()).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let replay: Value = serde_json::from_slice(&body).expect("replay JSON");
    assert_eq!(replay["outcome"], "refresh_required", "{replay}");
    assert!(
        replay["snapshot"].is_null(),
        "the replay carries no second instance: {replay}"
    );
}

/// A first model sync can only propose model fields: a proposal for the
/// public `saves` counter is refused and nothing is accepted.
#[tokio::test]
#[serial_test::serial]
async fn a_first_model_sync_cannot_write_a_non_model_field() {
    let _container = TestContainer::fake();
    form_fixture();
    let router = Arc::new(build_form_router());
    prepare_live_router_for_test(&router).expect("prepare Live runtime");
    let middleware = production_middleware();

    let (status, headers, body) =
        dispatch(router.clone(), middleware.clone(), get(FORM_DOCUMENT_PATH)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let cookie = session_cookie(&headers);
    let seed = decoded_snapshot(&body);

    let (status, _, body) = dispatch(
        router,
        middleware,
        form_action_request(
            ActionRequest {
                snapshot: seed,
                cookie: &cookie,
                fetch_site: Some("same-origin"),
                login: Some("user-7"),
                idempotency_key: "TExMTExMTExMTExMTExMTA",
            },
            json!({"saves": 99}),
            false,
        ),
    )
    .await;
    assert!(
        !status.is_success(),
        "a proposal for a non-model field is refused: {status} {}",
        String::from_utf8_lossy(&body)
    );
    assert!(!String::from_utf8_lossy(&body).contains(">99<"));
}

/// A first model sync without same-origin evidence is refused like any other
/// Live request, before promotion.
#[tokio::test]
#[serial_test::serial]
async fn a_cross_site_first_model_sync_is_refused() {
    let _container = TestContainer::fake();
    form_fixture();
    let router = Arc::new(build_form_router());
    prepare_live_router_for_test(&router).expect("prepare Live runtime");
    let middleware = production_middleware();

    let (status, headers, body) =
        dispatch(router.clone(), middleware.clone(), get(FORM_DOCUMENT_PATH)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let cookie = session_cookie(&headers);
    let seed = decoded_snapshot(&body);

    for fetch_site in [None, Some("cross-site")] {
        let (status, _, body) = dispatch(
            router.clone(),
            middleware.clone(),
            form_action_request(
                ActionRequest {
                    snapshot: seed.clone(),
                    cookie: &cookie,
                    fetch_site,
                    login: Some("user-7"),
                    idempotency_key: "TU1NTU1NTU1NTU1NTU1NTQ",
                },
                json!({"seats": 4}),
                false,
            ),
        )
        .await;
        assert!(
            status.is_client_error(),
            "{fetch_site:?}: a model sync without same-origin evidence is refused: {status} {}",
            String::from_utf8_lossy(&body)
        );
    }
}

/// The 2026-10-04 limits ruling, through the production stack: a model
/// proposal of 10,000 entries, one of them a 100 KiB string, saves under the
/// default limits, and the snapshot that carries it comes back in the next
/// request and is accepted. The old defaults refused it twice over: a
/// 1 MiB request, 8,192 JSON entries and a 64 KiB field encoding bound.
#[tokio::test]
#[serial_test::serial]
async fn a_large_model_state_round_trips_under_the_default_limits() {
    let _container = TestContainer::fake();
    form_fixture();
    let router = Arc::new(build_form_router());
    prepare_live_router_for_test(&router).expect("prepare Live runtime");
    let middleware = production_middleware();

    let (status, headers, body) =
        dispatch(router.clone(), middleware.clone(), get(FORM_DOCUMENT_PATH)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let cookie = session_cookie(&headers);
    let seed = decoded_snapshot(&body);

    let mut topics: Vec<String> = (0..10_000).map(|index| format!("topic-{index}")).collect();
    topics[0] = "t".repeat(100 * 1024);
    let (status, _, body) = dispatch(
        router.clone(),
        middleware.clone(),
        form_action_request(
            ActionRequest {
                snapshot: seed,
                cookie: &cookie,
                fetch_site: Some("same-origin"),
                login: Some("user-7"),
                idempotency_key: "SEhISEhISEhISEhISEhISA",
            },
            json!({"seats": 4, "topics": topics}),
            true,
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&body[..body.len().min(512)])
    );
    let answer: Value = serde_json::from_slice(&body).expect("action JSON");
    assert_eq!(answer["outcome"], "accepted");
    let snapshot = answer["snapshot"].clone();
    let state = &snapshot["body"]["state"]["topics"];
    assert_eq!(state.as_array().map(Vec::len), Some(10_000));
    assert_eq!(state[0].as_str().map(str::len), Some(100 * 1024));

    let (status, _, body) = dispatch(
        router,
        middleware,
        form_action_request(
            ActionRequest {
                snapshot,
                cookie: &cookie,
                fetch_site: Some("same-origin"),
                login: Some("user-7"),
                idempotency_key: "SUlJSUlJSUlJSUlJSUlJSQ",
            },
            json!({"seats": 5}),
            true,
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&body[..body.len().min(512)])
    );
    let answer: Value = serde_json::from_slice(&body).expect("second action JSON");
    assert_eq!(
        answer["outcome"], "accepted",
        "the large snapshot came back and was accepted"
    );
    let html = answer["render"]["html"].as_str().expect("render html");
    assert!(html.contains("<p id=\"saves\">2</p>"), "both saves ran");
    assert_eq!(
        answer["snapshot"]["body"]["state"]["topics"]
            .as_array()
            .map(Vec::len),
        Some(10_000)
    );
}
