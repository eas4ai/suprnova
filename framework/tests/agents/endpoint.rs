//! AGENT-003: the MCP endpoint over Streamable HTTP, revision 2026-07-28
//! and the `initialize`-based revisions 2025-06-18 and 2025-11-25.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use suprnova::agents::AgentsConfig;
use suprnova::{MiddlewareRegistry, ToolCaller};

use crate::fixture::{self, TestUser, scope};

/// A 2026-07-28 request's body, with the metadata every request carries.
fn modern_body(method: &str, params: Value) -> Value {
    let mut params = params.as_object().cloned().unwrap_or_default();
    params.insert(
        "_meta".into(),
        json!({
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientCapabilities": {},
        }),
    );
    json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })
}

#[tokio::test]
async fn agent_003_discover_list_call_2026_07_28() {
    let _container = fixture::setup();
    let client = fixture::client();
    let mcp = client.mcp();

    let discover = mcp.discover().await;
    assert_eq!(discover.status(), 200);
    let result = discover.result().cloned().unwrap_or_default();
    assert_eq!(result["resultType"], "complete");
    assert_eq!(
        result["supportedVersions"],
        json!(["2026-07-28", "2025-11-25", "2025-06-18"])
    );
    assert!(result["capabilities"]["tools"].is_object(), "{result:#}");
    assert!(
        result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"].is_string(),
        "{result:#}"
    );

    let list = mcp.list().await;
    assert_eq!(list.status(), 200);
    let names = list.tool_names();
    assert!(names.contains(&"echo".to_string()), "{names:?}");
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "tools/list is in a fixed order");
    assert_eq!(
        mcp.list().await.tool_names(),
        names,
        "the same order every time"
    );
    let echo = list
        .result()
        .and_then(|result| result["tools"].as_array().cloned())
        .and_then(|tools| tools.into_iter().find(|tool| tool["name"] == "echo"))
        .unwrap_or_default();
    assert_eq!(echo["description"], "Echo the text back.");
    assert_eq!(echo["inputSchema"]["type"], "object");
    assert_eq!(echo["annotations"]["readOnlyHint"], true);

    let call = mcp.call("echo", json!({ "text": "hello" })).await;
    assert!(!call.is_error(), "{:?}", call.response());
    assert_eq!(
        call.structured(),
        Some(&json!({ "text": "hello", "kind": "public" }))
    );
    assert!(
        call.text().contains("hello"),
        "the structured result is also text"
    );
}

#[tokio::test]
async fn agent_003_initialize_2025_06_18_and_2025_11_25_without_session() {
    let _container = fixture::setup();
    let client = fixture::client();
    for version in ["2025-06-18", "2025-11-25"] {
        let mcp = client.mcp().protocol_version(version);
        let init = mcp.initialize().await;
        assert_eq!(init.status(), 200, "{version}: {init:?}");
        let result = init.result().cloned().unwrap_or_default();
        assert_eq!(result["protocolVersion"], version);
        assert!(result["serverInfo"]["name"].is_string());
        assert!(result["capabilities"]["tools"].is_object());
        assert!(
            init.header("mcp-session-id").is_none(),
            "{version}: the endpoint mints no session"
        );
        let list = mcp.list().await;
        assert_eq!(list.status(), 200, "{version}: {list:?}");
        assert!(list.tool_names().contains(&"echo".to_string()));
        assert!(list.header("mcp-session-id").is_none());
        let call = mcp.call("echo", json!({ "text": version })).await;
        assert!(!call.is_error(), "{version}: {:?}", call.response());
        // A session id an old client sends is ignored.
        let ignored = mcp
            .clone()
            .header("Mcp-Session-Id", "made-up")
            .call("echo", json!({ "text": "again" }))
            .await;
        assert!(!ignored.is_error());
        assert!(ignored.response().header("mcp-session-id").is_none());
    }
}

