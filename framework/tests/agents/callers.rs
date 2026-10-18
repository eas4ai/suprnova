//! AGENT-002: the caller kinds a tool accepts, the caller the handler and
//! the policies see, and the per-caller limit.

use std::net::{IpAddr, Ipv4Addr};

use serde_json::json;
use suprnova::agents::{AgentsConfig, CallLimit};
use suprnova::{MiddlewareRegistry, ToolCaller};

use crate::fixture::{self, TestUser, scope};

#[tokio::test]
async fn agent_002_policy_reads_caller_kind() {
    let _container = fixture::setup();
    let client = fixture::client();
    let scopes = [scope::POSTS_READ, scope::POSTS_DELETE];
    let user = client
        .mcp()
        .as_caller(ToolCaller::user(TestUser::arc("7")).scopes(scopes))
        .call("posts.delete", json!({ "post": "1" }))
        .await;
    assert!(!user.is_error(), "a user may delete: {:?}", user.response());
    // The same user's agent account is signed in as the user, and the
    // policy still tells it apart by the caller.
    let agent = client
        .mcp()
        .as_caller(
            ToolCaller::agent("reg-7")
                .claimed(true)
                .bound_to(TestUser::arc("7"))
                .scopes(scopes),
        )
        .call("posts.delete", json!({ "post": "1" }))
        .await;
    assert!(
        agent.is_error(),
        "an agent may not delete: {:?}",
        agent.response()
    );
    assert!(agent.text().contains("unauthorized"), "{}", agent.text());
}

#[tokio::test]
async fn agent_002_user_tool_rejects_agent_bound_to_user() {
    let _container = fixture::setup();
    let client = fixture::client();
    let result = client
        .mcp()
        .as_caller(
            ToolCaller::agent("reg-8")
                .claimed(true)
                .bound_to(TestUser::arc("8")),
        )
        .call("users.profile", json!({}))
        .await;
    assert_eq!(result.status(), 403);
    assert!(result.structured().is_none(), "the tool did not run");
    let www = result
        .response()
        .header("www-authenticate")
        .unwrap_or_default();
    assert!(
        !www.contains("insufficient_scope"),
        "a wrong caller kind is not a missing scope: {www}"
    );
    // The user the agent is bound to may call it.
    let result = client
        .mcp()
        .as_caller(ToolCaller::user(TestUser::arc("8")))
        .call("users.profile", json!({}))
        .await;
    assert_eq!(result.structured(), Some(&json!({ "user": "8" })));
}

#[tokio::test]
async fn agent_002_handler_receives_tool_caller() {
    let _container = fixture::setup();
    let client = fixture::client();
    let user = client
        .mcp()
        .as_caller(
            ToolCaller::user(TestUser::arc("7"))
                .scopes([scope::POSTS_WRITE])
                .client("claude-code"),
        )
        .call("posts.create", json!({ "title": "From a client" }))
        .await;
    let user = user.structured().cloned().unwrap_or_default();
    assert_eq!(user["kind"], "user");
    assert_eq!(user["browser"], false);
    assert_eq!(user["user"], "7");
    assert_eq!(user["auth_user"], "7", "the call is signed in as the user");
    assert_eq!(user["client"], "claude-code");

    let agent = client
        .mcp()
        .as_caller(
            ToolCaller::agent("reg-9")
                .bound_to(TestUser::arc("9"))
                .scopes([scope::POSTS_WRITE])
                .client("https://issuer.example"),
        )
        .call("posts.create", json!({ "title": "From an agent" }))
        .await;
    let agent = agent.structured().cloned().unwrap_or_default();
    assert_eq!(agent["kind"], "agent");
    assert_eq!(agent["registration"], "reg-9");
    assert_eq!(agent["claimed"], false);
    assert_eq!(agent["user"], "9", "the user the agent is bound to");
    assert_eq!(agent["client"], "https://issuer.example");

    // The browser: the session's user, with every scope.
    let browser = client
        .clone()
        .acting_as(&TestUser {
            id: "7".to_string(),
        })
        .tool("posts.create", json!({ "title": "From a page" }))
        .send()
        .await;
    browser.assert_ok();
    let browser = browser.json();
    assert_eq!(browser["kind"], "user");
    assert_eq!(browser["browser"], true);
    assert_eq!(browser["client"], serde_json::Value::Null);
}

#[tokio::test]
async fn agent_002_per_caller_limit_returns_429() {
    let _container = fixture::setup();
    let client = fixture::client_with(
        AgentsConfig::new().caller_limit(CallLimit::per_minute(2)),
        MiddlewareRegistry::new(),
    );
    let mcp = client
        .mcp()
        .as_caller(ToolCaller::user(TestUser::arc("limit-1")));
    for _ in 0..2 {
        let result = mcp.call("users.profile", json!({})).await;
        assert_eq!(result.status(), 200, "{:?}", result.response());
    }
    let refused = mcp.call("users.profile", json!({})).await;
    assert_eq!(refused.status(), 429);
    assert!(refused.response().header("retry-after").is_some());
    // Another user has calls of their own.
    let other = client
        .mcp()
        .as_caller(ToolCaller::user(TestUser::arc("limit-2")))
        .call("users.profile", json!({}))
        .await;
    assert_eq!(other.status(), 200);
}

#[tokio::test]
async fn agent_002_public_limit_is_per_address() {
    let _container = fixture::setup();
    let config = AgentsConfig::new().address_limit(CallLimit::per_minute(1));
    let first = fixture::client_with(config.clone(), MiddlewareRegistry::new())
        .with_peer_address(IpAddr::V4(Ipv4Addr::new(10, 2, 0, 1)));
    let result = first.mcp().call("echo", json!({ "text": "one" })).await;
    assert_eq!(result.status(), 200);
    let result = first.mcp().call("echo", json!({ "text": "two" })).await;
    assert_eq!(result.status(), 429, "one address used its calls");
    let second = fixture::client_with(config, MiddlewareRegistry::new())
        .with_peer_address(IpAddr::V4(Ipv4Addr::new(10, 2, 0, 2)));
    let result = second.mcp().call("echo", json!({ "text": "three" })).await;
    assert_eq!(result.status(), 200, "another address has its own calls");
}

#[tokio::test]
async fn agent_002_browser_calls_count_against_the_caller() {
    let _container = fixture::setup();
    let client = fixture::client_with(
        AgentsConfig::new().caller_limit(CallLimit::per_minute(1)),
        MiddlewareRegistry::new(),
    );
    let user = TestUser {
        id: "limit-3".to_string(),
    };
    client
        .clone()
        .acting_as(&user)
        .tool("users.profile", json!({}))
        .send()
        .await
        .assert_ok();
    let refused = client
        .mcp()
        .as_caller(ToolCaller::user(TestUser::arc("limit-3")))
        .call("users.profile", json!({}))
        .await;
    assert_eq!(
        refused.status(),
        429,
        "the browser call used the user's only call"
    );
}
