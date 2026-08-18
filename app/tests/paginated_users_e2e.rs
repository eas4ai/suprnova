//! End-to-end tests for the three public user routes.
//!
//! Spins up a one-shot hyper server that mounts the app's full route
//! tree via the framework's `handle_request` adapter, then drives real
//! HTTP requests with a hyper client.
//!
//! - `GET /api/users` — cursor pagination. Both branches: the default
//!   Inertia path (`Inertia::paginate("Users/Index", "users", ...)` → a
//!   page object with `props.users` plus scroll metadata) and the
//!   `?format=json` path (raw paginator JSON).
//! - `GET /users` — offset pagination via `simple_paginate`, so the
//!   scroll metadata is page numbers rather than cursors.
//! - `GET /users/{id}` — primary-key fetch with the `profile` HasOne
//!   eager-loaded, including the absent-profile and missing-user cases.
//!
//! All three read the database. They used to serve fixtures, which is
//! why this harness now seeds one.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Empty};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;

use app::models::profiles::Profile;
use app::models::users::User;
use suprnova::{EncryptionKey, MiddlewareRegistry, Model, handle_request};

/// Process-wide guard so the `Crypt::init` call below is a single,
/// idempotent install across every test in this binary.
static CRYPT_INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();

/// Install a test-only encryption key into the framework's process-
/// wide `Crypt` facade. Cursor pagination refuses to emit a payload
/// without an authenticated cipher (codex review finding #1 / no
/// plaintext-base64 fallback), so any test that exercises
/// `Pagination::cursor` end-to-end needs `Crypt` initialised. We
/// generate a fresh test-only key here rather than rely on
/// `Server::from_config` because these tests assemble the router by
/// hand and never go through the server boot path.
fn ensure_crypt_initialised() {
    CRYPT_INIT.get_or_init(|| {
        suprnova::Crypt::init(EncryptionKey::generate());
    });
}

/// How many users the shared test database holds. Larger than the
/// default page size so the first page always has a `next` cursor.
const SEEDED_USERS: i64 = 25;

/// One database for the whole test binary.
///
/// The connection is installed into a *process-global* singleton, so
/// per-test databases would race: `#[tokio::test]` cases in one binary run
/// in parallel, and the second test's `App::singleton` call would swap the
/// connection out from under the first. That was harmless while these
/// routes served an in-memory fixture and only `SessionMiddleware` touched
/// the database. Now the routes read `users`, so which connection is
/// installed decides what the assertions see.
static DB: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

async fn ensure_seeded_db() {
    DB.get_or_init(|| async {
        // `SessionMiddleware` writes a session row on every request; without
        // a bound connection it answers "session persistence failed" with a
        // 500 before the paginator runs.
        // A pooled `sqlite::memory:` URL creates one database per connection.
        // Keep this fixture on one shared in-memory connection so migrations,
        // route reads, and session persistence always see the same schema.
        let config = suprnova::database::DatabaseConfig::builder()
            .url("sqlite:file:paginated-users-e2e?mode=memory&cache=shared")
            .max_connections(1)
            .min_connections(1)
            .logging(false)
            .build();
        let conn = suprnova::database::DbConnection::connect(&config)
            .await
            .expect("connect shared in-memory sqlite");
        <app::migrations::Migrator as sea_orm_migration::MigratorTrait>::up(conn.inner(), None)
            .await
            .expect("migrate shared in-memory sqlite");
        suprnova::App::singleton(conn);

        for i in 1..=SEEDED_USERS {
            User::create(suprnova::attrs! {
                name: format!("user-{i:03}"),
                email: format!("user-{i:03}@example.com"),
                password: "pw",
            })
            .await
            .expect("seed user");
        }

        // User 1 gets a profile, user 2 deliberately does not — the HasOne
        // is `Option`, and both arms of that need covering on the wire.
        Profile::create(suprnova::attrs! {
            user_id: 1_i64,
            bio: "the first user's biography",
        })
        .await
        .expect("seed profile");
    })
    .await;
}

/// Spawn a one-shot hyper server that serves the app's router for a
/// configurable number of inbound connections. Returns the bound
/// address. The accept loop terminates once the per-test budget is
/// drained.
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

