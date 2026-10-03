//! MEM-002 and MEM-003 on the HTTP paths.

use std::time::Duration;

use http_body_util::BodyExt;
use suprnova::http::text;
use suprnova::{
    CorsConfig, CorsMiddleware, HttpResponse, InertiaResponse, MiddlewareRegistry, Request,
    Response, Router,
};

use crate::support::{Heap, exchange, exclusive, get_request, serve, status};

async fn drain(response: HttpResponse) -> usize {
    let mut body = response.into_hyper().into_body();
    let mut read = 0;
    while let Some(frame) = body.frame().await {
        if let Ok(data) = frame.expect("a frame").into_data() {
            read += data.len();
        }
    }
    read
}

/// MEM-003: streaming a file allocates one buffer per chunk it sends,
/// none for a read that is not ready yet.
#[tokio::test]
async fn mem_audit_streaming_a_file_allocates_once_per_chunk() {
    let _lock = exclusive().await;
    const SIZE: usize = 4 * 1024 * 1024;
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("big.bin");
    std::fs::write(&path, vec![7u8; SIZE]).expect("write the file");
    drain(HttpResponse::file(&path, None).await.expect("a response")).await;

    let heap = Heap::start();
    let response = HttpResponse::file(&path, None).await.expect("a response");
    let before = heap.bytes();
    let read = drain(response).await;
    let used = heap.bytes() - before;
    assert_eq!(read, SIZE);
    assert!(
        used < (SIZE as u64) * 3 / 2,
        "streaming {SIZE} bytes allocated {used} bytes"
    );
}

/// MEM-003: the first Inertia page is written into one buffer, so it
/// allocates about what the same page as an Inertia request does.
#[tokio::test]
async fn mem_audit_the_first_inertia_page_is_built_in_one_buffer() {
    let _lock = exclusive().await;
    const PROP: usize = 1024 * 1024 - 4096;
    let blob = "a".repeat(PROP);
    let router = Router::new().get("/page", move |req: Request| {
        let blob = blob.clone();
        async move {
            let response: Response = InertiaResponse::new("Page")
                .with("blob", blob)
                .resolve(&req)
                .await
                .map_err(HttpResponse::from);
            response
        }
    });
    let addr = serve(router, MiddlewareRegistry::new()).await;
    let html = get_request("/page", &[("Accept", "text/html")]);
    let xhr = get_request(
        "/page",
        &[
            ("X-Inertia", "true"),
            ("Accept", "text/html, application/xhtml+xml"),
        ],
    );
    for request in [&html, &xhr] {
        let (_, head) = exchange(addr, request).await;
        assert_eq!(status(&head), 200, "{}", String::from_utf8_lossy(&head));
    }

    let heap = Heap::start();
    let before = heap.bytes();
    exchange(addr, &html).await;
    let page = heap.bytes() - before;
    let before = heap.bytes();
    exchange(addr, &xhr).await;
    let request = heap.bytes() - before;
    assert!(
        page < request + PROP as u64,
        "the first page allocated {page} bytes, the Inertia request {request}"
    );
}

fn cors(paths: &[&str]) -> MiddlewareRegistry {
    MiddlewareRegistry::new().append(CorsMiddleware::new(
        CorsConfig::allow_origins(["https://app.example"])
            .max_age(Duration::from_secs(600))
            .paths(paths.iter().copied()),
    ))
}

/// MEM-003: a wildcard CORS path is compiled once per configuration, not
/// once per request.
#[tokio::test]
async fn mem_audit_a_wildcard_cors_path_is_compiled_once() {
    let _lock = exclusive().await;
    let router = || Router::new().get("/api/data", |_req| async { text("data") });
    let wildcard = serve(router(), cors(&["api/*"])).await;
    let exact = serve(router(), cors(&["api/data"])).await;
    let request = get_request("/api/data", &[("Origin", "https://app.example")]);
    for _ in 0..20 {
        exchange(wildcard, &request).await;
        exchange(exact, &request).await;
    }

    let heap = Heap::start();
    let before = heap.blocks();
    for _ in 0..200 {
        exchange(wildcard, &request).await;
    }
    let with_wildcard = heap.blocks() - before;
    let before = heap.blocks();
    for _ in 0..200 {
        exchange(exact, &request).await;
    }
    let with_exact = heap.blocks() - before;
    assert!(
        with_wildcard < with_exact + 200,
        "a wildcard path allocated {with_wildcard} blocks over 200 requests, an exact one {with_exact}"
    );
}

/// MEM-002: a body that arrives in several frames keeps no room beyond
/// its length.
#[tokio::test]
async fn mem_audit_a_body_in_several_frames_holds_only_its_length() {
    let _lock = exclusive().await;
    let router = Router::new().post("/body", |req: Request| async move {
        let (_, bytes) = req.body_bytes().await.expect("the body");
        let length = bytes.len();
        let capacity = bytes.try_into_mut().map_or(0, |owned| owned.capacity());
        text(format!("{length} {capacity}"))
    });
    let addr = serve(router, MiddlewareRegistry::new()).await;
    let mut request = b"POST /body HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Type: application/octet-stream\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
    for _ in 0..3 {
        request.extend_from_slice(b"3e8\r\n");
        request.extend_from_slice(&[b'a'; 1000]);
        request.extend_from_slice(b"\r\n");
    }
    request.extend_from_slice(b"0\r\n\r\n");
    let (read, head) = exchange(addr, &request).await;
    let response = String::from_utf8_lossy(&head[..read.min(head.len())]).into_owned();
    assert!(
        response.ends_with("3000 3000"),
        "the body is not held at its length: {response}"
    );
}
