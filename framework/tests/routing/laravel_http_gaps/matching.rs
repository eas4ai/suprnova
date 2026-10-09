//! PAR-115: a request whose path a route of another method matches gets
//! Laravel's `405` with `Allow`, or for `OPTIONS` a `200` with `Allow`,
//! through the global middleware; and `route_has` answers whether every
//! name it is given is registered, as Laravel's `Route::has` does.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use suprnova::http::text;
use suprnova::routing::register_route_name;
use suprnova::{
    CorsConfig, CorsMiddleware, Middleware, MiddlewareRegistry, Next, Request, Response, Router,
    fallback, route_has,
};

use crate::http_wire::request;
use crate::laravel_delta::{server, server_with};

async fn ok(_req: Request) -> Response {
    text("ok")
}

/// The message of a `405` body.
fn message(body: &str) -> String {
    let json: serde_json::Value = serde_json::from_str(body).expect("a JSON error body");
    json["message"].as_str().expect("a message").to_owned()
}

#[tokio::test]
async fn delete_on_a_get_route_answers_405_with_allow_and_laravels_message() {
    let addr = server(Router::new().get("/posts/{id}", ok).into()).await;

    let (status, headers, body) = request(addr, "DELETE", "/posts/1", &[]).await;
    assert_eq!(status, 405, "{body}");
    assert_eq!(headers.get("allow").map(String::as_str), Some("GET, HEAD"));
    assert_eq!(
        message(&body),
        "The DELETE method is not supported for route posts/1. Supported methods: GET, HEAD."
    );
    let json: serde_json::Value = serde_json::from_str(&body).expect("JSON");
    assert!(
        json["request_id"].is_string(),
        "the usual error body carries the request id: {body}"
    );
    assert_eq!(
        headers.get("x-request-id").map(String::as_str),
        json["request_id"].as_str()
    );
}

#[tokio::test]
async fn options_on_such_a_path_answers_200_with_an_empty_body_and_a_compact_allow() {
    let addr = server(Router::new().get("/posts/{id}", ok).into()).await;

    let (status, headers, body) = request(addr, "OPTIONS", "/posts/1", &[]).await;
    assert_eq!(status, 200);
    assert_eq!(body, "");
    assert_eq!(headers.get("allow").map(String::as_str), Some("GET,HEAD"));
}

#[tokio::test]
async fn a_cors_preflight_to_that_path_keeps_the_cors_answer() {
    let registry = MiddlewareRegistry::new().append(CorsMiddleware::new(
        CorsConfig::allow_origins(["https://app.example"]).max_age(Duration::from_secs(600)),
    ));
    let addr = server_with(Router::new().get("/posts/{id}", ok).into(), registry).await;

    let (status, headers, _) = request(
        addr,
        "OPTIONS",
        "/posts/1",
        &[
            ("Origin", "https://app.example"),
            ("Access-Control-Request-Method", "DELETE"),
        ],
    )
    .await;
    assert_eq!(status, 204, "the CORS middleware answers the preflight");
    assert_eq!(
        headers
            .get("access-control-allow-origin")
            .map(String::as_str),
        Some("https://app.example")
    );
    assert!(
        !headers.contains_key("allow"),
        "the router's answer never ran"
    );
}

#[tokio::test]
async fn a_path_no_route_matches_keeps_the_404_or_the_fallback() {
    let addr = server(Router::new().get("/posts/{id}", ok).into()).await;
    let (status, headers, body) = request(addr, "DELETE", "/nothing/here", &[]).await;
    assert_eq!(status, 404);
    assert_eq!(body, "404 Not Found");
    assert!(!headers.contains_key("allow"));

    let router = fallback!(|_req: Request| async { text("fallback") })
        .register(Router::new().get("/posts/{id}", ok).into());
    let addr = server(router).await;
    let (status, _, body) = request(addr, "POST", "/nothing/here", &[]).await;
    assert_eq!((status, body.as_str()), (200, "fallback"));
    let (status, _, body) = request(addr, "OPTIONS", "/nothing/here", &[]).await;
    assert_eq!((status, body.as_str()), (200, "fallback"));
    // A path a route of another method matches is not the fallback's.
    let (status, headers, _) = request(addr, "POST", "/posts/1", &[]).await;
    assert_eq!(status, 405);
    assert_eq!(headers.get("allow").map(String::as_str), Some("GET, HEAD"));
}

