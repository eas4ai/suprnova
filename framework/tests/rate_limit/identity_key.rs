//! Per-identity rate limiting - `identity_key` + `key_reads_body`.
//!
//! Per-IP throttling answers "is one client noisy". It cannot answer "is
//! one mailbox being flooded": an attacker spread across a botnet or an
//! IPv6 /64 stays under every address budget while sending one victim
//! thousands of password-reset mails. These tests drive the identity half
//! over real HTTP, because the body-buffering path only exists once a
//! request has an actual streaming body - an in-process `Request` built
//! from bytes would already be `Buffered` and would prove nothing.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;

use suprnova::http::text;
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::rate_limit::{
    RateLimitMiddleware, RateLimiterDriver, SlidingWindowConfig, identity_key,
};
use suprnova::{MiddlewareRegistry, Router, handle_request};

async fn spawn_server(router: impl Into<Router>, accepts: usize) -> SocketAddr {
    let router = Arc::new(router.into());
    let middleware = Arc::new(MiddlewareRegistry::new());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");

    tokio::spawn(async move {
        for _ in 0..accepts {
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

/// POST `body` as a form and return `(status, echoed body)`.
///
/// The echoed body matters: it proves the handler still reads what the
/// client sent after the middleware buffered it.
async fn post_form(addr: SocketAddr, path: &str, body: &'static str) -> (u16, String) {
    post_with_type(addr, path, body, "application/x-www-form-urlencoded").await
}

async fn post_with_type(
    addr: SocketAddr,
    path: &str,
    body: &'static str,
    content_type: &str,
) -> (u16, String) {
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(io)
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let req = hyper::Request::builder()
        .method("POST")
        .uri(path)
        .header("Host", "localhost")
        .header("content-type", content_type)
        .header("content-length", body.len().to_string())
        .body(Full::new(Bytes::from(body)))
        .unwrap();

    let resp = tokio::time::timeout(Duration::from_secs(5), sender.send_request(req))
        .await
        .expect("send_request timeout")
        .expect("hyper send_request");

    let (parts, body) = resp.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes();
    (
        parts.status.as_u16(),
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

/// A route that echoes the `email` field back, so a test can tell
/// whether the handler still sees the body the middleware buffered.
fn echo_router(mw: impl suprnova::Middleware + 'static) -> Router {
    Router::new()
        .post("/issue", |req: suprnova::Request| async move {
            match req.body_bytes().await {
                Ok((_, bytes)) => text(String::from_utf8_lossy(&bytes).into_owned()),
                Err(e) => Err(suprnova::HttpResponse::from(e)),
            }
        })
        .middleware(mw)
        .into()
}

fn limiter() -> Arc<dyn RateLimiterDriver> {
    Arc::new(InMemoryRateLimiter::new())
}

fn one_per_window() -> SlidingWindowConfig {
    SlidingWindowConfig {
        max_requests: 1,
        window: Duration::from_secs(60),
    }
}

/// The core guarantee: two requests naming the same address share a
/// bucket even though nothing else about them matches.
#[tokio::test]
async fn a_second_request_for_the_same_address_is_throttled() {
    let mw = RateLimitMiddleware::new(limiter(), one_per_window(), |req| {
        identity_key(req, "email", "issuance")
    })
    .key_reads_body(4096);

    let addr = spawn_server(echo_router(mw), 6).await;

    let (first, echoed) = post_form(addr, "/issue", "email=victim@example.com").await;
    let (second, _) = post_form(addr, "/issue", "email=victim@example.com").await;

    assert_eq!(first, 200, "the first request for an address must pass");
    assert_eq!(
        second, 429,
        "a second request naming the same address must be throttled"
    );
    assert_eq!(
        echoed, "email=victim@example.com",
        "the handler must still read the body the middleware buffered"
    );
}

/// The limit must not be a global one wearing an identity's clothes:
/// a different address gets its own budget.
#[tokio::test]
async fn a_different_address_has_its_own_budget() {
    let mw = RateLimitMiddleware::new(limiter(), one_per_window(), |req| {
        identity_key(req, "email", "issuance")
    })
    .key_reads_body(4096);

    let addr = spawn_server(echo_router(mw), 6).await;

    let (first, _) = post_form(addr, "/issue", "email=alice@example.com").await;
    let (other, _) = post_form(addr, "/issue", "email=bob@example.com").await;

    assert_eq!(first, 200);
    assert_eq!(
        other, 200,
        "a different address must not inherit another's exhausted bucket"
    );
}

/// Case is not identity. `Victim@Example.com` reaches the same mailbox
/// as `victim@example.com`, so without normalisation the limit is
/// bypassed by holding down shift.
#[tokio::test]
async fn capitalisation_does_not_buy_a_fresh_bucket() {
    let mw = RateLimitMiddleware::new(limiter(), one_per_window(), |req| {
        identity_key(req, "email", "issuance")
    })
    .key_reads_body(4096);

    let addr = spawn_server(echo_router(mw), 6).await;

    let (first, _) = post_form(addr, "/issue", "email=victim@example.com").await;
    // Percent-encoded spaces, so the surrounding whitespace is part of
    // the *value* - a literal space in the body would rename the field.
    let (recased, _) = post_form(addr, "/issue", "email=%20ViCtIm@Example.COM%20").await;

    assert_eq!(first, 200);
    assert_eq!(
        recased, 429,
        "a recased, space-padded address must map to the same bucket, not a fresh one"
    );
}

/// The same key function serves a query-string route, so `?email=` and a
/// form body land in one budget rather than two.
#[tokio::test]
async fn the_query_string_and_the_body_share_one_bucket() {
    let mw = RateLimitMiddleware::new(limiter(), one_per_window(), |req| {
        identity_key(req, "email", "issuance")
    })
    .key_reads_body(4096);

    let addr = spawn_server(echo_router(mw), 6).await;

    let (from_query, _) = post_with_type(
        addr,
        "/issue?email=victim@example.com",
        "",
        "application/x-www-form-urlencoded",
    )
    .await;
    let (from_body, _) = post_form(addr, "/issue", "email=victim@example.com").await;

    assert_eq!(from_query, 200);
    assert_eq!(
        from_body, 429,
        "an address in the query and the same address in a body must share a bucket"
    );
}

/// A request naming nobody must still be throttled - by IP, never by a
/// shared `no-identity` constant that one caller could exhaust for
/// everyone.
#[tokio::test]
async fn a_request_without_the_field_still_gets_throttled() {
    let mw = RateLimitMiddleware::new(limiter(), one_per_window(), |req| {
        identity_key(req, "email", "issuance")
    })
    .key_reads_body(4096);

    let addr = spawn_server(echo_router(mw), 6).await;

    let (first, _) = post_form(addr, "/issue", "token=abc123").await;
    let (second, _) = post_form(addr, "/issue", "token=def456").await;

    assert_eq!(first, 200);
    assert_eq!(
        second, 429,
        "with no address to key on, the caller's IP must still bound them"
    );
}

/// An empty value is no value. Otherwise `email=` is a free bucket that
/// every attacker can share and no real user occupies.
#[tokio::test]
async fn an_empty_address_falls_back_rather_than_forming_its_own_bucket() {
    let mw = RateLimitMiddleware::new(limiter(), one_per_window(), |req| {
        identity_key(req, "email", "issuance")
    })
    .key_reads_body(4096);

    let addr = spawn_server(echo_router(mw), 6).await;

    let (blank, _) = post_form(addr, "/issue", "email=").await;
    let (fieldless, _) = post_form(addr, "/issue", "token=abc").await;

    assert_eq!(blank, 200);
    assert_eq!(
        fieldless, 429,
        "a blank address must fall back to the IP bucket, which the previous request already used"
    );
}

/// Padding the body past the cap must not be a way out of the limiter.
#[tokio::test]
async fn an_oversized_body_is_rejected_not_waved_through() {
    let mw = RateLimitMiddleware::new(limiter(), one_per_window(), |req| {
        identity_key(req, "email", "issuance")
    })
    .key_reads_body(64);

    let addr = spawn_server(echo_router(mw), 6).await;

    let (status, _) = post_form(
        addr,
        "/issue",
        "email=victim@example.com&pad=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .await;

    assert_eq!(
        status, 413,
        "a body over the keying cap must be refused, not passed through unkeyed"
    );
}

/// Without `key_reads_body` the body is never touched - the key falls
/// back to the IP, and the handler still gets its bytes.
#[tokio::test]
async fn the_body_is_left_alone_unless_the_key_asks_for_it() {
    let mw = RateLimitMiddleware::new(limiter(), one_per_window(), |req| {
        identity_key(req, "email", "issuance")
    });

    let addr = spawn_server(echo_router(mw), 6).await;

    let (first, echoed) = post_form(addr, "/issue", "email=alice@example.com").await;
    let (second, _) = post_form(addr, "/issue", "email=bob@example.com").await;

    assert_eq!(first, 200);
    assert_eq!(
        second, 429,
        "with no body access the addresses are invisible, so both land in the IP bucket"
    );
    assert_eq!(echoed, "email=alice@example.com");
}

/// The raw address must not appear in the key. A rate-limit backend is
/// often a shared Redis with weaker access control than the primary
/// database, and a key dump should not read as a list of who is
/// resetting their password.
#[tokio::test]
async fn the_key_does_not_carry_the_raw_address() {
    let captured: Arc<std::sync::Mutex<Vec<String>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = captured.clone();

    let mw = RateLimitMiddleware::new(
        limiter(),
        SlidingWindowConfig {
            max_requests: 10,
            window: Duration::from_secs(60),
        },
        move |req| {
            let key = identity_key(req, "email", "issuance");
            sink.lock().unwrap().push(key.clone());
            key
        },
    )
    .key_reads_body(4096);

    let addr = spawn_server(echo_router(mw), 6).await;
    let (status, _) = post_form(addr, "/issue", "email=victim@example.com").await;
    assert_eq!(status, 200);

    let keys = captured.lock().unwrap().clone();
    assert_eq!(keys.len(), 1, "expected one key, got {keys:?}");
    let key = &keys[0];
    assert!(
        !key.contains("victim") && !key.contains("example.com"),
        "the key must not embed the address: {key}"
    );
    assert!(
        key.starts_with("issuance:email:"),
        "the key must stay namespaced and self-describing: {key}"
    );
}

/// The fieldless fallback must not collide with a co-mounted per-IP
/// limiter's key.
///
/// This limiter is designed to be stacked on an address-keyed one, and
/// the two carry different windows and quotas. If both spelled their key
/// `{prefix}:ip:{addr}` they would share a bucket in the backend, and
/// each would then be evaluated under whichever config got there first -
/// a per-recipient limit silently enforcing the per-IP window, or the
/// reverse. Found by reading the app's route wiring, where the address
/// limiter already emits exactly `auth-issuance:ip:{addr}`.
#[tokio::test]
async fn the_fallback_key_cannot_collide_with_a_plain_ip_key() {
    let captured: Arc<std::sync::Mutex<Vec<String>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = captured.clone();

    let mw = RateLimitMiddleware::new(
        limiter(),
        SlidingWindowConfig {
            max_requests: 10,
            window: Duration::from_secs(60),
        },
        move |req| {
            let key = identity_key(req, "email", "auth-issuance");
            sink.lock().unwrap().push(key.clone());
            key
        },
    )
    .key_reads_body(4096);

    let addr = spawn_server(echo_router(mw), 6).await;
    let (status, _) = post_form(addr, "/issue", "token=no-address-here").await;
    assert_eq!(status, 200);

    let keys = captured.lock().unwrap().clone();
    let key = &keys[0];
    // The shape the app's per-IP limiter produces.
    assert!(
        !key.starts_with("auth-issuance:ip:"),
        "the fallback must not spell itself like a plain per-IP key, or the two \
         limiters share a bucket under mismatched configs: {key}"
    );
    assert!(
        key.contains("email-absent"),
        "the fallback should say why it fell back: {key}"
    );
}

/// `only_when` must actually skip - a request the predicate rejects
/// passes through however many times it is repeated.
#[tokio::test]
async fn a_skipped_request_is_not_counted_at_all() {
    let mw = RateLimitMiddleware::new(limiter(), one_per_window(), |req| {
        identity_key(req, "email", "issuance")
    })
    .key_reads_body(4096)
    .only_when(|req| suprnova::rate_limit::names_identity(req, "email"));

    let addr = spawn_server(echo_router(mw), 8).await;

    // Three fieldless requests, well past a quota of one.
    let (a, _) = post_form(addr, "/issue", "token=one").await;
    let (b, _) = post_form(addr, "/issue", "token=two").await;
    let (c, _) = post_form(addr, "/issue", "token=three").await;

    assert_eq!(
        (a, b, c),
        (200, 200, 200),
        "skipped requests must not be counted"
    );
}

/// The regression `only_when` exists for.
///
/// Without it, a request naming nobody falls into `identity_key`'s
/// address fallback and is counted against *this* limiter's quota. When
/// this limiter is the tighter of a stacked pair - which is the normal
/// arrangement, since a per-recipient budget is smaller than a per-IP
/// one - that quota silently becomes the binding limit for every route
/// that names nobody, overriding the budget those routes were given.
/// Behind one office NAT that reads as a lockout.
#[tokio::test]
async fn without_the_guard_a_tighter_limiter_binds_requests_it_should_ignore() {
    let mw = RateLimitMiddleware::new(limiter(), one_per_window(), |req| {
        identity_key(req, "email", "issuance")
    })
    .key_reads_body(4096);
    // Deliberately no `.only_when` - this is the shape being guarded against.

    let addr = spawn_server(echo_router(mw), 6).await;

    let (first, _) = post_form(addr, "/issue", "token=one").await;
    let (second, _) = post_form(addr, "/issue", "token=two").await;

    assert_eq!(first, 200);
    assert_eq!(
        second, 429,
        "this is the behaviour `only_when` exists to prevent: fieldless requests \
         consuming the per-identity budget. If this ever returns 200, the fallback \
         changed and the app wiring should be revisited"
    );
}
