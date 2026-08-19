//! `HttpResponse::event_stream` and `HttpResponse::stream_json` —
//! Laravel's `ResponseFactory::eventStream` / `streamJson`.
//!
//! Three harnesses, each earning its own keep:
//!
//! - `spawn_server` + `fetch`: a one-shot real-socket server that calls
//!   `HttpResponse::into_hyper()` directly inside a bare `service_fn`,
//!   bypassing the `Router` and `MiddlewareRegistry` entirely — same
//!   shape as `framework/tests/sse.rs`, because hyper's `body::Incoming`
//!   can't be constructed outside its own connection machinery. Used
//!   where the thing under test is response-building and wire framing
//!   (event ordering, JSON encoding, the terminal sentinel, a
//!   serialization failure mid-stream), not routing.
//! - `incoming_get_request` + `handle_request`: builds a genuine
//!   `hyper::Request<Incoming>` by parsing real HTTP/1.1 bytes through
//!   an in-memory `tokio::io::duplex` pipe — no TCP port bound, same
//!   idiom as `framework/tests/common.rs::request_from_http_bytes` —
//!   then drives it through the real `Router` + `MiddlewareRegistry`
//!   chain via `handle_request`, the in-process adapter `Server::run`
//!   itself calls per connection. This is the coverage the first
//!   harness structurally cannot provide: a response-mutating
//!   middleware that isn't stream-aware could corrupt or buffer
//!   `event_stream`/`stream_json` and the bare-`service_fn` tests would
//!   stay green, because they never run the middleware chain at all.
//! - The client-abort test binds a real TCP listener and closes a real
//!   socket mid-stream, because that's the one thing an in-memory pipe
//!   can't simulate: a genuine, OS-level disconnect.
//!
//! Every await that waits on a response — the thing that could actually
//! hang if `event_stream`/`stream_json`'s `.chain()`/`.scan()` composition
//! regressed and stopped signaling completion — is bounded by
//! `tokio::time::timeout`. The exception is the handful of foreground
//! `connect`/`write_all` calls that only get request bytes onto the wire
//! before that bounded wait begins: `fetch`'s `connect`,
//! `incoming_get_request`'s `write_all` into a `tokio::io::duplex` sized
//! larger than the payload, and the client-abort test's `connect` +
//! `write_all` onto a loopback socket whose peer has already spawned an
//! active reader by the time the write runs. None of those can block, so
//! wrapping them would only add noise.

use bytes::Bytes;
use futures::stream;
use http_body_util::{BodyExt, Empty};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use suprnova::sse::{EndSignal, StreamedEvent};
use suprnova::{
    CorsConfig, CorsMiddleware, HttpResponse, MiddlewareRegistry, Router, handle_request,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};

/// How long any single bounded wait in this file is allowed to take.
/// Generous for a loopback / in-memory-pipe round trip; short enough
/// that a genuine hang fails fast instead of stalling the suite.
const WAIT: Duration = Duration::from_secs(5);

/// Spawn a one-shot server answering exactly one connection with
/// `make_response()`'s result.
async fn spawn_server<F>(make_response: F) -> SocketAddr
where
    F: Fn() -> HttpResponse + Send + Sync + 'static,
{
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let make_response = Arc::new(make_response);
    tokio::spawn(async move {
        if let Ok((stream_tcp, _)) = listener.accept().await {
            let io = TokioIo::new(stream_tcp);
            let make_response = make_response.clone();
            let svc = service_fn(move |_req: hyper::Request<hyper::body::Incoming>| {
                let make_response = make_response.clone();
                async move { Ok::<_, Infallible>(make_response().into_hyper()) }
            });
            let _ = http1::Builder::new().serve_connection(io, svc).await;
        }
    });
    addr
}

/// GET `/`, fully collecting the response body. Mirrors `sse.rs::fetch`.
/// Both awaits are bounded — a regression that stops the response from
/// signaling completion fails this test instead of hanging the suite.
async fn fetch(addr: SocketAddr) -> hyper::Response<Bytes> {
    let stream_tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let io = TokioIo::new(stream_tcp);
    let (mut sender, conn) = hyper::client::conn::http1::handshake::<_, Empty<Bytes>>(io)
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let req = hyper::Request::builder()
        .method("GET")
        .uri("/")
        .header("Host", "localhost")
        .body(Empty::<Bytes>::new())
        .unwrap();

    let resp = tokio::time::timeout(WAIT, sender.send_request(req))
        .await
        .expect("send_request timed out")
        .unwrap();
    let (parts, body) = resp.into_parts();
    let collected = tokio::time::timeout(WAIT, body.collect())
        .await
        .expect("collecting the response body timed out")
        .unwrap();
    hyper::Response::from_parts(parts, collected.to_bytes())
}

