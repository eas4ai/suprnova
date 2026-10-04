//! `Router::ws` registration + path matching.

use async_trait::async_trait;
use suprnova::FrameworkError;
use suprnova::http::Request;
use suprnova::routing::Router;
use suprnova::ws::{WebSocketHandler, WsSocket};

struct NoopHandler;

#[async_trait]
impl WebSocketHandler for NoopHandler {
    async fn handle(&self, _socket: WsSocket, _request: Request) -> Result<(), FrameworkError> {
        Ok(())
    }
}

#[test]
fn ws_route_registers_and_resolves_by_path() {
    let router = Router::new().ws("/ws/echo", NoopHandler);
    assert!(router.match_ws("/ws/echo").is_some(), "ws route resolves");
}

#[test]
fn ws_route_with_params_captures_segments() {
    let router = Router::new().ws("/ws/rooms/{id}", NoopHandler);
    let m = router
        .match_ws("/ws/rooms/42")
        .expect("matches with params");
    let params = m.params();
    assert_eq!(
        params.get("id").map(String::as_str),
        Some("42"),
        "captured param: {params:?}"
    );
}

#[test]
fn ws_route_misses_when_path_does_not_match() {
    let router = Router::new().ws("/ws/echo", NoopHandler);
    assert!(router.match_ws("/api/echo").is_none());
}

#[test]
fn ws_routes_chain_directly_without_into() {
    // Router::ws returns Router (NOT RouteBuilder) so this chains cleanly.
    let router = Router::new()
        .ws("/ws/a", NoopHandler)
        .ws("/ws/b", NoopHandler);
    assert!(router.match_ws("/ws/a").is_some());
    assert!(router.match_ws("/ws/b").is_some());
}

/// ROOT-20: a WebSocket path capture is percent-decoded the way an HTTP
/// capture is. The router still matches the raw path, so an encoded
/// slash stays inside one segment; only the value the handler reads is
/// decoded.
#[test]
fn ws_route_params_are_percent_decoded_like_http_params() {
    let router = Router::new().ws("/ws/rooms/{id}", NoopHandler);

    let m = router
        .match_ws("/ws/rooms/a%20b")
        .expect("matches the raw path");
    assert_eq!(m.params().get("id").map(String::as_str), Some("a b"));

    let m = router
        .match_ws("/ws/rooms/caf%C3%A9")
        .expect("matches a UTF-8 escape");
    assert_eq!(m.params().get("id").map(String::as_str), Some("café"));

    let m = router
        .match_ws("/ws/rooms/a%2Fb")
        .expect("an encoded slash stays inside one segment");
    assert_eq!(m.params().get("id").map(String::as_str), Some("a/b"));
}