#[tokio::test]
async fn agent_003_header_mismatch_is_400_32020() {
    let _container = fixture::setup();
    let client = fixture::client();
    let mcp = client.mcp();
    let call = modern_body(
        "tools/call",
        json!({ "name": "echo", "arguments": { "text": "x" } }),
    );
    let cases: Vec<(&str, Vec<(&str, &str)>, Value)> = vec![
        (
            "method mismatch",
            vec![
                ("MCP-Protocol-Version", "2026-07-28"),
                ("Mcp-Method", "tools/list"),
                ("Mcp-Name", "echo"),
            ],
            call.clone(),
        ),
        (
            "name mismatch",
            vec![
                ("MCP-Protocol-Version", "2026-07-28"),
                ("Mcp-Method", "tools/call"),
                ("Mcp-Name", "posts.delete"),
            ],
            call.clone(),
        ),
        (
            "missing Mcp-Name",
            vec![
                ("MCP-Protocol-Version", "2026-07-28"),
                ("Mcp-Method", "tools/call"),
            ],
            call.clone(),
        ),
        (
            "missing Mcp-Method",
            vec![("MCP-Protocol-Version", "2026-07-28"), ("Mcp-Name", "echo")],
            call.clone(),
        ),
        (
            "missing MCP-Protocol-Version",
            vec![("Mcp-Method", "tools/call"), ("Mcp-Name", "echo")],
            call.clone(),
        ),
        (
            "_meta version differs from the header",
            vec![
                ("MCP-Protocol-Version", "2026-07-28"),
                ("Mcp-Method", "tools/call"),
                ("Mcp-Name", "echo"),
            ],
            {
                let mut body = call.clone();
                body["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"] =
                    json!("2025-11-25");
                body
            },
        ),
    ];
    for (case, headers, body) in cases {
        let response = mcp.raw(&headers, body).await;
        assert_eq!(response.status(), 400, "{case}: {}", response.body_text());
        assert_eq!(response.json()["error"]["code"], -32020, "{case}");
    }
    // A Base64-encoded name that decodes to the body's name matches.
    let response = mcp
        .raw(
            &[
                ("MCP-Protocol-Version", "2026-07-28"),
                ("Mcp-Method", "tools/call"),
                ("Mcp-Name", "=?base64?ZWNobw==?="),
            ],
            call,
        )
        .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
}

#[tokio::test]
async fn agent_003_unsupported_version_is_32022_with_supported_list() {
    let _container = fixture::setup();
    let client = fixture::client();
    let mcp = client.mcp();
    let response = mcp
        .raw(
            &[
                ("MCP-Protocol-Version", "2024-11-05"),
                ("Mcp-Method", "tools/list"),
            ],
            json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/list", "params": {} }),
        )
        .await;
    assert_eq!(response.status(), 400);
    let body = response.json();
    assert_eq!(body["error"]["code"], -32022);
    assert_eq!(
        body["error"]["data"]["supported"],
        json!(["2026-07-28", "2025-11-25", "2025-06-18"])
    );
    assert_eq!(body["error"]["data"]["requested"], "2024-11-05");

    let init = client
        .mcp()
        .protocol_version("2025-03-26")
        .initialize()
        .await;
    assert_eq!(init.status(), 400);
    assert_eq!(init.error_code(), Some(-32022));
}

#[tokio::test]
async fn agent_003_get_and_delete_are_405() {
    let _container = fixture::setup();
    let client = fixture::client();
    for response in [
        client
            .get("/mcp")
            .header("Accept", "text/event-stream")
            .send()
            .await,
        client.delete("/mcp").send().await,
    ] {
        assert_eq!(response.status(), 405);
        assert_eq!(response.header("allow"), Some("POST"));
        assert_ne!(response.header("content-type"), Some("text/event-stream"));
    }
}

#[tokio::test]
async fn agent_003_foreign_origin_is_403() {
    let _container = fixture::setup();
    let key = "agent-003-origin";
    let writer = || ToolCaller::user(TestUser::new("7")).scopes([scope::POSTS_WRITE]);
    let client = fixture::client_with(
        AgentsConfig::new().allow_origin("https://app.example.com"),
        MiddlewareRegistry::new(),
    );
    let refused = client
        .mcp()
        .as_caller(writer())
        .header("Origin", "https://evil.example")
        .header("User-Agent", key)
        .call("posts.create", json!({ "title": "Rebound" }))
        .await;
    assert_eq!(refused.status(), 403);
    assert_eq!(fixture::seen(key), 0, "nothing ran for the foreign origin");
    for origin in ["https://app.example.com", "http://localhost"] {
        let allowed = client
            .mcp()
            .as_caller(writer())
            .header("Origin", origin)
            .call("posts.create", json!({ "title": "Allowed" }))
            .await;
        assert_eq!(allowed.status(), 200, "{origin}: {:?}", allowed.response());
    }
}

#[tokio::test]
async fn agent_003_oversized_body_is_413() {
    let _container = fixture::setup();
    let client = fixture::client_with(
        AgentsConfig::new().max_body_bytes(256),
        MiddlewareRegistry::new(),
    );
    let result = client
        .mcp()
        .call("echo", json!({ "text": "x".repeat(1024) }))
        .await;
    assert_eq!(result.status(), 413);
}

