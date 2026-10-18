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
    assert_eq!(
        result.structured(),
        Some(&json!({ "user": "8", "browser": false }))
    );
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

#[tokio::test]
async fn agent_002_public_tools_admit_every_caller() {
    let _container = fixture::setup();
    let client = fixture::client();
    let callers = [
        ("user", ToolCaller::user(TestUser::arc("7"))),
        ("agent", ToolCaller::agent("reg-public")),
    ];
    for (kind, caller) in callers {
        let mcp = client.mcp().as_caller(caller);
        let listed = mcp.list().await.tool_names();
        assert!(listed.contains(&"echo".to_string()), "{kind}: {listed:?}");
        let result = mcp.call("echo", json!({ "text": kind })).await;
        assert_eq!(result.status(), 200, "{kind}: {:?}", result.response());
        assert_eq!(
            result.structured().map(|value| &value["kind"]),
            Some(&json!(kind)),
            "the handler still sees who called"
        );
    }
    // A signed-in browser session too.
    let page = client
        .clone()
        .acting_as(&TestUser {
            id: "7".to_string(),
        })
        .tool("echo", json!({ "text": "page" }))
        .send()
        .await;
    page.assert_ok();
    assert_eq!(page.json()["kind"], "user");
}

/// A stand-in for a token middleware: it signs the user in for this
/// request alone, as `BearerTokenMiddleware` or a custom guard would.
struct StandInToken;

#[suprnova::async_trait]
impl suprnova::Middleware for StandInToken {
    async fn handle(&self, request: suprnova::Request, next: suprnova::Next) -> suprnova::Response {
        if request.header("X-Stand-In-Token").is_some() {
            suprnova::Auth::set_user(TestUser::arc("token-user"));
        }
        next(request).await
    }
}

/// Knows one user, `7`, whose password is `secret`.
struct KnownUsers;

#[suprnova::async_trait]
impl suprnova::UserProvider for KnownUsers {
    async fn retrieve_by_id(
        &self,
        id: &str,
    ) -> Result<Option<std::sync::Arc<dyn suprnova::auth::Authenticatable>>, suprnova::FrameworkError>
    {
        Ok((id == "7").then(|| TestUser::arc("7")))
    }

    async fn retrieve_by_credentials(
        &self,
        credentials: &serde_json::Value,
    ) -> Result<Option<std::sync::Arc<dyn suprnova::auth::Authenticatable>>, suprnova::FrameworkError>
    {
        Ok((credentials["email"] == "seven@example.test").then(|| TestUser::arc("7")))
    }

    async fn validate_credentials(
        &self,
        _user: &dyn suprnova::auth::Authenticatable,
        credentials: &serde_json::Value,
    ) -> Result<bool, suprnova::FrameworkError> {
        Ok(credentials["password"] == "secret")
    }
}

/// `posts.create` and `users.profile`, each behind a middleware `make`
/// builds.
fn routes_behind<M: suprnova::Middleware + 'static>(make: impl Fn() -> M) -> suprnova::Router {
    let router: suprnova::Router = suprnova::Router::new()
        .post("/posts", fixture::create_post)
        .middleware(make())
        .post("/me", fixture::profile)
        .middleware(make())
        .into();
    router.agents(AgentsConfig::new())
}

#[tokio::test]
async fn agent_002_only_a_session_user_is_a_browser_caller() {
    let _container = fixture::setup();
    // A token user on the route is a direct caller: no scope, not a browser.
    let client = suprnova::testing::TestClient::new(
        routes_behind(|| StandInToken),
        MiddlewareRegistry::new(),
    );
    let scoped = client
        .post("/posts")
        .header("X-Stand-In-Token", "1")
        .json(&json!({ "title": "By token" }))
        .send()
        .await;
    scoped.assert_status(403);
    assert!(
        scoped
            .header("www-authenticate")
            .is_some_and(|value| value.contains("insufficient_scope")),
        "{:?}",
        scoped.header("www-authenticate")
    );
    let unscoped = client
        .post("/me")
        .header("X-Stand-In-Token", "1")
        .send()
        .await;
    unscoped.assert_ok();
    assert_eq!(
        unscoped.json(),
        json!({ "user": "token-user", "browser": false })
    );
}

#[tokio::test]
async fn agent_002_a_basic_auth_user_is_not_a_browser_caller() {
    use base64::Engine;
    let _container = fixture::setup();
    let manager = suprnova::AuthManager::new(suprnova::AuthConfig::default());
    manager.register_provider("users", std::sync::Arc::new(KnownUsers));
    suprnova::testing::TestContainer::singleton(manager);
    let client = suprnova::testing::TestClient::new(
        routes_behind(suprnova::BasicAuthMiddleware::once),
        MiddlewareRegistry::new(),
    );
    let credentials = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode("seven@example.test:secret")
    );
    let unscoped = client
        .post("/me")
        .header("Authorization", credentials.clone())
        .send()
        .await;
    unscoped.assert_ok();
    assert_eq!(unscoped.json(), json!({ "user": "7", "browser": false }));
    client
        .post("/posts")
        .header("Authorization", credentials)
        .json(&json!({ "title": "By basic auth" }))
        .send()
        .await
        .assert_status(403);
}

