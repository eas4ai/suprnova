//! PAR-049: the SSR controls of the `Inertia` facade - `disable_ssr`,
//! `disable_ssr_if`, `without_ssr` with Laravel's `ExcludesPaths` rules,
//! and `configure_ssr_request_using`.
//!
//! Laravel's references are `ResponseFactory::disableSsr`, `withoutSsr`,
//! `configureSsrRequestUsing` and `Ssr\HttpGateway` in inertia-laravel
//! 3.5.1, and `ExcludesPaths::inExceptArray` in Laravel 13.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http_body_util::Full;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use suprnova::testing::TestContainer;
use suprnova::{Inertia, InertiaConfig, InertiaRequestExt, InertiaResponse};

use crate::protocol_harness::MockReq;

/// A stand-in SSR worker that answers every render and records the
/// headers each request carried.
async fn worker() -> (SocketAddr, Arc<Mutex<Vec<hyper::HeaderMap>>>) {
    let seen: Arc<Mutex<Vec<hyper::HeaderMap>>> = Arc::default();
    let body = Bytes::from(
        serde_json::to_vec(&serde_json::json!({
            "head": [],
            "body": "<div data-server-rendered=\"true\" id=\"app\">rendered</div>",
        }))
        .unwrap(),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let recorded = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let body = body.clone();
            let recorded = recorded.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                    recorded.lock().unwrap().push(req.headers().clone());
                    let body = body.clone();
                    async move {
                        Ok::<_, Infallible>(
                            hyper::Response::builder()
                                .status(200)
                                .header("content-type", "application/json")
                                .body(Full::new(body))
                                .unwrap(),
                        )
                    }
                });
                let _ = http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), svc)
                    .await;
            });
        }
    });
    (addr, seen)
}

/// Whether a first visit to `path` was rendered by the worker.
async fn rendered_by_ssr(config: &InertiaConfig, path: &str) -> bool {
    let response = InertiaResponse::new("Page")
        .with_config(config.clone())
        .resolve(&MockReq::new(path))
        .await
        .unwrap();
    String::from_utf8_lossy(response.body()).contains("data-server-rendered")
}

#[tokio::test]
async fn inp_an_ssr_exclusion_matches_as_laravel_excludes_paths() {
    // Laravel trims the pattern's slashes and lets `*` match any
    // characters, `/` included, so a pattern copied from a Laravel app
    // excludes the same paths here.
    let (addr, _seen) = worker().await;
    let config = InertiaConfig::new()
        .ssr(format!("http://{addr}"))
        .ssr_exclude("admin/*")
        .ssr_exclude("/reports/");

    assert!(!rendered_by_ssr(&config, "/admin/users").await);
    assert!(!rendered_by_ssr(&config, "/admin/users/edit").await);
    assert!(!rendered_by_ssr(&config, "/reports").await);
    assert!(rendered_by_ssr(&config, "/adminx").await);
    assert!(rendered_by_ssr(&config, "/dashboard").await);
}

#[tokio::test]
async fn inp_disable_ssr_if_decides_per_request_and_can_turn_ssr_on() {
    // With the configuration's SSR off, the condition alone decides, as
    // Laravel's gateway reads `disabled` before `ssr.enabled`.
    let _guard = TestContainer::fake();
    let (addr, _seen) = worker().await;
    let config = InertiaConfig::new()
        .ssr(format!("http://{addr}"))
        .ssr_disabled();
    assert!(
        !rendered_by_ssr(&config, "/yes").await,
        "off by configuration"
    );

    Inertia::disable_ssr_if(|request: &dyn InertiaRequestExt| request.path() == "/no");
    assert!(rendered_by_ssr(&config, "/yes").await);
    assert!(!rendered_by_ssr(&config, "/no").await);
}

#[tokio::test]
async fn inp_disable_ssr_takes_a_plain_flag_either_way() {
    let _guard = TestContainer::fake();
    let (addr, _seen) = worker().await;
    let on = InertiaConfig::new().ssr(format!("http://{addr}"));
    let off = on.clone().ssr_disabled();

    Inertia::disable_ssr(true);
    assert!(!rendered_by_ssr(&on, "/").await);
    Inertia::disable_ssr(false);
    assert!(rendered_by_ssr(&off, "/").await);
}

#[tokio::test]
async fn inp_without_ssr_excludes_paths_and_full_urls_at_run_time() {
    let _guard = TestContainer::fake();
    let (addr, _seen) = worker().await;
    let config = InertiaConfig::new().ssr(format!("http://{addr}"));

    Inertia::without_ssr(["admin/*"]);
    Inertia::without_ssr(["http://localhost/reports*"]);
    assert!(!rendered_by_ssr(&config, "/admin/users").await);
    assert!(rendered_by_ssr(&config, "/adminx").await);
    assert!(!rendered_by_ssr(&config, "/reports/2026").await);
    assert!(rendered_by_ssr(&config, "/home").await);
}

#[tokio::test]
async fn inp_configure_ssr_request_using_adjusts_the_request_to_the_worker() {
    let _guard = TestContainer::fake();
    let (addr, seen) = worker().await;
    let config = InertiaConfig::new().ssr(format!("http://{addr}"));

    Inertia::configure_ssr_request_using(|request| request.header("X-Ssr-Token", "s3cret"));
    assert!(rendered_by_ssr(&config, "/").await);

    let headers = seen.lock().unwrap();
    assert_eq!(
        headers
            .last()
            .and_then(|h| h.get("x-ssr-token"))
            .and_then(|v| v.to_str().ok()),
        Some("s3cret")
    );
}
