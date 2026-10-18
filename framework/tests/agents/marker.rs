//! AGENT-001: a tool call steps past the CSRF and session middleware, and
//! nothing a client sends can make an ordinary request do the same.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::json;
use suprnova::agents::AgentsConfig;
use suprnova::{
    Crypt, CsrfMiddleware, EncryptionKey, FrameworkError, MiddlewareRegistry, SessionConfig,
    SessionData, SessionMiddleware, SessionStore, ToolCaller, async_trait,
};

use crate::fixture::{self, TestUser, scope};

/// A session store in memory.
#[derive(Default)]
struct MemoryStore(Mutex<HashMap<String, SessionData>>);

#[async_trait]
impl SessionStore for MemoryStore {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(self
            .0
            .lock()
            .map_err(|_| FrameworkError::internal("lock"))?
            .get(id)
            .cloned())
    }

    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        self.0
            .lock()
            .map_err(|_| FrameworkError::internal("lock"))?
            .insert(session.id.clone(), session.clone());
        Ok(())
    }

    async fn destroy(&self, id: &str) -> Result<(), FrameworkError> {
        self.0
            .lock()
            .map_err(|_| FrameworkError::internal("lock"))?
            .remove(id);
        Ok(())
    }

    async fn destroy_for_user(&self, _user_id: &str) -> Result<u64, FrameworkError> {
        Ok(0)
    }

    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

fn ensure_crypt() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
}

/// The session middleware alone, with an in-memory store.
pub(crate) fn session_only() -> MiddlewareRegistry {
    ensure_crypt();
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    MiddlewareRegistry::new().append(SessionMiddleware::with_store(
        config,
        Arc::new(MemoryStore::default()),
    ))
}

/// The session and CSRF middleware, as an application registers them.
fn session_and_csrf() -> MiddlewareRegistry {
    ensure_crypt();
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    let csrf = CsrfMiddleware::new().with_session_config(&config);
    MiddlewareRegistry::new()
        .append(SessionMiddleware::with_store(
            config,
            Arc::new(MemoryStore::default()),
        ))
        .append(csrf)
}

#[tokio::test]
async fn agent_001_wire_request_cannot_carry_the_tool_marker() {
    let _container = fixture::setup();
    let client = fixture::client_with(AgentsConfig::new(), session_and_csrf());
    // A token the endpoint would accept, the MCP headers, a cookie and a
    // made-up marker header: none of it is the private mark, so the
    // ordinary route still demands its CSRF token.
    let mcp = client
        .mcp()
        .as_caller(ToolCaller::user(TestUser::arc("7")).scopes([scope::POSTS_WRITE]));
    let token = mcp.token().unwrap_or_default().to_string();
    client
        .post("/posts")
        .json(&json!({ "title": "Forged" }))
        .header("Authorization", format!("Bearer {token}"))
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("Mcp-Method", "tools/call")
        .header("Mcp-Name", "posts.create")
        .header("X-Agent-Call", "posts.create")
        .header("Cookie", "suprnova_session=forged")
        .send()
        .await
        .assert_status(419);
}

#[tokio::test]
async fn agent_001_tool_call_skips_csrf_and_session_but_a_post_still_gets_419() {
    let _container = fixture::setup();
    let client = fixture::client_with(AgentsConfig::new(), session_and_csrf());
    let result = client
        .mcp()
        .as_caller(ToolCaller::user(TestUser::arc("7")).scopes([scope::POSTS_WRITE]))
        .call("posts.create", json!({ "title": "Through MCP" }))
        .await;
    assert!(!result.is_error(), "{:?}", result.response());
    assert_eq!(
        result.structured().map(|value| &value["title"]),
        Some(&json!("Through MCP"))
    );
    // Neither the endpoint nor the call started a session or handed out a
    // CSRF cookie.
    let response = result.response().response();
    assert!(
        response.header("set-cookie").is_none(),
        "{:?}",
        response.header("set-cookie")
    );

    // The same route called from the wire is still CSRF-checked.
    client
        .post("/posts")
        .json(&json!({ "title": "From the wire" }))
        .send()
        .await
        .assert_status(419);
}
