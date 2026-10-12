//! HRD-001: the server bounds how long a request body takes to arrive.
//!
//! The header-read timeout covers the request head only. Before the body
//! deadlines, a client that sent complete headers and then trickled the
//! body, or sent part of it and went quiet, held its connection for as long
//! as it liked: every reader of the body awaited each frame with no
//! deadline. Each test here boots a real `Server` whose deadlines come from
//! a registered `ServerConfig`, talks to it over a raw TCP socket, and
//! checks what a slow body gets: `408 Request Timeout` and a closed
//! connection, at the deadline that applies.
//!
//! `Server::run` boots process-wide singletons and reads the process-wide
//! `ServerConfig`, so each scenario runs alone in a child process
//! (`own_process_async::delegate`), as the header-read timeout test does.

use std::time::{Duration, Instant};

use suprnova::config::{Config, ServerConfig};
use suprnova::{HttpResponse, Request, Router, Server};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::env_snapshot::{EnvSnapshot, set_env};

/// How long a test waits for the server's answer before it calls the
/// connection held open. Well above every deadline these tests configure,
/// far below "indefinitely", so a regression fails instead of hanging.
const PATIENCE: Duration = Duration::from_secs(10);

/// A connection to a booted server, and the environment the server was
/// booted in, restored when the test ends.
struct Served {
    stream: TcpStream,
    _app_env: EnvSnapshot,
    _env: tokio::sync::MutexGuard<'static, ()>,
}

