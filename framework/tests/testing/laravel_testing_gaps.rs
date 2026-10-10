//! The test client and test response members of Laravel's testing surface
//! that Suprnova lacked.
//!
//! PAR-174: `TestClient::with_header`, `with_headers` and `flush_headers`
//! carry default headers onto every request the client builds, a request's
//! own header winning, and `query_json` sends a `QUERY` request with a JSON
//! body, as Laravel's `withHeaders`, `flushHeaders` and `queryJson` do.
//!
//! PAR-180: `TestResponse::assert_no_content` passes only for an empty
//! `204`, and `assert_no_content_status` for an empty response of another
//! status, as Laravel's `assertNoContent` does; a failure prints the
//! response's error report.

use std::panic::{AssertUnwindSafe, catch_unwind};

use serde_json::{Value, json};

use suprnova::testing::{TestClient, TestResponse};
use suprnova::{FrameworkError, HttpResponse, MiddlewareRegistry, Request, Response, Router};
use suprnova::{query, routes};

/// Answers every value of the `X-Team` and `X-Env` headers the request
/// carries, so a header sent twice shows as two values.
async fn echo_headers(request: Request) -> Response {
    let values = |name: &str| {
        let values: Vec<&str> = request
            .headers()
            .get_all(name)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .collect();
        if values.is_empty() {
            "none".to_string()
        } else {
            values.join(",")
        }
    };
    Ok(HttpResponse::text(format!(
        "team={} env={}",
        values("x-team"),
        values("x-env")
    )))
}

/// Answers the method, the content type, the accept header, the custom
/// `X-Team` header and the `q` field of the JSON body.
async fn search(request: Request) -> Response {
    let method = request.method().to_string();
    let content_type = request.header("content-type").unwrap_or("none").to_owned();
    let accept = request.header("accept").unwrap_or("none").to_owned();
    let team = request.header("x-team").unwrap_or("none").to_owned();
    let body: Value = request.json().await?;
    Ok(HttpResponse::json(json!({
        "method": method,
        "content_type": content_type,
        "accept": accept,
        "team": team,
        "q": body["q"],
    })))
}

/// A client over `/echo` for every method the tests send.
fn echo_client() -> TestClient {
    let router = Router::new()
        .get("/echo", echo_headers)
        .post("/echo", echo_headers)
        .delete("/echo", echo_headers);
    TestClient::new(router, MiddlewareRegistry::new())
}

/// A client whose `/search` route is registered with `query!` and so
/// answers `QUERY` only.
fn search_client() -> TestClient {
    routes! {
        query!("/search", search),
    }
    TestClient::new(register(), MiddlewareRegistry::new())
}

// ── PAR-174: default headers ────────────────────────────────────────────

#[tokio::test]
async fn a_default_header_is_sent_on_every_request_the_client_builds() {
    let client = echo_client().with_header("X-Team", "a");

    client
        .get("/echo")
        .send()
        .await
        .assert_ok()
        .assert_see("team=a ");
    client
        .post("/echo")
        .send()
        .await
        .assert_ok()
        .assert_see("team=a ");
    client
        .delete("/echo")
        .send()
        .await
        .assert_ok()
        .assert_see("team=a ");
}

#[tokio::test]
async fn a_header_set_on_one_request_wins_over_the_clients() {
    let client = echo_client().with_header("X-Team", "a");

    // The request's own value replaces the default: it is not sent next to
    // it, whatever the case of the name.
    client
        .get("/echo")
        .header("X-Team", "b")
        .send()
        .await
        .assert_see("team=b ");
    client
        .get("/echo")
        .header("x-team", "b")
        .send()
        .await
        .assert_see("team=b ");

    // The override belongs to that request alone.
    client.get("/echo").send().await.assert_see("team=a ");
}

#[tokio::test]
async fn with_headers_sets_several_and_a_later_default_replaces_an_earlier_one() {
    let client = echo_client()
        .with_headers([("X-Team", "a"), ("X-Env", "test")])
        .with_header("x-team", "c");

    client
        .get("/echo")
        .send()
        .await
        .assert_see("team=c env=test");

    let from_strings = echo_client().with_headers(vec![("X-Env".to_string(), "ci".to_string())]);
    from_strings
        .get("/echo")
        .send()
        .await
        .assert_see("team=none env=ci");
}

#[tokio::test]
async fn flush_headers_clears_every_default() {
    let mut client = echo_client().with_headers([("X-Team", "a"), ("X-Env", "test")]);
    client.flush_headers();

    client
        .get("/echo")
        .send()
        .await
        .assert_see("team=none env=none");

    // A default set after the flush is sent again.
    let client = client.with_header("X-Team", "d");
    client
        .get("/echo")
        .send()
        .await
        .assert_see("team=d env=none");
}

