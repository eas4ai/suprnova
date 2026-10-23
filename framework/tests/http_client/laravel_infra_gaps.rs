//! The Laravel infrastructure gaps of the HTTP client: `head`, global
//! middleware and options, base URLs, query and URL parameters, multipart
//! uploads and a per-request connect timeout (PAR-157), and the fake's
//! recorded headers, URL stubs, callbacks, sequences and stray-request
//! controls (PAR-158).
//!
//! Tests that touch the real network or the process-wide stray-request
//! switch hold `NETWORK_LOCK`, the lock `http_client.rs` holds for the
//! same reason. Process-wide global middleware lives for the rest of the
//! process, so every process-wide function registered here acts only on
//! requests to this module's own marker path, and leaves every other
//! request alone.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use suprnova::{
    ClientResponse, FakeResponse, Http, RecordedRequest, RequestBuilder, assert_not_sent,
    assert_sent, fake_response,
};

use crate::http_client::NETWORK_LOCK;

/// The path every process-wide global function of this module acts on.
const MARKER: &str = "/laravel-infra-gaps-global";

/// What the echo server saw.
#[derive(Debug, Clone)]
struct EchoCapture {
    method: String,
    uri: String,
    content_type: Option<String>,
    x_app: Option<String>,
    x_option: Vec<String>,
    body: String,
}

/// One-shot echo server, as `http_client.rs` has it, keeping `x-app` and
/// every `x-option` value too, and answering with an
/// `x-laravel-infra-gaps` header a response middleware can recognise.
async fn spawn_echo() -> (SocketAddr, Arc<Mutex<Option<EchoCapture>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let captured: Arc<Mutex<Option<EchoCapture>>> = Arc::new(Mutex::new(None));
    let cap_for_task = captured.clone();
    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let io = TokioIo::new(stream);
            let captured = cap_for_task.clone();
            let svc = service_fn(move |req: hyper::Request<Incoming>| {
                let captured = captured.clone();
                async move {
                    let header = |name: &str| {
                        req.headers()
                            .get(name)
                            .and_then(|h| h.to_str().ok())
                            .map(str::to_string)
                    };
                    let method = req.method().to_string();
                    let uri = req.uri().to_string();
                    let content_type = header("content-type");
                    let x_app = header("x-app");
                    let x_option = req
                        .headers()
                        .get_all("x-option")
                        .iter()
                        .filter_map(|value| value.to_str().ok().map(str::to_string))
                        .collect();
                    let body_bytes = req.into_body().collect().await.unwrap().to_bytes();
                    *captured.lock().unwrap() = Some(EchoCapture {
                        method,
                        uri,
                        content_type,
                        x_app,
                        x_option,
                        body: String::from_utf8_lossy(&body_bytes).to_string(),
                    });
                    Ok::<_, Infallible>(
                        hyper::Response::builder()
                            .status(200)
                            .header("content-type", "application/json")
                            .header("x-laravel-infra-gaps", "echo")
                            .body(Full::new(Bytes::from_static(b"{}")))
                            .unwrap(),
                    )
                }
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, svc)
                .await;
        }
    });
    (addr, captured)
}

