//! `GET /_suprnova/lang/<locale>.ftl` — the merged-catalog HTTP endpoint.
//!
//! Drives the real `handle_request` short-circuit end to end through a
//! socket, the same way `health_readiness_gate.rs` and
//! `health_error_redaction.rs` exercise the health endpoints beside which
//! this one lives in `framework/src/server.rs`. Binding conventions
//! (`config()`, `write_lang`) mirror `localization_translate.rs`'s
//! `lang_facade` module.

#![cfg(feature = "localization")]

use std::convert::Infallible;
use std::fs;
use std::net::SocketAddr;
use std::sync::Arc;

use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;

use suprnova::{
    FluentTranslator, Locale, LocalizationConfig, MiddlewareRegistry, Router, Translator,
    handle_request,
};

fn config() -> LocalizationConfig {
    LocalizationConfig {
        default_locale: Locale::parse("en").unwrap(),
        fallback_locale: Locale::parse("en").unwrap(),
        use_isolating: false,
        detection: vec![],
        session_key: "locale".into(),
        cookie_name: "locale".into(),
    }
}

fn write_lang(dir: &std::path::Path, locale: &str, file: &str, ftl: &str) {
    let d = dir.join(locale);
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join(file), ftl).unwrap();
}

/// Binds a `FluentTranslator` loaded from `dir` into the process-global
/// container, exactly like `localization_translate.rs`'s `lang_facade`
/// helper. Every test in this file does this, hence `#[serial]` on all of
/// them below: `App::bind` is a last-write-wins global, so two of these
/// tests running concurrently in this binary would race each other's
/// translator.
fn bind_translator(dir: &std::path::Path) {
    let t = FluentTranslator::from_dir(dir, &config()).unwrap();
    suprnova::container::App::bind::<dyn Translator>(Arc::new(t));
}

/// Route through the real `handle_request`, so these exercise the actual
/// short-circuit branch in `server.rs` rather than a re-implementation of
/// it. Copied from `health_readiness_gate.rs`'s `spawn_server`.
async fn spawn_server() -> SocketAddr {
    let router = Arc::new(Router::new());
    let middleware = Arc::new(MiddlewareRegistry::new());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");

    tokio::spawn(async move {
        loop {
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

struct Reply {
    status: u16,
    headers: hyper::HeaderMap,
    body: String,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
}

async fn get(addr: SocketAddr, path: &str, headers: &[(&str, &str)]) -> Reply {
    let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
        .await
        .expect("handshake");
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let mut builder = hyper::Request::builder()
        .method("GET")
        .uri(path)
        .header("Host", "localhost");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let req = builder
        .body(http_body_util::Empty::<bytes::Bytes>::new())
        .expect("request");

    let res = sender.send_request(req).await.expect("send");
    let status = res.status().as_u16();
    let headers = res.headers().clone();
    let bytes = res.into_body().collect().await.expect("body").to_bytes();
    Reply {
        status,
        headers,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

#[tokio::test]
#[serial_test::serial]
async fn serves_catalog_with_etag() {
    let tmp = tempfile::tempdir().unwrap();
    write_lang(tmp.path(), "en", "app.ftl", "greeting = Hello, world!\n");
    bind_translator(tmp.path());
    let addr = spawn_server().await;

    let reply = get(addr, "/_suprnova/lang/en.ftl", &[]).await;

    assert_eq!(reply.status, 200);
    assert_eq!(
        reply.header("content-type"),
        Some("text/plain; charset=utf-8")
    );
    assert!(
        reply.body.contains("greeting"),
        "body must contain the written message id: {}",
        reply.body
    );
    assert!(reply.header("etag").is_some(), "expected an ETag header");
}

#[tokio::test]
#[serial_test::serial]
async fn immutable_when_version_matches() {
    let tmp = tempfile::tempdir().unwrap();
    write_lang(tmp.path(), "en", "app.ftl", "greeting = Hello, world!\n");
    bind_translator(tmp.path());
    let addr = spawn_server().await;

    let first = get(addr, "/_suprnova/lang/en.ftl", &[]).await;
    let etag = first
        .header("etag")
        .expect("first response must carry an ETag")
        .to_string();
    let hash = etag.trim_matches('"');

    let no_version = get(addr, "/_suprnova/lang/en.ftl", &[]).await;
    assert_eq!(
        no_version.header("cache-control"),
        Some("no-cache"),
        "no ?v= must not be treated as cacheable-forever"
    );

    let fresh = get(addr, &format!("/_suprnova/lang/en.ftl?v={hash}"), &[]).await;
    assert!(
        fresh
            .header("cache-control")
            .is_some_and(|v| v.contains("immutable")),
        "?v=<current hash> must be cacheable forever, got {:?}",
        fresh.header("cache-control")
    );

    let stale = get(addr, "/_suprnova/lang/en.ftl?v=not-the-current-hash", &[]).await;
    assert_eq!(
        stale.header("cache-control"),
        Some("no-cache"),
        "a stale ?v= must not be treated as cacheable-forever"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn if_none_match_304() {
    let tmp = tempfile::tempdir().unwrap();
    write_lang(tmp.path(), "en", "app.ftl", "greeting = Hello, world!\n");
    bind_translator(tmp.path());
    let addr = spawn_server().await;

    let first = get(addr, "/_suprnova/lang/en.ftl", &[]).await;
    let etag = first
        .header("etag")
        .expect("first response must carry an ETag")
        .to_string();

    let revalidated = get(addr, "/_suprnova/lang/en.ftl", &[("If-None-Match", &etag)]).await;
    assert_eq!(revalidated.status, 304);
    assert!(revalidated.body.is_empty(), "304 must carry no body");
    assert!(
        revalidated.header("etag").is_some(),
        "the ETag must still be present on a 304"
    );

    // A weak-prefixed form of the same tag must also hit.
    let weak = format!("W/{etag}");
    let weak_revalidated = get(addr, "/_suprnova/lang/en.ftl", &[("If-None-Match", &weak)]).await;
    assert_eq!(
        weak_revalidated.status, 304,
        "a weak (W/) form of the current ETag must still revalidate"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn unknown_locale_404() {
    let tmp = tempfile::tempdir().unwrap();
    write_lang(tmp.path(), "en", "app.ftl", "greeting = Hello, world!\n");
    bind_translator(tmp.path());
    let addr = spawn_server().await;

    let unrouted = get(addr, "/_suprnova/no-such-path", &[]).await;
    assert_eq!(unrouted.status, 404, "sanity: the control really is a 404");

    let reply = get(addr, "/_suprnova/lang/de.ftl", &[]).await;
    assert_eq!(
        reply.status, unrouted.status,
        "a locale with no loaded catalog must 404 exactly like an unrouted path"
    );
}
