//! Shared plumbing for the framework's built-in vendor REST drivers.
//!
//! These are the drivers that talk to a third-party HTTP API on the
//! framework's behalf — the HTTP mail providers (Postmark, SES, SendGrid,
//! Resend, Mailgun) and the Pinecone vector driver. They deliberately use
//! `reqwest` directly rather than the [`Http`](super::Http) facade: the
//! facade's fakes are task-local, and a driver invoked from a queue worker
//! or a background task would silently escape a test's fake scope. A driver
//! that hits the network from a spawned task should do so predictably.
//!
//! What they *should* share is the safety envelope, which is what lives
//! here: bounded timeouts so a black-holed peer can't hold an `await`
//! forever, and a capped read of error bodies so a hostile or misconfigured
//! peer can't force an unbounded buffer.

use reqwest::Client;
use std::time::Duration;

/// Per-request total timeout for vendor REST calls. Matches the
/// `suprnova-web-push` `DEFAULT_REQUEST_TIMEOUT` so the entire framework
/// uses one upper bound on outbound provider calls.
pub(crate) const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Connect-only timeout for vendor REST calls. A separate, shorter budget
/// so a black-holed TLS handshake fails fast rather than burning the entire
/// request budget.
pub(crate) const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Maximum number of error-response body bytes a vendor driver will buffer
/// before dropping the rest. Every one of these drivers takes an
/// operator-overridable endpoint (`MAIL_<PROVIDER>_ENDPOINT`,
/// `PINECONE_CONTROLLER_HOST`), so the peer is not strictly trusted;
/// capping the diagnostic snippet stops a hostile or misconfigured server
/// from forcing an unbounded read into RAM. Matches the
/// `suprnova-web-push` client's 8 KiB cap.
pub(crate) const MAX_ERROR_BODY_BYTES: usize = 8 * 1024;

/// Build a connection-pooled, rustls-backed client carrying the framework's
/// standard request and connect timeouts.
///
/// `user_agent` identifies the calling subsystem (`suprnova-mail/1.2.3`)
/// rather than the framework as a whole, so a provider-side rate limit or
/// abuse report points at the right driver.
pub(crate) fn build_client(user_agent: &'static str) -> Client {
    Client::builder()
        .user_agent(user_agent)
        .timeout(DEFAULT_REQUEST_TIMEOUT)
        .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
        .build()
        .expect("reqwest client builder")
}

/// Stream and accumulate up to [`MAX_ERROR_BODY_BYTES`] of an error
/// response body, then drop the response so the remainder is not buffered.
/// The returned string is UTF-8-lossy — a provider may emit arbitrary
/// bytes, but the snippet is for diagnostic surfacing only.
///
/// Dropping the response once the cap is reached closes the connection (or
/// returns it to the pool) so a hostile peer can't hold the socket open by
/// dribbling more bytes after we've stopped reading.
pub(crate) async fn read_error_body(resp: reqwest::Response) -> String {
    read_capped_body(resp, MAX_ERROR_BODY_BYTES).await
}

async fn read_capped_body(mut resp: reqwest::Response, cap: usize) -> String {
    let mut buf: Vec<u8> = Vec::new();
    while buf.len() < cap {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                let remaining = cap - buf.len();
                let take = remaining.min(chunk.len());
                buf.extend_from_slice(&chunk[..take]);
                if buf.len() >= cap {
                    break;
                }
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }
    drop(resp);
    String::from_utf8_lossy(&buf).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An oversized error body is truncated to the cap rather than read in
    /// full. We drive `read_capped_body` against a local server that
    /// streams far more than the cap and assert the buffered snippet is
    /// exactly `cap` bytes long.
    #[tokio::test]
    async fn oversized_error_body_is_truncated_to_cap() {
        use std::io::Write;
        use std::net::TcpListener;

        const CAP: usize = 64;
        // Body twice the cap so a correct reader stops well before EOF.
        let body_len = CAP * 2;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");

        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            // Drain the request line/headers so the client's write completes.
            {
                use std::io::Read;
                let mut probe = [0u8; 1024];
                let _ = stream.read(&mut probe);
            }
            let response = format!(
                "HTTP/1.1 500 Internal Server Error\r\nContent-Type: text/plain\r\nContent-Length: {body_len}\r\nConnection: close\r\n\r\n{}",
                "x".repeat(body_len)
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        });

        let resp = reqwest::Client::new()
            .get(format!("http://{addr}/"))
            .send()
            .await
            .expect("send");
        assert_eq!(resp.status().as_u16(), 500);

        let snippet = read_capped_body(resp, CAP).await;
        assert_eq!(
            snippet.len(),
            CAP,
            "body must be truncated to the cap, not read in full"
        );
        assert!(snippet.bytes().all(|b| b == b'x'));

        handle.join().expect("server thread");
    }

    /// A body smaller than the cap is returned whole.
    #[tokio::test]
    async fn undersized_error_body_is_returned_whole() {
        use std::io::Write;
        use std::net::TcpListener;

        const CAP: usize = 8 * 1024;
        let body = "boom";

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");

        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            {
                use std::io::Read;
                let mut probe = [0u8; 1024];
                let _ = stream.read(&mut probe);
            }
            let response = format!(
                "HTTP/1.1 422 Unprocessable Entity\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        });

        let resp = reqwest::Client::new()
            .get(format!("http://{addr}/"))
            .send()
            .await
            .expect("send");

        let snippet = read_capped_body(resp, CAP).await;
        assert_eq!(snippet, "boom");

        handle.join().expect("server thread");
    }
}