/// What the echo server captured, once its task has published it.
async fn captured(capture: &Arc<Mutex<Option<EchoCapture>>>) -> Option<EchoCapture> {
    for _ in 0..50 {
        if let Some(seen) = capture.lock().unwrap().clone() {
            return Some(seen);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    None
}

/// Turns the stray-request refusal off again when a test ends, panicked
/// or not, so no other test inherits it.
struct StrayRefusal;

impl StrayRefusal {
    fn on() -> Self {
        Http::prevent_stray_requests(true);
        Self
    }
}

impl Drop for StrayRefusal {
    fn drop(&mut self) {
        Http::prevent_stray_requests(false);
    }
}

/// How many values of the header `name` the request was recorded with.
fn values_of(request: &RecordedRequest, name: &str) -> usize {
    request
        .headers
        .iter()
        .filter(|(header, _)| header.eq_ignore_ascii_case(name))
        .count()
}

// ───────────────────────── PAR-157 ─────────────────────────

#[tokio::test]
async fn head_sends_head() {
    let _net = NETWORK_LOCK.lock().await;
    let (addr, capture) = spawn_echo().await;

    let response = Http::head(format!("http://{addr}/probe"))
        .send()
        .await
        .expect("send");

    assert_eq!(response.status(), 200);
    assert_eq!(captured(&capture).await.expect("captured").method, "HEAD");
}

/// The response middleware below records the statuses of the responses
/// the echo server of this module sent.
static ECHOED_STATUSES: Mutex<Vec<u16>> = Mutex::new(Vec::new());

fn register_process_wide_marker_configuration() {
    Http::global_request_middleware(|request: RequestBuilder| {
        if request.url().contains(MARKER) {
            request.header("X-App", "1")
        } else {
            request
        }
    });
    Http::global_response_middleware(|response: ClientResponse| {
        if response.header("x-laravel-infra-gaps").as_deref() == Some("echo") {
            ECHOED_STATUSES.lock().unwrap().push(response.status());
        }
        response
    });
    Http::global_options(|request: RequestBuilder| {
        if request.url().contains(MARKER) {
            request.header("X-Option", "global")
        } else {
            request
        }
    });
}

#[tokio::test]
async fn process_wide_global_configuration_reaches_the_wire_and_can_be_left_out() {
    let _net = NETWORK_LOCK.lock().await;
    register_process_wide_marker_configuration();
    ECHOED_STATUSES.lock().unwrap().clear();

    let (addr, capture) = spawn_echo().await;
    let response = Http::get(format!("http://{addr}{MARKER}"))
        .header("X-Option", "own")
        .send()
        .await
        .expect("send");
    assert_eq!(response.status(), 200);
    let seen = captured(&capture).await.expect("captured");
    assert_eq!(
        seen.x_app.as_deref(),
        Some("1"),
        "the request middleware ran"
    );
    assert_eq!(
        seen.x_option,
        vec!["global".to_owned(), "own".to_owned()],
        "the global options come before the request's own"
    );
    assert_eq!(
        *ECHOED_STATUSES.lock().unwrap(),
        vec![200],
        "the response middleware saw the echo server's response"
    );

    let (addr, capture) = spawn_echo().await;
    let response = Http::without_global_configuration(|| async move {
        Http::get(format!("http://{addr}{MARKER}")).send().await
    })
    .await
    .expect("send");
    assert_eq!(response.status(), 200);
    let seen = captured(&capture).await.expect("captured");
    assert_eq!(seen.x_app, None, "no request middleware inside");
    assert!(seen.x_option.is_empty(), "no global options inside");
    assert_eq!(
        *ECHOED_STATUSES.lock().unwrap(),
        vec![200],
        "no response middleware inside"
    );
}

#[tokio::test]
async fn global_configuration_inside_a_fake_belongs_to_that_fake() {
    Http::fake(|| async {
        Http::global_request_middleware(|request: RequestBuilder| request.header("X-App", "1"));
        Http::post("https://api.test/orders")
            .json(&serde_json::json!({"id": 1}))
            .send()
            .await
            .expect("send");
        assert_sent(|request| request.has_header("x-app", "1"));
    })
    .await;

    // A later fake does not see the middleware of an earlier one.
    Http::fake(|| async {
        Http::get("https://api.test/orders")
            .send()
            .await
            .expect("send");
        assert_sent(|request| request.url.ends_with("/orders"));
        assert_not_sent(|request| request.header("X-App").is_some());
    })
    .await;

    // Two fakes running at once each see only their own.
    let first = Http::fake(|| async {
        Http::global_request_middleware(|request: RequestBuilder| request.header("X-App", "first"));
        tokio::task::yield_now().await;
        Http::get("https://api.test/a").send().await.expect("send");
        assert_sent(|request| {
            request.has_header("x-app", "first") && values_of(request, "x-app") == 1
        });
    });
    let second = Http::fake(|| async {
        Http::global_request_middleware(|request: RequestBuilder| {
            request.header("X-App", "second")
        });
        tokio::task::yield_now().await;
        Http::get("https://api.test/b").send().await.expect("send");
        assert_sent(|request| {
            request.has_header("x-app", "second") && values_of(request, "x-app") == 1
        });
    });
    tokio::join!(first, second);
}

#[tokio::test]
async fn global_response_middleware_sees_a_faked_response_and_can_be_left_out() {
    Http::fake(|| async {
        let seen: Arc<Mutex<Vec<u16>>> = Arc::default();
        let record = Arc::clone(&seen);
        Http::global_response_middleware(move |response: ClientResponse| {
            record.lock().unwrap().push(response.status());
            response
        });
        fake_response("GET", "/teapot", 418, serde_json::json!({}));
        Http::get("https://api.test/teapot")
            .send()
            .await
            .expect("send");
        assert_eq!(*seen.lock().unwrap(), vec![418]);

        Http::without_global_configuration(|| async {
            Http::get("https://api.test/other")
                .send()
                .await
                .expect("send");
        })
        .await;
        assert_eq!(*seen.lock().unwrap(), vec![418], "left out inside");
    })
    .await;
}

#[tokio::test]
async fn base_url_and_query_build_the_recorded_url() {
    Http::fake(|| async {
        Http::get("users")
            .base_url("https://api.test/base")
            .query(&[("page", "2")])
            .send()
            .await
            .expect("send");
        Http::get("/users/")
            .base_url("https://api.test/base/")
            .send()
            .await
            .expect("send");
        Http::get("https://other.test/x?a=1&page=1")
            .base_url("https://api.test/base")
            .query(&[("page", "2"), ("q", "a b")])
            .send()
            .await
            .expect("send");

        assert_sent(|request| request.url == "https://api.test/base/users?page=2");
        assert_sent(|request| request.url == "https://api.test/base/users/");
        assert_sent(|request| request.url == "https://other.test/x?a=1&page=2&q=a+b");
    })
    .await;
}

#[tokio::test]
async fn url_parameters_are_percent_encoded_into_one_segment() {
    Http::fake(|| async {
        Http::get("https://a.test/users/{id}")
            .url_parameters([("id", "a/b")])
            .send()
            .await
            .expect("send");
        Http::get("https://a.test/{section}/{id}/{unknown}")
            .url_parameters([("section", "x?admin=1#top"), ("id", "@evil.test:80")])
            .send()
            .await
            .expect("send");

        assert_sent(|request| request.url == "https://a.test/users/a%2Fb");
        assert_sent(|request| {
            request.url == "https://a.test/x%3Fadmin%3D1%23top/%40evil.test%3A80/{unknown}"
        });
    })
    .await;
}

#[tokio::test]
async fn a_query_on_a_url_that_does_not_parse_fails() {
    Http::fake(|| async {
        let error = Http::get("users")
            .query(&[("page", "2")])
            .send()
            .await
            .err()
            .expect("a relative URL without a base URL takes no query");
        assert!(error.to_string().contains("users"), "{error}");
    })
    .await;
}

#[tokio::test]
async fn attach_sends_a_multipart_body() {
    let _net = NETWORK_LOCK.lock().await;
    let (addr, capture) = spawn_echo().await;

    Http::post(format!("http://{addr}/upload"))
        .attach("doc", b"x", Some("a.txt"))
        .attach("note", "hello", None)
        .send()
        .await
        .expect("send");

    let seen = captured(&capture).await.expect("captured");
    let content_type = seen.content_type.expect("a content type");
    assert!(
        content_type.starts_with("multipart/form-data; boundary="),
        "{content_type}"
    );
    assert!(
        seen.body.contains(
            "Content-Disposition: form-data; name=\"doc\"; filename=\"a.txt\"\r\n\r\nx\r\n"
        ),
        "{}",
        seen.body
    );
    assert!(
        seen.body
            .contains("Content-Disposition: form-data; name=\"note\"\r\n\r\nhello\r\n"),
        "{}",
        seen.body
    );
}

#[tokio::test]
async fn a_connect_timeout_applies_to_its_own_request() {
    let _net = NETWORK_LOCK.lock().await;
    let (addr, capture) = spawn_echo().await;
    let response = Http::get(format!("http://{addr}/quick"))
        .connect_timeout(Duration::from_secs(2))
        .send()
        .await
        .expect("a reachable host connects within its connect timeout");
    assert_eq!(response.status(), 200);
    assert!(captured(&capture).await.is_some());

    // 10.255.255.1 is not routed: it either drops the connection attempt,
    // which the 300 ms connect timeout ends, or fails at once. Either way
    // the request does not wait for the default 10 s connect timeout.
    let started = std::time::Instant::now();
    Http::get("http://10.255.255.1:81/")
        .connect_timeout(Duration::from_millis(300))
        .send()
        .await
        .err()
        .expect("nothing answers");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the request's own connect timeout ended it, took {:?}",
        started.elapsed()
    );
}

// ───────────────────────── PAR-158 ─────────────────────────

#[tokio::test]
async fn a_recorded_request_carries_the_headers_it_is_sent_with() {
    Http::fake(|| async {
        Http::post("https://api.test/json")
            .json(&serde_json::json!({"name": "Ada"}))
            .send()
            .await
            .expect("send");
        Http::post("https://api.test/form")
            .form(&serde_json::json!({"name": "Ada"}))
            .send()
            .await
            .expect("send");
        Http::post("https://api.test/multipart")
            .attach("doc", b"x", Some("a.txt"))
            .send()
            .await
            .expect("send");
        Http::post("https://api.test/own")
            .header("Content-Type", "application/vnd.api+json")
            .json(&serde_json::json!({}))
            .send()
            .await
            .expect("send");

        assert_sent(|request| {
            request.url.ends_with("/json")
                && request.has_header("Content-Type", "application/json")
                && request.header("CONTENT-TYPE") == Some("application/json")
                && request.is_json()
                && !request.is_form()
                && !request.is_multipart()
        });
        assert_sent(|request| {
            request.url.ends_with("/form")
                && request.has_header("content-type", "application/x-www-form-urlencoded")
                && request.is_form()
                && !request.is_json()
        });
        assert_sent(|request| {
            request.url.ends_with("/multipart")
                && request.is_multipart()
                && request.body.as_deref().is_some_and(|body| {
                    String::from_utf8_lossy(body).contains("filename=\"a.txt\"")
                })
        });
        assert_sent(|request| {
            request.url.ends_with("/own")
                && request.header("content-type") == Some("application/vnd.api+json")
                && values_of(request, "content-type") == 1
                && request.is_json()
        });
    })
    .await;
}

#[tokio::test]
async fn fake_url_answers_every_matching_request_and_stays() {
    Http::fake(|| async {
        Http::fake_url(
            "a.test/users/*",
            FakeResponse::json(200, serde_json::json!({"id": 7})),
        );

        for _ in 0..2 {
            let response = Http::get("https://a.test/users/7")
                .send()
                .await
                .expect("send");
            assert_eq!(response.status(), 200);
            let body: serde_json::Value = response.json().await.expect("json");
            assert_eq!(body["id"], 7, "the stub answers, and is not used up");
        }

        let other = Http::get("https://a.test/teams/1")
            .send()
            .await
            .expect("send");
        let body: serde_json::Value = other.json().await.expect("json");
        assert_eq!(body, serde_json::json!({}), "a URL the pattern misses");
    })
    .await;
}

#[tokio::test]
async fn fake_using_none_falls_through_to_the_other_stubs() {
    Http::fake(|| async {
        let asked: Arc<Mutex<Vec<String>>> = Arc::default();
        let record = Arc::clone(&asked);
        Http::fake_using(move |request| {
            record.lock().unwrap().push(request.url.clone());
            request
                .url
                .contains("/mine")
                .then(|| FakeResponse::text(202, "mine"))
        });
        Http::fake_url("*", FakeResponse::new(204));

        let mine = Http::get("https://api.test/mine")
            .send()
            .await
            .expect("send");
        assert_eq!(mine.status(), 202);
        assert_eq!(mine.text().await.expect("text"), "mine");
        let other = Http::get("https://api.test/other")
            .send()
            .await
            .expect("send");
        assert_eq!(other.status(), 204, "None let the URL stub answer");
        assert_eq!(asked.lock().unwrap().len(), 2);
    })
    .await;
}

#[tokio::test]
async fn fake_sequence_answers_in_turn_then_when_empty() {
    Http::fake(|| async {
        Http::fake_sequence("api.test/jobs*")
            .push(FakeResponse::new(201))
            .push_status(202)
            .when_empty(FakeResponse::new(204));

        let mut statuses = Vec::new();
        for _ in 0..4 {
            statuses.push(
                Http::get("https://api.test/jobs")
                    .send()
                    .await
                    .expect("send")
                    .status(),
            );
        }
        assert_eq!(statuses, vec![201, 202, 204, 204]);
    })
    .await;

    Http::fake(|| async {
        let sequence = Http::fake_sequence("*").push_status(201);
        Http::get("https://api.test/one")
            .send()
            .await
            .expect("send");
        assert!(sequence.is_empty());
        let error = Http::get("https://api.test/two")
            .send()
            .await
            .err()
            .expect("an exhausted sequence without a when_empty response fails");
        assert!(error.to_string().contains("sequence"), "{error}");
    })
    .await;

    Http::fake(|| async {
        Http::fake_sequence("*").dont_fail_when_empty();
        let response = Http::get("https://api.test/one")
            .send()
            .await
            .expect("send");
        assert_eq!(response.status(), 200);
    })
    .await;
}

#[tokio::test]
async fn fake_response_stays_use_once_ahead_of_the_stubs() {
    Http::fake(|| async {
        fake_response("GET", "/once", 201, serde_json::json!({}));
        Http::fake_url("*", FakeResponse::new(209));
        let first = Http::get("https://api.test/once")
            .send()
            .await
            .expect("send");
        let second = Http::get("https://api.test/once")
            .send()
            .await
            .expect("send");
        assert_eq!((first.status(), second.status()), (201, 209));
    })
    .await;
}

#[tokio::test]
async fn prevent_stray_requests_moves_the_fail_on_real_calls_switch() {
    let _net = NETWORK_LOCK.lock().await;
    assert!(!Http::preventing_stray_requests());

    Http::prevent_stray_requests(true);
    assert!(Http::preventing_stray_requests());
    assert!(
        Http::is_guarded(),
        "the same switch fail_on_real_calls moves"
    );
    let error = Http::get("http://127.0.0.1:9/stray")
        .send()
        .await
        .err()
        .expect("a stray request is refused");
    assert!(error.to_string().contains("127.0.0.1:9/stray"), "{error}");

    Http::prevent_stray_requests(false);
    assert!(!Http::preventing_stray_requests());

    Http::fail_on_real_calls();
    assert!(Http::preventing_stray_requests());
    Http::allow_real_calls();
    assert!(!Http::preventing_stray_requests());
}

#[tokio::test]
async fn allowed_stray_requests_reach_the_network_inside_a_fake() {
    let _net = NETWORK_LOCK.lock().await;
    let _refusal = StrayRefusal::on();
    let (addr, capture) = spawn_echo().await;

    Http::fake(|| async move {
        Http::allow_stray_requests(&["http://127.0.0.1:*"]);
        Http::global_request_middleware(|request: RequestBuilder| request.header("X-App", "1"));

        let response = Http::get(format!("http://{addr}/allowed"))
            .send()
            .await
            .expect("an allowed stray request is sent");
        assert_eq!(response.status(), 200);
        assert_eq!(
            response.header("x-laravel-infra-gaps").as_deref(),
            Some("echo"),
            "the answer came from the echo server"
        );

        let refused = Http::get("https://not-allowed.test/x")
            .send()
            .await
            .err()
            .expect("a stray request outside the allowlist is refused");
        assert!(
            refused.to_string().contains("not-allowed.test"),
            "{refused}"
        );
        assert_sent(|request| {
            request.url.ends_with("/allowed") && request.has_header("x-app", "1")
        });
    })
    .await;

    let seen = captured(&capture)
        .await
        .expect("the request reached the echo server");
    assert_eq!(seen.uri, "/allowed");
    assert_eq!(seen.x_app.as_deref(), Some("1"));
}
