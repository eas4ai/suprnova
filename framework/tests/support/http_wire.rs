//! Raw HTTP/1.1 client for tests that serve a router on a real socket.
//!
//! Driving the server through a genuine connection exercises the whole
//! hyper path, which the in-process `handle_request` adapter skips.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper_util::rt::TokioIo;

/// Send a request and return `(status, lowercased response headers, body)`.
pub async fn request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
) -> (u16, HashMap<String, String>, String) {
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(io)
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let mut builder = hyper::Request::builder()
        .method(method)
        .uri(path)
        .header("Host", "localhost")
        .header("Content-Length", "0");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let req = builder.body(Full::new(Bytes::new())).unwrap();

    let resp = tokio::time::timeout(Duration::from_secs(5), sender.send_request(req))
        .await
        .expect("send_request timeout")
        .expect("hyper send_request");

    let (parts, body) = resp.into_parts();
    let status = parts.status.as_u16();
    let header_map = parts
        .headers
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_lowercase(),
                v.to_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    let bytes = body.collect().await.unwrap().to_bytes();
    (
        status,
        header_map,
        String::from_utf8_lossy(&bytes).to_string(),
    )
}
