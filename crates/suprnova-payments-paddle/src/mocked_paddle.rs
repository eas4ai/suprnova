//! A mocked Paddle API for the tests of the adapter.
//!
//! Each connection is answered with the next canned response, and the
//! request is written to the log before the response goes out. The adapter
//! reads the response before it returns, so every request it made is in the
//! log by the time its call returns: a test reads the log without waiting.
//! A request past the canned responses finds no listener and fails.
//!
//! [`stalled`] is the other shape: a Paddle that takes the connection and
//! never answers, with the paused clock moved past the deadline.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use paddle_rust_sdk::Paddle;
use serde_json::{Value, json};
use suprnova::payments::PaymentResult;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

use crate::deadline::REQUEST_TIMEOUT;
use crate::{PaddleEnvironment, PaddleProvider};

/// One request as the mocked Paddle API received it.
pub(crate) struct Captured {
    /// The request line, `PATCH /customers/ctm_test HTTP/1.1`.
    pub(crate) request_line: String,
    /// The body, empty for a request without one.
    pub(crate) body: String,
}

impl Captured {
    /// The body as JSON, which is what the SDK sends.
    pub(crate) fn json(&self) -> Value {
        serde_json::from_str(&self.body).expect("Paddle request body is JSON")
    }
}

/// The requests the mocked Paddle API received, in order.
pub(crate) type RequestLog = Arc<Mutex<Vec<Captured>>>;

/// A provider that talks to a mocked Paddle API answering with
/// `responses`, one `(status, body)` per request, and the log of what it
/// was sent.
pub(crate) async fn paddle_with(responses: Vec<(u16, Value)>) -> (PaddleProvider, RequestLog) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mocked Paddle API");
    let address = listener.local_addr().expect("mocked Paddle API address");
    let log = RequestLog::default();
    let server_log = Arc::clone(&log);
    tokio::spawn(async move {
        for (status, body) in responses {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let request = read_request(&mut stream).await;
            server_log.lock().expect("request log").push(request);
            let body = body.to_string();
            let response = format!(
                "HTTP/1.1 {status} Mocked\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write mocked Paddle response");
        }
    });
    (provider_at(address), log)
}

/// Run `call` against a Paddle API that takes the connection and never
/// answers, move the paused clock past the deadline, and return what the
/// call returned. Nothing waits in real time: the clock is paused before it
/// moves.
pub(crate) async fn stalled<T, F, Fut>(call: F) -> PaymentResult<T>
where
    F: FnOnce(PaddleProvider) -> Fut,
    Fut: Future<Output = PaymentResult<T>> + Send + 'static,
    T: Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind stalled Paddle API");
    let address = listener.local_addr().expect("stalled Paddle API address");
    let task = tokio::spawn(call(provider_at(address)));
    let (stream, _) = timeout(Duration::from_secs(5), listener.accept())
        .await
        .expect("the call reaches Paddle")
        .expect("accept the call");
    tokio::time::pause();
    tokio::time::advance(REQUEST_TIMEOUT + Duration::from_secs(1)).await;
    let result = timeout(Duration::from_secs(1), task)
        .await
        .expect("the deadline ends the call")
        .expect("the task of the call");
    tokio::time::resume();
    drop(stream);
    result
}

/// A provider that talks to the mocked Paddle API at `address`.
fn provider_at(address: SocketAddr) -> PaddleProvider {
    PaddleProvider {
        client: Arc::new(
            Paddle::new("pdl_sdbx_apikey_test", format!("http://{address}"))
                .expect("mocked Paddle API URL"),
        ),
        webhook_key: "pdl_ntfset_test".into(),
        client_token: "test_client_token".into(),
        environment: PaddleEnvironment::Sandbox,
    }
}

/// The body Paddle answers a failed request with, for the error `code`.
pub(crate) fn paddle_error(code: &str) -> Value {
    paddle_error_with_detail(code, "mocked Paddle error")
}

/// The body Paddle answers a failed request with, for the error `code` and
/// the human-readable `detail`, which Paddle fills with the entity it names.
pub(crate) fn paddle_error_with_detail(code: &str, detail: &str) -> Value {
    json!({
        "error": {
            "type": "request_error",
            "code": code,
            "detail": detail,
            "documentation_url": format!("https://developer.paddle.com/v1/errors/shared/{code}"),
        },
        "meta": {"request_id": "test-request"}
    })
}

/// A provider whose Paddle API refuses every connection: the listener is
/// closed before the first request.
pub(crate) async fn unreachable() -> PaddleProvider {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mocked Paddle API");
    let address = listener.local_addr().expect("mocked Paddle API address");
    drop(listener);
    provider_at(address)
}

/// Read one HTTP request: the head, then as many body bytes as its
/// `content-length` names.
async fn read_request(stream: &mut TcpStream) -> Captured {
    let mut bytes = Vec::new();
    let head_end = loop {
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).await.expect("read Paddle request");
        assert!(read > 0, "Paddle request ended before it was complete");
        bytes.extend_from_slice(&chunk[..read]);
        assert!(bytes.len() < 65536, "unexpectedly large Paddle request");
        let Some(head_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let content_length = String::from_utf8_lossy(&bytes[..head_end])
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .map(|(_, value)| {
                value
                    .trim()
                    .parse::<usize>()
                    .expect("numeric content-length")
            })
            .unwrap_or(0);
        if bytes.len() >= head_end + 4 + content_length {
            break head_end;
        }
    };
    let head = String::from_utf8(bytes[..head_end].to_vec()).expect("request head is UTF-8");
    Captured {
        request_line: head.lines().next().unwrap_or_default().to_string(),
        body: String::from_utf8(bytes[head_end + 4..].to_vec())
            .expect("Paddle request body is UTF-8"),
    }
}