/// Build a genuine `hyper::Request<hyper::body::Incoming>` for a `GET`
/// on `path` carrying `headers`, by parsing real HTTP/1.1 bytes through
/// an in-memory `tokio::io::duplex` pipe rather than binding a TCP
/// port. `hyper::body::Incoming` is privately constructed in hyper
/// 1.x, so this is the only way to obtain a genuine one without a live
/// connection — same idiom as
/// `framework/tests/common.rs::request_from_http_bytes` (see that
/// file's doc comment for the full rationale).
async fn incoming_get_request(
    path: &str,
    headers: &[(&str, &str)],
) -> hyper::Request<hyper::body::Incoming> {
    let mut http_bytes = Vec::new();
    http_bytes.extend_from_slice(format!("GET {path} HTTP/1.1\r\n").as_bytes());
    http_bytes.extend_from_slice(b"Host: localhost\r\n");
    for (name, value) in headers {
        http_bytes.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
    }
    http_bytes.extend_from_slice(b"\r\n");

    let (req_tx, req_rx) = oneshot::channel::<hyper::Request<hyper::body::Incoming>>();
    let req_tx = Mutex::new(Some(req_tx));

    let (client_io, server_io) = tokio::io::duplex(http_bytes.len() + 64 * 1024);

    tokio::spawn(async move {
        let svc = service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
            if let Ok(mut guard) = req_tx.lock()
                && let Some(tx) = guard.take()
            {
                let _ = tx.send(req);
            }
            // Never resolve — the caller only wants the parsed
            // `Incoming` request, not a response over this connection.
            // Returning `Ok` here would stop hyper from pumping any
            // remaining body bytes (see `common.rs` for the same note).
            async {
                std::future::pending::<()>().await;
                Ok::<_, Infallible>(hyper::Response::new(Empty::<Bytes>::new()))
            }
        });
        let _ = http1::Builder::new()
            .serve_connection(TokioIo::new(server_io), svc)
            .await;
    });

    let mut client = client_io;
    client.write_all(&http_bytes).await.unwrap();

    tokio::time::timeout(WAIT, req_rx)
        .await
        .expect("timed out building the incoming request")
        .expect("server should have received the request")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Item {
    id: u32,
    label: String,
}

/// A `Serialize` type that fails deliberately for one variant, to drive
/// `stream_json`'s per-item serialization-failure path. `f64::NAN`
/// looks like the obvious choice but doesn't exercise it: `serde_json`
/// substitutes JSON `null` for non-finite floats instead of erroring,
/// so a genuine `Err` needs a hand-rolled `Serialize` impl instead.
enum MaybeBad {
    Good(u32),
    Bad,
}

impl Serialize for MaybeBad {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            MaybeBad::Good(n) => serializer.serialize_u32(*n),
            MaybeBad::Bad => Err(serde::ser::Error::custom(
                "deliberately unserializable, for the test",
            )),
        }
    }
}

#[tokio::test]
async fn event_stream_frames_bare_messages_as_update_and_json_encodes_structs() {
    let addr = spawn_server(|| {
        let events = vec![
            StreamedEvent::message("hello").unwrap(),
            StreamedEvent::message(&Item {
                id: 1,
                label: "a".into(),
            })
            .unwrap(),
        ];
        HttpResponse::event_stream(stream::iter(events), EndSignal::default())
    })
    .await;

    let resp = fetch(addr).await;
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers().get("Content-Type").unwrap(),
        "text/event-stream"
    );

    let body = resp.body().clone();
    let s = std::str::from_utf8(&body).unwrap();
    let frames: Vec<&str> = s.split("\n\n").filter(|f| !f.is_empty()).collect();

    assert_eq!(
        frames.len(),
        3,
        "2 items + the default sentinel: {frames:?}"
    );
    assert_eq!(frames[0], "event: update\ndata: hello");
    assert_eq!(frames[1], "event: update\ndata: {\"id\":1,\"label\":\"a\"}");
    assert_eq!(
        frames[2], "event: update\ndata: </stream>",
        "the sentinel must be last and use the default text"
    );
}

