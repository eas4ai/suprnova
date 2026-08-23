//! End-to-end test: an image pipeline served straight out of a handler.
//!
//! Drives `Image::to_response()` through the real `handle_request` adapter on
//! a live hyper connection, so it covers the parts an in-process driver test
//! cannot: that the `Content-Type` the subsystem picks survives onto the
//! wire, and that the bytes a browser would receive are a real, decodable
//! image of the requested size.
//!
//! The fixture is a 1x1 red PNG byte literal, upscaled by the pipeline
//! itself - the framework's own subsystem is the only image dependency this
//! test has.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Empty};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;

use suprnova::image::Image;
use suprnova::{MiddlewareRegistry, OutputFormat, Response, get, handle_request, handler, routes};

/// 1x1 red PNG (verified: `file` reports `PNG image data, 1 x 1, 8-bit/color
/// RGB, non-interlaced`).
const RED_PNG_1X1: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
    0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0xF8, 0xCF, 0xC0, 0x00,
    0x00, 0x03, 0x01, 0x01, 0x00, 0xF7, 0x03, 0x41, 0x43, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E,
    0x44, 0xAE, 0x42, 0x60, 0x82,
];

/// The shape an app actually writes: build the pipeline, hand back the
/// response, let the subsystem set the content type.
#[handler]
async fn thumbnail() -> Response {
    Ok(Image::from_bytes(RED_PNG_1X1)
        .resize(48, 24)
        .to_format(OutputFormat::WebP)
        .to_response()
        .await?)
}

routes! {
    get!("/thumbnail.webp", thumbnail),
}

async fn spawn(accepts: usize) -> SocketAddr {
    let router = Arc::new(register());
    let middleware = Arc::new(MiddlewareRegistry::new());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");

    tokio::spawn(async move {
        for _ in 0..accepts {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let io = TokioIo::new(stream);
            let router = router.clone();
            let middleware = middleware.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req: hyper::Request<Incoming>| {
                    let router = router.clone();
                    let middleware = middleware.clone();
                    async move { Ok::<_, Infallible>(handle_request(router, middleware, req).await) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, svc)
                    .await;
            });
        }
    });

    addr
}

async fn fetch(
    addr: SocketAddr,
    path: &str,
) -> (hyper::http::StatusCode, Vec<(String, String)>, Bytes) {
    let stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connect to test server");
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake::<_, Empty<Bytes>>(io)
        .await
        .expect("client handshake");
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let request = hyper::Request::builder()
        .method("GET")
        .uri(path)
        .header("Host", "localhost")
        .body(Empty::<Bytes>::new())
        .expect("build request");

    let response = tokio::time::timeout(Duration::from_secs(30), sender.send_request(request))
        .await
        .expect("request timed out")
        .expect("send request");

    let status = response.status();
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_string(),
                value.to_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    (status, headers, body)
}

#[tokio::test]
async fn image_pipeline_serves_a_decodable_webp_with_the_right_content_type() {
    let addr = spawn(1).await;
    let (status, headers, body) = fetch(addr, "/thumbnail.webp").await;

    assert_eq!(status, 200, "the handler must succeed");

    let content_type = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.as_str())
        .expect("a Content-Type must reach the wire");
    assert_eq!(
        content_type, "image/webp",
        "the converted format must set the content type, not the source format"
    );

    assert!(!body.is_empty(), "the response body must carry the image");
    assert!(
        body.starts_with(b"RIFF"),
        "the body must be a real WebP container, got {:02x?}",
        &body[..body.len().min(12)]
    );

    // The bytes a browser receives must decode back to the size that was
    // asked for - this is what makes it an end-to-end assertion rather than
    // a content-type assertion.
    let (width, height) = Image::from_bytes(body.to_vec())
        .dimensions()
        .await
        .expect("the served bytes must decode");
    assert_eq!((width, height), (48, 24));
}
