//! RTC-002: a file above the buffered limit streams as one blocking task
//! that reads chunks of at least 256 KiB and hands them to the response
//! over a channel of four, so the body holds one blocking-pool thread
//! for its whole length, a stalled client pins a bounded read-ahead, and
//! a dropped response stops its reader.

use crate::common::incoming_get_request;
use http_body_util::BodyExt;
use http_body_util::combinators::BoxBody;
use hyper::body::Bytes;
use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use suprnova::{HttpResponse, MiddlewareRegistry, Request, Response, Router, handle_request};

const SIZE: usize = 4 * 1024 * 1024;
const CHUNK: usize = 256 * 1024;
/// How many chunks the channel between the reader and the body holds.
const QUEUED: usize = 4;

type Body = BoxBody<Bytes, Infallible>;

/// Write a `SIZE`-byte file whose bytes encode their offset, so a body
/// with a chunk missing, repeated or out of order differs from it.
fn write_file(dir: &Path) -> (PathBuf, Vec<u8>) {
    let bytes: Vec<u8> = (0..SIZE).map(|i| (i % 251) as u8).collect();
    let path = dir.join("big.bin");
    std::fs::write(&path, &bytes).expect("write the file");
    (path, bytes)
}

/// Serve `path` with `HttpResponse::file` through `handle_request`.
async fn get_file(path: PathBuf) -> hyper::Response<Body> {
    let router: Router = Router::new()
        .get("/big", move |_req: Request| {
            let path = path.clone();
            async move {
                let response: Response = HttpResponse::file(&path, None)
                    .await
                    .map_err(HttpResponse::from);
                response
            }
        })
        .into();
    let req = incoming_get_request("/big", &[]).await;
    let response = handle_request(Arc::new(router), Arc::new(MiddlewareRegistry::new()), req).await;
    assert_eq!(response.status(), 200);
    response
}

/// The next data frame of `body`, or `None` once it ends.
async fn next_data(body: &mut Body) -> Option<Bytes> {
    while let Some(frame) = body.frame().await {
        if let Ok(data) = frame.expect("a body frame").into_data() {
            return Some(data);
        }
    }
    None
}

/// A runtime whose blocking pool has one thread, so a blocking task can
/// run only when no other holds that thread.
fn one_blocking_thread() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .expect("a runtime")
}

/// The read position of the descriptor this process holds open on
/// `path`, from `/proc/self/fdinfo`, or `None` when nothing holds it open.
#[cfg(target_os = "linux")]
fn open_file_position(path: &Path) -> Option<u64> {
    let target = std::fs::canonicalize(path).expect("the file's real path");
    let entries = std::fs::read_dir("/proc/self/fd").expect("the descriptor table");
    for entry in entries.flatten() {
        let Ok(link) = std::fs::read_link(entry.path()) else {
            continue;
        };
        if link != target {
            continue;
        }
        let info = format!("/proc/self/fdinfo/{}", entry.file_name().to_string_lossy());
        let info = std::fs::read_to_string(info).expect("the descriptor's fdinfo");
        return info
            .lines()
            .find_map(|line| line.strip_prefix("pos:"))
            .map(|pos| pos.trim().parse().expect("a numeric position"));
    }
    None
}

/// RTC-002: every chunk before the last is at least 256 KiB, the body is
/// the file byte for byte, and `Content-Length` is its size.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rtc_a_streamed_file_arrives_in_chunks_of_at_least_256_kib() {
    let dir = tempfile::tempdir().expect("a directory");
    let (path, bytes) = write_file(dir.path());
    let response = get_file(path).await;
    assert_eq!(
        response
            .headers()
            .get("content-length")
            .and_then(|value| value.to_str().ok()),
        Some(SIZE.to_string().as_str()),
        "the Content-Length is the file's size"
    );
    let mut body = response.into_body();
    let mut sizes = Vec::new();
    let mut received = Vec::with_capacity(SIZE);
    while let Some(data) = next_data(&mut body).await {
        sizes.push(data.len());
        received.extend_from_slice(&data);
    }
    assert!(received == bytes, "the body is the file byte for byte");
    let short = sizes[..sizes.len() - 1]
        .iter()
        .filter(|&&size| size < CHUNK)
        .count();
    assert_eq!(
        short,
        0,
        "{short} of {} chunks before the last one are under 256 KiB: {:?}",
        sizes.len(),
        &sizes[..sizes.len().min(4)]
    );
}