#[tokio::test]
#[should_panic(expected = "GET /echo is not a valid request")]
async fn an_invalid_default_header_fails_the_request_naming_it() {
    let client = echo_client().with_header("X Team", "a");
    client.get("/echo").send().await;
}

// ── PAR-174: query_json ─────────────────────────────────────────────────

#[tokio::test]
async fn query_json_sends_a_query_request_with_the_json_body_and_headers() {
    let client = search_client();

    let response = client
        .query_json("/search", &json!({"q": "x"}))
        .send()
        .await;

    response.assert_ok().assert_json(json!({
        "method": "QUERY",
        "content_type": "application/json",
        "accept": "application/json",
        "q": "x",
    }));
}

#[tokio::test]
async fn query_json_carries_the_defaults_and_its_json_headers_win_over_them() {
    let client = search_client().with_headers([("X-Team", "a"), ("Accept", "text/html")]);

    client
        .query_json("/search", &json!({"q": "y"}))
        .send()
        .await
        .assert_ok()
        .assert_json(json!({
            "accept": "application/json",
            "team": "a",
            "q": "y",
        }));

    // A header set on the request still wins over the JSON accept header.
    client
        .query_json("/search", &json!({"q": "z"}))
        .header("Accept", "application/vnd.api+json")
        .send()
        .await
        .assert_json(json!({"accept": "application/vnd.api+json", "q": "z"}));
}

#[tokio::test]
async fn a_query_route_does_not_answer_another_method() {
    // `query_json` reaching the route above proves the method; this proves
    // the route would refuse a GET, so the check means something.
    let response = search_client().get("/search").send().await;
    assert_ne!(response.status(), 200, "{}", response.body_text());
}

// ── PAR-180: assert_no_content ──────────────────────────────────────────

/// The panic message of `check`, which must panic.
fn panic_message(check: impl FnOnce()) -> String {
    let payload = catch_unwind(AssertUnwindSafe(check)).expect_err("the assertion must fail");
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
        .unwrap_or_default()
}

#[tokio::test]
async fn assert_no_content_passes_an_empty_204_from_the_client() {
    let router = Router::new().delete("/posts/1", |_req: Request| async {
        Ok(HttpResponse::new().status(204))
    });
    let client = TestClient::new(router, MiddlewareRegistry::new());

    client.delete("/posts/1").send().await.assert_no_content();
}

#[test]
fn assert_no_content_passes_an_empty_204() {
    TestResponse::new(204, Vec::new(), "").assert_no_content();
}

#[test]
#[should_panic(expected = "assert_no_content()")]
fn assert_no_content_fails_an_empty_200() {
    TestResponse::new(200, Vec::new(), "").assert_no_content();
}

#[test]
fn assert_no_content_fails_a_204_that_carries_a_body() {
    let message = panic_message(|| {
        TestResponse::new(204, Vec::new(), "left over").assert_no_content();
    });
    assert!(message.contains("assert_no_content()"), "{message}");
    assert!(message.contains("left over"), "{message}");
}

#[test]
fn assert_no_content_status_passes_an_empty_response_of_that_status() {
    TestResponse::new(205, Vec::new(), "").assert_no_content_status(205);
}

#[test]
#[should_panic(expected = "assert_no_content_status(205)")]
fn assert_no_content_status_fails_another_status() {
    TestResponse::new(204, Vec::new(), "").assert_no_content_status(205);
}

#[test]
#[should_panic(expected = "assert_no_content_status(205)")]
fn assert_no_content_status_fails_a_body() {
    TestResponse::new(205, Vec::new(), "{}").assert_no_content_status(205);
}

#[tokio::test]
async fn an_assert_no_content_failure_prints_the_error_report() {
    let router = Router::new().delete("/posts/1", |_req: Request| async {
        let response: Response = Err(FrameworkError::domain("ledger closed", 503).into());
        response
    });
    let client = TestClient::new(router, MiddlewareRegistry::new());
    let response = client.delete("/posts/1").send().await;

    let message = panic_message(|| {
        response.assert_no_content();
    });
    assert!(message.contains("assert_no_content()"), "{message}");
    assert!(message.contains("error report"), "{message}");
    assert!(message.contains("ledger closed"), "{message}");

    let message = panic_message(|| {
        response.assert_no_content_status(205);
    });
    assert!(message.contains("error report"), "{message}");
    assert!(message.contains("ledger closed"), "{message}");
}
