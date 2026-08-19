//! Validation failure → `303` redirect-back on an Inertia visit.
//!
//! The Inertia client treats a response with no `X-Inertia` header as
//! non-Inertia (`inertia-3.6.1/packages/core/src/response.ts:68,173-175`)
//! and hands it to the error modal, so a `422` body never reaches
//! `form.errors`. These drive `handle_request` over a loopback socket
//! with real Inertia headers — the only way to exercise the middleware
//! chain and the session flash together.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;

use serde_json::json;
use suprnova::session::SessionData;
use suprnova::{
    FrameworkError, HttpResponse, InertiaConfig, InertiaResponse,
    InertiaValidationRedirectMiddleware, Middleware, MiddlewareRegistry, Next, Request, Response,
    Router, ValidationErrors, handle_request,
};

/// Test-only stand-in for `SessionMiddleware`: scopes a caller-supplied
/// session slot into the task-local for the whole chain, so the test can
/// seed flash data before the request and inspect it afterwards.
struct SeededSessionScope(Arc<Mutex<Option<SessionData>>>);

#[async_trait::async_trait]
impl Middleware for SeededSessionScope {
    async fn handle(&self, request: Request, next: Next) -> Response {
        suprnova::session::session_scope_for_test(self.0.clone(), next(request)).await
    }
}

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

/// Send a request; return `(status, lowercased headers, body)`.
async fn request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
) -> (u16, HashMap<String, String>, String) {
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(io)
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let mut builder = hyper::Request::builder()
        .method(method)
        .uri(path)
        .header("Host", "localhost")
        .header("Content-Length", "0");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let req = builder.body(Full::new(Bytes::new())).unwrap();

    let resp = tokio::time::timeout(Duration::from_secs(5), sender.send_request(req))
        .await
        .expect("send_request timeout")
        .expect("hyper send_request");

    let (parts, body) = resp.into_parts();
    let status = parts.status.as_u16();
    let header_map = parts
        .headers
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_lowercase(),
                v.to_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    let bytes = body.collect().await.unwrap().to_bytes();
    (
        status,
        header_map,
        String::from_utf8_lossy(&bytes).to_string(),
    )
}

type Slot = std::sync::Arc<std::sync::Mutex<Option<SessionData>>>;

/// A failure shaped exactly like a `FormRequest` extraction failure.
fn failing_validation() -> HttpResponse {
    let mut errs = ValidationErrors::new();
    errs.add("email", "The email field is required.");
    errs.add("email", "The email must be a valid address.");
    errs.add("password", "The password is too short.");
    HttpResponse::from(FrameworkError::validation_errors(errs))
}

fn router() -> Router {
    Router::new()
        .post("/register", |_req| async {
            let resp: Response = Err(failing_validation());
            resp
        })
        // A 422 that is not a validation failure: no `errors` object.
        .post("/teapot", |_req| async {
            let resp: Response = Err(HttpResponse::json(json!({ "message": "no" })).status(422));
            resp
        })
        // A Precognition dry-run: 422 WITH errors, read off this response.
        .post("/precog", |_req| async {
            let mut errs = ValidationErrors::new();
            errs.add("email", "Required.");
            let resp: Response = Err(HttpResponse::from(FrameworkError::PrecognitionFailure(
                errs,
            )));
            resp
        })
        .get("/register", |req: Request| async move {
            InertiaResponse::new("auth/Register")
                .resolve(&req)
                .await
                .map_err(HttpResponse::from)
        })
        .get("/register-all", |req: Request| async move {
            InertiaResponse::new("auth/Register")
                .with_config(InertiaConfig::new().with_all_errors(true))
                .resolve(&req)
                .await
                .map_err(HttpResponse::from)
        })
        .into()
}

/// A session that already knows where the user came from.
fn seeded_slot() -> Slot {
    let slot = suprnova::session::new_session_slot_for_test();
    slot.lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .put("_previous.url", "/register");
    slot
}

fn stack(slot: &Slot) -> MiddlewareRegistry {
    MiddlewareRegistry::new()
        .append(SeededSessionScope(slot.clone()))
        .append(InertiaValidationRedirectMiddleware::new())
}

/// `SessionMiddleware` ages `_flash.new.*` → `_flash.old.*` at the start
/// of the next request; the test scope does not, so do it by hand.
fn age(slot: &Slot) {
    slot.lock().unwrap().as_mut().unwrap().age_flash_data();
}

const INERTIA_POST: &[(&str, &str)] = &[
    ("X-Inertia", "true"),
    ("Referer", "http://localhost/register"),
];

