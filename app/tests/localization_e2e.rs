//! Task 11 — localization dogfood, driven end to end through the real
//! app stack.
//!
//! Harness mirrors `admin_users_bounded_and_gated.rs` (spawn the app's
//! actual router behind `register_http_stack()`'s global middleware on
//! a one-shot hyper server) plus `csrf_protects_state_changes.rs`'s
//! `TEST_LOCK` serialization and sqlite-in-memory DB setup.
//!
//! The DB turned out not to be optional: `SessionMiddleware` (global,
//! ahead of `LocaleMiddleware` in the chain) unconditionally uses
//! `DatabaseSessionDriver` — the only session driver the framework
//! ships — and a *successful* `GET` response makes it write the
//! session's `_previous.url` (`Redirect::back` bookkeeping), which
//! fails closed with 500 when there is nowhere to write it. The
//! DB-less `admin_users_bounded_and_gated.rs` harness never hits this:
//! its routes only ever return 401, and that "record previous_url"
//! write is gated on a 2xx/3xx response. `/lang-demo`'s `GET` returns a
//! real 200, so it needs the same DB setup
//! `csrf_protects_state_changes.rs` uses.
//!
//! These tests bind a `dyn Translator` and call `Crypt::init` in
//! addition to the DB, all process-global state that must not race
//! across the `#[tokio::test]`s in this file.
//!
//! `Localization::bootstrap()` — the call that normally binds the
//! catalog — is `pub(crate)` to the framework and only runs from
//! `Server::run()` / the daemon subcommand bootstraps, neither of which
//! this harness goes through (same reason every other app e2e test
//! binds `Crypt`/`DB`/`UserProvider` itself rather than calling
//! `Application::run()`). So the harness binds a `dyn Translator`
//! directly via `FluentTranslator::from_dir`, exactly the pattern
//! `framework/tests/localization_middleware.rs`'s own `bind_translator`
//! helper uses.
//!
//! (a) `Accept-Language: es` -> the Spanish greeting on `GET /lang-demo`.
//! (b) cookie `locale=en` (+ `Accept-Language: es`) -> English wins —
//!     cookie precedes header in the default detection chain.
//! (c) `POST /lang-demo` with no `name` + `Accept-Language: es` -> the
//!     `validation-required` message for the missing field renders in
//!     Spanish, proving keyed validation messages translate at the
//!     response boundary without any per-route plumbing.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Empty, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use sea_orm_migration::MigratorTrait;
use tokio::sync::Mutex;

use app::migrations::Migrator;
use suprnova::{Crypt, EncryptionKey, FluentTranslator, LocalizationConfig, Translator};
use suprnova::{DbConnection, MiddlewareRegistry, handle_request};

/// `Crypt`, the container's `dyn Translator` binding, and the global
/// middleware registry are all process-global, so these tests take
/// turns — the same reason `csrf_protects_state_changes.rs` serializes.
static TEST_LOCK: Mutex<()> = Mutex::const_new(());

/// Build the same `LocalizationConfig` `LocaleMiddleware::from_env()`
/// will build in `bootstrap::register_http_stack()` (no `APP_LOCALE` /
/// `APP_FALLBACK_LOCALE` set in the test environment -> `en`/`en`,
/// detection order Session -> Cookie -> Header).
fn config() -> LocalizationConfig {
    LocalizationConfig::from_env().expect("default locale config parses")
}

/// Bind a `dyn Translator` loaded from this crate's `lang/` directory
/// directly (rather than via `lang_path()`'s cwd-relative default), so
/// the test is hermetic to whatever directory `cargo test` happens to
/// run from.
fn bind_translator() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("lang");
    let translator = FluentTranslator::from_dir(&dir, &config()).expect("load app/lang catalogs");
    suprnova::App::bind::<dyn Translator>(Arc::new(translator));
}

