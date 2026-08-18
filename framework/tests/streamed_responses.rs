//! `HttpResponse::event_stream` and `HttpResponse::stream_json` —
//! Laravel's `ResponseFactory::eventStream` / `streamJson`.
//!
//! Same harness shape as `framework/tests/sse.rs`: hyper's
//! `body::Incoming` can't be constructed outside its own connection
//! machinery, so these tests bind a one-shot TCP listener and drive a
//! real socket rather than calling `into_hyper()` in-process.

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
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use suprnova::HttpResponse;
use suprnova::sse::{EndSignal, StreamedEvent};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

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

    let resp = sender.send_request(req).await.unwrap();
    let (parts, body) = resp.into_parts();
    let collected = body.collect().await.unwrap();
    hyper::Response::from_parts(parts, collected.to_bytes())
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Item {
    id: u32,
    label: String,
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
        let n = tokio::time::timeout(Duration::from_secs(2), client.read(&mut buf))
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

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while !dropped.load(Ordering::SeqCst) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "producer did not stop within 2s of the client aborting"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
