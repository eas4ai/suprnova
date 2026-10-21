//! The framework's frames below a Live action fit a thread with Tokio's
//! default 2 MiB stack in a debug build, with room left for the action.
//!
//! The child process below runs the real server on a thread whose stack the
//! test sizes itself, under a current-thread runtime, so the server's tasks
//! and the request's frames run on that thread; the dogfood counter's action
//! records how deep below the thread's entry it ran. The test fails when
//! that depth passes the budget, before an application's own action, a
//! little deeper, overflows the default stack.
// Its own test binary: `Server::run` binds the process-global Live runtime,
// and the measuring thread must be the only one running the action.
#[path = "../support/env_lock.rs"]
mod env_lock;
#[path = "../support/env_snapshot.rs"]
mod env_snapshot;
#[path = "../support/live_dogfood_support/mod.rs"]
mod live_dogfood_support;
#[path = "../support/own_process.rs"]
mod own_process;

use std::sync::Arc;
use std::time::Duration;

use live_dogfood_support::{
    ActionRequest, DOCUMENT_PATH, LoginHeader, MemorySessionStore, action_request, build_router,
    decoded_snapshot, fixture, get, send, session_cookie, stack_probe,
};
use suprnova::{
    CsrfMiddleware, OriginPolicy, Server, SessionConfig, SessionMiddleware, StatusCode,
};

/// Tokio's default thread stack: what a runtime built without
/// `thread_stack_size` gives each worker and blocking thread.
const TOKIO_DEFAULT_STACK: usize = 2 * 1024 * 1024;

/// The most stack the framework's own frames below an action may use on
/// that thread, leaving the rest to the action and whatever it calls.
const FRAMES_BUDGET: usize = 1024 * 1024;

#[test]
fn frames_below_a_live_action_fit_the_budget() {
    own_process::run_alone("frames_below_a_live_action_fit_the_budget_child");
}

#[test]
fn frames_below_a_live_action_fit_the_budget_child() {
    if !own_process::is_child() {
        return;
    }
    let depth = std::thread::Builder::new()
        .name("live-action-on-a-default-stack".to_owned())
        .stack_size(TOKIO_DEFAULT_STACK)
        .spawn(|| {
            stack_probe::mark_base();
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build the runtime");
            runtime.block_on(run_a_live_action());
            stack_probe::depth().expect("the counter's action recorded its depth")
        })
        .expect("spawn the measuring thread")
        .join()
        .expect("the measuring thread finished, so the frames fit its stack");
    eprintln!("frames below the action: {depth} bytes");
    assert!(
        depth <= FRAMES_BUDGET,
        "the framework's frames below a Live action use {depth} bytes of stack in this build, \
         past the budget of {FRAMES_BUDGET}: box the large futures that grew"
    );
}

/// Serve the dogfood document and run its counter action through the real
/// server, as `live_dogfood_server` does.
async fn run_a_live_action() {
    // The server requires APP_KEY outside development, and an unset APP_ENV
    // is production; this process names itself a test one.
    let _app_env = env_snapshot::EnvSnapshot::capture(&["APP_ENV"]);
    env_snapshot::set_env("APP_ENV", Some("testing"));
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
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
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