/// Stand up the app's real router behind the real global middleware
/// chain (`register_http_stack()` — the same function
/// `bootstrap::register` calls), so `LocaleMiddleware` and the
/// translated-validation seam run for real rather than being
/// hand-mocked.
async fn spawn_app() -> (SocketAddr, tokio::sync::MutexGuard<'static, ()>) {
    let lock = TEST_LOCK.lock().await;

    // `SessionMiddleware` fails closed without `Crypt`; `LocaleMiddleware`
    // runs immediately after it in the chain.
    Crypt::init(EncryptionKey::generate());

    // `SessionMiddleware` unconditionally uses `DatabaseSessionDriver` —
    // see the module doc comment above for why a successful `GET`
    // needs this even though `/lang-demo` itself never touches the DB.
    let conn = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("connect sqlite::memory:");
    Migrator::up(&conn, None)
        .await
        .expect("run migrations against sqlite::memory:");
    suprnova::App::singleton(DbConnection::from_raw(conn));

    bind_translator();

    app::bootstrap::register_http_stack();
    let router = Arc::new(app::routes::register());
    let middleware = Arc::new(MiddlewareRegistry::from_global());

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

    (addr, lock)
}

/// GET `path` with the given extra headers; returns `(status, body)`.
async fn get(addr: SocketAddr, path: &str, headers: &[(&str, &str)]) -> (u16, String) {
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
    let req = builder.body(Empty::<Bytes>::new()).expect("request");

    let res = sender.send_request(req).await.expect("send");
    let status = res.status().as_u16();
    let bytes = BodyExt::collect(res.into_body())
        .await
        .expect("body")
        .to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// POST a JSON `body` to `path` with the given extra headers; returns
/// `(status, body)`.
async fn post_json(
    addr: SocketAddr,
    path: &str,
    headers: &[(&str, &str)],
    body: suprnova::serde_json::Value,
) -> (u16, String) {
    let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(io)
        .await
        .expect("handshake");
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let body_bytes = suprnova::serde_json::to_vec(&body).expect("serialize body");
    let mut builder = hyper::Request::builder()
        .method("POST")
        .uri(path)
        .header("Host", "localhost")
        .header("content-type", "application/json")
        .header("content-length", body_bytes.len());
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let req = builder
        .body(Full::new(Bytes::from(body_bytes)))
        .expect("request");

    let res = sender.send_request(req).await.expect("send");
    let status = res.status().as_u16();
    let bytes = BodyExt::collect(res.into_body())
        .await
        .expect("body")
        .to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// (a) `Accept-Language: es` negotiates the Spanish catalog.
#[tokio::test]
async fn accept_language_es_returns_spanish_greeting() {
    let (addr, _lock) = spawn_app().await;

    let (status, body) = get(addr, "/lang-demo", &[("Accept-Language", "es")]).await;

    assert_eq!(status, 200, "body: {body}");
    assert_eq!(body, "¡Bienvenido a Suprnova!");
}

/// (b) A `locale=en` cookie beats `Accept-Language: es` — cookie
/// precedes header in the detection chain (session, which would
/// otherwise come first, is empty here).
#[tokio::test]
async fn cookie_locale_beats_accept_language_header() {
    let (addr, _lock) = spawn_app().await;

    let (status, body) = get(
        addr,
        "/lang-demo",
        &[("Cookie", "locale=en"), ("Accept-Language", "es")],
    )
    .await;

    assert_eq!(status, 200, "body: {body}");
    assert_eq!(body, "Welcome to Suprnova!");
}

/// (c) A validation failure from `POST /lang-demo` (missing required
/// `name`) arrives translated when `Accept-Language: es` — proving the
/// `validation-<rule>` keyed-message seam, not just plain `Lang::get`.
#[tokio::test]
async fn validation_failure_translates_under_accept_language_es() {
    let (addr, _lock) = spawn_app().await;

    let (status, body) = post_json(
        addr,
        "/lang-demo",
        &[("Accept-Language", "es")],
        suprnova::serde_json::json!({}),
    )
    .await;

    assert_eq!(
        status, 422,
        "missing `name` must fail validation: body was {body}"
    );
    assert!(
        body.contains("El campo name es obligatorio."),
        "expected the translated validation-required message, got: {body}"
    );
}

/// Sanity check on the untranslated path: the same missing-`name`
/// request with no `Accept-Language` header falls back to the English
/// `validation.ftl` shipped with the framework, so the Spanish result
/// above is proven to come from translation and not from the English
/// fallback happening to contain the same words.
#[tokio::test]
async fn validation_failure_defaults_to_english() {
    let (addr, _lock) = spawn_app().await;

    let (status, body) = post_json(addr, "/lang-demo", &[], suprnova::serde_json::json!({})).await;

    assert_eq!(
        status, 422,
        "missing `name` must fail validation: body was {body}"
    );
    assert!(
        body.contains("The name field is required."),
        "expected the default English validation-required message, got: {body}"
    );
}
