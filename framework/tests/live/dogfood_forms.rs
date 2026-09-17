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
