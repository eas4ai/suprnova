//! End-to-end tests for the `/bench/*` routes.
//!
//! Two halves, and both matter:
//!
//! - Under `--features bench`, each route answers correctly against a
//!   real (small) database. A benchmark route that 500s is worse than a
//!   missing one — the load generator reports a rate and a latency for a
//!   500 just as happily as for a 200, so a broken route produces a
//!   plausible number rather than an obvious failure.
//! - Without the feature, the routes are **absent**. That is the whole
//!   justification for the gate, and it is asserted rather than assumed.
//!
//! Run the first half with:
//!
//! ```sh
//! cargo test -p app --features bench --test bench_routes_e2e
//! ```

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Empty};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;

use app::models::posts::Post;
use app::models::profiles::Profile;
use app::models::users::User;
use suprnova::{EncryptionKey, MiddlewareRegistry, Model, handle_request};

static CRYPT_INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();

fn ensure_crypt_initialised() {
    CRYPT_INIT.get_or_init(|| {
        suprnova::Crypt::init(EncryptionKey::generate());
    });
}

const SEEDED_USERS: i64 = 5;
const POSTS_PER_USER: i64 = 3;

/// One database for the whole binary — see the note in
/// `paginated_users_e2e`: the connection lands in a process-global
/// singleton, so per-test databases race.
static DB: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

async fn ensure_seeded_db() {
    DB.get_or_init(|| async {
        let conn = sea_orm::Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite::memory:");
        <app::migrations::Migrator as sea_orm_migration::MigratorTrait>::up(&conn, None)
            .await
            .expect("migrate sqlite::memory:");
        suprnova::App::singleton(suprnova::DbConnection::from_raw(conn));

        for i in 1..=SEEDED_USERS {
            User::create(suprnova::attrs! {
                name: format!("user-{i:03}"),
                email: format!("user-{i:03}@example.com"),
                password: "pw",
            })
            .await
            .expect("seed user");

            Profile::create(suprnova::attrs! {
                user_id: i,
                bio: format!("bio for user {i}"),
            })
            .await
            .expect("seed profile");

            for p in 1..=POSTS_PER_USER {
                Post::create(suprnova::attrs! {
                    author_id: i,
                    title: format!("post {p} by {i}"),
                    body: "body",
                    is_public: true,
                })
                .await
                .expect("seed post");
            }
        }
    })
    .await;
}