/// Send a GET to `path` against `addr`. `inertia_headers=true` sets
/// `X-Inertia: true` + `Accept: text/html, application/xhtml+xml`,
/// matching what the Inertia client sends after the initial visit.
async fn get(
    addr: SocketAddr,
    path: &str,
    inertia_headers: bool,
) -> (hyper::http::StatusCode, hyper::HeaderMap, Bytes) {
    let stream_tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let io = TokioIo::new(stream_tcp);
    let (mut sender, conn) = hyper::client::conn::http1::handshake::<_, Empty<Bytes>>(io)
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let mut builder = hyper::Request::builder()
        .method("GET")
        .uri(path)
        .header("Host", "localhost");
    if inertia_headers {
        builder = builder
            .header("X-Inertia", "true")
            .header("X-Inertia-Version", app::bootstrap::inertia_version())
            .header("Accept", "text/html, application/xhtml+xml");
    }
    let req = builder.body(Empty::<Bytes>::new()).unwrap();

    let resp = sender.send_request(req).await.unwrap();
    let (parts, body) = resp.into_parts();
    let collected = body.collect().await.unwrap();
    (parts.status, parts.headers, collected.to_bytes())
}

#[tokio::test]
async fn inertia_path_emits_users_prop_and_scroll_metadata() {
    let addr = spawn_app_server(2).await;

    // Request 1: as an Inertia XHR (X-Inertia: true) — expect JSON page object.
    let (status, headers, body) = get(addr, "/api/users?per_page=20", true).await;
    assert_eq!(status.as_u16(), 200, "Inertia route should 200");
    // X-Inertia echo confirms the response came from the Inertia builder.
    let ct = headers
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    assert!(
        ct.contains("application/json"),
        "Inertia XHR response should be JSON, got: {ct}"
    );

    let v: serde_json::Value =
        serde_json::from_slice(&body).expect("response should be JSON-parseable");
    // Page object shape: `component`, `props`, `url`, `version`.
    assert_eq!(
        v.get("component").and_then(|c| c.as_str()),
        Some("Users/Index"),
        "expected component 'Users/Index' in page object: {v}"
    );
    let users = v
        .get("props")
        .and_then(|p| p.get("users"))
        .expect("props.users must be present");
    let arr = users.as_array().expect("props.users is the rows array");
    assert_eq!(arr.len(), 20, "first page returns 20 rows by default");
    // First and last row IDs sanity.
    assert_eq!(
        arr.first().and_then(|r| r.get("id")),
        Some(&serde_json::json!(1))
    );
    assert_eq!(
        arr.last().and_then(|r| r.get("id")),
        Some(&serde_json::json!(20))
    );

    // Scroll metadata: the Inertia v3 protocol attaches scroll info
    // under `scrollProps.<key>`. Confirm `next` cursor is set (we have
    // more rows) and `previous` is None (first page).
    let scroll = v
        .get("scrollProps")
        .expect("scrollProps must be present (paginator was wired via Inertia::paginate)");
    let users_scroll = scroll
        .get("users")
        .expect("scrollProps.users must be present");
    assert_eq!(
        users_scroll.get("pageName").and_then(|p| p.as_str()),
        Some("cursor"),
        "page_name should be 'cursor' for CursorPaginator"
    );
    let next = users_scroll
        .get("next")
        .or_else(|| users_scroll.get("nextPage"));
    assert!(
        next.is_some() && !next.unwrap().is_null(),
        "next cursor must be set (more rows remain): {users_scroll:?}"
    );
    let prev = users_scroll
        .get("previous")
        .or_else(|| users_scroll.get("previousPage"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    assert!(
        prev.is_null(),
        "first page must have no prev_cursor: {prev:?}"
    );
}

#[tokio::test]
async fn json_fallback_returns_raw_paginator() {
    let addr = spawn_app_server(1).await;
    let (status, headers, body) = get(addr, "/api/users?per_page=5&format=json", false).await;
    assert_eq!(
        status.as_u16(),
        200,
        "body: {}",
        String::from_utf8_lossy(&body)
    );
    let ct = headers
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    assert!(ct.contains("application/json"), "expected JSON, got {ct}");

    let v: serde_json::Value =
        serde_json::from_slice(&body).expect("response should be JSON-parseable");
    let arr = v["data"].as_array().expect("data must be an array");
    assert_eq!(arr.len(), 5);
    assert_eq!(arr[0]["id"], 1);
    assert_eq!(arr[4]["id"], 5);
    assert_eq!(v["meta"]["page_name"], "cursor");
    assert!(v["meta"]["next"].is_string(), "next cursor must be set");
}

/// The reason `PublicUserProps` exists. `UserProps` serialises `email`,
/// and this route carries no session gate, so the projection is the only
/// thing between an anonymous request and every address in the table.
///
/// Asserted on the wire rather than on the type: a future edit that
/// swapped the projection back to `UserProps` would still compile and
/// still pass every shape assertion above.
#[tokio::test]
async fn public_listing_does_not_leak_email() {
    let addr = spawn_app_server(1).await;
    let (status, _, body) = get(addr, "/api/users?per_page=5&format=json", false).await;
    assert_eq!(status.as_u16(), 200);

    let raw = String::from_utf8_lossy(&body);
    assert!(
        !raw.contains("@example.com"),
        "public listing leaked an email address: {raw}"
    );

    let v: serde_json::Value = serde_json::from_slice(&body).expect("JSON");
    let row = &v["data"][0];
    assert!(
        row.get("email").is_none(),
        "row carried an email field: {row}"
    );
    assert!(
        row.get("name").is_some(),
        "row should still carry name: {row}"
    );
}

/// `GET /users` pages with `simple_paginate`, so its scroll metadata is
/// page numbers under the `page` name — not cursors. That difference is
/// the point of having both routes.
#[tokio::test]
async fn users_index_pages_with_page_numbers() {
    let addr = spawn_app_server(1).await;
    let (status, _, body) = get(addr, "/users", true).await;
    assert_eq!(
        status.as_u16(),
        200,
        "body: {}",
        String::from_utf8_lossy(&body)
    );

    let v: serde_json::Value = serde_json::from_slice(&body).expect("JSON");
    assert_eq!(
        v.get("component").and_then(|c| c.as_str()),
        Some("Users/Index"),
        "page object: {v}"
    );

    let arr = v["props"]["users"]
        .as_array()
        .unwrap_or_else(|| panic!("props.users must be an array: {v}"));
    assert_eq!(arr.len(), 20, "fixed page size of 20");
    assert_eq!(arr[0]["id"], 1);
    assert_eq!(arr[19]["id"], 20);
    assert!(
        arr[0].get("email").is_none(),
        "directory must not carry email: {}",
        arr[0]
    );

    let scroll = &v["scrollProps"]["users"];
    assert_eq!(
        scroll.get("pageName").and_then(|p| p.as_str()),
        Some("page"),
        "simple_paginate must report page numbers, not cursors: {scroll}"
    );
    // 25 seeded users, 20 per page → one more page.
    let next = scroll.get("next").or_else(|| scroll.get("nextPage"));
    assert_eq!(next, Some(&serde_json::json!(2)), "scroll: {scroll}");
    let prev = scroll
        .get("previous")
        .or_else(|| scroll.get("previousPage"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    assert!(prev.is_null(), "first page has no previous: {prev}");
}

#[tokio::test]
async fn users_show_returns_the_eager_loaded_profile() {
    let addr = spawn_app_server(1).await;
    let (status, _, body) = get(addr, "/users/1", true).await;
    assert_eq!(
        status.as_u16(),
        200,
        "body: {}",
        String::from_utf8_lossy(&body)
    );

    let v: serde_json::Value = serde_json::from_slice(&body).expect("JSON");
    assert_eq!(
        v.get("component").and_then(|c| c.as_str()),
        Some("Users/Show"),
        "page object: {v}"
    );
    let user = &v["props"]["user"];
    assert_eq!(user["id"], 1);
    assert_eq!(user["name"], "user-001");
    assert_eq!(
        user["bio"], "the first user's biography",
        "the HasOne must be eager-loaded onto the prop: {user}"
    );
    assert!(
        user.get("email").is_none(),
        "detail page must not carry email: {user}"
    );
}

/// User 2 was seeded without a profile. The HasOne must report that as
/// `null` rather than borrowing user 1's — the failure mode the
/// `relations_dogfood` eager-load test pins at the model layer, asserted
/// here at the HTTP layer.
#[tokio::test]
async fn users_show_reports_a_missing_profile_as_null() {
    let addr = spawn_app_server(1).await;
    let (status, _, body) = get(addr, "/users/2", true).await;
    assert_eq!(status.as_u16(), 200);

    let v: serde_json::Value = serde_json::from_slice(&body).expect("JSON");
    let user = &v["props"]["user"];
    assert_eq!(user["id"], 2);
    assert!(
        user["bio"].is_null(),
        "user 2 has no profile row; bio must be null, not another user's: {user}"
    );
}

#[tokio::test]
async fn users_show_404s_for_an_unknown_id() {
    let addr = spawn_app_server(1).await;
    let (status, _, _) = get(addr, "/users/999999", true).await;
    assert_eq!(
        status.as_u16(),
        404,
        "a primary key with no row must 404, not 500"
    );
}
