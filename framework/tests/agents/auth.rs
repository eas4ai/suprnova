//! AGENT-004: what each credential may list and call, the 401 and 403
//! challenges, the token resolvers, and the OAuth discovery documents.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use serde_json::{Value, json};
use suprnova::agents::{
    AgentsConfig, AuthorizationServerMetadata, CallerResolver, MetadataKind, ResolveContext,
    ResolveError,
};
use suprnova::{FrameworkError, MiddlewareRegistry, ToolCaller};

use crate::fixture::{self, TestUser, scope};

#[tokio::test]
async fn agent_004_list_shows_only_public_tools_without_token() {
    let _container = fixture::setup();
    let client = fixture::client();
    assert_eq!(client.mcp().list().await.tool_names(), PUBLIC_TOOLS);
}

/// The fixture's tools that accept `public`, sorted: every caller may call
/// them.
const PUBLIC_TOOLS: [&str; 10] = [
    "bus.later",
    "echo",
    "math.eval",
    "page.html",
    "request.peek",
    "slow.bytes",
    "slow.stream",
    "slow.wait",
    "tags.group",
    "throttled.ping",
];

/// `names` without the public tools, which every caller sees.
fn beyond_public(names: Vec<String>) -> Vec<String> {
    for public in PUBLIC_TOOLS {
        assert!(
            names.iter().any(|name| name == public),
            "{public} missing from {names:?}"
        );
    }
    names
        .into_iter()
        .filter(|name| !PUBLIC_TOOLS.contains(&name.as_str()))
        .collect()
}

#[tokio::test]
async fn agent_004_list_filters_by_kind_and_scope() {
    let _container = fixture::setup();
    let client = fixture::client();
    let reader = client
        .mcp()
        .as_caller(ToolCaller::user(TestUser::arc("7")).scopes([scope::POSTS_READ]))
        .list()
        .await;
    assert_eq!(
        beyond_public(reader.tool_names()),
        vec![
            "posts.show",
            "profile.update",
            "users.api",
            "users.named",
            "users.profile"
        ],
        "a user sees the tools that accept users and whose scopes the token holds"
    );
    let writer = client
        .mcp()
        .as_caller(ToolCaller::agent("reg-1").scopes([scope::POSTS_WRITE]))
        .list()
        .await;
    assert_eq!(
        beyond_public(writer.tool_names()),
        vec!["posts.create"],
        "an agent sees no tool that does not accept agents"
    );
}

