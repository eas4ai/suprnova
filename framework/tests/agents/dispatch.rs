//! AGENT-001: a tool call runs through its route: the route's middleware,
//! form request validation, `#[authorize]` policy and throttle, with the
//! path built so an argument cannot reach another route, and without the
//! caller's token.

use std::net::{IpAddr, Ipv4Addr};

use serde_json::json;
use suprnova::ToolCaller;

use crate::fixture::{self, TestUser, scope};

fn writer() -> ToolCaller {
    ToolCaller::user(TestUser::arc("7")).scopes([scope::POSTS_WRITE])
}

#[tokio::test]
async fn agent_001_tool_call_runs_route_middleware() {
    let _container = fixture::setup();
    let client = fixture::client();
    let key = "agent-001-route-middleware";
    let result = client
        .mcp()
        .as_caller(writer())
        .header("User-Agent", key)
        .call("posts.create", json!({ "title": "Counted" }))
        .await;
    assert!(!result.is_error(), "{:?}", result.response());
    assert_eq!(
        fixture::seen(key),
        1,
        "the route's middleware saw the call once"
    );
}

#[tokio::test]
async fn agent_001_tool_call_runs_form_request_validation_and_returns_is_error_per_field() {
    let _container = fixture::setup();
    let client = fixture::client();
    let result = client
        .mcp()
        .as_caller(writer())
        .call("posts.create", json!({ "title": "", "stars": 9 }))
        .await;
    assert_eq!(result.status(), 200);
    assert_eq!(
        result.error_code(),
        None,
        "a validation failure is not a protocol error"
    );
    assert!(result.is_error(), "{:?}", result.response());
    let text = result.text();
    assert!(text.contains("- title:"), "{text}");
    assert!(text.contains("- stars:"), "{text}");
}

#[tokio::test]
async fn agent_001_tool_call_runs_authorize_policy() {
    let _container = fixture::setup();
    let client = fixture::client();
    // An agent bound to no user is a guest to `#[authorize]`, and the
    // delete policy is not nullable: the check refuses it.
    let result = client
        .mcp()
        .as_caller(ToolCaller::agent("reg-1").scopes([scope::POSTS_READ, scope::POSTS_DELETE]))
        .call("posts.delete", json!({ "post": "42" }))
        .await;
    assert!(result.is_error(), "{:?}", result.response());
    // A user passes the same check.
    let result = client
        .mcp()
        .as_caller(
            ToolCaller::user(TestUser::arc("7")).scopes([scope::POSTS_READ, scope::POSTS_DELETE]),
        )
        .call("posts.delete", json!({ "post": "42" }))
        .await;
    assert!(!result.is_error(), "{:?}", result.response());
    assert_eq!(
        result.structured(),
        Some(&json!({ "deleted": "42", "same": true }))
    );
}

#[tokio::test]
async fn agent_001_tool_call_runs_route_throttle() {
    let _container = fixture::setup();
    let client = fixture::client().with_peer_address(IpAddr::V4(Ipv4Addr::new(10, 1, 0, 1)));
    let mcp = client.mcp();
    for _ in 0..2 {
        let result = mcp.call("throttled.ping", json!({})).await;
        assert!(!result.is_error(), "{:?}", result.response());
    }
    let third = mcp.call("throttled.ping", json!({})).await;
    assert!(
        third.is_error(),
        "the route's throttle refused the third call"
    );
    assert!(third.text().contains("Too many"), "{}", third.text());
}

#[tokio::test]
async fn agent_001_path_parameter_cannot_inject_a_segment() {
    let _container = fixture::setup();
    let client = fixture::client();
    let mcp = client
        .mcp()
        .as_caller(ToolCaller::user(TestUser::arc("7")).scopes([scope::POSTS_READ]));
    for value in ["a/b", "../admin", "%2F", "x?y=1#z"] {
        let result = mcp.call("posts.show", json!({ "post": value })).await;
        assert!(!result.is_error(), "{value}: {:?}", result.response());
        assert_eq!(
            result.structured(),
            Some(&json!({ "post": value, "same": true })),
            "the whole value reached the tool's own route"
        );
    }
    // `new` would match the literal `/posts/new` route, which is not the
    // tool's: the call is refused rather than sent there.
    let result = mcp.call("posts.show", json!({ "post": "new" })).await;
    assert!(result.is_error(), "{:?}", result.response());
}

#[tokio::test]
async fn agent_001_tool_call_does_not_forward_the_bearer_token() {
    let _container = fixture::setup();
    let client = fixture::client().with_peer_address(IpAddr::V4(Ipv4Addr::new(10, 1, 0, 2)));
    let result = client
        .mcp()
        .as_caller(ToolCaller::user(TestUser::arc("7")))
        .header("Cookie", "suprnova_session=abc")
        .header("User-Agent", "peek-agent")
        .call("request.peek", json!({}))
        .await;
    assert!(!result.is_error(), "{:?}", result.response());
    assert_eq!(
        result.structured(),
        Some(&json!({
            "authorization": false,
            "cookie": false,
            "ip": "10.1.0.2",
            "user_agent": "peek-agent",
        }))
    );
}

#[tokio::test]
async fn agent_001_tool_error_body_is_text_not_html() {
    let _container = fixture::setup();
    let client = fixture::client();
    let result = client.mcp().call("page.html", json!({})).await;
    assert!(result.is_error(), "a page is not a tool result");
    assert!(!result.text().contains("<p>"), "{}", result.text());
}

