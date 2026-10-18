use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;
use suprnova::bus::command::{Command, Handler};
use suprnova::{
    Bus, FrameworkError, HttpResponse, MiddlewareRegistry, Router, after_response_connection,
    async_trait, dispatched, dispatched_after_response, dispatched_sync,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

static RUNS: AtomicU32 = AtomicU32::new(0);

#[derive(Serialize, Deserialize, Debug)]
struct Mark {
    id: u32,
}
impl Command for Mark {
    type Output = u32;
    fn command_name() -> &'static str {
        "delta-mark"
    }
}
struct MarkHandler;
#[async_trait]
impl Handler<Mark> for MarkHandler {
    async fn handle(&self, command: Mark) -> Result<u32, FrameworkError> {
        RUNS.fetch_add(1, Ordering::SeqCst);
        if command.id == 99 {
            Err(FrameworkError::internal("mark failed"))
        } else {
            Ok(command.id)
        }
    }
}

#[tokio::test]
#[serial]
async fn bus_fake_returns_each_dispatch_mode_in_order_for_typed_filters() {
    let _fake = Bus::fake();
    assert!(dispatched::<Mark>().is_empty());
    assert!(dispatched_sync::<Mark>().is_empty());
    assert!(dispatched_after_response::<Mark>().is_empty());
    for id in [7, 8] {
        Bus::dispatch(Mark { id }).await.expect("dispatch");
    }
    Bus::dispatch_sync(Mark { id: 9 }).await.expect("sync");
    Bus::dispatch_after_response(Mark { id: 10 }).expect("after response");
    assert_eq!(
        dispatched::<Mark>()
            .into_iter()
            .map(|c| c.id)
            .collect::<Vec<_>>(),
        [7, 8]
    );
    assert_eq!(
        dispatched::<Mark>()
            .into_iter()
            .filter(|c| c.id == 7)
            .count(),
        1
    );
    assert_eq!(
        dispatched_sync::<Mark>()
            .into_iter()
            .map(|c| c.id)
            .collect::<Vec<_>>(),
        [9]
    );
    assert_eq!(
        dispatched_after_response::<Mark>()
            .into_iter()
            .map(|c| c.id)
            .collect::<Vec<_>>(),
        [10]
    );
    assert_eq!(RUNS.load(Ordering::SeqCst), 0);
}

#[tokio::test]
#[serial]
async fn sync_dispatch_runs_the_handler_and_propagates_failures() {
    RUNS.store(0, Ordering::SeqCst);
    Bus::register::<Mark, _>(MarkHandler);
    assert_eq!(
        Bus::dispatch_sync(Mark { id: 7 })
            .await
            .expect("sync")
            .executed(),
        Some(7)
    );
    assert!(Bus::dispatch_sync(Mark { id: 99 }).await.is_err());
    assert_eq!(RUNS.load(Ordering::SeqCst), 2);
}

#[test]
fn after_response_without_a_response_writer_returns_an_error() {
    let error = Bus::dispatch_after_response(Mark { id: 7 }).expect_err("no response");
    assert!(error.to_string().contains("response"));
}

#[derive(Default)]
struct FlushGate {
    ready: AtomicBool,
    fail: AtomicBool,
    polled: AtomicBool,
    waker: Mutex<Option<Waker>>,
}

impl FlushGate {
    fn open(&self) {
        self.ready.store(true, Ordering::SeqCst);
        if let Some(waker) = self.waker.lock().expect("waker").take() {
            waker.wake();
        }
    }
}

struct GatedIo {
    io: tokio::io::DuplexStream,
    gate: Arc<FlushGate>,
}
impl AsyncRead for GatedIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_read(cx, buf)
    }
}
impl AsyncWrite for GatedIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.io).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        self.gate.polled.store(true, Ordering::SeqCst);
        let mut waker = self.gate.waker.lock().expect("waker");
        if !self.gate.ready.load(Ordering::SeqCst) {
            *waker = Some(cx.waker().clone());
            return Poll::Pending;
        }
        drop(waker);
        if self.gate.fail.load(Ordering::SeqCst) {
            return Poll::Ready(Err(std::io::ErrorKind::BrokenPipe.into()));
        }
        Pin::new(&mut self.io).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(cx)
    }
}

async fn wait_for(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("condition");
}

