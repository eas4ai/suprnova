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

use suprnova::session::{new_session_slot_for_test, session_mut, session_scope_for_test};
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

/// Like [`drive`], but seeds a task-local session (`session_key` =
/// `session_locale`) around the `LocaleMiddleware::handle` call — the
/// same idiom `FakeSessionScope` in `framework/tests/auth_http_middleware.rs`
/// uses to fake `SessionMiddleware` for a downstream reader.
///
/// The session scope is installed *inside* the spawned server task's
/// request-processing closure, not around the outer call to this
/// function: bare `tokio::spawn`'d child tasks do not inherit the
/// spawning task's `tokio::task_local!` state (documented on
/// `TASK_CONTAINER` in `framework/src/container/mod.rs`; `SESSION_CONTEXT`
/// is the same mechanism), so installing it any further out would leave
/// `crate::session::session()` seeing nothing by the time `detect()`
/// runs on the server side of the TCP round trip.
async fn drive_with_session(
    session_locale: &'static str,
    headers: &[(&str, &str)],
) -> (u16, String) {
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
                let slot = new_session_slot_for_test();
                let response = session_scope_for_test(slot, async {
                    session_mut(|s| s.put("locale", session_locale));
                    mw.handle(req, next).await
                })
                .await;
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

#[tokio::test]
#[serial_test::serial]
async fn session_beats_cookie_and_header() {
    let _tmp = bind_translator();
    // `Session` precedes `Cookie` and `Header` in the default detection
    // order — a session-stored locale must win over both, even when a
    // cookie and the Accept-Language header both point elsewhere.
    let (status, body) =
        drive_with_session("es", &[("Cookie", "locale=en"), ("Accept-Language", "en")]).await;
    assert_eq!(status, 200);
    assert_eq!(body, "Hola");
}

/// `LocaleShare` — the Inertia `lang` shared prop. Reuses this file's
/// `config`/`write_lang`/`bind_translator` helpers rather than
/// duplicating catalog setup.
mod locale_share {
    use super::bind_translator;

    use suprnova::{
        App, Config, InertiaRequestExt, InertiaSharedData, Locale, LocaleShare, LocalizationConfig,
        Prop, Translator, scope_locale,
    };

    use serde_json::Value;

    struct DummyReq;
    impl InertiaRequestExt for DummyReq {
        fn path(&self) -> &str {
            "/"
        }
        fn header(&self, _: &str) -> Option<&str> {
            None
        }
    }

    /// Pull the `Prop::Eager` JSON value out of the shared map, panicking
    /// with a useful message on any other `Prop` variant — mirrors the
    /// match-or-panic idiom `inertia::shared`'s own
    /// `trait_provider_round_trip` test uses (`Prop` has no `Debug`).
    fn eager_value(shared: &indexmap::IndexMap<String, Prop>) -> Value {
        match shared.get("lang").expect("must emit a `lang` key") {
            Prop::Eager(v) => v.clone(),
            _ => panic!("expected Prop::Eager for the `lang` key"),
        }
    }

    /// Registers a `LocalizationConfig` whose `fallback_locale` is
    /// `fallback` — a value distinct from both `en` (the env-default
    /// every `resolved_config()` call in this binary would otherwise
    /// silently fall back to, since nothing here ever calls
    /// `Localization::bootstrap()` to seed the `LOCALIZATION_CONFIG`
    /// `OnceLock`) and from whatever locale the caller scopes as
    /// "current". Asserting against that distinguishing value is the
    /// only way a `fallback` assertion can actually prove
    /// `LocaleShare::share` reads `resolved_config().fallback_locale`,
    /// rather than passing identically for a regression that hardcodes
    /// `"en"` or swaps in `default_locale` (which also defaults to
    /// `en` via the same env fallback).
    ///
    /// `Config::register` writes to a process-global repository with no
    /// unregister — same constraint `config_debug_gating.rs`'s
    /// `install_app_config` documents for `AppConfig`. Each test below
    /// calls this itself, immediately before its own `share()` call, so
    /// last-write-wins makes every test self-contained regardless of
    /// what a sibling test (in this module or the six above) registered
    /// or bound first.
    fn register_config_with_fallback(fallback: &str) {
        Config::register(LocalizationConfig {
            default_locale: Locale::parse("en").unwrap(),
            fallback_locale: Locale::parse(fallback).unwrap(),
            use_isolating: false,
            detection: vec![],
            session_key: "locale".into(),
            cookie_name: "locale".into(),
        });
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn emits_locale_fallback_and_catalog_when_translator_bound() {
        let _tmp = bind_translator();
        let translator = App::resolve_make::<dyn Translator>().unwrap();
        let hash = translator
            .catalog(&Locale::parse("es").unwrap())
            .expect("bind_translator loads an es catalog")
            .hash;
        register_config_with_fallback("fr");

        let shared = scope_locale(Locale::parse("es").unwrap(), async {
            LocaleShare.share(&DummyReq).await
        })
        .await
        .unwrap();

        assert_eq!(shared.len(), 1, "LocaleShare emits exactly the `lang` key");
        let value = eager_value(&shared);
        assert_eq!(value["locale"], Value::String("es".into()));
        assert_eq!(
            value["fallback"],
            Value::String("fr".into()),
            "fallback must come from the registered LocalizationConfig, \
             not a hardcoded/env-default 'en' or the current locale"
        );
        assert_eq!(
            value["catalog"]["url"],
            Value::String(format!("/_suprnova/lang/es.ftl?v={hash}"))
        );
        assert_eq!(value["catalog"]["hash"], Value::String(hash));
    }

    /// `catalog` must be JSON `null` — never an error — for every reason
    /// the adjudicated shape names: no `Translator` bound at all, or a
    /// bound `Translator` with nothing loaded for the current locale.
    ///
    /// This deliberately does not bind a `Translator` in this test.
    /// `App::bind` (used by `bind_translator` above, and by every
    /// `LocaleMiddleware` test in this file) writes to the
    /// process-global container with no unbind — `TestContainer`
    /// overrides what it explicitly binds but does not block fallthrough
    /// to that global container for a type it leaves unbound (see
    /// `App::make`'s doc comment), so nothing in this binary can prove
    /// *no* `Translator` is bound once a sibling test has run. Scoping
    /// locale `zz` sidesteps that: it's the same "parses but never has a
    /// loaded catalog" sentinel `unavailable_cookie_locale_is_skipped`
    /// uses above, and no fixture in this file ever writes a `zz`
    /// catalog — so `catalog` resolves to `null` deterministically
    /// whether this run sees no bound `Translator` at all (the
    /// `resolve_make` error path) or a sibling's leftover `en`/`es`
    /// translator (the "no catalog for this locale" path). Both are
    /// real branches `LocaleShare::share` must handle identically.
    #[tokio::test]
    #[serial_test::serial]
    async fn catalog_is_null_without_a_usable_translator() {
        register_config_with_fallback("de");

        let shared = scope_locale(Locale::parse("zz").unwrap(), async {
            LocaleShare.share(&DummyReq).await
        })
        .await
        .unwrap();

        let value = eager_value(&shared);
        assert_eq!(value["locale"], Value::String("zz".into()));
        assert_eq!(
            value["fallback"],
            Value::String("de".into()),
            "fallback must come from the registered LocalizationConfig, \
             not a hardcoded/env-default 'en' or the current locale"
        );
        assert_eq!(value["catalog"], Value::Null);
    }
}
