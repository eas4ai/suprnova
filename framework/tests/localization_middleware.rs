//! `LocaleMiddleware` detection chain, end to end.
//!
//! Each test boots a one-shot TCP server that wraps `LocaleMiddleware`
//! directly (the same pattern `framework/tests/data_middleware.rs`
//! uses for `IncludeMiddleware`), sends a real HTTP request through a
//! hyper client with the headers/cookies under test, and reads back a
//! response body produced by `Lang::get("greet")` running downstream —
//! proving `scope_locale` actually bound the detected locale for the
//! rest of the request, not just that `detect()` returned the right
//! value in isolation.

#![cfg(feature = "localization")]

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;

use suprnova::{
    Detect, FluentTranslator, Lang, Locale, LocaleMiddleware, LocalizationConfig, Middleware, Next,
    Translator,
};
use suprnova::{HttpResponse, Request};

fn config() -> LocalizationConfig {
    LocalizationConfig {
        default_locale: Locale::parse("en").unwrap(),
        fallback_locale: Locale::parse("en").unwrap(),
        use_isolating: false,
        detection: vec![Detect::Session, Detect::Cookie, Detect::Header],
        session_key: "locale".into(),
        cookie_name: "locale".into(),
    }
}

fn write_lang(dir: &std::path::Path, locale: &str, file: &str, ftl: &str) {
    let d = dir.join(locale);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join(file), ftl).unwrap();
}

/// Bind a fresh catalog (en `greet = Hello`, es `greet = Hola`) as the
/// container's `dyn Translator`. Global container state, so every
/// caller runs under `#[serial_test::serial]`.
///
/// Returns the backing `TempDir` — the caller must keep it alive for
/// as long as the binding is in use. `LocaleMiddleware` calls
/// `reload_if_stale()` on every request in the (default, unset
/// `APP_ENV`) `Local` environment; if the directory were dropped
/// (deleted) before the request ran, that reload would see a missing
/// catalog tree and silently empty the bound translator's catalogs.
fn bind_translator() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    write_lang(tmp.path(), "en", "app.ftl", "greet = Hello\n");
    write_lang(tmp.path(), "es", "app.ftl", "greet = Hola\n");
    let t = FluentTranslator::from_dir(tmp.path(), &config()).unwrap();
    suprnova::container::App::bind::<dyn Translator>(Arc::new(t));
    tmp
}

/// Boot a one-shot server wrapping `LocaleMiddleware`, send one GET
/// with `headers` attached, and return `(status, body)`. The
/// downstream "handler" is `Lang::get("greet")` — its output reflects
/// whichever locale `LocaleMiddleware` scoped around it.
async fn drive(headers: &[(&str, &str)]) -> (u16, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();

    let mw = Arc::new(LocaleMiddleware::new(config()));

    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let io = TokioIo::new(stream);
        let mw = mw.clone();
        let service = service_fn(move |hyper_req: hyper::Request<hyper::body::Incoming>| {
            let mw = mw.clone();
            async move {
                let req = Request::new(hyper_req);
                let next: Next = Arc::new(move |_req: Request| {
                    Box::pin(async move { Ok(HttpResponse::text(Lang::get("greet"))) })
                });
                let response = mw.handle(req, next).await;
                let http = response.unwrap_or_else(|e| e);
                Ok::<_, Infallible>(http.into_hyper())
            }
        });
        http1::Builder::new()
            .serve_connection(io, service)
            .await
            .ok();
    });

    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(io)
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let mut builder = hyper::Request::builder()
        .method("GET")
        .uri("http://localhost/greet");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let req = builder.body(Full::<Bytes>::from(Bytes::new())).unwrap();

    let resp = sender.send_request(req).await.unwrap();
    let status = resp.status().as_u16();
    let (_, body) = resp.into_parts();
    let body_bytes = body.collect().await.unwrap().to_bytes();
    (status, String::from_utf8(body_bytes.to_vec()).unwrap())
}

#[tokio::test]
#[serial_test::serial]
async fn accept_language_negotiates() {
    let _tmp = bind_translator();
    let (status, body) = drive(&[("Accept-Language", "fr, es;q=0.8")]).await;
    assert_eq!(status, 200);
    assert_eq!(body, "Hola");
}

#[tokio::test]
#[serial_test::serial]
async fn cookie_beats_header() {
    let _tmp = bind_translator();
    let (status, body) =
        drive(&[("Cookie", "locale=en"), ("Accept-Language", "fr, es;q=0.8")]).await;
    assert_eq!(status, 200);
    assert_eq!(body, "Hello");
}

#[tokio::test]
#[serial_test::serial]
async fn unavailable_cookie_locale_is_skipped() {
    let _tmp = bind_translator();
    // `zz` parses as a locale but has no loaded catalog — must be
    // skipped silently (not a 500), falling through to the header.
    let (status, body) = drive(&[("Cookie", "locale=zz"), ("Accept-Language", "es")]).await;
    assert_eq!(status, 200);
    assert_eq!(body, "Hola");
}

#[tokio::test]
#[serial_test::serial]
async fn default_when_nothing_matches() {
    let _tmp = bind_translator();
    let (status, body) = drive(&[]).await;
    assert_eq!(status, 200);
    assert_eq!(body, "Hello");
}

#[tokio::test]
#[serial_test::serial]
async fn garbage_header_does_not_500() {
    let _tmp = bind_translator();
    let (status, body) = drive(&[("Accept-Language", ";;;===")]).await;
    assert_eq!(status, 200, "a malformed Accept-Language must not 500");
    assert_eq!(body, "Hello");
}