#[tokio::test]
async fn agent_004_missing_token_is_401_with_resource_metadata_and_scope() {
    let _container = fixture::setup();
    let client = fixture::client();
    let result = client
        .mcp()
        .call("posts.delete", json!({ "post": "1" }))
        .await;
    assert_eq!(result.status(), 401);
    let www = result
        .response()
        .header("www-authenticate")
        .unwrap_or_default()
        .to_string();
    assert!(www.starts_with("Bearer "), "{www}");
    assert!(
        www.contains(
            r#"resource_metadata="http://localhost/.well-known/oauth-protected-resource/mcp""#
        ),
        "{www}"
    );
    assert!(www.contains(r#"scope="posts:read posts:delete""#), "{www}");
}

#[tokio::test]
async fn agent_004_missing_scope_is_403_insufficient_scope_with_all_scopes() {
    let _container = fixture::setup();
    let client = fixture::client();
    let result = client
        .mcp()
        .as_caller(ToolCaller::user(TestUser::arc("7")).scopes([scope::POSTS_READ]))
        .call("posts.delete", json!({ "post": "1" }))
        .await;
    assert_eq!(result.status(), 403);
    let www = result
        .response()
        .header("www-authenticate")
        .unwrap_or_default()
        .to_string();
    assert!(www.contains(r#"error="insufficient_scope""#), "{www}");
    assert!(
        www.contains(r#"scope="posts:read posts:delete""#),
        "every scope the call needs, held or not: {www}"
    );
    assert!(www.contains("resource_metadata="), "{www}");
}

/// A resolver that recognizes one token.
struct OneToken {
    token: &'static str,
    caller: fn() -> ToolCaller,
    asked: Arc<AtomicUsize>,
}

#[async_trait]
impl CallerResolver for OneToken {
    async fn resolve(
        &self,
        token: &str,
        _ctx: &ResolveContext<'_>,
    ) -> Result<ToolCaller, ResolveError> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        if token == self.token {
            Ok((self.caller)())
        } else {
            Err(ResolveError::NotMine)
        }
    }

    fn authorization_servers(&self) -> Vec<String> {
        vec!["https://auth.example".to_string()]
    }
}

/// A resolver that claims every token and refuses it.
struct RefusesAll {
    asked: Arc<AtomicUsize>,
}

#[async_trait]
impl CallerResolver for RefusesAll {
    async fn resolve(
        &self,
        _token: &str,
        _ctx: &ResolveContext<'_>,
    ) -> Result<ToolCaller, ResolveError> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        Err(ResolveError::Invalid("the token was revoked"))
    }
}

fn user_caller() -> ToolCaller {
    ToolCaller::user(TestUser::arc("7")).scopes([scope::POSTS_READ])
}

#[tokio::test]
async fn agent_004_resolver_chain_not_mine_falls_through_invalid_stops() {
    let _container = fixture::setup();
    let first = Arc::new(AtomicUsize::new(0));
    let second = Arc::new(AtomicUsize::new(0));
    let third = Arc::new(AtomicUsize::new(0));
    let config = AgentsConfig::new()
        .resolver(Arc::new(OneToken {
            token: "tok-user",
            caller: user_caller,
            asked: first.clone(),
        }))
        .resolver(Arc::new(RefusesAll {
            asked: second.clone(),
        }))
        .resolver(Arc::new(OneToken {
            token: "tok-never",
            caller: user_caller,
            asked: third.clone(),
        }));
    let client = fixture::client_with(config, MiddlewareRegistry::new());

    let listed = client.mcp().with_token("tok-user").list().await;
    assert_eq!(listed.status(), 200);
    assert!(listed.tool_names().contains(&"users.profile".to_string()));
    assert_eq!(
        second.load(Ordering::SeqCst),
        0,
        "the first resolver decided"
    );

    let refused = client.mcp().with_token("tok-never").list().await;
    assert_eq!(refused.status(), 401);
    let www = refused.header("www-authenticate").unwrap_or_default();
    assert!(www.contains(r#"error="invalid_token""#), "{www}");
    assert_eq!(
        second.load(Ordering::SeqCst),
        1,
        "NotMine fell through to the second"
    );
    assert_eq!(third.load(Ordering::SeqCst), 0, "Invalid stopped the chain");
}

/// A resolver that refuses a token issued for another resource.
struct ResourceBound;

#[async_trait]
impl CallerResolver for ResourceBound {
    async fn resolve(
        &self,
        token: &str,
        ctx: &ResolveContext<'_>,
    ) -> Result<ToolCaller, ResolveError> {
        let Some(resource) = token.strip_prefix("for:") else {
            return Err(ResolveError::NotMine);
        };
        if resource != ctx.resource {
            return Err(ResolveError::Invalid(
                "the token was issued for another resource",
            ));
        }
        Ok(user_caller())
    }
}

#[tokio::test]
async fn agent_004_wrong_resource_token_is_refused() {
    let _container = fixture::setup();
    let client = fixture::client_with(
        AgentsConfig::new().resolver(Arc::new(ResourceBound)),
        MiddlewareRegistry::new(),
    );
    let ours = client
        .mcp()
        .with_token("for:http://localhost/mcp")
        .list()
        .await;
    assert_eq!(ours.status(), 200, "{ours:?}");
    let theirs = client
        .mcp()
        .with_token("for:https://other.example/mcp")
        .list()
        .await;
    assert_eq!(theirs.status(), 401);
    // A token no resolver recognizes is refused too, never served as a guest.
    let unknown = client.mcp().with_token("random").list().await;
    assert_eq!(unknown.status(), 401);
}

/// A resolver whose store is down.
struct Down;

#[async_trait]
impl CallerResolver for Down {
    async fn resolve(
        &self,
        _token: &str,
        _ctx: &ResolveContext<'_>,
    ) -> Result<ToolCaller, ResolveError> {
        Err(ResolveError::Unavailable(FrameworkError::internal(
            "the store is down",
        )))
    }
}

#[tokio::test]
async fn agent_004_unavailable_resolver_is_503_never_a_guest() {
    let _container = fixture::setup();
    let client = fixture::client_with(
        AgentsConfig::new().resolver(Arc::new(Down)),
        MiddlewareRegistry::new(),
    );
    let response = client.mcp().with_token("anything").list().await;
    assert_eq!(response.status(), 503);
}

#[tokio::test]
async fn agent_004_protected_resource_metadata_documents() {
    let _container = fixture::setup();
    let client = fixture::client_with(
        AgentsConfig::new().resolver(Arc::new(OneToken {
            token: "x",
            caller: user_caller,
            asked: Arc::new(AtomicUsize::new(0)),
        })),
        MiddlewareRegistry::new(),
    );
    for path in [
        "/.well-known/oauth-protected-resource",
        "/.well-known/oauth-protected-resource/mcp",
    ] {
        let response = client.get(path).send().await;
        assert_eq!(response.status(), 200, "{path}");
        let document = response.json();
        assert_eq!(document["resource"], "http://localhost/mcp", "{path}");
        assert_eq!(
            document["authorization_servers"],
            json!(["https://auth.example"])
        );
        assert_eq!(
            document["scopes_supported"],
            json!(["posts:delete", "posts:read", "posts:write"])
        );
        assert_eq!(document["bearer_methods_supported"], json!(["header"]));
        assert!(document["resource_name"].is_string());
        assert!(response.header("set-cookie").is_none());
    }
}

#[tokio::test]
async fn agent_004_metadata_under_a_path_prefix() {
    if crate::own_process_async::delegate(module_path!(), "agent_004_metadata_under_a_path_prefix")
        .await
    {
        return;
    }
    suprnova::Config::register(
        suprnova::AppConfig::builder()
            .url("https://example.test/billing")
            .debug(false)
            .build(),
    );
    let _container = fixture::setup();
    let client = fixture::client();
    let document = client
        .get("/.well-known/oauth-protected-resource/mcp")
        .send()
        .await
        .json();
    assert_eq!(document["resource"], "https://example.test/billing/mcp");
    let challenge = client
        .mcp()
        .call("posts.delete", json!({ "post": "1" }))
        .await;
    assert_eq!(challenge.status(), 401);
    let www = challenge
        .response()
        .header("www-authenticate")
        .unwrap_or_default()
        .to_string();
    assert!(
        www.contains(
            r#"resource_metadata="https://example.test/billing/.well-known/oauth-protected-resource/mcp""#
        ),
        "{www}"
    );
}

/// An authorization server that publishes a document of each kind.
struct Issuer;

#[async_trait]
impl AuthorizationServerMetadata for Issuer {
    async fn document(&self, issuer: &str, kind: MetadataKind) -> Option<Value> {
        Some(json!({ "issuer": issuer, "kind": format!("{kind:?}") }))
    }
}

#[tokio::test]
async fn agent_004_as_metadata_hook_404_without_provider() {
    let _container = fixture::setup();
    let without = fixture::client();
    for path in [
        "/.well-known/oauth-authorization-server",
        "/.well-known/openid-configuration",
    ] {
        without.get(path).send().await.assert_status(404);
    }
    let with = fixture::client_with(
        AgentsConfig::new().authorization_server(Arc::new(Issuer)),
        MiddlewareRegistry::new(),
    );
    let oauth = with
        .get("/.well-known/oauth-authorization-server")
        .send()
        .await;
    assert_eq!(oauth.status(), 200);
    assert_eq!(
        oauth.json(),
        json!({ "issuer": "http://localhost", "kind": "OAuthAuthorizationServer" })
    );
    let oidc = with
        .get("/.well-known/openid-configuration")
        .send()
        .await
        .json();
    assert_eq!(oidc["kind"], "OpenIdConfiguration");
    let resource = with
        .get("/.well-known/oauth-protected-resource")
        .send()
        .await
        .json();
    assert_eq!(
        resource["authorization_servers"],
        json!(["http://localhost"])
    );
}

/// A resolver that never answers.
struct Hangs;

#[async_trait]
impl CallerResolver for Hangs {
    async fn resolve(
        &self,
        _token: &str,
        _ctx: &ResolveContext<'_>,
    ) -> Result<ToolCaller, ResolveError> {
        std::future::pending().await
    }
}

#[tokio::test]
async fn agent_004_hanging_resolver_is_503() {
    let _container = fixture::setup();
    let client = fixture::client_with(
        AgentsConfig::new()
            .resolver(Arc::new(Hangs))
            .resolver_timeout(std::time::Duration::from_millis(100)),
        MiddlewareRegistry::new(),
    );
    let response = client.mcp().with_token("anything").list().await;
    assert_eq!(response.status(), 503);
}

/// An authorization server whose issuer is a path of this application.
struct PathIssuer;

#[async_trait]
impl AuthorizationServerMetadata for PathIssuer {
    async fn document(&self, issuer: &str, kind: MetadataKind) -> Option<Value> {
        Some(json!({ "issuer": self.issuer(issuer), "kind": format!("{kind:?}") }))
    }

    fn issuer(&self, issuer: &str) -> String {
        format!("{issuer}/oauth")
    }
}

#[tokio::test]
async fn agent_004_custom_issuer_metadata_at_its_path_inserted_location() {
    let _container = fixture::setup();
    let client = fixture::client_with(
        AgentsConfig::new().authorization_server(Arc::new(PathIssuer)),
        MiddlewareRegistry::new(),
    );
    for (path, kind) in [
        (
            "/.well-known/oauth-authorization-server/oauth",
            "OAuthAuthorizationServer",
        ),
        (
            "/.well-known/openid-configuration/oauth",
            "OpenIdConfiguration",
        ),
    ] {
        let response = client.get(path).send().await;
        assert_eq!(response.status(), 200, "{path}");
        assert_eq!(
            response.json(),
            json!({ "issuer": "http://localhost/oauth", "kind": kind })
        );
    }
    let resource = client
        .get("/.well-known/oauth-protected-resource")
        .send()
        .await
        .json();
    assert_eq!(
        resource["authorization_servers"],
        json!(["http://localhost/oauth"])
    );
}

/// The API guard's provider: it knows user `7`, by its own record.
struct ApiUsers;

#[async_trait]
impl suprnova::UserProvider for ApiUsers {
    async fn retrieve_by_id(
        &self,
        id: &str,
    ) -> Result<Option<Arc<dyn suprnova::auth::Authenticatable>>, FrameworkError> {
        Ok((id == "7").then(|| {
            Arc::new(fixture::ApiUser {
                id: "7".to_string(),
                name: "from the api provider".to_string(),
            }) as Arc<dyn suprnova::auth::Authenticatable>
        }))
    }
}

#[tokio::test]
async fn agent_004_guarded_tool_loads_the_user_through_its_guard() {
    let _container = fixture::setup();
    let config = suprnova::AuthConfig::default()
        .guard("api", suprnova::auth::GuardConfig::session("api-users"));
    let manager = suprnova::AuthManager::new(config);
    manager.register_provider("api-users", Arc::new(ApiUsers));
    suprnova::testing::TestContainer::singleton(manager);
    let client = fixture::client();
    let known = client
        .mcp()
        .as_caller(ToolCaller::user(TestUser::arc("7")))
        .call("users.api", json!({}))
        .await;
    assert_eq!(known.status(), 200, "{:?}", known.response());
    assert_eq!(
        known.structured(),
        Some(&json!({ "user": "7", "name": "from the api provider" })),
        "the guard's own provider loaded the user"
    );
    let unknown = client
        .mcp()
        .as_caller(ToolCaller::user(TestUser::arc("999")))
        .call("users.api", json!({}))
        .await;
    assert_eq!(
        unknown.status(),
        401,
        "the guard's provider does not know the user"
    );
}