/// RTC-002: on a runtime whose blocking pool has one thread, a blocking
/// task spawned after the body began cannot run while the file's reader
/// still has chunks to read; it runs once the reader is done.
///
/// The reader is provably still running while the bytes still owed to
/// the consumer exceed what the channel can hold: some of them have not
/// been read yet. Once the last chunks sit in the channel the reader may
/// have returned and the probe may run, so the check stops there.
#[test]
fn rtc_a_streamed_file_holds_one_blocking_thread_for_its_whole_body() {
    one_blocking_thread().block_on(async {
        let dir = tempfile::tempdir().expect("a directory");
        let (path, _) = write_file(dir.path());
        let mut body = get_file(path).await.into_body();
        let mut received = next_data(&mut body).await.expect("the first chunk").len();
        let probe = tokio::task::spawn_blocking(|| ());
        while let Some(data) = next_data(&mut body).await {
            received += data.len();
            if SIZE - received > QUEUED * CHUNK {
                assert!(
                    !probe.is_finished(),
                    "a blocking task spawned after the first chunk ran before the \
                     file's reader finished: {received} of {SIZE} bytes received"
                );
            }
        }
        assert_eq!(received, SIZE, "the body is the whole file");
        probe
            .await
            .expect("the probe runs once the body has been read");
    });
}

/// RTC-002: a consumer that stops after the first chunk leaves the file
/// read at most five chunks further: the four the channel holds and the
/// one the reader holds while it waits for room.
///
/// The test owns its runtime and shuts it down without waiting, so a
/// reader that outlives its body fails the dropped-response test below
/// instead of holding this one open.
#[cfg(target_os = "linux")]
#[test]
fn rtc_a_stalled_consumer_bounds_the_read_ahead() {
    let dir = tempfile::tempdir().expect("a directory");
    let (path, _) = write_file(dir.path());
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("a runtime");
    let position = runtime.block_on(async {
        let mut body = get_file(path.clone()).await.into_body();
        next_data(&mut body).await.expect("the first chunk");

        // Give the reader time to fill the channel; a reader that reads
        // ahead without a bound reads the whole file in far less.
        tokio::time::sleep(Duration::from_millis(300)).await;

        // The reader closes the file only after reading all of it, so a
        // file no longer open was read to its end.
        open_file_position(&path).unwrap_or(SIZE as u64)
    });
    runtime.shutdown_background();
    assert!(
        position <= ((1 + QUEUED + 1) * CHUNK) as u64,
        "the file was read to byte {position} while the consumer took one chunk; at most \
         {} may be read",
        (1 + QUEUED + 1) * CHUNK
    );
}

/// RTC-002: dropping a body part way through stops its reader, so the
/// blocking pool's only thread comes free and a task spawned afterwards
/// runs. The deadline is only reached by a reader that never stops.
#[test]
fn rtc_dropping_a_streamed_response_stops_its_reader() {
    let dir = tempfile::tempdir().expect("a directory");
    let (path, _) = write_file(dir.path());
    let runtime = one_blocking_thread();
    let probe_ran = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(10), async {
            let mut body = get_file(path.clone()).await.into_body();
            next_data(&mut body).await.expect("the first chunk");
            drop(body);
            tokio::task::spawn_blocking(|| ())
                .await
                .expect("the probe runs");
        })
        .await
    });
    // A reader that never stops would hold up a shutdown that waits for it.
    runtime.shutdown_background();
    assert!(
        probe_ran.is_ok(),
        "a dropped response's reader still holds the blocking pool's only thread"
    );

    // The reader closed the file when it stopped.
    #[cfg(target_os = "linux")]
    assert_eq!(open_file_position(&path), None, "the file is still open");
}

/// RTC-002: a file truncated while it streams ends the body short of its
/// declared length (on a connection, the abort the client sees), and the
/// bytes that arrived from the part the truncation kept are the file's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rtc_a_file_truncated_while_streaming_ends_the_body_short() {
    const KEPT: usize = 512 * 1024;
    let dir = tempfile::tempdir().expect("a directory");
    let (path, bytes) = write_file(dir.path());
    let response = get_file(path.clone()).await;
    assert_eq!(
        response
            .headers()
            .get("content-length")
            .and_then(|value| value.to_str().ok()),
        Some(SIZE.to_string().as_str()),
    );
    let mut body = response.into_body();
    let mut received = next_data(&mut body)
        .await
        .expect("the first chunk")
        .to_vec();

    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("open the file for writing")
        .set_len(KEPT as u64)
        .expect("truncate the file");

    while let Some(data) = next_data(&mut body).await {
        received.extend_from_slice(&data);
    }
    assert!(
        received.len() < SIZE,
        "a truncated file completed a body of its declared {SIZE} bytes"
    );
    // A read that races the truncation may see the cut pages as zeros, so
    // only the bytes the truncation leaves in place are compared.
    let kept = received.len().min(KEPT);
    assert!(
        received[..kept] == bytes[..kept],
        "the bytes that arrived are the start of the file"
    );
}
