//! Optional route parameters and parameter constraints.
//!
//! An optional parameter matches with and without its segment, and a
//! constraint turns a value it refuses into a 404 before the handler runs.
//! Every test drives real requests through `handle_request`.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use suprnova::http::text;
use suprnova::routing::{RouteUrlError, try_route};
use suprnova::{
    Middleware, MiddlewareRegistry, Next, ParamConstraint, Request, Response, Router, any, get,
    group, handle_request, route,
};

/// Answers with the parameters it was given, so a test reads what the
/// handler saw: `id=42`, `year=2026 month=05`, or `-` for none.
async fn echo(request: Request) -> Response {
    let mut seen: Vec<String> = ["id", "year", "month", "team", "locale"]
        .iter()
        .filter_map(|name| {
            request
                .param(name)
                .ok()
                .map(|value| format!("{name}={value}"))
        })
        .collect();
    if seen.is_empty() {
        seen.push("-".to_owned());
    }
    text(seen.join(" "))
}

/// A server for `router` that serves `accepts` connections.
async fn serve(router: impl Into<Router>, accepts: usize) -> SocketAddr {
    let router = Arc::new(router.into());
    let registry = Arc::new(MiddlewareRegistry::new());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind an ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        for _ in 0..accepts {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let router = router.clone();
            let registry = registry.clone();
            tokio::spawn(async move {
                let service = service_fn(move |request: hyper::Request<Incoming>| {
                    let router = router.clone();
                    let registry = registry.clone();
                    async move { Ok::<_, Infallible>(handle_request(router, registry, request).await) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    addr
}

async fn send(addr: SocketAddr, method: &str, path: &str) -> (u16, String) {
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(stream))
            .await
            .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let request = hyper::Request::builder()
        .method(method)
        .uri(path)
        .header("Host", "localhost")
        .header("Content-Length", "0")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(5), sender.send_request(request))
        .await
        .expect("the request timed out")
        .expect("the request failed");
    let status = response.status().as_u16();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).into_owned())
}

async fn get_path(addr: SocketAddr, path: &str) -> (u16, String) {
    send(addr, "GET", path).await
}

fn ok(body: &str) -> (u16, String) {
    (200, body.to_owned())
}

// --- Optional parameters -----------------------------------------------------

#[tokio::test]
async fn an_optional_parameter_matches_with_and_without_its_segment() {
    let router = Router::new().get("/posts/{id?}", echo);
    let addr = serve(router, 3).await;

    assert_eq!(get_path(addr, "/posts").await, ok("-"));
    assert_eq!(get_path(addr, "/posts/42").await, ok("id=42"));
    assert_eq!(
        get_path(addr, "/posts/42/extra").await.0,
        404,
        "the route has one segment to give, not two"
    );
}

#[tokio::test]
async fn the_colon_spelling_takes_the_question_mark_too() {
    let router = Router::new().get("/posts/:id?", echo);
    let addr = serve(router, 2).await;

    assert_eq!(get_path(addr, "/posts").await, ok("-"));
    assert_eq!(get_path(addr, "/posts/7").await, ok("id=7"));
}

#[tokio::test]
async fn several_optional_parameters_fill_from_the_left() {
    let router = Router::new().get("/archive/{year?}/{month?}", echo);
    let addr = serve(router, 3).await;

    assert_eq!(get_path(addr, "/archive").await, ok("-"));
    assert_eq!(get_path(addr, "/archive/2026").await, ok("year=2026"));
    assert_eq!(
        get_path(addr, "/archive/2026/05").await,
        ok("year=2026 month=05")
    );
}

#[tokio::test]
async fn an_optional_parameter_at_the_root_matches_the_root() {
    let router = Router::new().get("/{locale?}", echo);
    let addr = serve(router, 2).await;

    assert_eq!(get_path(addr, "/").await, ok("-"));
    assert_eq!(get_path(addr, "/fr").await, ok("locale=fr"));
}

/// Counts the requests that reached it.
struct Counts(Arc<AtomicUsize>);

#[async_trait]
impl Middleware for Counts {
    async fn handle(&self, request: Request, next: Next) -> Response {
        self.0.fetch_add(1, Ordering::SeqCst);
        next(request).await
    }
}

#[tokio::test]
async fn the_middleware_of_the_route_runs_for_every_form_of_it() {
    let reached = Arc::new(AtomicUsize::new(0));
    let router = Router::new()
        .get("/posts/{id?}", echo)
        .middleware(Counts(reached.clone()));
    let addr = serve(router, 2).await;

    assert_eq!(get_path(addr, "/posts").await.0, 200);
    assert_eq!(get_path(addr, "/posts/42").await.0, 200);

    assert_eq!(
        reached.load(Ordering::SeqCst),
        2,
        "a form that ran without the route's middleware would be a way around it"
    );
}

#[test]
fn a_named_route_leaves_an_optional_segment_out_when_it_has_no_value() {
    let _router = Router::new()
        .get("/rp-archive/{year?}/{month?}", echo)
        .name("rp.archive");

    assert_eq!(route("rp.archive", &[]).as_deref(), Some("/rp-archive"));
    assert_eq!(
        route("rp.archive", &[("year", "2026")]).as_deref(),
        Some("/rp-archive/2026")
    );
    assert_eq!(
        route("rp.archive", &[("year", "2026"), ("month", "05")]).as_deref(),
        Some("/rp-archive/2026/05")
    );
    assert_eq!(try_route("rp.archive", &[]).as_deref(), Ok("/rp-archive"));

    // The month alone has no place to go: left out, the year would be `05`.
    assert_eq!(
        try_route("rp.archive", &[("month", "05")]),
        Err(RouteUrlError::MissingParams {
            name: "rp.archive".to_owned(),
            missing: vec!["year".to_owned()],
        })
    );
    assert_eq!(
        route("rp.archive", &[("month", "05")]).as_deref(),
        Some("/rp-archive/{year}/05")
    );
}

#[test]
fn an_optional_parameter_at_the_root_generates_the_root() {
    let _router = Router::new().get("/{rp_locale?}", echo).name("rp.home");

    assert_eq!(route("rp.home", &[]).as_deref(), Some("/"));
    assert_eq!(
        route("rp.home", &[("rp_locale", "fr")]).as_deref(),
        Some("/fr")
    );
}

#[test]
fn a_required_segment_behind_an_optional_parameter_is_refused_at_registration() {
    let error = Router::new()
        .try_get("/posts/{id?}/comments", echo)
        .err()
        .expect("the router could not tell `/posts/comments` apart");

    let message = error.to_string();
    assert!(message.contains("/posts/{id?}/comments"), "{message}");
    assert!(
        message.contains("behind an optional parameter"),
        "{message}"
    );
}

#[test]
fn a_named_route_reads_an_empty_value_as_no_value() {
    // `/rp-notes/` is a URL no form of the route matches.
    let _router = Router::new().get("/rp-notes/{id?}", echo).name("rp.notes");

    assert_eq!(
        route("rp.notes", &[("id", "")]).as_deref(),
        Some("/rp-notes")
    );
    assert_eq!(
        try_route("rp.notes", &[("id", "")]).as_deref(),
        Ok("/rp-notes")
    );
}

#[test]
fn a_route_that_is_one_form_of_another_is_refused_at_registration() {
    // `/posts/{id?}` is `/posts` and `/posts/{id}`. A second route for
    // either would be reached or not by the order of two lines.
    for (first, second) in [
        ("/posts/{id?}", "/posts"),
        ("/posts", "/posts/{id?}"),
        ("/posts/{id?}", "/posts/{id}"),
        ("/posts/{id}", "/posts/{id?}"),
    ] {
        let error = Router::new()
            .get(first, echo)
            .try_get(second, echo)
            .err()
            .unwrap_or_else(|| panic!("`{second}` behind `{first}` must be refused"));

        let message = error.to_string();
        assert!(message.contains(second), "{message}");
        assert_eq!(
            message.matches("Internal server error").count(),
            1,
            "the error must be said once: {message}"
        );
    }
}

#[test]
fn an_empty_segment_beside_an_optional_parameter_is_refused_at_registration() {
    for pattern in ["/posts/{id?}/", "/posts//{id?}"] {
        let error = Router::new()
            .try_get(pattern, echo)
            .err()
            .unwrap_or_else(|| panic!("`{pattern}` must be refused"));

        let message = error.to_string();
        assert!(message.contains("empty segment"), "{message}");
        assert_eq!(
            message.matches("Internal server error").count(),
            1,
            "the error must be said once: {message}"
        );
    }
}

struct Silent;

#[async_trait]
impl suprnova::ws::WebSocketHandler for Silent {
    async fn handle(
        &self,
        _socket: suprnova::ws::WsSocket,
        _request: Request,
    ) -> Result<(), suprnova::FrameworkError> {
        Ok(())
    }
}

#[test]
fn a_websocket_route_takes_an_optional_parameter() {
    let router = Router::new().ws("/ws/rooms/{room?}", Silent);

    let lobby = router.match_ws("/ws/rooms").expect("the short form");
    assert_eq!(lobby.pattern(), "/ws/rooms/{room?}");
    assert!(lobby.params().is_empty());

    let room = router.match_ws("/ws/rooms/7").expect("the long form");
    assert_eq!(room.pattern(), "/ws/rooms/{room?}");
    assert_eq!(room.params().get("room").map(String::as_str), Some("7"));

    assert!(router.match_ws("/ws/rooms/7/extra").is_none());
}

// --- Constraints -------------------------------------------------------------

#[tokio::test]
async fn a_constraint_is_checked_on_the_value_the_handler_reads() {
    // The value is decoded before it is checked, so what the constraint
    // allows is what the handler finds. An encoded slash is a slash in
    // the value and no separator of the path.
    let router = Router::new().get("/posts/{id}", echo).where_number("id");
    let addr = serve(router, 3).await;

    assert_eq!(get_path(addr, "/posts/%34%32").await, ok("id=42"));
    assert_eq!(get_path(addr, "/posts/4%2F2").await.0, 404);
    assert_eq!(get_path(addr, "/posts/42%20").await.0, 404);
}

#[tokio::test]
async fn a_pattern_has_to_match_the_whole_value() {
    let router = Router::new()
        .get("/archive/{year}", echo)
        .where_pattern("year", "[0-9]{4}");
    let addr = serve(router, 4).await;

    assert_eq!(get_path(addr, "/archive/2024").await, ok("year=2024"));
    assert_eq!(get_path(addr, "/archive/x2024abc").await.0, 404);
    assert_eq!(get_path(addr, "/archive/20245").await.0, 404);
    assert_eq!(get_path(addr, "/archive/x2024").await.0, 404);
}

#[test]
fn an_expression_that_is_none_on_its_own_is_refused() {
    // Between `^(?:` and `)$` this one compiles, and matches whatever
    // starts with `a` or ends with `b`.
    let error = ParamConstraint::pattern("a)|(b").unwrap_err();
    assert!(error.to_string().contains("is not a pattern"), "{error}");
}

#[test]
fn a_pattern_constraint_shows_the_expression_it_was_built_from() {
    let ParamConstraint::Pattern(pattern) =
        ParamConstraint::pattern("[a-z]+").expect("a regular expression")
    else {
        panic!("`pattern` builds the pattern constraint");
    };

    assert_eq!(pattern.as_str(), "[a-z]+");
    assert!(pattern.matches("abc"));
    assert!(!pattern.matches("abc1"));
}

#[tokio::test]
async fn a_value_the_constraint_refuses_is_a_404_and_the_handler_is_not_run() {
    let reached = Arc::new(AtomicUsize::new(0));
    let router = Router::new()
        .get("/posts/{id}", echo)
        .where_number("id")
        .middleware(Counts(reached.clone()));
    let addr = serve(router, 3).await;

    assert_eq!(get_path(addr, "/posts/42").await, ok("id=42"));
    assert_eq!(get_path(addr, "/posts/abc").await.0, 404);
    assert_eq!(get_path(addr, "/posts/4x2").await.0, 404);

    assert_eq!(
        reached.load(Ordering::SeqCst),
        1,
        "a refused request must not reach the route's middleware either"
    );
}

#[tokio::test]
async fn each_named_constraint_guards_its_route() {
    let router = Router::new()
        .get("/by-uuid/{id}", echo)
        .where_uuid("id")
        .get("/by-name/{id}", echo)
        .where_alpha("id")
        .get("/by-code/{id}", echo)
        .where_alpha_numeric("id")
        .get("/by-ulid/{id}", echo)
        .where_ulid("id")
        .get("/by-status/{id}", echo)
        .where_in("id", ["draft", "published"])
        .get("/by-year/{id}", echo)
        .where_pattern("id", "(19|20)[0-9]{2}");
    let addr = serve(router, 12).await;

    for (path, expected) in [
        ("/by-uuid/550e8400-e29b-41d4-a716-446655440000", 200),
        ("/by-uuid/42", 404),
        ("/by-name/ada", 200),
        ("/by-name/ada1", 404),
        ("/by-code/ada1", 200),
        ("/by-code/ada-1", 404),
        ("/by-ulid/01ARZ3NDEKTSV4RRFFQ69G5FAV", 200),
        ("/by-ulid/not-a-ulid", 404),
        ("/by-status/draft", 200),
        ("/by-status/deleted", 404),
        ("/by-year/2026", 200),
        ("/by-year/20260", 404),
    ] {
        assert_eq!(get_path(addr, path).await.0, expected, "{path}");
    }
}

#[tokio::test]
async fn an_optional_parameter_is_checked_only_when_it_is_there() {
    let router = Router::new()
        .get("/archive/{year?}", echo)
        .where_pattern("year", "[0-9]{4}");
    let addr = serve(router, 3).await;

    assert_eq!(get_path(addr, "/archive").await, ok("-"));
    assert_eq!(get_path(addr, "/archive/2026").await, ok("year=2026"));
    assert_eq!(get_path(addr, "/archive/soon").await.0, 404);
}

#[tokio::test]
async fn a_constraint_holds_for_the_method_it_was_set_on() {
    let router = Router::new()
        .get("/items/{id}", echo)
        .where_number("id")
        .post("/items/{id}", echo);
    let addr = serve(router, 4).await;

    assert_eq!(send(addr, "GET", "/items/abc").await.0, 404);
    assert_eq!(
        send(addr, "POST", "/items/abc").await,
        ok("id=abc"),
        "the POST route set no constraint"
    );
    // A HEAD request with no HEAD route runs the GET route, constraint included.
    assert_eq!(send(addr, "HEAD", "/items/abc").await.0, 404);
    assert_eq!(send(addr, "HEAD", "/items/42").await.0, 200);
}

#[tokio::test]
async fn the_route_macros_take_constraints_and_a_group_prefix_may_be_constrained() {
    let router = group!("/teams/{team}", {
        get!("/members/{id?}", echo).where_number("team").where_number("id"),
        any!("/anything/{id}", echo).where_alpha("id"),
    })
    .register(
        get!("/plain/{id}", echo)
            .where_number("id")
            .register(Router::new()),
    );
    let addr = serve(router, 9).await;

    assert_eq!(get_path(addr, "/plain/1").await, ok("id=1"));
    assert_eq!(get_path(addr, "/plain/one").await.0, 404);

    assert_eq!(get_path(addr, "/teams/7/members").await, ok("team=7"));
    assert_eq!(
        get_path(addr, "/teams/7/members/3").await,
        ok("id=3 team=7")
    );
    assert_eq!(get_path(addr, "/teams/seven/members").await.0, 404);
    assert_eq!(get_path(addr, "/teams/7/members/three").await.0, 404);

    assert_eq!(send(addr, "DELETE", "/teams/7/anything/abc").await.0, 200);
    assert_eq!(send(addr, "DELETE", "/teams/7/anything/123").await.0, 404);
    assert_eq!(send(addr, "PUT", "/teams/7/anything/123").await.0, 404);
}

#[test]
fn a_constraint_on_a_parameter_the_route_does_not_have_is_refused() {
    let error = Router::new()
        .get("/posts/{id}", echo)
        .try_constrain("slug", ParamConstraint::Alpha)
        .err()
        .expect("a constraint that is never checked would look like a guard");

    let message = error.to_string();
    assert!(message.contains("has no parameter `slug`"), "{message}");
    assert!(
        message.contains("id"),
        "the error names what there is: {message}"
    );
}

#[test]
#[should_panic(expected = "has no parameter `slug`")]
fn a_macro_route_refuses_it_when_it_is_registered() {
    let _router = get!("/posts/{id}", echo)
        .where_alpha("slug")
        .register(Router::new());
}

#[test]
fn a_pattern_that_is_no_regular_expression_is_an_error() {
    let error = ParamConstraint::pattern("[0-9").unwrap_err();
    assert!(error.to_string().contains("is not a pattern"), "{error}");
}