#[tokio::test]
async fn event_stream_honours_a_custom_event_name_and_end_signal_none_omits_the_sentinel() {
    let addr = spawn_server(|| {
        let events = vec![StreamedEvent::named("progress", json!({ "pct": 50 })).unwrap()];
        HttpResponse::event_stream(stream::iter(events), EndSignal::None)
    })
    .await;

    let resp = fetch(addr).await;
    let body = resp.body().clone();
    let s = std::str::from_utf8(&body).unwrap();
    let frames: Vec<&str> = s.split("\n\n").filter(|f| !f.is_empty()).collect();

    assert_eq!(
        frames.len(),
        1,
        "EndSignal::None must omit the sentinel: {frames:?}"
    );
    assert_eq!(frames[0], "event: progress\ndata: {\"pct\":50}");
}

#[tokio::test]
async fn stream_json_round_trips_three_items() {
    let addr = spawn_server(|| {
        let items = vec![
            Item {
                id: 1,
                label: "a".into(),
            },
            Item {
                id: 2,
                label: "b".into(),
            },
            Item {
                id: 3,
                label: "c".into(),
            },
        ];
        HttpResponse::stream_json(stream::iter(items))
    })
    .await;

    let resp = fetch(addr).await;
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers().get("Content-Type").unwrap(),
        "application/json"
    );

    let parsed: Vec<Item> = serde_json::from_slice(resp.body()).expect("valid JSON array");
    assert_eq!(
        parsed,
        vec![
            Item {
                id: 1,
                label: "a".into()
            },
            Item {
                id: 2,
                label: "b".into()
            },
            Item {
                id: 3,
                label: "c".into()
            },
        ]
    );
}

/// The documented failure mode in `response.rs::stream_json`: an item
/// that fails `serde_json::to_vec` (here, `f64::NAN` — JSON has no NaN
/// literal) is dropped with a `tracing::warn!` rather than corrupting
/// the array with a stray comma, an empty slot, or a panic.
#[tokio::test]
async fn stream_json_drops_an_item_that_fails_to_serialize_without_corrupting_the_array() {
    let addr = spawn_server(|| {
        let items = vec![MaybeBad::Good(1), MaybeBad::Bad, MaybeBad::Good(3)];
        HttpResponse::stream_json(stream::iter(items))
    })
    .await;

    let resp = fetch(addr).await;
    assert_eq!(resp.status(), 200);

    let body = resp.body().clone();
    let s = std::str::from_utf8(&body).unwrap();
    assert!(
        !s.contains(",,") && !s.starts_with("[,") && !s.contains(",]"),
        "a dropped item must not leave a stray comma or empty slot on the wire: {s:?}"
    );

    let parsed: serde_json::Value = serde_json::from_str(s)
        .expect("the array must stay syntactically valid JSON despite the dropped item");
    assert_eq!(
        parsed,
        json!([1, 3]),
        "the unserializable item must be dropped outright, not left as null or a stray token"
    );
}

/// Drives `event_stream` through the real `Router` + `MiddlewareRegistry`
/// chain via `handle_request`, not just `HttpResponse::into_hyper()` in a
/// bare `service_fn`. `CorsMiddleware`, registered globally, proves two
/// things at once: the request actually reached the router (only a
/// matched route produces this body) and the middleware chain actually
/// ran on the way out (the CORS header is middleware-applied, not part
/// of `event_stream` itself) — without disturbing the streamed body.
#[tokio::test]
async fn event_stream_survives_the_router_and_middleware_chain() {
    let router: Router = Router::new()
        .get("/progress", |_req| async {
            let events = vec![StreamedEvent::message("hello").unwrap()];
            Ok(HttpResponse::event_stream(
                stream::iter(events),
                EndSignal::default(),
            ))
        })
        .into();
    let registry = MiddlewareRegistry::new().append(CorsMiddleware::new(
        CorsConfig::allow_origins(["https://app.example"]).allow_credentials(true),
    ));

    let req = incoming_get_request("/progress", &[("Origin", "https://app.example")]).await;
    let resp = tokio::time::timeout(
        WAIT,
        handle_request(Arc::new(router), Arc::new(registry), req),
    )
    .await
    .expect("handle_request timed out");

    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers().get("Content-Type").unwrap(),
        "text/event-stream",
        "the streamed response's own headers must survive the middleware chain"
    );
    assert_eq!(
        resp.headers()
            .get("access-control-allow-origin")
            .expect("global CORS middleware must have actually run on this route"),
        "https://app.example"
    );

    let body = tokio::time::timeout(WAIT, resp.into_body().collect())
        .await
        .expect("collecting the streamed body timed out")
        .unwrap()
        .to_bytes();
    let s = std::str::from_utf8(&body).unwrap();
    let frames: Vec<&str> = s.split("\n\n").filter(|f| !f.is_empty()).collect();

    assert_eq!(frames.len(), 2, "1 item + the default sentinel: {frames:?}");
    assert_eq!(frames[0], "event: update\ndata: hello");
    assert_eq!(frames[1], "event: update\ndata: </stream>");
}

