#![cfg(all(feature = "filesystem", feature = "testing"))]

//! `Storage::download` and `Storage::response`: file responses for a path
//! on a named disk, resolved through the disk's own API.
//!
//! The local-filesystem disk's path guard must apply, so every traversal
//! test plants a real `secret.txt` one level ABOVE the disk root and
//! proves it never reaches the response. The in-memory disk stands in for
//! S3 and every other disk without local paths: the response has to read
//! through the disk, since there is no file to open.
//!
//! Each response is driven through the real `Router` and
//! `MiddlewareRegistry` via `handle_request`, so what is asserted is what
//! reaches the wire.

use crate::common::incoming_get_request;
use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::HeaderMap;
use std::sync::Arc;
use std::time::Duration;
use suprnova::{FrameworkError, HttpResponse, MiddlewareRegistry, Router, Storage, handle_request};

const WAIT: Duration = Duration::from_secs(10);

/// Answer one `GET path` through `handle_request` on a router whose only
/// route is `path`, and collect the whole body.
async fn serve<F, Fut>(path: &'static str, handler: F) -> (u16, HeaderMap, Bytes)
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = suprnova::Response> + Send + 'static,
{
    let handler = Arc::new(handler);
    let router: Router = Router::new()
        .get(path, move |_req| {
            let handler = handler.clone();
            async move { handler().await }
        })
        .into();
    let req = incoming_get_request(path, &[]).await;
    let resp = tokio::time::timeout(
        WAIT,
        handle_request(Arc::new(router), Arc::new(MiddlewareRegistry::new()), req),
    )
    .await
    .expect("handle_request timed out");
    let status = resp.status().as_u16();
    let (parts, body) = resp.into_parts();
    let body = tokio::time::timeout(WAIT, body.collect())
        .await
        .expect("collecting the body timed out")
        .expect("body collects")
        .to_bytes();
    (status, parts.headers, body)
}

/// `expect_err` for a file response: `HttpResponse` has no `Debug`, so the
/// standard one is not available.
trait ExpectError {
    fn expect_error(self, why: &str) -> FrameworkError;
}

impl ExpectError for Result<HttpResponse, FrameworkError> {
    fn expect_error(self, why: &str) -> FrameworkError {
        match self {
            Ok(response) => panic!("{why}; got status {}", response.status_code()),
            Err(error) => error,
        }
    }
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> &'a str {
    headers
        .get(name)
        .unwrap_or_else(|| panic!("response must carry `{name}`; it had {headers:?}"))
        .to_str()
        .expect("header is visible ASCII")
}

/// A local disk rooted at `<tmp>/root` holding `reports/q3.pdf`, with a
/// planted `<tmp>/secret.txt` one level ABOVE the root.
fn local_disk_with_outside_secret(name: &str) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let root = tmp.path().join("root");
    std::fs::create_dir_all(root.join("reports")).expect("create disk root");
    std::fs::write(root.join("reports/q3.pdf"), b"%PDF-1.7 q3").expect("write report");
    std::fs::write(tmp.path().join("secret.txt"), b"TOP SECRET").expect("plant secret");
    Storage::register_fs(name.to_string(), &root).expect("fs disk init");
    tmp
}

#[tokio::test]
async fn download_from_a_local_disk_defaults_to_the_file_own_name() {
    let _guard = Storage::fake();
    let _tmp = local_disk_with_outside_secret("local");

    let (status, headers, body) = serve("/q3", || async {
        Storage::download("local", "reports/q3.pdf", None)
            .await
            .map_err(HttpResponse::from)
    })
    .await;

    assert_eq!(status, 200);
    assert_eq!(header(&headers, "content-type"), "application/pdf");
    assert_eq!(
        header(&headers, "content-disposition"),
        "attachment; filename=\"q3.pdf\""
    );
    assert_eq!(header(&headers, "content-length"), "11");
    assert_eq!(&body[..], b"%PDF-1.7 q3");
}

#[tokio::test]
async fn response_from_a_local_disk_is_inline_with_an_optional_name() {
    let _guard = Storage::fake();
    let _tmp = local_disk_with_outside_secret("local");

    let (status, headers, body) = serve("/q3-inline", || async {
        Storage::response("local", "reports/q3.pdf", Some("Informe P\u{e9}rez.pdf"))
            .await
            .map_err(HttpResponse::from)
    })
    .await;

    assert_eq!(status, 200);
    assert_eq!(header(&headers, "content-type"), "application/pdf");
    assert_eq!(
        header(&headers, "content-disposition"),
        "inline; filename=\"Informe Perez.pdf\"; filename*=UTF-8''Informe%20P%C3%A9rez.pdf"
    );
    assert_eq!(&body[..], b"%PDF-1.7 q3");

    let response = Storage::response("local", "reports/q3.pdf", None)
        .await
        .unwrap();
    assert_eq!(
        response.header_value("Content-Disposition"),
        Some("inline; filename=\"q3.pdf\"")
    );
}