async fn spawn_app_server(max_connections: usize) -> SocketAddr {
    ensure_crypt_initialised();
    ensure_seeded_db().await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = Arc::new(app::routes::register());
    let middleware = Arc::new({
        app::bootstrap::register_http_stack();
        MiddlewareRegistry::from_global()
    });

    tokio::spawn(async move {
        for _ in 0..max_connections {
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

async fn request(
    addr: SocketAddr,
    method: &str,
    path: &str,
) -> (hyper::StatusCode, serde_json::Value) {
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await.unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let req = hyper::Request::builder()
        .method(method)
        .uri(format!("http://{addr}{path}"))
        .header("host", addr.to_string())
        .body(Empty::<Bytes>::new())
        .unwrap();

    let res = sender.send_request(req).await.unwrap();
    let status = res.status();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
    (status, json)
}

// ---------------------------------------------------------------------
// Without the feature: the routes do not exist.
// ---------------------------------------------------------------------

/// The gate's whole purpose. `controllers::bench` is not compiled into a
/// default build, so `routes::register` never mounts the group and every
/// path under it 404s.
#[cfg(not(feature = "bench"))]
#[tokio::test]
async fn bench_routes_are_absent_without_the_feature() {
    let addr = spawn_app_server(4).await;
    for path in [
        "/bench/dashboard",
        "/bench/users/hydrate",
        "/bench/posts/paginated",
        "/bench/posts/1/deep",
    ] {
        let (status, _) = request(addr, "GET", path).await;
        assert_eq!(
            status.as_u16(),
            404,
            "{path} must not exist in a build without --features bench"
        );
    }
}

// ---------------------------------------------------------------------
// With the feature: each route answers.
// ---------------------------------------------------------------------

#[cfg(feature = "bench")]
#[tokio::test]
async fn dashboard_runs_five_queries_concurrently() {
    let addr = spawn_app_server(1).await;
    let (status, v) = request(addr, "GET", "/bench/dashboard?user_id=1").await;
    assert_eq!(status.as_u16(), 200, "body: {v}");

    assert_eq!(v["user_id"], 1);
    assert_eq!(v["found"], true);
    assert_eq!(
        v["posts"], POSTS_PER_USER,
        "user 1 authored {POSTS_PER_USER} posts: {v}"
    );
    assert_eq!(v["roles"], 0, "no role_user rows are seeded here: {v}");
    assert!(
        v["query_us"].is_u64(),
        "the handler must report its own query time: {v}"
    );
}

#[cfg(feature = "bench")]
#[tokio::test]
async fn users_hydrate_honours_and_bounds_the_row_count() {
    let addr = spawn_app_server(2).await;

    let (status, v) = request(addr, "GET", "/bench/users/hydrate?rows=3").await;
    assert_eq!(status.as_u16(), 200, "body: {v}");
    assert_eq!(v["count"], 3);
    assert_eq!(v["users"].as_array().unwrap().len(), 3);

    // The seeded table is smaller than the request, so this also pins
    // that asking for more rows than exist is not an error.
    let (status, v) = request(addr, "GET", "/bench/users/hydrate?rows=999999999").await;
    assert_eq!(status.as_u16(), 200, "body: {v}");
    assert_eq!(
        v["count"], SEEDED_USERS,
        "an absurd ?rows= must clamp, not fail: {v}"
    );
}

/// The hydration route must never emit the password hash. It serialises
/// explicitly rather than through the model for exactly this reason.
#[cfg(feature = "bench")]
#[tokio::test]
async fn users_hydrate_does_not_emit_credentials() {
    let addr = spawn_app_server(1).await;
    let (status, v) = request(addr, "GET", "/bench/users/hydrate?rows=5").await;
    assert_eq!(status.as_u16(), 200);
    let raw = v.to_string();
    assert!(!raw.contains("password"), "leaked a password field: {raw}");
    assert!(!raw.contains("remember_token"), "leaked a token: {raw}");
    assert!(
        v["users"][0].get("email").is_none(),
        "hydration route should not carry email: {v}"
    );
}

#[cfg(feature = "bench")]
#[tokio::test]
async fn post_deep_loads_every_relation_kind() {
    let addr = spawn_app_server(2).await;
    let (status, v) = request(addr, "GET", "/bench/posts/1/deep").await;
    assert_eq!(status.as_u16(), 200, "body: {v}");

    assert_eq!(v["id"], 1);
    assert_eq!(
        v["author"]["id"], 1,
        "the BelongsTo must be eager-loaded: {v}"
    );
    assert!(v["comments"].is_u64());
    assert!(v["tags"].is_u64());

    let (status, _) = request(addr, "GET", "/bench/posts/999999/deep").await;
    assert_eq!(status.as_u16(), 404, "an unknown post must 404, not 500");
}

#[cfg(feature = "bench")]
#[tokio::test]
async fn posts_paginated_returns_a_bounded_page() {
    let addr = spawn_app_server(1).await;
    let (status, v) = request(addr, "GET", "/bench/posts/paginated?per_page=5").await;
    assert_eq!(status.as_u16(), 200, "body: {v}");

    assert_eq!(v["per_page"], 5);
    let posts = v["posts"].as_array().expect("posts array");
    assert!(posts.len() <= 5, "page must respect per_page: {v}");
    assert!(
        posts[0].get("author").is_some(),
        "the eager-loaded author must be on the row: {v}"
    );
}

#[cfg(feature = "bench")]
#[tokio::test]
async fn posts_bulk_inserts_ten_rows_in_one_transaction() {
    let addr = spawn_app_server(1).await;

    let before = <Post as Model>::query()
        .count()
        .await
        .expect("count before");
    let (status, v) = request(addr, "POST", "/bench/posts/bulk?author_id=1").await;
    assert_eq!(status.as_u16(), 201, "body: {v}");
    assert_eq!(v["inserted"], 10);

    let after = <Post as Model>::query().count().await.expect("count after");
    assert_eq!(
        after - before,
        10,
        "the transaction must have committed all ten rows"
    );
}

/// The gauges must be real numbers off the live pool, not zeroes. A
/// collector plotting saturation cannot tell a flat healthy line from a
/// pool that was never sampled, so "readable" is the property under test.
#[cfg(feature = "bench")]
#[tokio::test]
async fn pool_stats_reports_the_live_pool() {
    let addr = spawn_app_server(1).await;
    let (status, v) = request(addr, "GET", "/debug/pool-stats").await;
    assert_eq!(status.as_u16(), 200, "body: {v}");

    let size = v["size"].as_u64().expect("size must be a number");
    let idle = v["idle"].as_u64().expect("idle must be a number");
    let in_use = v["in_use"].as_u64().expect("in_use must be a number");

    assert!(size >= 1, "an established pool holds at least one: {v}");
    assert_eq!(
        in_use,
        size.saturating_sub(idle),
        "in_use must agree with size - idle: {v}"
    );
}

/// Without `BENCH_ECHO_URL` the route has no downstream. It must say so
/// as a 503 rather than a 500: in the vegeta output a misconfigured
/// harness and a failing server would otherwise look identical.
#[cfg(feature = "bench")]
#[tokio::test]
async fn external_reports_a_missing_downstream_as_503() {
    // Deliberately not setting the variable: mutating process env from a
    // test races every other test in the binary.
    if std::env::var("BENCH_ECHO_URL").is_ok() {
        return;
    }
    let addr = spawn_app_server(1).await;
    let (status, _) = request(addr, "GET", "/bench/external?delay=0").await;
    assert_eq!(status.as_u16(), 503);
}