async fn response_must_flush_before_dispatch(kind: &'static str, fail_flush: bool) {
    RUNS.store(0, Ordering::SeqCst);
    Bus::register::<Mark, _>(MarkHandler);
    let router: Router = Router::new()
        .get("/delta-after-response", move |_| async move {
            if kind == "failed-command" {
                Bus::dispatch_after_response(Mark { id: 99 }).map_err(HttpResponse::from)?;
            }
            Bus::dispatch_after_response(Mark { id: 7 }).map_err(HttpResponse::from)?;
            let response = match kind {
                "empty" => HttpResponse::text(""),
                "no-content" => HttpResponse::text("suppressed by HTTP").status(204),
                "not-modified" => HttpResponse::text("suppressed by HTTP").status(304),
                "stream" => HttpResponse::stream_bytes(futures::stream::iter([
                    Ok::<_, Infallible>(Bytes::from_static(b"first")),
                    Ok(Bytes::from_static(b"last")),
                ])),
                "stream-content-length" => {
                    let frame = Bytes::from_static(b"sent");
                    let length = frame.len().to_string();
                    HttpResponse::stream_bytes(futures::stream::iter([Ok::<_, Infallible>(frame)]))
                        .header("Content-Length", length)
                }
                _ => HttpResponse::text("sent"),
            };
            Ok(response)
        })
        .into();
    let router = Arc::new(router);
    let gate = Arc::new(FlushGate::default());
    gate.fail.store(fail_flush, Ordering::SeqCst);
    let (client, server) = tokio::io::duplex(1024);
    let io = GatedIo {
        io: server,
        gate: gate.clone(),
    };
    let task = tokio::spawn(after_response_connection(io, move |io| async move {
        let service = hyper::service::service_fn(move |request| {
            let router = router.clone();
            async move {
                Ok::<_, Infallible>(
                    suprnova::server::handle_request(
                        router,
                        Arc::new(MiddlewareRegistry::new()),
                        request,
                    )
                    .await,
                )
            }
        });
        hyper::server::conn::http1::Builder::new()
            .serve_connection(hyper_util::rt::TokioIo::new(io), service)
            .await
    }));
    let (mut sender, connection) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(
        hyper_util::rt::TokioIo::new(client),
    )
    .await
    .expect("handshake");
    let client_task = tokio::spawn(connection);
    let request = hyper::Request::builder()
        .method(if kind == "head" { "HEAD" } else { "GET" })
        .uri("/delta-after-response")
        .header("Host", "localhost")
        .body(Full::new(Bytes::new()))
        .expect("request");
    let response = sender
        .send_request(request)
        .await
        .expect("headers already written");
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    assert_eq!(
        body,
        match kind {
            "empty" | "head" | "no-content" | "not-modified" => "",
            "stream" => "firstlast",
            _ => "sent",
        }
    );
    wait_for(|| gate.polled.load(Ordering::SeqCst)).await;
    assert_eq!(
        RUNS.load(Ordering::SeqCst),
        0,
        "pending socket flush must hold the command"
    );
    gate.open();
    if fail_flush {
        assert!(
            tokio::time::timeout(Duration::from_secs(5), task)
                .await
                .expect("connection exits")
                .expect("join")
                .is_err()
        );
        tokio::task::yield_now().await;
        assert_eq!(
            RUNS.load(Ordering::SeqCst),
            0,
            "failed flush must discard commands"
        );
    } else {
        wait_for(|| RUNS.load(Ordering::SeqCst) == if kind == "failed-command" { 2 } else { 1 })
            .await;
        drop(sender);
        task.abort();
    }
    client_task.abort();
}

#[tokio::test]
#[serial]
async fn buffered_response_dispatch_waits_for_successful_socket_flush() {
    response_must_flush_before_dispatch("buffered", false).await;
}
#[tokio::test]
#[serial]
async fn streamed_response_dispatch_waits_for_successful_socket_flush() {
    response_must_flush_before_dispatch("stream", false).await;
}
#[tokio::test]
#[serial]
async fn streamed_response_with_content_length_dispatches_after_successful_socket_flush() {
    response_must_flush_before_dispatch("stream-content-length", false).await;
}
#[tokio::test]
#[serial]
async fn empty_response_dispatch_waits_for_successful_socket_flush() {
    response_must_flush_before_dispatch("empty", false).await;
}
#[tokio::test]
#[serial]
async fn head_response_dispatch_waits_for_successful_socket_flush() {
    response_must_flush_before_dispatch("head", false).await;
}
#[tokio::test]
#[serial]
async fn failed_socket_flush_discards_after_response_commands() {
    response_must_flush_before_dispatch("buffered", true).await;
    response_must_flush_before_dispatch("stream-content-length", true).await;
}

#[tokio::test]
#[serial]
#[tracing_test::traced_test]
async fn a_failed_after_response_command_is_logged_and_the_next_still_runs() {
    response_must_flush_before_dispatch("failed-command", false).await;
    assert!(logs_contain("after-response command failed"));
}

#[tokio::test]
#[serial]
async fn no_content_response_still_dispatches_after_its_headers_flush() {
    response_must_flush_before_dispatch("no-content", false).await;
}

#[tokio::test]
#[serial]
async fn not_modified_response_still_dispatches_after_its_headers_flush() {
    response_must_flush_before_dispatch("not-modified", false).await;
}
