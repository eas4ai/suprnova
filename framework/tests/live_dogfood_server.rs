//! The real server boot path serves a guarded Live document and action.
//!
//! `Server::run` binds the process-wide Live runtime, so this test owns its
//! process; the in-process suite lives in `live_dogfood.rs`.

mod live_dogfood_support;

use std::sync::Arc;
use std::time::Duration;

use live_dogfood_support::{
    ActionRequest, DOCUMENT_PATH, LoginHeader, MemorySessionStore, action_request, build_router,
    decoded_snapshot, fixture, get, send, session_cookie,
};
use suprnova::{
    CsrfMiddleware, OriginPolicy, Server, SessionConfig, SessionMiddleware, StatusCode,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn the_real_server_boot_path_serves_a_live_document() {
    fixture();
    let probe = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("probe port");
    let port = probe.local_addr().expect("probe address").port();
    drop(probe);
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    let server = Server::new(build_router())
        .host("127.0.0.1")
        .port(port)
        .middleware(SessionMiddleware::with_store(
            config,
            Arc::new(MemorySessionStore::default()),
        ))
        .middleware(CsrfMiddleware::new().with_origin_policy(OriginPolicy::SameOriginOnly))
        .middleware(LoginHeader);
    let task = tokio::spawn(async move {
        if let Err(error) = server.run().await {
            eprintln!("server run failed: {error}");
        }
    });
    let address: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().expect("address");
    let mut attempts = 0;
    let (status, headers, body) = loop {
        if tokio::net::TcpStream::connect(address).await.is_ok() {
            break send(address, get(DOCUMENT_PATH)).await;
        }
        attempts += 1;
        assert!(attempts < 200, "server never started listening");
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    assert_eq!(status, StatusCode::OK);
    assert!(
        std::str::from_utf8(&body)
            .expect("html")
            .contains("data-suprnova-live-island")
    );
    let cookie = session_cookie(&headers);
    let snapshot = decoded_snapshot(&body);
    let (status, _, body) = send(
        address,
        action_request(ActionRequest {
            snapshot,
            cookie: &cookie,
            fetch_site: Some("same-origin"),
            login: Some("user-7"),
            idempotency_key: "UFFSU1RVVldYWVpbXF1eXw",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    task.abort();
}
