//! SEC-07: the header-read timeout must actually be enforced.
//!
//! Hyper documents a 30s default `header_read_timeout`, but that default
//! only arms when a `Timer` is installed on the connection builder. Before
//! `Server::run` started installing `hyper_util::rt::TokioTimer`, the
//! "default" was silently inert — hyper logged a warning and enforced
//! nothing, so a client that opened a connection and never completed its
//! request head could hold it open indefinitely (a slowloris-style
//! exhaustion; worse when `SERVER_MAX_CONNECTIONS` is set, since the
//! stalled connection also pins a semaphore permit forever).
//!
//! `Server::run` boots telemetry, cache, queue, mail, and rate-limit
//! drivers as process-wide singletons, so — like the other boot-time
//! process-global tests in this crate (see
//! `app_key_production_fail_closed.rs`) — this scenario lives in its own
//! test binary rather than sharing one with unrelated tests.

use std::time::Duration;
use suprnova::{Router, Server};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn incomplete_request_head_is_closed_within_the_configured_deadline() {
    // Learn a free port by binding ephemeral (`:0`) and reading back the
    // OS-assigned port, then release it immediately so `Server::run` can
    // bind the same address. Small TOCTOU window, but this is the
    // standard trick for handing a test server a free port and is good
    // enough for a local-loopback test.
    let probe = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind probe listener");
    let port = probe.local_addr().expect("probe local_addr").port();
    drop(probe);

    // Keep the deadline short so the suite stays fast — the production
    // default (unset `SERVER_HEADER_READ_TIMEOUT`) is 30s.
    let deadline = Duration::from_millis(300);

    let server = Server::from_config(Router::new())
        .expect("Server::from_config must succeed in a bare local-environment test process")
        .host("127.0.0.1")
        .port(port)
        .header_read_timeout(deadline);

    tokio::spawn(async move {
        // The test ends (and the per-test tokio runtime tears down,
        // aborting this task) long before any graceful shutdown signal
        // would arrive — that's fine, there's nothing to clean up.
        let _ = server.run().await;
    });

    // Poll for the accept loop coming up instead of a fixed sleep, so
    // this isn't flaky on a loaded machine.
    let mut stream = None;
    for _ in 0..100 {
        match TcpStream::connect(("127.0.0.1", port)).await {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
        }
    }
    let mut stream = stream.expect("server should start accepting connections promptly");

    // Deliberately incomplete request head: a request line + one header,
    // no terminating CRLFCRLF. Mirrors the slowloris probe shape (partial
    // head, then silence) without ever completing the request.
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n")
        .await
        .expect("write partial request head");

    // If the header-read deadline is active, hyper closes the connection
    // (clean FIN) once `deadline` elapses without a complete head. If the
    // deadline is inert (the SEC-07 bug), this read just hangs forever —
    // bound the wait well above `deadline` so the assertion is reliable,
    // but far short of "indefinitely" so a regression fails fast instead
    // of hanging the suite.
    let mut buf = [0u8; 8];
    let read_result = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf))
        .await
        .expect(
            "server must close a connection with an incomplete request head within the \
             configured header-read timeout, not hold it open indefinitely (SEC-07)",
        );
    let n = read_result.expect("reading from a server-closed connection should not error");
    assert_eq!(
        n, 0,
        "expected EOF (server-initiated close) after the header-read timeout fires, got {n} bytes"
    );
}