/// Same proof as `event_stream_survives_the_router_and_middleware_chain`,
/// for `stream_json`: real `Router` + `MiddlewareRegistry` via
/// `handle_request`, global `CorsMiddleware` decorating the response
/// without corrupting the incrementally-built JSON array.
#[tokio::test]
async fn stream_json_survives_the_router_and_middleware_chain() {
    let router: Router = Router::new()
        .get("/export", |_req| async {
            let items = vec![
                Item {
                    id: 1,
                    label: "a".into(),
                },
                Item {
                    id: 2,
                    label: "b".into(),
                },
            ];
            Ok(HttpResponse::stream_json(stream::iter(items)))
        })
        .into();
    let registry = MiddlewareRegistry::new().append(CorsMiddleware::new(
        CorsConfig::allow_origins(["https://app.example"]).allow_credentials(true),
    ));

    let req = incoming_get_request("/export", &[("Origin", "https://app.example")]).await;
    let resp = tokio::time::timeout(
        WAIT,
        handle_request(Arc::new(router), Arc::new(registry), req),
    )
    .await
    .expect("handle_request timed out");

    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers().get("Content-Type").unwrap(),
        "application/json",
        "the streamed response's own headers must survive the middleware chain"
    );
    assert_eq!(
        resp.headers()
            .get("access-control-allow-origin")
            .expect("global CORS middleware must have actually run on this route"),
        "https://app.example"
    );

    let body = tokio::time::timeout(WAIT, resp.into_body().collect())
        .await
        .expect("collecting the streamed body timed out")
        .unwrap()
        .to_bytes();

    let parsed: Vec<Item> = serde_json::from_slice(&body).expect("valid JSON array");
    assert_eq!(
        parsed,
        vec![
            Item {
                id: 1,
                label: "a".into()
            },
            Item {
                id: 2,
                label: "b".into()
            },
        ]
    );
}

#[tokio::test]
async fn event_stream_client_abort_ends_the_producer() {
    let dropped = Arc::new(AtomicBool::new(false));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let dropped_for_server = dropped.clone();
    tokio::spawn(async move {
        if let Ok((stream_tcp, _)) = listener.accept().await {
            let io = TokioIo::new(stream_tcp);
            let dropped = dropped_for_server.clone();
            let svc = service_fn(move |_req: hyper::Request<hyper::body::Incoming>| {
                let dropped = dropped.clone();
                async move {
                    let (tx, rx) = mpsc::channel::<StreamedEvent>(4);

                    tokio::spawn(async move {
                        struct DropSignal(Arc<AtomicBool>);
                        impl Drop for DropSignal {
                            fn drop(&mut self) {
                                self.0.store(true, Ordering::SeqCst);
                            }
                        }
                        let _signal = DropSignal(dropped);

                        loop {
                            let evt = StreamedEvent::message("tick").unwrap();
                            if tx.send(evt).await.is_err() {
                                break; // receiver dropped: client disconnected
                            }
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                    });

                    // `mpsc::Receiver` -> `Stream` without a `tokio-stream`
                    // dependency (Design note 5): `futures::stream::unfold`.
                    let rx_stream = stream::unfold(rx, |mut rx| async move {
                        rx.recv().await.map(|item| (item, rx))
                    });
                    let resp = HttpResponse::event_stream(rx_stream, EndSignal::default());
                    Ok::<_, Infallible>(resp.into_hyper())
                }
            });
            let _ = http1::Builder::new().serve_connection(io, svc).await;
        }
    });

    let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
    client
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();

    // Read until at least one `data:` frame has arrived, proving the
    // producer is live before we abort it.
    let mut received = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        let n = tokio::time::timeout(WAIT, client.read(&mut buf))
            .await
            .expect("timed out waiting for the first SSE frame")
            .unwrap();
        assert!(n > 0, "connection closed before any data arrived");
        received.extend_from_slice(&buf[..n]);
        if received.windows(6).any(|w| w == b"data: ") {
            break;
        }
    }

    drop(client); // abort mid-stream

    let deadline = tokio::time::Instant::now() + WAIT;
    while !dropped.load(Ordering::SeqCst) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "producer did not stop within {WAIT:?} of the client aborting"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
