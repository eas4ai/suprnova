//! Real Suprnova server for the `agents_browser` suite's consent cases
//! (AGENT-005, AGENT-017): the framework's default approval page under the
//! default security headers, served the way an application without a kit
//! serves it.
//!
//! `suprnova-cli/tests/agents_browser/consent.rs` starts this binary and
//! drives it in Chromium. It serves:
//!
//! - the OAuth server with its default pages, `SecurityHeaders::from_env()`
//!   (the balanced preset, `form-action 'self'` included, with
//!   `SECURITY_HEADERS` unset), the session middleware and the CSRF
//!   middleware;
//! - `/login/{user}`, which signs the browser in as that user;
//! - `/test/global-policy-approval?authorization=<id>`, the control: the
//!   same decision form on a page that answers under the global policy, as
//!   a kit's page would without a way out of `form-action`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use suprnova::agents::{AgentsConfig, OAuthServer};
use suprnova::auth::{Authenticatable, UserProvider};
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::{
    App, Auth, AuthConfig, AuthManager, CsrfMiddleware, FrameworkError, HttpResponse,
    RateLimiterDriver, Request, Router, SecurityHeaders, Server, SessionConfig, SessionData,
    SessionMiddleware, SessionStore,
};

/// A user known by its name.
struct User {
    id: String,
    password_hash: String,
}

impl Authenticatable for User {
    fn get_auth_identifier(&self) -> String {
        self.id.clone()
    }
    fn get_auth_password(&self) -> Option<&str> {
        Some(&self.password_hash)
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn into_arc_any(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync> {
        self
    }
}

/// Every name is a user, with a password hash that never changes, so the
/// grant's epoch holds for the whole run.
struct Users;

#[async_trait]
impl UserProvider for Users {
    async fn retrieve_by_id(
        &self,
        id: &str,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        Ok(Some(Arc::new(User {
            id: id.to_owned(),
            password_hash: format!("$argon2id$fixed-{id}"),
        })))
    }

    async fn retrieve_by_credentials(
        &self,
        _credentials: &serde_json::Value,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        Ok(None)
    }
}

#[derive(Default)]
struct MemorySessionStore(Mutex<HashMap<String, SessionData>>);

fn lock_error<T>(_: T) -> FrameworkError {
    FrameworkError::internal("the session store's lock is poisoned")
}

#[async_trait]
impl SessionStore for MemorySessionStore {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(self.0.lock().map_err(lock_error)?.get(id).cloned())
    }
    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        self.0
            .lock()
            .map_err(lock_error)?
            .insert(session.id.clone(), session.clone());
        Ok(())
    }
    async fn destroy(&self, id: &str) -> Result<(), FrameworkError> {
        self.0.lock().map_err(lock_error)?.remove(id);
        Ok(())
    }
    async fn destroy_for_user(&self, user_id: &str) -> Result<u64, FrameworkError> {
        let mut sessions = self.0.lock().map_err(lock_error)?;
        let before = sessions.len();
        sessions.retain(|_, session| session.user_id.as_deref() != Some(user_id));
        Ok((before - sessions.len()) as u64)
    }
    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

/// `value` escaped for HTML text and quoted attributes.
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// The control page: the default page's decision form, answered under the
/// global policy.
async fn global_policy_approval(request: Request) -> suprnova::Response {
    let id = request.query_param("authorization").unwrap_or_default();
    let token = suprnova::session::get_csrf_token().unwrap_or_default();
    let id = escape(&id);
    let token = escape(&token);
    Ok(HttpResponse::html(format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <title>Allow under the global policy</title>\n</head>\n<body>\n\
         <form method=\"post\" action=\"/oauth/authorizations/{id}\">\n\
         <input type=\"hidden\" name=\"_token\" value=\"{token}\">\n\
         <button type=\"submit\" name=\"decision\" value=\"approve\">Allow</button>\n\
         </form>\n</body>\n</html>\n"
    )))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let port = std::env::var("SUPRNOVA_AGENTS_HOST_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(4179);
    App::init();
    let manager = AuthManager::new(AuthConfig::default());
    manager.register_provider("users", Arc::new(Users));
    App::singleton(manager);
    App::bind::<dyn RateLimiterDriver>(Arc::new(InMemoryRateLimiter::new()));
    let db = suprnova::sea_orm::Database::connect("sqlite::memory:").await?;
    OAuthServer::migrate(&db).await?;
    let router: Router = Router::new()
        .get("/health", |_request: Request| async {
            Ok(HttpResponse::text("ok"))
        })
        .get("/login/{user}", |request: Request| async move {
            let user = request.param("user").unwrap_or("").to_owned();
            Auth::login_id(user).map_err(HttpResponse::from)?;
            Ok(HttpResponse::text("signed in"))
        })
        .get("/test/global-policy-approval", global_policy_approval)
        .into();
    let router = router.try_agents(AgentsConfig::new().oauth_server(OAuthServer::new().connection(db)))?;
    let mut session = SessionConfig::default();
    session.cookie_secure = false;
    Server::new(router)
        .middleware(SecurityHeaders::from_env()?)
        .middleware(SessionMiddleware::with_store(
            session,
            Arc::new(MemorySessionStore::default()),
        ))
        .middleware(CsrfMiddleware::new())
        .host("127.0.0.1")
        .port(port)
        .run()
        .await
}