#[tokio::test]
async fn agent_001_render_cache_does_not_serve_tool_calls() {
    use sea_orm_migration::{MigrationTrait, MigratorTrait};
    use std::sync::atomic::Ordering;
    use suprnova::render_cache::config::RenderCacheConfig;
    use suprnova::render_cache::registry::GroupPolicy;
    use suprnova::render_cache::testing::RenderCacheProbe;
    use suprnova::render_cache::{
        CoordinatorConfig, FreshnessPolicy, L1Config, RenderCache, RenderCachePolicy,
        RepresentationClass,
    };
    use suprnova::{ConnectionTrait, MiddlewareRegistry};

    struct Migrator;

    #[async_trait::async_trait]
    impl MigratorTrait for Migrator {
        fn migrations() -> Vec<Box<dyn MigrationTrait>> {
            vec![Box::new(suprnova::render_cache::migration::Migration)]
        }
    }

    // The RenderCache runtime and the global middleware are process-wide.
    if crate::own_process_async::delegate(
        module_path!(),
        "agent_001_render_cache_does_not_serve_tool_calls",
    )
    .await
    {
        return;
    }
    suprnova::Crypt::init(suprnova::EncryptionKey::generate());
    suprnova::Config::register(
        suprnova::AppConfig::builder()
            .url("http://localhost")
            .debug(false)
            .build(),
    );
    suprnova::App::init();
    suprnova::middleware::clear_global_middleware_for_test();
    let _container = fixture::setup();
    let tempdir = tempfile::tempdir().expect("a database directory");
    let config = suprnova::database::DatabaseConfig::builder()
        .url(format!(
            "sqlite://{}",
            tempdir.path().join("cache.sqlite3").display()
        ))
        .max_connections(4)
        .min_connections(1)
        .logging(false)
        .build();
    let conn = suprnova::database::DbConnection::connect(&config)
        .await
        .expect("connect SQLite");
    conn.inner()
        .execute_unprepared("PRAGMA journal_mode=WAL")
        .await
        .expect("WAL journaling");
    Migrator::up(conn.inner(), None)
        .await
        .expect("the RenderCache migration");
    suprnova::testing::TestContainer::singleton(conn);

    // `page.html` answers a public page, which this policy would store and
    // serve to the next request.
    let policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("a freshness"))
        .build()
        .expect("a policy");
    let router = fixture::router()
        .try_render_cache("/page", GroupPolicy::from(policy))
        .expect("attach the policy");
    let mut cache = RenderCacheConfig::from_env().expect("a RenderCache configuration");
    cache.enabled = true;
    cache.l1 = L1Config::Disabled;
    cache.coordinator = CoordinatorConfig::Local {
        lease_ms: 30_000,
        max_waiters: 128,
    };
    let router = RenderCache::install(router, cache)
        .await
        .expect("install the RenderCache");
    let client = suprnova::testing::TestClient::new(router, MiddlewareRegistry::from_global());

    let before = fixture::PAGE_RENDERS.load(Ordering::SeqCst);
    for _ in 0..2 {
        let result = client.mcp().call("page.html", json!({})).await;
        assert_eq!(result.status(), 200, "{:?}", result.response());
    }
    assert_eq!(
        fixture::PAGE_RENDERS.load(Ordering::SeqCst) - before,
        2,
        "each tool call ran the handler; none was served from the cache"
    );
    let stored = RenderCacheProbe::route("/page")
        .l0()
        .await
        .expect("probe the cache");
    assert!(stored.is_none(), "no tool call was stored");
}

#[tokio::test]
async fn agent_001_dispatch_after_response_runs_after_a_tool_call() {
    use bytes::Bytes;
    use http_body_util::{BodyExt, Full};
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    let _container = fixture::setup();
    suprnova::Bus::register::<fixture::Later, _>(fixture::LaterHandler);
    let router = Arc::new(fixture::router());
    let registry = Arc::new(suprnova::MiddlewareRegistry::new());
    let before = fixture::LATER_RUNS.load(Ordering::SeqCst);

    // A connection with the framework server's write-completion hook, so
    // the queued command can start once the answer's bytes are out.
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let server = tokio::spawn(suprnova::after_response_connection(
        server_io,
        move |io| async move {
            let service = hyper::service::service_fn(move |request| {
                let router = router.clone();
                let registry = registry.clone();
                async move {
                    Ok::<_, std::convert::Infallible>(
                        suprnova::handle_request(router, registry, request).await,
                    )
                }
            });
            hyper::server::conn::http1::Builder::new()
                .serve_connection(hyper_util::rt::TokioIo::new(io), service)
                .await
        },
    ));
    let (mut sender, connection) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(
        hyper_util::rt::TokioIo::new(client_io),
    )
    .await
    .expect("handshake");
    let connection = tokio::spawn(connection);
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "bus.later",
            "arguments": {},
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientCapabilities": {},
            },
        },
    });
    let request = hyper::Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("Host", "localhost")
        .header("Content-Type", "application/json")
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("Mcp-Method", "tools/call")
        .header("Mcp-Name", "bus.later")
        .body(Full::new(Bytes::from(body.to_string())))
        .expect("a valid request");
    let response = sender.send_request(request).await.expect("the answer");
    let answer: serde_json::Value = serde_json::from_slice(
        &response
            .into_body()
            .collect()
            .await
            .expect("the body")
            .to_bytes(),
    )
    .expect("a JSON answer");
    assert_eq!(answer["result"]["isError"], false, "{answer}");
    let ran = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while fixture::LATER_RUNS.load(Ordering::SeqCst) == before {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await;
    assert!(
        ran.is_ok(),
        "the command queued in the tool call ran after its answer"
    );
    drop(sender);
    connection.abort();
    server.abort();
}
