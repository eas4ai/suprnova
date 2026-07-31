//! P2-02(a)/(c) — the credential-issuance routes must be throttled.
//!
//! Before this, the only rate-limited route in the whole app was the
//! `/ping` demo. `/auth/password/request`, `/auth/password/reset`,
//! `/auth/verify` and `/auth/verify/resend` — every endpoint that mints
//! or consumes a single-use credential — carried no limiter at all, so
//! password-reset mail could be issued to any address as fast as the
//! process could send it.
//!
//! Two properties are pinned here, and the second is the one that is easy
//! to get wrong:
//!
//! 1. The limit engages at all.
//! 2. The budget is shared across the four routes rather than granted
//!    per-route, so rotating between endpoints does not reset it.
//!
//! These drive the real `app::routes::register()` router through the real
//! `handle_request`, so the assertion covers the actual middleware wiring
//! rather than a hand-assembled approximation of it.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::Empty;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;

use suprnova::http::cookie::Cookie;
use suprnova::session::driver::database::DatabaseSessionDriver;
use suprnova::session::{
    SessionConfig, SessionData, SessionStore, generate_csrf_token, generate_session_id,
};
use suprnova::{MiddlewareRegistry, handle_request};
use tokio::sync::Mutex;

/// `Crypt`, the App container's DB singleton and the route limiter are all
/// process-global, so these tests take turns. Without this each test's
/// `spawn_app` rebinds the connection under the others: a session written
/// by one is looked up in another's database, the cookie resolves to
/// nothing, and CSRF answers 419 where the test expects 429.
static TEST_LOCK: Mutex<()> = Mutex::const_new(());

/// Matches `max_requests` on the issuance group in `app::routes`.
const BUDGET: usize = 10;

/// The session every request in a burst reuses.
///
/// `CsrfMiddleware` is global and sits ahead of the route-level limiter,
/// so a tokenless POST is refused at 419 and never reaches the throttle.
/// Reusing one session also matches the threat being modelled: a single
/// client hammering the issuance surface, not fifteen unrelated ones.
struct Client {
    cookie: String,
    csrf: String,
}

async fn seed_client() -> Client {
    let store = DatabaseSessionDriver::new(SessionConfig::default().lifetime);
    let session_id = generate_session_id();
    let mut session = SessionData::new(session_id.clone(), generate_csrf_token());
    session.dirty = true;
    store.write(&session).await.expect("write session");
    let cookie = Cookie::encrypted("suprnova_session", &session_id)
        .expect("Crypt installed by spawn_app")
        .value()
        .to_string();
    Client {
        cookie,
        csrf: session.csrf_token.clone(),
    }
}

struct TestApp {
    addr: SocketAddr,
    _lock: tokio::sync::MutexGuard<'static, ()>,
}

async fn spawn_app() -> TestApp {
    let lock = TEST_LOCK.lock().await;
    // One router for the whole server, so every request shares the one
    // limiter instance the route builder created — which is the point.
    let router = Arc::new(app::routes::register());

    // `SessionMiddleware` fails closed without `Crypt`; without this every
    // request 500s and the burst below proves nothing about the throttle.
    suprnova::Crypt::init(suprnova::crypto::EncryptionKey::generate());

    let conn = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("connect sqlite::memory:");
    <app::migrations::Migrator as sea_orm_migration::MigratorTrait>::up(&conn, None)
        .await
        .expect("migrate sqlite::memory:");
    suprnova::App::singleton(suprnova::DbConnection::from_raw(conn));

    let middleware = Arc::new({
        app::bootstrap::register_http_stack();
        MiddlewareRegistry::from_global()
    });

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

async fn send(addr: SocketAddr, method: &str, path: &str, client: &Client) -> u16 {
    let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
        .await
        .expect("handshake");
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let req = hyper::Request::builder()
        .method(method)
        .uri(path)
        .header("Host", "localhost")
        .header("Cookie", format!("suprnova_session={}", client.cookie))
        .header("X-CSRF-TOKEN", &client.csrf)
        .body(Empty::<Bytes>::new())
        .expect("request");

    sender
        .send_request(req)
        .await
        .expect("send")
        .status()
        .as_u16()
}

/// The headline property. Somewhere inside a modest burst the issuance
/// surface must start refusing.
#[tokio::test]
async fn the_issuance_routes_start_refusing_within_the_budget() {
    let app = spawn_app().await;
    let addr = app.addr;
    let client = seed_client().await;

    let mut saw_429 = false;
    let mut statuses = Vec::new();
    for _ in 0..(BUDGET + 5) {
        let status = send(addr, "POST", "/auth/password/request", &client).await;
        statuses.push(status);
        if status == 429 {
            saw_429 = true;
            break;
        }
    }

    assert!(
        saw_429,
        "no request in a burst of {} was refused — the issuance routes are \
         unthrottled, which is the defect. Statuses: {statuses:?}",
        BUDGET + 5
    );
}

/// The property worth more than the first one. If each route carried its
/// own bucket, an attacker would simply rotate between the four and get
/// four budgets. Spend the budget on one route, then assert a *different*
/// issuance route is already refused.
#[tokio::test]
async fn the_budget_is_shared_across_the_issuance_surface() {
    let app = spawn_app().await;
    let addr = app.addr;
    let client = seed_client().await;

    // Exhaust on one route.
    let mut exhausted = false;
    for _ in 0..(BUDGET + 5) {
        if send(addr, "POST", "/auth/password/request", &client).await == 429 {
            exhausted = true;
            break;
        }
    }
    assert!(exhausted, "could not exhaust the budget to set up the test");

    // A different issuance route must already be out of budget.
    let other = send(
        addr,
        "POST",
        "/auth/verify/resend?email=a@example.org",
        &client,
    )
    .await;
    assert_eq!(
        other, 429,
        "`/auth/verify/resend` still had budget after `/auth/password/request` \
         was exhausted, so each route carries its own bucket. Rotating between \
         the four endpoints would multiply the limit by four"
    );
}

/// The throttle must not have leaked onto unrelated routes. A limit that
/// quietly applies to the whole app is its own outage.
#[tokio::test]
async fn ordinary_routes_are_unaffected_by_the_issuance_throttle() {
    let app = spawn_app().await;
    let addr = app.addr;
    let client = seed_client().await;

    for _ in 0..(BUDGET + 5) {
        let _ = send(addr, "POST", "/auth/password/request", &client).await;
    }

    let home = send(addr, "GET", "/", &client).await;
    assert_ne!(
        home, 429,
        "exhausting the issuance budget must not throttle the home page"
    );
}