#[tokio::test]
async fn a_failed_inertia_form_bounces_303_and_the_destination_carries_the_errors() {
    let slot = seeded_slot();
    let addr = spawn_server(router(), stack(&slot), 4).await;

    let (status, headers, body) = request(addr, "POST", "/register", INERTIA_POST).await;
    assert_eq!(
        status, 303,
        "a 422 the client would modal must become a redirect"
    );
    assert_eq!(
        headers.get("location").map(String::as_str),
        Some("/register")
    );
    assert!(body.is_empty(), "the 422 body must not survive; got {body}");
    {
        let guard = slot.lock().unwrap();
        let bag: serde_json::Value = guard
            .as_ref()
            .unwrap()
            .get("_flash.new.errors.default")
            .expect("errors must be flashed for the follow-up GET");
        assert_eq!(bag["email"][0], "The email field is required.");
    }

    age(&slot);
    let (status, _h, body) = request(addr, "GET", "/register", &[("X-Inertia", "true")]).await;
    assert_eq!(status, 200);
    let page: serde_json::Value = serde_json::from_str(&body).expect("page object");
    assert_eq!(
        page["props"]["errors"]["email"], "The email field is required.",
        "errors arrive as plain first-message strings; got {body}"
    );
    assert_eq!(
        page["props"]["errors"]["password"],
        "The password is too short."
    );
}

#[tokio::test]
async fn with_all_errors_keeps_every_message_as_an_array() {
    let slot = seeded_slot();
    let addr = spawn_server(router(), stack(&slot), 4).await;

    request(addr, "POST", "/register", INERTIA_POST).await;
    age(&slot);
    let (status, _h, body) = request(addr, "GET", "/register-all", &[("X-Inertia", "true")]).await;

    assert_eq!(status, 200);
    let page: serde_json::Value = serde_json::from_str(&body).expect("page object");
    assert_eq!(
        page["props"]["errors"]["email"][0],
        "The email field is required."
    );
    assert_eq!(
        page["props"]["errors"]["email"][1],
        "The email must be a valid address."
    );
}

#[tokio::test]
async fn the_error_bag_header_scopes_the_flashed_bag() {
    let slot = seeded_slot();
    let addr = spawn_server(router(), stack(&slot), 4).await;

    let mut headers = INERTIA_POST.to_vec();
    headers.push(("X-Inertia-Error-Bag", "registration"));
    request(addr, "POST", "/register", &headers).await;
    {
        let guard = slot.lock().unwrap();
        let session = guard.as_ref().unwrap();
        assert!(
            session.has("_flash.new.errors.registration"),
            "the bag named by the request header owns the flash"
        );
        assert!(!session.has("_flash.new.errors.default"));
    }

    age(&slot);
    // The browser replays the header across the same-origin redirect, so
    // the render reads the same bag back and nests it under the name.
    let (_s, _h, body) = request(
        addr,
        "GET",
        "/register",
        &[
            ("X-Inertia", "true"),
            ("X-Inertia-Error-Bag", "registration"),
        ],
    )
    .await;
    let page: serde_json::Value = serde_json::from_str(&body).expect("page object");
    assert_eq!(
        page["props"]["errors"]["registration"]["email"],
        "The email field is required."
    );
}

#[tokio::test]
async fn a_plain_browser_post_still_gets_the_422_json() {
    let slot = seeded_slot();
    let addr = spawn_server(router(), stack(&slot), 2).await;

    let (status, _h, body) = request(addr, "POST", "/register", &[]).await;

    assert_eq!(status, 422, "a REST client keeps its machine-readable body");
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("422 JSON body");
    assert_eq!(parsed["errors"]["email"][0], "The email field is required.");
}

#[tokio::test]
async fn the_two_422s_that_must_never_be_bridged() {
    // A 422 with no `errors` object has no form to bounce to, and a
    // Precognition dry-run's whole contract is that the client reads
    // these very errors off this response.
    let slot = seeded_slot();
    let addr = spawn_server(router(), stack(&slot), 4).await;

    let (status, _h, body) = request(addr, "POST", "/teapot", &[("X-Inertia", "true")]).await;
    assert_eq!(status, 422, "the gate is body shape, not status alone");
    assert!(body.contains("\"no\""), "got {body}");

    let (status, headers, body) = request(
        addr,
        "POST",
        "/precog",
        &[("X-Inertia", "true"), ("Precognition", "true")],
    )
    .await;
    assert_eq!(status, 422);
    assert_eq!(
        headers.get("precognition").map(String::as_str),
        Some("true")
    );
    assert!(body.contains("Required."), "got {body}");
}
