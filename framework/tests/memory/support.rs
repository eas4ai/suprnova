//! The lock, the profiler and a loopback server every test shares.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use suprnova::{MiddlewareRegistry, Router, handle_request};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Held for the whole of every test: dhat runs one profiler at a time,
/// and a test's numbers must not include another test's allocations.
pub async fn exclusive() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    LOCK.lock().await
}

/// A running heap profiler; counts read from it are process-wide, so a
/// test takes the difference across the operation it measures.
pub struct Heap {
    _profiler: dhat::Profiler,
}

impl Heap {
    pub fn start() -> Self {
        Self {
            _profiler: dhat::Profiler::builder().testing().build(),
        }
    }

    /// Bytes allocated since the profiler started.
    pub fn bytes(&self) -> u64 {
        dhat::HeapStats::get().total_bytes
    }

    /// Allocations made since the profiler started.
    pub fn blocks(&self) -> u64 {
        dhat::HeapStats::get().total_blocks
    }

    /// Bytes live now.
    pub fn live(&self) -> usize {
        dhat::HeapStats::get().curr_bytes
    }

    /// The most bytes live at once since the profiler started.
    pub fn peak(&self) -> usize {
        dhat::HeapStats::get().max_bytes
    }
}

/// Serves `router` through `registry` on a loopback port until the test
/// ends.
pub async fn serve(router: impl Into<Router>, registry: MiddlewareRegistry) -> SocketAddr {
    let router = Arc::new(router.into());
    let middleware = Arc::new(registry);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback port");
    let addr = listener.local_addr().expect("its address");
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let router = router.clone();
            let middleware = middleware.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req: hyper::Request<Incoming>| {
                    let router = router.clone();
                    let middleware = middleware.clone();
                    async move { Ok::<_, Infallible>(handle_request(router, middleware, req).await) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), svc)
                    .await;
            });
        }
    });
    addr
}

/// Sends `request` (a whole HTTP/1.1 request, `Connection: close`) and
/// reads the response into a fixed buffer, so the client allocates the
/// same whatever comes back. Returns the bytes read and the first 512 of
/// them.
pub async fn exchange(addr: SocketAddr, request: &[u8]) -> (usize, [u8; 512]) {
    let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
    stream.write_all(request).await.expect("send");
    let mut head = [0u8; 512];
    let mut buf = [0u8; 16 * 1024];
    let mut read = 0;
    loop {
        let n = stream.read(&mut buf).await.expect("read");
        if n == 0 {
            break;
        }
        if read < head.len() {
            let take = n.min(head.len() - read);
            head[read..read + take].copy_from_slice(&buf[..take]);
        }
        read += n;
    }
    (read, head)
}

/// A GET for `path` with `headers`, built before any measurement starts.
pub fn get_request(path: &str, headers: &[(&str, &str)]) -> Vec<u8> {
    let mut request = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    request.into_bytes()
}

/// The status code of a response head.
pub fn status(head: &[u8]) -> u16 {
    std::str::from_utf8(&head[9..12])
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}