#[tokio::test]
async fn a_path_every_routes_constraints_refuse_answers_404() {
    let router: Router = Router::new()
        .get("/posts/{id}", ok)
        .where_number("id")
        .delete("/posts/{id}", ok)
        .where_number("id")
        .into();
    let addr = server(router).await;

    for method in ["GET", "PUT", "DELETE", "OPTIONS"] {
        let (status, headers, _) = request(addr, method, "/posts/abc", &[]).await;
        assert_eq!(status, 404, "{method}: every route refuses `abc`");
        assert!(!headers.contains_key("allow"));
    }
    // The constraint counts as it does for a match: a value it takes
    // lists the route.
    let (status, headers, _) = request(addr, "PUT", "/posts/7", &[]).await;
    assert_eq!(status, 405);
    assert_eq!(
        headers.get("allow").map(String::as_str),
        Some("GET, HEAD, DELETE")
    );
}

#[tokio::test]
async fn allow_lists_the_other_methods_in_laravels_order() {
    let router: Router = Router::new()
        .query("/things", ok)
        .delete("/things", ok)
        .post("/things", ok)
        .get("/things", ok)
        .head("/probe", ok)
        .post("/submit", ok)
        .into();
    let addr = server(router).await;

    let (status, headers, body) = request(addr, "PUT", "/things", &[]).await;
    assert_eq!(status, 405);
    assert_eq!(
        headers.get("allow").map(String::as_str),
        Some("GET, HEAD, POST, DELETE, QUERY")
    );
    assert_eq!(
        message(&body),
        "The PUT method is not supported for route things. Supported methods: GET, HEAD, POST, \
         DELETE, QUERY."
    );

    // An explicit HEAD route is listed without a GET beside it.
    let (status, headers, _) = request(addr, "GET", "/probe", &[]).await;
    assert_eq!(status, 405);
    assert_eq!(headers.get("allow").map(String::as_str), Some("HEAD"));

    // A HEAD request is left out of its own list and gets no body.
    let (status, headers, body) = request(addr, "HEAD", "/submit", &[]).await;
    assert_eq!(status, 405);
    assert_eq!(headers.get("allow").map(String::as_str), Some("POST"));
    assert_eq!(body, "");

    // A method the router keeps no routes for is refused the same way.
    let (status, headers, body) = request(addr, "PROPFIND", "/submit", &[]).await;
    assert_eq!(status, 405);
    assert_eq!(headers.get("allow").map(String::as_str), Some("POST"));
    assert_eq!(
        message(&body),
        "The PROPFIND method is not supported for route submit. Supported methods: POST."
    );
}

#[tokio::test]
async fn the_root_path_is_named_by_a_slash() {
    let addr = server(Router::new().get("/", ok).into()).await;

    let (status, _, body) = request(addr, "POST", "/", &[]).await;
    assert_eq!(status, 405);
    assert_eq!(
        message(&body),
        "The POST method is not supported for route /. Supported methods: GET, HEAD."
    );
}

/// A global middleware that marks the response, to show the chain ran.
struct Marker(Arc<Mutex<Vec<String>>>);

#[suprnova::async_trait]
impl Middleware for Marker {
    async fn handle(&self, request: Request, next: Next) -> Response {
        self.0.lock().unwrap().push(request.method().to_string());
        next(request).await
    }
}

#[tokio::test]
async fn both_answers_run_through_the_global_middleware() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let registry = MiddlewareRegistry::new().append(Marker(seen.clone()));
    let addr = server_with(Router::new().get("/posts/{id}", ok).into(), registry).await;

    assert_eq!(request(addr, "DELETE", "/posts/1", &[]).await.0, 405);
    assert_eq!(request(addr, "OPTIONS", "/posts/1", &[]).await.0, 200);
    assert_eq!(*seen.lock().unwrap(), ["DELETE", "OPTIONS"]);
}

#[test]
fn route_has_is_true_only_when_every_name_is_registered() {
    register_route_name("h2.matching.home", "/h2-matching");
    register_route_name("h2.matching.about", "/h2-matching/about");

    assert!(route_has(&["h2.matching.home"]));
    assert!(route_has(&["h2.matching.home", "h2.matching.about"]));
    assert!(!route_has(&["h2.matching.home", "h2.matching.missing"]));
    assert!(!route_has(&["h2.matching.missing"]));
    // Laravel's check over no names is true.
    assert!(route_has(&[]));
}
