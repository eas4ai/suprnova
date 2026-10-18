//! AGENT-018: a test calls a tool through the MCP endpoint and through the
//! browser as each kind of caller, without a ceremony, and the test path
//! applies every check the real path does.

use serde_json::json;
use suprnova::agents::{Agents, AgentsConfig};
use suprnova::{MiddlewareRegistry, ToolCaller};

use crate::fixture::{self, TestUser, scope};

#[tokio::test]
async fn agent_018_guest_user_agent_through_mcp_and_browser() {
    let _container = fixture::setup();
    let client = fixture::client();

    // Over MCP, as each caller.
    let guest = client.mcp().call("echo", json!({ "text": "guest" })).await;
    assert_eq!(
        guest.structured().map(|value| &value["kind"]),
        Some(&json!("public"))
    );
    let user = client
        .mcp()
        .as_caller(ToolCaller::user(TestUser::new("7")).scopes([scope::POSTS_WRITE]))
        .call("posts.create", json!({ "title": "User" }))
        .await;
    assert_eq!(
        user.structured().map(|value| &value["kind"]),
        Some(&json!("user"))
    );
    let agent = client
        .mcp()
        .as_caller(ToolCaller::agent("reg-18").scopes([scope::POSTS_WRITE]))
        .call("posts.create", json!({ "title": "Agent" }))
        .await;
    assert_eq!(
        agent.structured().map(|value| &value["kind"]),
        Some(&json!("agent"))
    );

    // From the browser, as a guest and as the signed-in user.
    let page_guest = client.tool("echo", json!({ "text": "page" })).send().await;
    page_guest.assert_ok();
    assert_eq!(page_guest.json()["kind"], "public");
    let page_user = client
        .clone()
        .acting_as(&TestUser {
            id: "7".to_string(),
        })
        .tool("posts.create", json!({ "title": "Page" }))
        .send()
        .await;
    page_user.assert_ok();
    assert_eq!(page_user.json()["browser"], true);
}

#[tokio::test]
async fn agent_018_test_path_applies_scope_and_origin_checks() {
    let _container = fixture::setup();
    let client = fixture::client();
    let unscoped = client
        .mcp()
        .as_caller(ToolCaller::user(TestUser::new("7")))
        .call("posts.create", json!({ "title": "No scope" }))
        .await;
    assert_eq!(
        unscoped.status(),
        403,
        "a fake token holds only the scopes it was given"
    );
    let foreign = client
        .mcp()
        .header("Origin", "https://evil.example")
        .call("echo", json!({ "text": "x" }))
        .await;
    assert_eq!(foreign.status(), 403);
    let invalid = client
        .mcp()
        .as_caller(ToolCaller::user(TestUser::new("7")).scopes([scope::POSTS_WRITE]))
        .call("posts.create", json!({ "title": "" }))
        .await;
    assert!(invalid.is_error(), "validation still runs on the test path");
}

#[tokio::test]
async fn agent_018_claimed_and_unclaimed_agent() {
    let _container = fixture::setup();
    let client = fixture::client();
    for claimed in [false, true] {
        let result = client
            .mcp()
            .as_caller(
                ToolCaller::agent("reg-claim")
                    .claimed(claimed)
                    .scopes([scope::POSTS_WRITE]),
            )
            .call("posts.create", json!({ "title": "Claim" }))
            .await;
        assert_eq!(
            result.structured().map(|value| &value["claimed"]),
            Some(&json!(claimed))
        );
    }
}

#[tokio::test]
async fn agent_018_browser_path_refuses_agent() {
    let _container = fixture::setup();
    let client = fixture::client();
    // An agent's token on the tool's route is not a session: the browser
    // sees a guest, which this tool does not accept.
    let fake = Agents::fake();
    let token = fake.token_for(ToolCaller::agent("reg-browser").scopes([scope::POSTS_WRITE]));
    client
        .tool("posts.create", json!({ "title": "Agent in a page" }))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .assert_status(401);
    // A signed-in user on a tool that accepts only guests is refused too.
    client
        .clone()
        .acting_as(&TestUser {
            id: "7".to_string(),
        })
        .tool("echo", json!({ "text": "x" }))
        .send()
        .await
        .assert_status(403);
}

#[tokio::test]
async fn agent_018_fake_tokens_end_with_their_fake() {
    let _container = fixture::setup();
    let client = fixture::client_with(AgentsConfig::new(), MiddlewareRegistry::new());
    let token = {
        let fake = Agents::fake();
        fake.token_for(ToolCaller::user(TestUser::new("7")))
    };
    let response = client.mcp().with_token(token).list().await;
    // `client.mcp()` keeps a fake of its own alive, so the dropped fake's
    // token is unknown rather than accepted.
    assert_eq!(response.status(), 401);
}
