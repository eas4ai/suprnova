//! P2-04 - the dogfood shipped without CSRF while its frontend was
//! already sending the token.
//!
//! `app/frontend/src/main.ts` subscribes to Inertia's `before` event and
//! injects `X-CSRF-TOKEN` on every visit. The server never installed
//! `CsrfMiddleware`, so it validated nothing: the client held up its half
//! of the protocol and the other half did not exist. The backend scaffold
//! template installs it directly after `SessionMiddleware`; the dogfood -
//! the other thing people copy - did not, the same shape as the anonymous
//! `/api/v3/users` defect.
//!
//! Worth recording why the suite stayed green when the middleware was
//! added: every other HTTP test in this crate builds
//! `MiddlewareRegistry::new()`, an *empty* registry, and never runs
//! `bootstrap::register()`. No global middleware has ever executed in an
//! app test - not CSRF, not sessions, not the feature context. These
//! tests stand the stack up for real.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::Empty;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use sea_orm_migration::MigratorTrait;
use tokio::sync::Mutex;

use app::migrations::Migrator;
use app::providers::DatabaseUserProvider;
use suprnova::crypto::EncryptionKey;
use suprnova::{MiddlewareRegistry, UserProvider, bind, handle_request};

/// `Crypt`, `DB` and the container bindings below are process-global, so
/// these tests take turns.
static TEST_LOCK: Mutex<()> = Mutex::const_new(());

struct TestApp {
    addr: SocketAddr,
    _lock: tokio::sync::MutexGuard<'static, ()>,
}

/// Stand up the app's real router behind the same middleware pair
/// `bootstrap::register` installs.
///
/// `csrf_is_installed_after_the_session` is what keeps that mirror
/// honest - this helper alone would keep passing if bootstrap dropped
/// the middleware again, which is exactly the regression that shipped.
async fn setup_app() -> TestApp {
    let lock = TEST_LOCK.lock().await;

    // `SessionMiddleware` fails closed without `Crypt`; every request
    // would 500 before reaching the CSRF check.
    suprnova::Crypt::init(EncryptionKey::generate());

    let conn = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("connect sqlite::memory:");
    Migrator::up(&conn, None)
        .await
        .expect("run migrations against sqlite::memory:");
    suprnova::App::singleton(suprnova::DbConnection::from_raw(conn));

    bind!(dyn UserProvider, DatabaseUserProvider);

    // The real chain, not a hand-written mirror of it: `register_http_stack`
    // is the same function `bootstrap::register` calls, so a middleware
    // dropped from the app is dropped from these tests too.
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

    TestApp { addr, _lock: lock }
}

async fn post(addr: SocketAddr, path: &str) -> u16 {
    let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
        .await
        .expect("handshake");
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let req = hyper::Request::builder()
        .method("POST")
        .uri(format!("http://{addr}{path}"))
        .header("host", addr.to_string())
        .body(Empty::<Bytes>::new())
        .expect("build request");

    sender
        .send_request(req)
        .await
        .expect("send")
        .status()
        .as_u16()
}

/// The property: a cookie-session state change with no CSRF token is
/// refused.
///
/// Asserted as the specific 419 rather than "not 2xx". A `!= 200`
/// assertion would pass with the middleware removed entirely - the
/// toothless shape that has already bitten this repo once.
#[tokio::test]
async fn a_state_changing_post_without_a_token_is_refused() {
    let app = setup_app().await;

    assert_eq!(
        post(app.addr, "/users").await,
        419,
        "POST /users is a cookie-session state change and must be refused \
         without a CSRF token"
    );
}

/// A cookie-authenticated route must not be waved through merely because
/// its path starts with `/api`.
///
/// `POST /api/posts` sits behind `SessionAuthMiddleware`, so it is exactly
/// what CSRF is for. Excepting all of `/api/*` - the tempting shortcut -
/// would have left it open.
#[tokio::test]
async fn a_cookie_authenticated_api_route_is_still_gated() {
    let app = setup_app().await;

    assert_eq!(
        post(app.addr, "/api/posts").await,
        419,
        "POST /api/posts is session-authenticated; sitting under /api does \
         not make it stateless"
    );
}

/// The stateless demo endpoints are excepted deliberately - no session,
/// no cookie, nothing ambient for a cross-site POST to abuse. A 419 here
/// means the exception list has drifted away from bootstrap.
#[tokio::test]
async fn the_excepted_stateless_endpoints_are_not_gated() {
    let app = setup_app().await;

    assert_ne!(
        post(app.addr, "/api/ping").await,
        419,
        "/api/ping is excepted from CSRF; a 419 means the exception list no \
         longer matches bootstrap"
    );
}

/// The wiring guard.
///
/// The behavioural tests build the stack themselves, so they would all
/// still pass if `bootstrap.rs` dropped the middleware - which is the
/// regression that actually shipped. This reads the real file.
#[test]
fn csrf_is_installed_after_the_session() {
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bootstrap.rs"),
    )
    .expect("read bootstrap.rs");

    let code: String = src
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");

    let session = code
        .find("SessionMiddleware::new")
        .expect("the app must install SessionMiddleware");
    let csrf = code.find("CsrfMiddleware::new").expect(
        "the app must install CsrfMiddleware - the frontend sends \
         X-CSRF-TOKEN on every Inertia visit and something has to check it",
    );

    assert!(
        csrf > session,
        "CsrfMiddleware must be installed after SessionMiddleware; it reads \
         the token out of the session that middleware establishes"
    );
}
