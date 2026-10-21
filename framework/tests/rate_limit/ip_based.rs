//! `RateLimitMiddleware::ip_based`: N requests per window for each client
//! IP, against the rate limiter the application installed.
//!
//! Each test drives real requests through `handle_request_with_peer`, the
//! entry the server's accept loop uses, so `Request::ip` is the peer the
//! test names.

use std::convert::Infallible;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;

use suprnova::container::testing::TestContainer;
use suprnova::http::text;
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::rate_limit::{BackendErrorPolicy, RateLimitMiddleware, RateLimiterDriver};
use suprnova::{MiddlewareRegistry, Router, handle_request_with_peer};

const MINUTE: Duration = Duration::from_secs(60);
const FIRST: IpAddr = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 1));
const SECOND: IpAddr = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 2));

/// A server for `router` whose every request comes from `peer`.
async fn serve(router: &Arc<Router>, peer: Option<IpAddr>, accepts: usize) -> SocketAddr {
    let router = router.clone();
    let registry = Arc::new(MiddlewareRegistry::new());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind an ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        for _ in 0..accepts {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let router = router.clone();
            let registry = registry.clone();
            tokio::spawn(async move {
                let service = service_fn(move |request: hyper::Request<Incoming>| {
                    let router = router.clone();
                    let registry = registry.clone();
                    async move {
                        Ok::<_, Infallible>(
                            handle_request_with_peer(router, registry, request, peer).await,
                        )
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    addr
}

async fn get(addr: SocketAddr, path: &str) -> u16 {
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(stream))
            .await
            .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let request = hyper::Request::builder()
        .method("GET")
        .uri(path)
        .header("Host", "localhost")
        .header("Content-Length", "0")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(5), sender.send_request(request))
        .await
        .expect("the request timed out")
        .expect("the request failed");
    let status = response.status().as_u16();
    let _ = response.into_body().collect().await.unwrap();
    status
}

/// A container of the test's own, with the in-memory limiter installed the
/// way `rate_limit::bootstrap_default` installs it.
fn install_limiter() -> impl Drop {
    let guard = TestContainer::fake();
    TestContainer::bind::<dyn RateLimiterDriver>(Arc::new(InMemoryRateLimiter::new()));
    guard
}

fn one_route(limit: u32) -> Arc<Router> {
    Arc::new(
        Router::new()
            .get("/login", |_req| async { text("ok") })
            .middleware(RateLimitMiddleware::ip_based(limit, MINUTE))
            .into(),
    )
}

#[tokio::test]
async fn each_address_has_a_budget_of_its_own() {
    let _container = install_limiter();
    let router = one_route(2);
    let first = serve(&router, Some(FIRST), 4).await;
    let second = serve(&router, Some(SECOND), 2).await;

    assert_eq!(get(first, "/login").await, 200);
    assert_eq!(get(first, "/login").await, 200);
    assert_eq!(get(first, "/login").await, 429, "the third request is over");

    assert_eq!(
        get(second, "/login").await,
        200,
        "another address has spent nothing"
    );
}

#[tokio::test]
async fn the_limiter_may_be_installed_after_the_middleware_is_built() {
    // Routes are registered before the drivers boot. The middleware has to
    // find the limiter when the request arrives, not when it was built.
    let guard = TestContainer::fake();
    let router = one_route(1);
    TestContainer::bind::<dyn RateLimiterDriver>(Arc::new(InMemoryRateLimiter::new()));
    let addr = serve(&router, Some(FIRST), 3).await;

    assert_eq!(get(addr, "/login").await, 200);
    assert_eq!(get(addr, "/login").await, 429);
    drop(guard);
}

#[tokio::test]
async fn with_no_limiter_installed_the_backend_error_policy_decides() {
    let _container = TestContainer::fake();
    let open: Arc<Router> = Arc::new(
        Router::new()
            .get("/open", |_req| async { text("ok") })
            .middleware(RateLimitMiddleware::ip_based(1, MINUTE))
            .get("/closed", |_req| async { text("ok") })
            .middleware(
                RateLimitMiddleware::ip_based(1, MINUTE)
                    .on_backend_error(BackendErrorPolicy::FailClosed),
            )
            .into(),
    );
    let addr = serve(&open, Some(FIRST), 4).await;

    assert_eq!(get(addr, "/open").await, 200, "the default fails open");
    assert_eq!(get(addr, "/open").await, 200);
    assert_eq!(get(addr, "/closed").await, 503, "fail closed refuses");
}

#[tokio::test]
async fn two_limits_with_different_numbers_share_no_bucket() {
    let _container = install_limiter();
    let router: Arc<Router> = Arc::new(
        Router::new()
            .get("/login", |_req| async { text("ok") })
            .middleware(RateLimitMiddleware::ip_based(1, MINUTE))
            .get("/search", |_req| async { text("ok") })
            .middleware(RateLimitMiddleware::ip_based(2, MINUTE))
            .into(),
    );
    let addr = serve(&router, Some(FIRST), 6).await;

    assert_eq!(get(addr, "/login").await, 200);
    assert_eq!(get(addr, "/login").await, 429);
    // Had the two shared a bucket, the two requests above would have
    // spent the budget of `/search`.
    assert_eq!(get(addr, "/search").await, 200);
    assert_eq!(get(addr, "/search").await, 200);
    assert_eq!(get(addr, "/search").await, 429);
}

#[tokio::test]
async fn two_limits_with_the_same_numbers_share_the_clients_budget() {
    let _container = install_limiter();
    let router: Arc<Router> = Arc::new(
        Router::new()
            .get("/login", |_req| async { text("ok") })
            .middleware(RateLimitMiddleware::ip_based(2, MINUTE))
            .get("/register", |_req| async { text("ok") })
            .middleware(RateLimitMiddleware::ip_based(2, MINUTE))
            .into(),
    );
    let addr = serve(&router, Some(FIRST), 4).await;

    assert_eq!(get(addr, "/login").await, 200);
    assert_eq!(get(addr, "/register").await, 200);
    assert_eq!(
        get(addr, "/login").await,
        429,
        "the budget is the client's, whichever route spends it"
    );
}

#[tokio::test]
async fn a_request_with_no_address_gets_a_bucket_of_its_own() {
    let _container = install_limiter();
    let router = one_route(1);
    let addr = serve(&router, None, 4).await;

    // One shared bucket for every request without an address would let
    // the first of them lock the rest out.
    assert_eq!(get(addr, "/login").await, 200);
    assert_eq!(get(addr, "/login").await, 200);
    assert_eq!(get(addr, "/login").await, 200);
}