#[tokio::test]
async fn a_traversal_path_is_refused_by_the_disk_guard_and_never_read() {
    let _guard = Storage::fake();
    let _tmp = local_disk_with_outside_secret("local");

    for path in [
        "../secret.txt",
        "reports/../../secret.txt",
        "..\\secret.txt",
    ] {
        let err = Storage::download("local", path, None)
            .await
            .expect_error("a path outside the disk root must be refused");
        assert_eq!(err.status_code(), 403, "download of {path:?}");
        let err = Storage::response("local", path, None)
            .await
            .expect_error("a path outside the disk root must be refused");
        assert_eq!(err.status_code(), 403, "response of {path:?}");
    }

    let (status, _headers, body) = serve("/secret", || async {
        Storage::download("local", "../secret.txt", Some("secret.txt"))
            .await
            .map_err(HttpResponse::from)
    })
    .await;
    assert_eq!(status, 403);
    assert!(
        !String::from_utf8_lossy(&body).contains("TOP SECRET"),
        "the out-of-root file must never be read into the response"
    );
}

#[tokio::test]
async fn a_missing_path_or_a_directory_on_a_disk_is_a_404() {
    let _guard = Storage::fake();
    let _tmp = local_disk_with_outside_secret("local");

    let err = Storage::download("local", "reports/q4.pdf", None)
        .await
        .expect_error("a missing file must be an error, not a panic");
    assert_eq!(err.status_code(), 404);
    let err = Storage::response("local", "reports", None)
        .await
        .expect_error("a directory is not a file");
    assert_eq!(err.status_code(), 404);

    let (status, _headers, _body) = serve("/q4", || async {
        Storage::download("local", "reports/q4.pdf", None)
            .await
            .map_err(HttpResponse::from)
    })
    .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn an_unregistered_disk_is_an_error() {
    let _guard = Storage::fake();
    let err = Storage::download("nowhere", "a.pdf", None)
        .await
        .expect_error("an unregistered disk must be an error");
    assert_eq!(err.status_code(), 500);
}

#[tokio::test]
async fn a_disk_without_local_paths_is_read_through_the_disk() {
    let _guard = Storage::fake();
    Storage::register_memory("objects");
    let disk = Storage::disk("objects").unwrap();
    disk.write("img/logo.png", b"\x89PNG\r\n\x1a\nlogo".to_vec())
        .await
        .unwrap();
    disk.write(
        "blobs/upload.xyz",
        b"<svg onload=\"alert(1)\"></svg>".to_vec(),
    )
    .await
    .unwrap();

    let (status, headers, body) = serve("/logo", || async {
        Storage::response("objects", "img/logo.png", None)
            .await
            .map_err(HttpResponse::from)
    })
    .await;
    assert_eq!(status, 200);
    assert_eq!(header(&headers, "content-type"), "image/png");
    assert_eq!(
        header(&headers, "content-disposition"),
        "inline; filename=\"logo.png\""
    );
    assert_eq!(&body[..], b"\x89PNG\r\n\x1a\nlogo");

    let (status, headers, _body) = serve("/blob", || async {
        Storage::download("objects", "blobs/upload.xyz", Some("upload"))
            .await
            .map_err(HttpResponse::from)
    })
    .await;
    assert_eq!(status, 200);
    assert_eq!(header(&headers, "content-type"), "application/octet-stream");
    assert_eq!(
        header(&headers, "content-disposition"),
        "attachment; filename=\"upload\""
    );

    let err = Storage::download("objects", "img/missing.png", None)
        .await
        .expect_error("a missing object must be an error");
    assert_eq!(err.status_code(), 404);
}

#[tokio::test]
async fn a_large_file_on_a_disk_is_streamed_and_arrives_whole() {
    let _guard = Storage::fake();
    let tmp = tempfile::tempdir().unwrap();
    Storage::register_fs("local", tmp.path()).unwrap();
    Storage::register_memory("objects");

    let len = 4 * 1024 * 1024 + 77;
    let contents: Vec<u8> = (0..len).map(|i| (i % 253) as u8).collect();
    for disk in ["local", "objects"] {
        Storage::disk(disk)
            .unwrap()
            .write("exports/big.zip", contents.clone())
            .await
            .unwrap();
        let response = Storage::download(disk, "exports/big.zip", None)
            .await
            .unwrap();
        assert!(
            response.is_streaming(),
            "a file above the buffering limit on `{disk}` must stream"
        );
    }

    for (route, disk) in [("/big-local", "local"), ("/big-objects", "objects")] {
        let (status, headers, body) = serve(route, move || async move {
            Storage::download(disk, "exports/big.zip", Some("export.zip"))
                .await
                .map_err(HttpResponse::from)
        })
        .await;
        assert_eq!(status, 200, "{disk}");
        assert_eq!(header(&headers, "content-type"), "application/zip");
        assert_eq!(header(&headers, "content-length"), len.to_string());
        assert_eq!(body.len(), len, "{disk}");
        assert!(
            body[..] == contents[..],
            "the body from `{disk}` must match"
        );
    }
}