/// Boot a server on a free loopback port whose body deadlines are `read`
/// (per frame) and `total`, set through the registered `ServerConfig`, and
/// return a connection to it.
///
/// `POST /echo` answers the body it read as text; `POST /form` reads a
/// `multipart/form-data` body through the streaming multipart parser and
/// answers its fields as JSON.
async fn serve(read: Duration, total: Duration) -> Served {
    // The server requires `APP_KEY` outside development, and the drivers it
    // boots read an unset `APP_ENV` as production; this process is a test.
    let env = crate::env_lock::lock_env_async().await;
    let app_env = EnvSnapshot::capture(&["APP_ENV"]);
    set_env("APP_ENV", Some("testing"));

    // A free port: bind ephemeral, read the port back, release it for the
    // server. The window between is the standard trade for a loopback test.
    let probe = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind probe listener");
    let port = probe.local_addr().expect("probe local_addr").port();
    drop(probe);

    Config::register(
        ServerConfig::builder()
            .host("127.0.0.1")
            .port(port)
            .body_read_timeout(read)
            .body_total_timeout(total)
            .build(),
    );

    let router: Router = Router::new()
        .post("/echo", |req: Request| async move {
            let (_, body) = req.body_bytes().await?;
            Ok(HttpResponse::text(
                String::from_utf8_lossy(&body).into_owned(),
            ))
        })
        .post("/form", |req: Request| async move {
            let fields: serde_json::Value = req.input().await?;
            Ok(HttpResponse::json(fields))
        })
        .into();
    let server = Server::from_config(router)
        .expect("Server::from_config must succeed in a testing environment");
    tokio::spawn(async move {
        // The runtime ends with the test and aborts this task. A boot that
        // fails says why, rather than leaving only the connect loop's panic.
        if let Err(error) = server.run().await {
            eprintln!("Server::run failed: {error}");
        }
    });

    for _ in 0..250 {
        if let Ok(stream) = TcpStream::connect(("127.0.0.1", port)).await {
            return Served {
                stream,
                _app_env: app_env,
                _env: env,
            };
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the server did not start accepting connections on port {port}");
}

/// Read until the server closes the connection, bounded by [`PATIENCE`].
/// Returns what arrived and how long the read waited.
async fn read_until_closed(stream: &mut TcpStream) -> (String, Duration) {
    let started = Instant::now();
    let mut answer = Vec::new();
    let read = tokio::time::timeout(PATIENCE, stream.read_to_end(&mut answer))
        .await
        .unwrap_or_else(|_| {
            panic!(
                "the server held the connection open for {PATIENCE:?}; it had answered {:?}",
                String::from_utf8_lossy(&answer)
            )
        });
    read.expect("reading until the server closes");
    (
        String::from_utf8_lossy(&answer).into_owned(),
        started.elapsed(),
    )
}

/// Assert `answer` is a `408` that tells the client the connection closes.
fn assert_abandoned(answer: &str) {
    assert!(
        answer.starts_with("HTTP/1.1 408"),
        "a body that misses its deadline answers 408 Request Timeout; got:\n{answer}"
    );
    assert!(
        answer
            .to_ascii_lowercase()
            .contains("\r\nconnection: close\r\n"),
        "the 408 tells the client the connection closes; got:\n{answer}"
    );
}

/// The head of a request for `path` declaring a body of `length` bytes.
fn head(path: &str, content_type: &str, length: usize) -> String {
    format!(
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: {content_type}\r\n\
         Content-Length: {length}\r\nConnection: close\r\n\r\n"
    )
}

/// The falsifier's client: complete headers, one byte of a five-byte body,
/// then silence. The per-frame deadline answers it.
#[tokio::test]
async fn hardening_a_trickled_body_is_abandoned_at_the_deadline() {
    if crate::own_process_async::delegate(
        module_path!(),
        "hardening_a_trickled_body_is_abandoned_at_the_deadline",
    )
    .await
    {
        return;
    }
    let read = Duration::from_millis(300);
    let mut served = serve(read, Duration::from_secs(60)).await;

    served
        .stream
        .write_all(head("/echo", "text/plain", 5).as_bytes())
        .await
        .expect("write the head");
    served.stream.write_all(b"a").await.expect("write one byte");

    let (answer, waited) = read_until_closed(&mut served.stream).await;
    assert_abandoned(&answer);
    assert!(
        waited >= read - Duration::from_millis(50),
        "the body is abandoned at its deadline, not before; answered after {waited:?}"
    );
}

/// A body that keeps arriving, a byte at a time and each byte inside the
/// per-frame deadline, is still abandoned at the total deadline.
#[tokio::test]
async fn hardening_a_trickling_body_is_abandoned_at_the_total_deadline() {
    if crate::own_process_async::delegate(
        module_path!(),
        "hardening_a_trickling_body_is_abandoned_at_the_total_deadline",
    )
    .await
    {
        return;
    }
    let read = Duration::from_secs(5);
    let total = Duration::from_millis(800);
    let mut served = serve(read, total).await;

    served
        .stream
        .write_all(head("/echo", "text/plain", 20).as_bytes())
        .await
        .expect("write the head");
    // Five bytes, 100 ms apart, then silence: the last byte lands well
    // before the total deadline, and the per-frame deadline after it is
    // five seconds away. Nothing is in flight when the server closes.
    for _ in 0..5 {
        served.stream.write_all(b"a").await.expect("write one byte");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let (answer, _) = read_until_closed(&mut served.stream).await;
    assert_abandoned(&answer);
}

/// The control: a body that arrives in two parts, the gap inside the
/// per-frame deadline, is read whole.
#[tokio::test]
async fn hardening_a_body_that_arrives_in_time_is_read() {
    if crate::own_process_async::delegate(
        module_path!(),
        "hardening_a_body_that_arrives_in_time_is_read",
    )
    .await
    {
        return;
    }
    let mut served = serve(Duration::from_secs(3), Duration::from_secs(10)).await;

    served
        .stream
        .write_all(head("/echo", "text/plain", 5).as_bytes())
        .await
        .expect("write the head");
    served
        .stream
        .write_all(b"hel")
        .await
        .expect("write the start");
    tokio::time::sleep(Duration::from_millis(200)).await;
    served
        .stream
        .write_all(b"lo")
        .await
        .expect("write the rest");

    let (answer, _) = read_until_closed(&mut served.stream).await;
    assert!(
        answer.starts_with("HTTP/1.1 200"),
        "a body that arrives in time is read; got:\n{answer}"
    );
    assert!(
        answer.ends_with("\r\n\r\nhello"),
        "the handler reads the whole body; got:\n{answer}"
    );
}

/// The streaming multipart parser reads the body under the same
/// deadlines: a form whose body stops mid-part is abandoned.
#[tokio::test]
async fn hardening_a_trickled_multipart_body_is_abandoned_at_the_deadline() {
    if crate::own_process_async::delegate(
        module_path!(),
        "hardening_a_trickled_multipart_body_is_abandoned_at_the_deadline",
    )
    .await
    {
        return;
    }
    let read = Duration::from_millis(300);
    let mut served = serve(read, Duration::from_secs(60)).await;

    let body = "--hrd\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nslow\r\n--hrd--\r\n";
    served
        .stream
        .write_all(head("/form", "multipart/form-data; boundary=hrd", body.len()).as_bytes())
        .await
        .expect("write the head");
    served
        .stream
        .write_all(&body.as_bytes()[..20])
        .await
        .expect("write the start of the form");

    let (answer, waited) = read_until_closed(&mut served.stream).await;
    assert_abandoned(&answer);
    assert!(
        waited >= read - Duration::from_millis(50),
        "the form is abandoned at its deadline, not before; answered after {waited:?}"
    );
}

/// The multipart control: the same form, sent in two parts inside the
/// per-frame deadline, is parsed.
#[tokio::test]
async fn hardening_a_multipart_body_that_arrives_in_time_is_read() {
    if crate::own_process_async::delegate(
        module_path!(),
        "hardening_a_multipart_body_that_arrives_in_time_is_read",
    )
    .await
    {
        return;
    }
    let mut served = serve(Duration::from_secs(3), Duration::from_secs(10)).await;

    let body = "--hrd\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nslow\r\n--hrd--\r\n";
    served
        .stream
        .write_all(head("/form", "multipart/form-data; boundary=hrd", body.len()).as_bytes())
        .await
        .expect("write the head");
    served
        .stream
        .write_all(&body.as_bytes()[..20])
        .await
        .expect("write the start of the form");
    tokio::time::sleep(Duration::from_millis(200)).await;
    served
        .stream
        .write_all(&body.as_bytes()[20..])
        .await
        .expect("write the rest of the form");

    let (answer, _) = read_until_closed(&mut served.stream).await;
    assert!(
        answer.starts_with("HTTP/1.1 200"),
        "a form that arrives in time is parsed; got:\n{answer}"
    );
    assert!(
        answer.ends_with(r#"{"title":"slow"}"#),
        "the handler reads every field; got:\n{answer}"
    );
}