#[tokio::test]
async fn agent_002_a_session_user_is_a_browser_caller_with_every_scope() {
    let _container = fixture::setup();
    suprnova::testing::TestContainer::bind::<dyn suprnova::UserProvider>(std::sync::Arc::new(
        KnownUsers,
    ));
    let router: suprnova::Router = fixture::routes()
        .get("/sign-in", |_request: suprnova::Request| async {
            suprnova::Auth::login_id("7")?;
            suprnova::json_response!({ "signed_in": true })
        })
        .into();
    let client = suprnova::testing::TestClient::new(
        router.agents(AgentsConfig::new()),
        crate::marker::session_only(),
    );
    client.get("/sign-in").send().await.assert_ok();
    let page = client
        .tool("posts.create", json!({ "title": "From the session" }))
        .send()
        .await;
    page.assert_ok();
    let page = page.json();
    assert_eq!(page["user"], "7");
    assert_eq!(page["browser"], true, "a session user holds every scope");
}

#[tokio::test]
async fn agent_002_ordinary_requests_to_a_tool_route_use_no_budget() {
    let _container = fixture::setup();
    let client = fixture::client_with(
        AgentsConfig::new().caller_limit(CallLimit::per_minute(1)),
        MiddlewareRegistry::new(),
    )
    .acting_as(&TestUser {
        id: "budget-1".to_string(),
    });
    for _ in 0..3 {
        client.post("/me").send().await.assert_ok();
    }
    client
        .tool("users.profile", json!({}))
        .send()
        .await
        .assert_ok();
    client
        .tool("users.profile", json!({}))
        .send()
        .await
        .assert_status(429);
}

/// A cache store whose every operation fails, as a store that is down does.
struct DownStore;

#[suprnova::async_trait]
impl suprnova::cache::CacheStore for DownStore {
    async fn get_raw(&self, _key: &str) -> Result<Option<String>, suprnova::FrameworkError> {
        Err(suprnova::FrameworkError::internal("the store is down"))
    }
    async fn put_raw(
        &self,
        _key: &str,
        _value: &str,
        _ttl: Option<std::time::Duration>,
    ) -> Result<(), suprnova::FrameworkError> {
        Err(suprnova::FrameworkError::internal("the store is down"))
    }
    async fn has(&self, _key: &str) -> Result<bool, suprnova::FrameworkError> {
        Err(suprnova::FrameworkError::internal("the store is down"))
    }
    async fn forget(&self, _key: &str) -> Result<bool, suprnova::FrameworkError> {
        Err(suprnova::FrameworkError::internal("the store is down"))
    }
    async fn flush(&self) -> Result<(), suprnova::FrameworkError> {
        Err(suprnova::FrameworkError::internal("the store is down"))
    }
    async fn increment(&self, _key: &str, _amount: i64) -> Result<i64, suprnova::FrameworkError> {
        Err(suprnova::FrameworkError::internal("the store is down"))
    }
    async fn decrement(&self, _key: &str, _amount: i64) -> Result<i64, suprnova::FrameworkError> {
        Err(suprnova::FrameworkError::internal("the store is down"))
    }
    async fn tagged_put_raw(
        &self,
        _tags: &[&str],
        _key: &str,
        _value: &str,
        _ttl: Option<std::time::Duration>,
    ) -> Result<(), suprnova::FrameworkError> {
        Err(suprnova::FrameworkError::internal("the store is down"))
    }
    async fn flush_tags(&self, _tags: &[&str]) -> Result<(), suprnova::FrameworkError> {
        Err(suprnova::FrameworkError::internal("the store is down"))
    }
    async fn acquire_lock(
        &self,
        _key: &str,
        _ttl: std::time::Duration,
    ) -> Result<Option<String>, suprnova::FrameworkError> {
        Err(suprnova::FrameworkError::internal("the store is down"))
    }
    async fn release_lock(
        &self,
        _key: &str,
        _token: &str,
    ) -> Result<bool, suprnova::FrameworkError> {
        Err(suprnova::FrameworkError::internal("the store is down"))
    }
    async fn refresh_lock(
        &self,
        _key: &str,
        _token: &str,
        _ttl: std::time::Duration,
    ) -> Result<bool, suprnova::FrameworkError> {
        Err(suprnova::FrameworkError::internal("the store is down"))
    }
    async fn touch(
        &self,
        _key: &str,
        _ttl: std::time::Duration,
    ) -> Result<bool, suprnova::FrameworkError> {
        Err(suprnova::FrameworkError::internal("the store is down"))
    }
}

#[tokio::test]
async fn agent_002_ordinary_requests_survive_a_failed_limiter_store() {
    let _container = fixture::setup();
    suprnova::testing::TestContainer::bind::<dyn suprnova::cache::CacheStore>(std::sync::Arc::new(
        DownStore,
    ));
    let client = fixture::client().acting_as(&TestUser {
        id: "budget-2".to_string(),
    });
    client.post("/me").send().await.assert_ok();
    client
        .tool("users.profile", json!({}))
        .send()
        .await
        .assert_status(503);
}

#[tokio::test]
async fn agent_002_caller_checks_apply_without_the_header() {
    let _container = fixture::setup();
    let client = fixture::client();
    // No tool header: still a guest on a tool that accepts only users.
    client.post("/me").send().await.assert_status(401);
}
