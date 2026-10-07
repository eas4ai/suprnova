//! RTC-002: a file above the buffered limit streams as one blocking task
//! handing chunks of at least 256 KiB to the response.

use crate::common::incoming_get_request;
use http_body_util::BodyExt;
use std::sync::Arc;
use suprnova::{HttpResponse, MiddlewareRegistry, Request, Response, Router, handle_request};

const SIZE: usize = 4 * 1024 * 1024;
const CHUNK: usize = 256 * 1024;

/// Serve a `SIZE`-byte file through `handle_request` and return the size of
/// every data frame the body yields, in order.
async fn frame_sizes() -> Vec<usize> {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("big.bin");
    std::fs::write(&path, vec![7u8; SIZE]).expect("write the file");
    let router: Router = Router::new()
        .get("/big", move |_req: Request| {
            let path = path.clone();
            async move {
                let response: Response =
                    HttpResponse::file(&path, None).await.map_err(HttpResponse::from);
                response
            }
        })
        .into();
    let req = incoming_get_request("/big", &[]).await;
    let response =
        handle_request(Arc::new(router), Arc::new(MiddlewareRegistry::new()), req).await;
    assert_eq!(response.status(), 200);
    let mut body = response.into_body();
    let mut sizes = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.expect("a body frame");
        if let Some(data) = frame.data_ref() {
            sizes.push(data.len());
        }
    }
    sizes
}

/// RTC-002: every chunk before the last is at least 256 KiB, and the body
/// is the whole file.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rtc_a_streamed_file_arrives_in_chunks_of_at_least_256_kib() {
    let sizes = frame_sizes().await;
    assert_eq!(sizes.iter().sum::<usize>(), SIZE, "the body is the whole file");
    let short = sizes[..sizes.len() - 1]
        .iter()
        .filter(|&&size| size < CHUNK)
        .count();
    assert_eq!(
        short, 0,
        "{short} of {} chunks before the last one are under 256 KiB: {:?}",
        sizes.len(),
        &sizes[..sizes.len().min(4)]
    );
}