#[tokio::test]
async fn agent_003_slow_call_is_cut_off() {
    let _container = fixture::setup();
    let client = fixture::client_with(
        AgentsConfig::new().call_timeout(Duration::from_millis(200)),
        MiddlewareRegistry::new(),
    );
    let key = "agent-003-slow";
    let result = client
        .mcp()
        .call("slow.wait", json!({ "millis": 20_000, "key": key }))
        .await;
    assert_eq!(result.status(), 200);
    assert!(result.is_error(), "{:?}", result.response());
    assert!(
        result.text().contains("did not finish"),
        "{}",
        result.text()
    );
    assert!(fixture::cancelled(key), "the overlong call was stopped");
}

#[tokio::test]
async fn agent_003_invalid_input_is_is_error_result_not_protocol_error() {
    let _container = fixture::setup();
    let client = fixture::client();
    let result = client.mcp().call("echo", json!({ "text": "" })).await;
    assert_eq!(result.status(), 200);
    assert_eq!(result.error_code(), None);
    assert!(result.is_error());
    assert!(result.text().contains("- text:"), "{}", result.text());
    // A missing required field too.
    let result = client.mcp().call("echo", json!({})).await;
    assert!(result.is_error(), "{:?}", result.response());
    assert_eq!(result.error_code(), None);
}

#[tokio::test]
async fn agent_003_unknown_tool_and_unknown_method() {
    let _container = fixture::setup();
    let client = fixture::client();
    let unknown = client.mcp().call("no.such.tool", json!({})).await;
    assert_eq!(unknown.error_code(), Some(-32602));
    let method = client.mcp().request("resources/list", json!({})).await;
    assert_eq!(method.status(), 404);
    assert_eq!(method.error_code(), Some(-32601));
}

#[tokio::test]
async fn agent_003_notification_is_202_and_batch_is_refused() {
    let _container = fixture::setup();
    let client = fixture::client();
    let mcp = client.mcp();
    let notification = mcp
        .raw(
            &[],
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        )
        .await;
    assert_eq!(notification.status(), 202);
    assert!(notification.body_text().is_empty());
    let batch = mcp
        .raw(
            &[],
            json!([{ "jsonrpc": "2.0", "id": 1, "method": "ping" }]),
        )
        .await;
    assert_eq!(batch.status(), 400);
    assert_eq!(batch.json()["error"]["code"], -32600);
}

#[tokio::test]
async fn agent_003_sse_answer_when_accept_prefers_it() {
    let _container = fixture::setup();
    let client = fixture::client();
    let result = client
        .mcp()
        .header("Accept", "text/event-stream")
        .call("echo", json!({ "text": "streamed" }))
        .await;
    let response = result.response().response();
    assert_eq!(response.status(), 200);
    assert!(
        response
            .header("content-type")
            .is_some_and(|value| value.starts_with("text/event-stream")),
        "{:?}",
        response.header("content-type")
    );
    assert!(
        response.body_text().starts_with("event: message"),
        "{}",
        response.body_text()
    );
    assert_eq!(
        result.structured(),
        Some(&json!({ "text": "streamed", "kind": "public" }))
    );
}

#[tokio::test]
async fn agent_003_dropped_sse_body_cancels_call() {
    let _container = fixture::setup();
    let router = Arc::new(fixture::router());
    let registry = Arc::new(MiddlewareRegistry::new());
    let key = "agent-003-dropped";
    let body = modern_body(
        "tools/call",
        json!({ "name": "slow.wait", "arguments": { "millis": 30_000, "key": key } }),
    );
    let request = hyper::Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("Host", "localhost")
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("Mcp-Method", "tools/call")
        .header("Mcp-Name", "slow.wait")
        .body(Full::new(Bytes::from(body.to_string())))
        .expect("a valid request");

    let started = fixture::SLOW_STARTED.load(std::sync::atomic::Ordering::SeqCst);
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let service = service_fn(move |req: hyper::Request<Incoming>| {
        let router = router.clone();
        let registry = registry.clone();
        async move { Ok::<_, Infallible>(suprnova::handle_request(router, registry, req).await) }
    });
    let server = tokio::spawn(
        hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(server_io), service),
    );
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(client_io))
            .await
            .expect("handshake");
    let connection = tokio::spawn(connection);
    let response = sender
        .send_request(request)
        .await
        .expect("the response head");
    assert_eq!(response.status(), 200);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("text/event-stream")
    );
    // Wait until the call is running, then close the stream.
    let mut body = response.into_body();
    let poll = tokio::time::timeout(Duration::from_millis(300), body.frame()).await;
    assert!(poll.is_err(), "the 30 second call answered early");
    for _ in 0..100 {
        if fixture::SLOW_STARTED.load(std::sync::atomic::Ordering::SeqCst) > started {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    drop(body);
    drop(sender);
    connection.abort();
    let _ = tokio::time::timeout(Duration::from_secs(5), server).await;
    for _ in 0..100 {
        if fixture::cancelled(key) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        fixture::cancelled(key),
        "closing the stream cancelled the call"
    );
}
