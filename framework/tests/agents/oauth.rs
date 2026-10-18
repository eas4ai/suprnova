//! AGENT-005, AGENT-010 and AGENT-017: the Magnetar authorization server
//! wired into the framework, driven through `handle_request` as an MCP
//! client following the 2026-07-28 authorization flow would drive it.
//!
//! The user's sessions are the framework's own session guard's here, so a
//! grant follows the HRD-007 epoch; `oauth_magnetar` covers Magnetar-owned
//! sessions.

use std::any::Any;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use base64::Engine;
use sea_orm::{ConnectionTrait, Database, DatabaseConnection};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use suprnova::agents::{
    AgentsConfig, CallLimit, ClientMetadataFetcher, ClientMetadataPolicy, ConnectedAgents,
    HostResolver, MagnetarError, MagnetarResult, OAuthServer, PinnedHttpClient, PinnedResponse,
    ResponseBody,
};
use suprnova::auth::Authenticatable;
use suprnova::container::testing::TestContainer;
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::testing::{TestClient, TestResponse};
use suprnova::{
    Auth, AuthConfig, AuthManager, Crypt, CsrfMiddleware, EncryptionKey, FrameworkError,
    HttpResponse, MiddlewareRegistry, RateLimiterDriver, Request, Router, SessionConfig,
    SessionData, SessionMiddleware, SessionStore, UserProvider,
};
use url::Url;

use crate::fixture;

/// The redirect URI a native client registers: a loopback IP literal, on
/// whatever port it listens.
const LOOPBACK_REDIRECT: &str = "http://127.0.0.1:7777/callback";
/// A client identified by its metadata document.
const DOCUMENT_CLIENT: &str = "https://tools.client.test/oauth/client.json";
const DOCUMENT_REDIRECT: &str = "https://tools.client.test/callback";
/// The MCP endpoint's canonical URL under the test's `APP_URL`.
const RESOURCE: &str = "http://localhost/mcp";

// ------------------------------------------------------------- users ----

/// Password hashes by user id. A password change writes a new hash, and the
/// HRD-007 epoch is a digest of it.
fn accounts() -> &'static Mutex<HashMap<String, String>> {
    static ACCOUNTS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    ACCOUNTS.get_or_init(|| Mutex::new(HashMap::new()))
}

static NEXT_ACCOUNT: AtomicUsize = AtomicUsize::new(0);

/// A new user of its own for the calling test.
pub fn new_account(label: &str) -> String {
    let id = format!(
        "{label}-{}-{}",
        std::process::id(),
        NEXT_ACCOUNT.fetch_add(1, Ordering::SeqCst)
    );
    accounts()
        .lock()
        .unwrap()
        .insert(id.clone(), format!("$argon2id$first-{id}"));
    id
}

/// Let the provider know a user another store created, such as a Magnetar
/// account, by its id.
pub fn remember_account(id: &str) {
    accounts()
        .lock()
        .unwrap()
        .insert(id.to_owned(), format!("$argon2id$elsewhere-{id}"));
}

/// Change the user's password: their stored hash is a new one.
pub fn change_password(id: &str) {
    accounts()
        .lock()
        .unwrap()
        .insert(id.to_owned(), format!("$argon2id$second-{id}"));
}

struct Account {
    id: String,
    password_hash: String,
}

impl Authenticatable for Account {
    fn get_auth_identifier(&self) -> String {
        self.id.clone()
    }
    fn get_auth_password(&self) -> Option<&str> {
        Some(&self.password_hash)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

/// The application's user provider: it knows every account a test made.
struct Accounts;

#[async_trait]
impl UserProvider for Accounts {
    async fn retrieve_by_id(
        &self,
        id: &str,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        Ok(accounts().lock().unwrap().get(id).map(|hash| {
            Arc::new(Account {
                id: id.to_owned(),
                password_hash: hash.clone(),
            }) as Arc<dyn Authenticatable>
        }))
    }
}

// ---------------------------------------------------------- sessions ----

#[derive(Default)]
struct MemorySessions(Mutex<HashMap<String, SessionData>>);

#[async_trait]
impl SessionStore for MemorySessions {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(self.0.lock().unwrap().get(id).cloned())
    }
    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        self.0
            .lock()
            .unwrap()
            .insert(session.id.clone(), session.clone());
        Ok(())
    }
    async fn destroy(&self, id: &str) -> Result<(), FrameworkError> {
        self.0.lock().unwrap().remove(id);
        Ok(())
    }
    async fn destroy_for_user(&self, user_id: &str) -> Result<u64, FrameworkError> {
        let mut sessions = self.0.lock().unwrap();
        let before = sessions.len();
        sessions.retain(|_, session| session.user_id.as_deref() != Some(user_id));
        Ok((before - sessions.len()) as u64)
    }
    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

// --------------------------------------------- client metadata fetch ----

/// A resolver that answers from a table.
#[derive(Default)]
pub struct FakeResolver(Mutex<HashMap<String, Vec<IpAddr>>>);

impl FakeResolver {
    pub fn answer(&self, host: &str, address: &str) {
        self.0
            .lock()
            .unwrap()
            .insert(host.to_owned(), vec![address.parse().unwrap()]);
    }
}

#[async_trait]
impl HostResolver for FakeResolver {
    async fn resolve(&self, host: &str, _port: u16) -> MagnetarResult<Vec<IpAddr>> {
        self.0
            .lock()
            .unwrap()
            .get(host)
            .cloned()
            .ok_or_else(|| MagnetarError::NotFound {
                resource: "host".to_owned(),
                identifier: host.to_owned(),
            })
    }
}

/// A pinned client that serves documents from a table and records every
/// request it was asked to make.
#[derive(Default)]
pub struct FakeDocuments {
    documents: Mutex<HashMap<String, Vec<u8>>>,
    pub requests: Mutex<Vec<String>>,
}

impl FakeDocuments {
    pub fn serve(&self, url: &str, body: &Value) {
        self.documents
            .lock()
            .unwrap()
            .insert(url.to_owned(), serde_json::to_vec(body).unwrap());
    }
}

struct OneChunk(Option<Vec<u8>>);

#[async_trait]
impl ResponseBody for OneChunk {
    async fn next_chunk(&mut self) -> MagnetarResult<Option<Vec<u8>>> {
        Ok(self.0.take())
    }
}

#[async_trait]
impl PinnedHttpClient for FakeDocuments {
    async fn get(&self, url: &Url, _address: SocketAddr) -> MagnetarResult<PinnedResponse> {
        self.requests.lock().unwrap().push(url.to_string());
        let body = self.documents.lock().unwrap().get(url.as_str()).cloned();
        Ok(PinnedResponse {
            status: if body.is_some() { 200 } else { 404 },
            cache_control: None,
            body: Box::new(OneChunk(body)),
        })
    }
}

// ------------------------------------------------------------ world ----

/// One application with the OAuth server on, and a browser-like client.
pub struct World {
    pub client: TestClient,
    pub db: DatabaseConnection,
    pub resolver: Arc<FakeResolver>,
    pub documents: Arc<FakeDocuments>,
    _container: Box<dyn Any>,
}

/// The authorization server's tables on a fresh in-memory database.
pub async fn database() -> DatabaseConnection {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    OAuthServer::migrate(&db).await.unwrap();
    db
}

/// The per-test container: the fixture's, plus a rate limiter for the
/// registration budget and the users provider behind the default guard.
pub fn container() -> Box<dyn Any> {
    static CRYPT: OnceLock<()> = OnceLock::new();
    CRYPT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
    let guard = Box::new(fixture::setup());
    TestContainer::bind::<dyn RateLimiterDriver>(Arc::new(InMemoryRateLimiter::new()));
    let manager = AuthManager::new(AuthConfig::default());
    manager.register_provider("users", Arc::new(Accounts));
    TestContainer::singleton(manager);
    guard
}

/// The fixture's routes, a sign-in route, two routes an application's own
/// connected-agents page would call, and agents served with `agents`.
pub fn router(agents: AgentsConfig) -> Router {
    fixture::define_policies();
    let routes: Router = fixture::routes()
        .get("/test/login", |request: Request| async move {
            let user = request.query_param("user").unwrap_or_default();
            Auth::login_id(user).map_err(HttpResponse::from)?;
            Ok(HttpResponse::text("signed in"))
        })
        .get("/test/facade/grants", |_request: Request| async {
            let grants = ConnectedAgents::client_grants()
                .await
                .map_err(HttpResponse::from)?;
            Ok(HttpResponse::json(json!(grants)))
        })
        .get("/test/facade/revoke", |request: Request| async move {
            let grant = request.query_param("grant").unwrap_or_default();
            let revoked = ConnectedAgents::revoke_client_grant(&grant)
                .await
                .map_err(HttpResponse::from)?;
            Ok(HttpResponse::json(json!({ "revoked": revoked })))
        })
        .into();
    routes.agents(agents)
}

/// The session and CSRF middleware every browser request passes.
pub fn registry() -> MiddlewareRegistry {
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    let csrf = CsrfMiddleware::new().with_session_config(&config);
    MiddlewareRegistry::new()
        .append(SessionMiddleware::with_store(
            config,
            Arc::new(MemorySessions::default()),
        ))
        .append(csrf)
}

/// A world whose OAuth server `configure` adjusts.
pub async fn world_with(configure: impl FnOnce(OAuthServer) -> OAuthServer) -> World {
    let container = container();
    let db = database().await;
    let resolver = Arc::new(FakeResolver::default());
    let documents = Arc::new(FakeDocuments::default());
    let fetcher = Arc::new(ClientMetadataFetcher::new(
        resolver.clone(),
        documents.clone(),
        ClientMetadataPolicy::default(),
    ));
    let server = configure(
        OAuthServer::new()
            .connection(db.clone())
            .client_metadata_fetcher(fetcher),
    );
    let client = TestClient::new(router(AgentsConfig::new().oauth_server(server)), registry())
        .with_peer_address("203.0.113.9".parse().unwrap());
    World {
        client,
        db,
        resolver,
        documents,
        _container: container,
    }
}

pub async fn world() -> World {
    world_with(|server| server).await
}

// ------------------------------------------------------------ steps ----

/// A PKCE verifier and its S256 challenge.
pub fn pkce() -> (String, String) {
    let verifier = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// The path and query of an absolute or host-relative address.
pub fn path_of(address: &str) -> String {
    let url = Url::parse("http://localhost")
        .unwrap()
        .join(address)
        .unwrap();
    match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_owned(),
    }
}

/// A query parameter of an address.
pub fn query_param(address: &str, name: &str) -> Option<String> {
    Url::parse("http://localhost")
        .unwrap()
        .join(address)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

pub fn location(response: &TestResponse) -> String {
    response
        .header("location")
        .unwrap_or_else(|| {
            panic!(
                "a redirect, got {} {}",
                response.status(),
                response.body_text()
            )
        })
        .to_owned()
}

/// Register a client dynamically (RFC 7591) and return its id.
pub async fn register(client: &TestClient, redirect_uri: &str) -> String {
    let response = client
        .post("/oauth/register")
        .json(&json!({
            "client_name": "Example Tool",
            "redirect_uris": [redirect_uri],
            "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
        }))
        .send()
        .await;
    assert_eq!(response.status(), 201, "{}", response.body_text());
    response.json()["client_id"].as_str().unwrap().to_owned()
}

pub async fn sign_in(client: &TestClient, user: &str) {
    client
        .get(format!("/test/login?user={user}"))
        .send()
        .await
        .assert_ok();
}

/// The authorization request a client sends the browser to.
pub fn authorize_path(client_id: &str, redirect_uri: &str, challenge: &str, scope: &str) -> String {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("response_type", "code")
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("scope", scope)
        .append_pair("resource", RESOURCE)
        .append_pair("state", "state-123")
        .finish();
    format!("/oauth/authorize?{query}")
}

/// The CSRF token the page's form carries.
pub fn form_token(page: &str) -> String {
    let start = page
        .find("name=\"_token\" value=\"")
        .unwrap_or_else(|| panic!("a form with the CSRF token in {page}"))
        + "name=\"_token\" value=\"".len();
    let end = page[start..].find('"').unwrap() + start;
    page[start..end].to_owned()
}

/// Authorize as the signed-in user and land on the approval page; return
/// the pending authorization's id and the page.
pub async fn open_approval(
    client: &TestClient,
    client_id: &str,
    redirect_uri: &str,
    challenge: &str,
    scope: &str,
) -> (String, TestResponse) {
    let authorized = client
        .get(authorize_path(client_id, redirect_uri, challenge, scope))
        .send()
        .await;
    assert_eq!(authorized.status(), 302, "{}", authorized.body_text());
    let approval = location(&authorized);
    assert!(
        approval.starts_with("http://localhost/oauth/approve?"),
        "{approval}"
    );
    let id = query_param(&approval, "authorization").unwrap();
    let page = client.get(path_of(&approval)).send().await;
    (id, page)
}

/// Approve through the CSRF-protected decision route; return where the
/// browser is sent.
pub async fn decide(
    client: &TestClient,
    id: &str,
    page: &TestResponse,
    decision: &str,
) -> TestResponse {
    client
        .post(format!("/oauth/authorizations/{id}"))
        .form(&[
            ("_token", form_token(&page.body_text()).as_str()),
            ("decision", decision),
        ])
        .send()
        .await
}

pub async fn exchange(
    client: &TestClient,
    client_id: &str,
    redirect_uri: &str,
    code: &str,
    verifier: &str,
) -> TestResponse {
    client
        .post("/oauth/token")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("client_id", client_id),
            ("code_verifier", verifier),
            ("resource", RESOURCE),
        ])
        .send()
        .await
}

pub async fn refresh(client: &TestClient, client_id: &str, refresh_token: &str) -> TestResponse {
    client
        .post("/oauth/token")
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", client_id),
            ("resource", RESOURCE),
        ])
        .send()
        .await
}

/// The tokens one completed flow gave a client.
pub struct Tokens {
    pub client_id: String,
    pub access: String,
    pub refresh: String,
}

/// Register a loopback client, sign `user` in, approve `scope` and
/// exchange the code.
pub async fn connect(client: &TestClient, user: &str, scope: &str) -> Tokens {
    let client_id = register(client, LOOPBACK_REDIRECT).await;
    sign_in(client, user).await;
    let (verifier, challenge) = pkce();
    let (id, page) = open_approval(client, &client_id, LOOPBACK_REDIRECT, &challenge, scope).await;
    assert_eq!(page.status(), 200, "{}", page.body_text());
    let back = location(&decide(client, &id, &page, "approve").await);
    let code = query_param(&back, "code").unwrap();
    let tokens = exchange(client, &client_id, LOOPBACK_REDIRECT, &code, &verifier).await;
    assert_eq!(tokens.status(), 200, "{}", tokens.body_text());
    let body = tokens.json();
    Tokens {
        client_id,
        access: body["access_token"].as_str().unwrap().to_owned(),
        refresh: body["refresh_token"].as_str().unwrap().to_owned(),
    }
}

/// The `error_description` of the 401 a call with `token` gets, which
/// tells a token the resolver refused from one no resolver recognized.
pub async fn refusal_reason(client: &TestClient, token: &str) -> String {
    let refused = client
        .mcp()
        .with_token(token)
        .call("users.profile", json!({}))
        .await;
    assert_eq!(refused.status(), 401, "{:?}", refused.response());
    let www = refused
        .response()
        .header("www-authenticate")
        .unwrap_or_default()
        .to_owned();
    assert!(www.contains("error=\"invalid_token\""), "{www}");
    www.split("error_description=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap_or_default()
        .to_owned()
}

/// Call `users.profile` with `token` and answer the status.
pub async fn profile_status(client: &TestClient, token: &str) -> u16 {
    client
        .mcp()
        .with_token(token)
        .call("users.profile", json!({}))
        .await
        .status()
}

// ------------------------------------------------------------ tests ----

#[tokio::test]
async fn agent_005_client_registers_signs_in_and_calls_a_user_tool() {
    let world = world().await;
    let client = &world.client;

    // 1. The endpoint refuses the call and names its metadata.
    let refused = client.mcp().call("users.profile", json!({})).await;
    assert_eq!(refused.status(), 401);
    let www = refused
        .response()
        .header("www-authenticate")
        .unwrap()
        .to_owned();
    let metadata_url = www
        .split("resource_metadata=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap()
        .to_owned();
    assert_eq!(
        metadata_url,
        "http://localhost/.well-known/oauth-protected-resource/mcp"
    );

    // 2. The protected resource metadata names this application as the
    // authorization server, whose metadata is at its well-known location.
    let resource = client.get(path_of(&metadata_url)).send().await.json();
    assert_eq!(resource["resource"], RESOURCE);
    assert_eq!(
        resource["authorization_servers"],
        json!(["http://localhost"])
    );
    let metadata = client
        .get("/.well-known/oauth-authorization-server")
        .send()
        .await
        .json();
    assert_eq!(metadata["issuer"], "http://localhost");
    assert_eq!(
        metadata["authorization_endpoint"],
        "http://localhost/oauth/authorize"
    );
    assert_eq!(
        metadata["code_challenge_methods_supported"],
        json!(["S256"])
    );
    assert_eq!(
        metadata["scopes_supported"],
        json!(["posts:delete", "posts:read", "posts:write"])
    );
    let openid = client
        .get("/.well-known/openid-configuration")
        .send()
        .await
        .json();
    assert_eq!(openid["token_endpoint"], "http://localhost/oauth/token");

    // 3. Dynamic registration, 4. sign-in, approval and the code exchange.
    let user = new_account("flow");
    let client_id = register(client, LOOPBACK_REDIRECT).await;
    sign_in(client, &user).await;
    let (verifier, challenge) = pkce();
    let (id, page) = open_approval(
        client,
        &client_id,
        LOOPBACK_REDIRECT,
        &challenge,
        "posts:read",
    )
    .await;
    assert_eq!(page.status(), 200, "{}", page.body_text());
    let back = location(&decide(client, &id, &page, "approve").await);
    assert!(
        back.starts_with("http://127.0.0.1:7777/callback?"),
        "{back}"
    );
    assert_eq!(query_param(&back, "state").as_deref(), Some("state-123"));
    assert_eq!(
        query_param(&back, "iss").as_deref(),
        Some("http://localhost"),
        "every authorization response carries iss (RFC 9207)"
    );
    let code = query_param(&back, "code").unwrap();
    let tokens = exchange(client, &client_id, LOOPBACK_REDIRECT, &code, &verifier).await;
    assert_eq!(tokens.status(), 200, "{}", tokens.body_text());
    assert_eq!(tokens.header("cache-control"), Some("no-store"));
    let access = tokens.json()["access_token"].as_str().unwrap().to_owned();

    // 5. The token calls a `user` tool as the user, through the client.
    let called = client
        .mcp()
        .with_token(&access)
        .call("users.profile", json!({}))
        .await;
    assert_eq!(called.status(), 200, "{:?}", called.response());
    assert_eq!(
        called.structured(),
        Some(&json!({ "user": user, "browser": false }))
    );
    let listed = client.mcp().with_token(&access).list().await.tool_names();
    assert!(listed.contains(&"posts.show".to_owned()), "{listed:?}");
    assert!(
        !listed.contains(&"posts.create".to_owned()),
        "a tool needing a scope the token lacks is not listed: {listed:?}"
    );

    // Tokens are stored only as digests.
    let stored = world
        .db
        .query_all_raw(sea_orm::Statement::from_string(
            world.db.get_database_backend(),
            "SELECT token_hash FROM auth_oauth_tokens".to_owned(),
        ))
        .await
        .unwrap();
    assert!(!stored.is_empty());
    for row in stored {
        let hash: String = row.try_get("", "token_hash").unwrap();
        assert_ne!(hash, access, "a token is never stored in plain text");
    }
}

#[tokio::test]
async fn agent_005_client_identified_by_metadata_document_signs_in() {
    let world = world().await;
    let client = &world.client;
    world.resolver.answer("tools.client.test", "93.184.215.14");
    world.documents.serve(
        DOCUMENT_CLIENT,
        &json!({
            "client_id": DOCUMENT_CLIENT,
            "client_name": "Document Tool",
            "redirect_uris": [DOCUMENT_REDIRECT],
            "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
        }),
    );
    let user = new_account("document");
    sign_in(client, &user).await;
    let (verifier, challenge) = pkce();
    let (id, page) = open_approval(
        client,
        DOCUMENT_CLIENT,
        DOCUMENT_REDIRECT,
        &challenge,
        "posts:read",
    )
    .await;
    let html = page.body_text();
    assert!(html.contains("Document Tool"), "{html}");
    assert!(
        html.contains("tools.client.test"),
        "the page names the document's host: {html}"
    );
    let back = location(&decide(client, &id, &page, "approve").await);
    let code = query_param(&back, "code").unwrap();
    let tokens = exchange(client, DOCUMENT_CLIENT, DOCUMENT_REDIRECT, &code, &verifier).await;
    assert_eq!(tokens.status(), 200, "{}", tokens.body_text());
    let access = tokens.json()["access_token"].as_str().unwrap().to_owned();
    assert_eq!(profile_status(client, &access).await, 200);
}

#[tokio::test]
async fn agent_005_metadata_document_on_a_private_address_is_not_fetched() {
    let world = world().await;
    let client = &world.client;
    world.resolver.answer("internal.client.test", "10.0.0.5");
    let user = new_account("private");
    sign_in(client, &user).await;
    let (_, challenge) = pkce();
    let refused = client
        .get(authorize_path(
            "https://internal.client.test/client.json",
            "https://internal.client.test/callback",
            &challenge,
            "posts:read",
        ))
        .send()
        .await;
    assert_eq!(refused.status(), 400, "{}", refused.body_text());
    assert_eq!(refused.json()["error"], "invalid_client");
    assert!(
        world.documents.requests.lock().unwrap().is_empty(),
        "nothing connected to the private address"
    );
}

#[tokio::test]
async fn agent_005_pkce_is_required_and_plain_is_refused() {
    let world = world().await;
    let client = &world.client;
    let user = new_account("pkce");
    let client_id = register(client, LOOPBACK_REDIRECT).await;
    sign_in(client, &user).await;

    // No challenge, and the `plain` method, are refused back to the client.
    let query = |method: Option<&str>, challenge: Option<&str>| {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        query
            .append_pair("response_type", "code")
            .append_pair("client_id", &client_id)
            .append_pair("redirect_uri", LOOPBACK_REDIRECT)
            .append_pair("scope", "posts:read")
            .append_pair("resource", RESOURCE);
        if let Some(challenge) = challenge {
            query.append_pair("code_challenge", challenge);
        }
        if let Some(method) = method {
            query.append_pair("code_challenge_method", method);
        }
        format!("/oauth/authorize?{}", query.finish())
    };
    let (verifier, challenge) = pkce();
    for path in [
        query(None, None),
        query(Some("plain"), Some(&verifier)),
        query(None, Some(&challenge)),
    ] {
        let refused = client.get(&path).send().await;
        let back = location(&refused);
        assert_eq!(
            query_param(&back, "error").as_deref(),
            Some("invalid_request"),
            "{path} -> {back}"
        );
        assert!(query_param(&back, "code").is_none());
    }

    // An exchange without the verifier, or with the wrong one, fails.
    let (id, page) = open_approval(
        client,
        &client_id,
        LOOPBACK_REDIRECT,
        &challenge,
        "posts:read",
    )
    .await;
    let code = query_param(
        &location(&decide(client, &id, &page, "approve").await),
        "code",
    )
    .unwrap();
    let without = client
        .post("/oauth/token")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", LOOPBACK_REDIRECT),
            ("client_id", client_id.as_str()),
            ("resource", RESOURCE),
        ])
        .send()
        .await;
    assert_eq!(without.status(), 400, "{}", without.body_text());
    let (wrong, _) = pkce();
    let mismatched = exchange(client, &client_id, LOOPBACK_REDIRECT, &code, &wrong).await;
    assert_eq!(mismatched.status(), 400, "{}", mismatched.body_text());
    assert_eq!(mismatched.json()["error"], "invalid_grant");
}

#[tokio::test]
async fn agent_005_approval_page_names_the_client_its_host_and_every_scope() {
    let world = world().await;
    let client = &world.client;
    let user = new_account("page");
    let client_id = register(client, LOOPBACK_REDIRECT).await;
    sign_in(client, &user).await;
    let (_, challenge) = pkce();
    let (_, page) = open_approval(
        client,
        &client_id,
        LOOPBACK_REDIRECT,
        &challenge,
        "posts:read posts:write",
    )
    .await;
    assert_eq!(page.status(), 200);
    let html = page.body_text();
    for needle in [
        "Example Tool",
        "127.0.0.1",
        "Read your posts",
        "posts:read",
        "Create and edit your posts",
        "posts:write",
        "name=\"decision\" value=\"approve\"",
        "name=\"decision\" value=\"deny\"",
    ] {
        assert!(html.contains(needle), "{needle} missing from {html}");
    }
    assert!(
        !html.contains("posts:delete"),
        "an unrequested scope is not shown"
    );
    assert_eq!(page.header("x-frame-options"), Some("DENY"));
    assert_eq!(
        page.header("content-security-policy"),
        Some("frame-ancestors 'none'")
    );
    assert_eq!(page.header("cache-control"), Some("no-store"));
    assert!(
        page.header("content-type")
            .is_some_and(|value| value.starts_with("text/html"))
    );
}

#[tokio::test]
async fn agent_005_approval_needs_a_signed_in_user() {
    let world = world().await;
    let client = &world.client;
    let client_id = register(client, LOOPBACK_REDIRECT).await;
    let (_, challenge) = pkce();

    // A guest is sent to sign in, and comes back to the same request.
    let authorize = authorize_path(&client_id, LOOPBACK_REDIRECT, &challenge, "posts:read");
    let to_login = location(&client.get(&authorize).send().await);
    assert!(
        to_login.starts_with("http://localhost/login?"),
        "{to_login}"
    );
    let pairs = |address: &str| {
        let mut pairs: Vec<(String, String)> = Url::parse("http://localhost")
            .unwrap()
            .join(address)
            .unwrap()
            .query_pairs()
            .into_owned()
            .collect();
        pairs.sort();
        pairs
    };
    let return_to = query_param(&to_login, "return_to").unwrap();
    assert!(return_to.starts_with("/oauth/authorize?"), "{return_to}");
    assert_eq!(pairs(&return_to), pairs(&authorize), "the same request");
    let page = client
        .get("/oauth/approve?authorization=anything")
        .send()
        .await;
    assert_eq!(page.status(), 302);
    assert!(location(&page).starts_with("http://localhost/login?"));
    assert_eq!(page.header("x-frame-options"), Some("DENY"));

    // The JSON route and the decision route refuse a guest outright.
    let pending = client.get("/oauth/authorizations/anything").send().await;
    assert_eq!(pending.status(), 401, "{}", pending.body_text());
    assert_eq!(pending.header("x-frame-options"), Some("DENY"));
}

#[tokio::test]
async fn agent_005_decision_needs_the_csrf_token() {
    let world = world().await;
    let client = &world.client;
    let user = new_account("csrf");
    let client_id = register(client, LOOPBACK_REDIRECT).await;
    sign_in(client, &user).await;
    let (_, challenge) = pkce();
    let (id, page) = open_approval(
        client,
        &client_id,
        LOOPBACK_REDIRECT,
        &challenge,
        "posts:read",
    )
    .await;
    for form in [
        vec![("decision", "approve")],
        vec![("_token", "forged"), ("decision", "approve")],
    ] {
        let refused = client
            .post(format!("/oauth/authorizations/{id}"))
            .form(&form)
            .send()
            .await;
        assert_eq!(refused.status(), 419, "{form:?}: {}", refused.body_text());
    }
    // The pending authorization survived both, and the real form decides it.
    let decided = decide(client, &id, &page, "approve").await;
    assert_eq!(decided.status(), 303, "{}", decided.body_text());
}

#[tokio::test]
async fn agent_005_decision_route_checks_csrf_without_global_middleware() {
    // An application that installed no CSRF middleware still gets the
    // check on the decision route.
    let container = container();
    let db = database().await;
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    let registry = MiddlewareRegistry::new().append(SessionMiddleware::with_store(
        config,
        Arc::new(MemorySessions::default()),
    ));
    let client = TestClient::new(
        router(AgentsConfig::new().oauth_server(OAuthServer::new().connection(db))),
        registry,
    );
    let user = new_account("bare");
    let client_id = register(&client, LOOPBACK_REDIRECT).await;
    sign_in(&client, &user).await;
    let (_, challenge) = pkce();
    let (id, page) = open_approval(
        &client,
        &client_id,
        LOOPBACK_REDIRECT,
        &challenge,
        "posts:read",
    )
    .await;
    let refused = client
        .post(format!("/oauth/authorizations/{id}"))
        .form(&[("decision", "approve")])
        .send()
        .await;
    assert_eq!(refused.status(), 419, "{}", refused.body_text());
    assert_eq!(decide(&client, &id, &page, "approve").await.status(), 303);
    drop(container);
}

#[tokio::test]
async fn agent_005_redirect_uri_must_match_exactly() {
    let world = world().await;
    let client = &world.client;
    let user = new_account("redirect");
    let client_id = register(client, LOOPBACK_REDIRECT).await;
    sign_in(client, &user).await;
    let (_, challenge) = pkce();
    for other in [
        "http://127.0.0.1:7777/elsewhere",
        "http://localhost:7777/callback",
        "https://attacker.test/callback",
    ] {
        let refused = client
            .get(authorize_path(&client_id, other, &challenge, "posts:read"))
            .send()
            .await;
        assert_eq!(refused.status(), 400, "{other}: {}", refused.body_text());
        assert!(
            refused.header("location").is_none(),
            "nothing redirects to {other}"
        );
    }
    // A loopback IP literal may use any port.
    let other_port = client
        .get(authorize_path(
            &client_id,
            "http://127.0.0.1:9999/callback",
            &challenge,
            "posts:read",
        ))
        .send()
        .await;
    assert_eq!(other_port.status(), 302, "{}", other_port.body_text());
}

#[tokio::test]
async fn agent_005_refresh_token_works_once_and_its_reuse_revokes_the_grant() {
    let world = world().await;
    let client = &world.client;
    let user = new_account("refresh");
    let tokens = connect(client, &user, "posts:read").await;
    let rotated = refresh(client, &tokens.client_id, &tokens.refresh).await;
    assert_eq!(rotated.status(), 200, "{}", rotated.body_text());
    let rotated = rotated.json();
    let new_access = rotated["access_token"].as_str().unwrap().to_owned();
    assert_ne!(rotated["refresh_token"], json!(tokens.refresh), "rotated");
    assert_eq!(profile_status(client, &new_access).await, 200);

    // The spent refresh token presented again: refused, and every token of
    // the grant stops working.
    let reused = refresh(client, &tokens.client_id, &tokens.refresh).await;
    assert_eq!(reused.status(), 400, "{}", reused.body_text());
    assert_eq!(reused.json()["error"], "invalid_grant");
    assert_eq!(profile_status(client, &new_access).await, 401);
    assert_eq!(profile_status(client, &tokens.access).await, 401);
    let newest_refresh = rotated["refresh_token"].as_str().unwrap();
    assert_eq!(
        refresh(client, &tokens.client_id, newest_refresh)
            .await
            .status(),
        400
    );
}

#[tokio::test]
async fn agent_005_password_change_ends_the_token_under_the_framework_guard() {
    let world = world().await;
    let client = &world.client;
    let user = new_account("password");
    let tokens = connect(client, &user, "posts:read").await;
    assert_eq!(profile_status(client, &tokens.access).await, 200);
    change_password(&user);
    let reason = refusal_reason(client, &tokens.access).await;
    assert!(
        reason.contains("ended with the user's sign-in"),
        "the resolver refused it for the epoch: {reason}"
    );
    assert_eq!(
        refresh(client, &tokens.client_id, &tokens.refresh)
            .await
            .status(),
        400,
        "the refresh token ended with the grant"
    );
}

#[tokio::test]
async fn agent_005_token_for_another_resource_is_refused() {
    let container = container();
    let db = database().await;
    let server = || OAuthServer::new().connection(db.clone());
    let client = TestClient::new(
        router(AgentsConfig::new().oauth_server(server())),
        registry(),
    );
    let user = new_account("resource");
    let tokens = connect(&client, &user, "posts:read").await;
    assert_eq!(profile_status(&client, &tokens.access).await, 200);
    // The same server's store behind an endpoint at another URL: the token
    // was bound to the first.
    let elsewhere = TestClient::new(
        router(
            AgentsConfig::new()
                .path("/other-mcp")
                .oauth_server(server()),
        ),
        registry(),
    );
    assert_eq!(
        refusal_reason(&elsewhere, &tokens.access).await,
        "The token was issued for another resource."
    );
    drop(container);
}

#[tokio::test]
async fn agent_005_client_sent_client_ip_is_ignored() {
    let world = world_with(|server| {
        server.registration_limit(CallLimit::new(2, std::time::Duration::from_secs(3600)))
    })
    .await;
    let register_as = |claimed: &'static str| {
        let client = world.client.clone();
        async move {
            client
                .post("/oauth/register")
                .header("X-Client-Ip", claimed)
                .json(&json!({
                    "client_name": "Spoofer",
                    "redirect_uris": [LOOPBACK_REDIRECT],
                    "token_endpoint_auth_method": "none",
                }))
                .send()
                .await
                .status()
        }
    };
    // Each request claims another address; all count against the one the
    // connection came from.
    assert_eq!(register_as("198.51.100.1").await, 201);
    assert_eq!(register_as("198.51.100.2").await, 201);
    assert_eq!(register_as("198.51.100.3").await, 429);
    // Another real address has a budget of its own.
    let other = world
        .client
        .clone()
        .with_peer_address("203.0.113.77".parse().unwrap());
    assert!(!register(&other, LOOPBACK_REDIRECT).await.is_empty());
}

#[tokio::test]
async fn agent_005_repeated_oauth_parameter_is_refused() {
    let world = world().await;
    let client = &world.client;
    let user = new_account("repeat");
    let client_id = register(client, LOOPBACK_REDIRECT).await;
    sign_in(client, &user).await;
    let (_, challenge) = pkce();
    let repeated_query = format!(
        "{}&scope=posts:write",
        authorize_path(&client_id, LOOPBACK_REDIRECT, &challenge, "posts:read")
    );
    let refused = client.get(&repeated_query).send().await;
    assert_eq!(refused.status(), 400, "{}", refused.body_text());
    assert_eq!(refused.json()["error"], "invalid_request");
    assert!(refused.header("location").is_none());

    let repeated_form = client
        .post("/oauth/token")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", "one"),
            ("code", "two"),
            ("client_id", client_id.as_str()),
        ])
        .send()
        .await;
    assert_eq!(repeated_form.status(), 400, "{}", repeated_form.body_text());
    assert_eq!(repeated_form.json()["error"], "invalid_request");
}

#[tokio::test]
async fn agent_005_oversized_body_is_refused() {
    let world = world().await;
    let client = &world.client;
    let big = "x".repeat(20 * 1024);
    for path in ["/oauth/token", "/oauth/revoke", "/oauth/register"] {
        let refused = client
            .post(path)
            .form(&[("token", big.as_str())])
            .send()
            .await;
        assert_eq!(refused.status(), 413, "{path}: {}", refused.body_text());
    }
    // On the decision route the host's own bound answers, before the
    // plugin reads anything: its refusal of a browser route is unframed.
    let user = new_account("oversized");
    let client_id = register(client, LOOPBACK_REDIRECT).await;
    sign_in(client, &user).await;
    let (_, challenge) = pkce();
    let (id, page) = open_approval(
        client,
        &client_id,
        LOOPBACK_REDIRECT,
        &challenge,
        "posts:read",
    )
    .await;
    let refused = client
        .post(format!("/oauth/authorizations/{id}"))
        .form(&[
            ("_token", form_token(&page.body_text()).as_str()),
            ("decision", "approve"),
            ("padding", big.as_str()),
        ])
        .send()
        .await;
    assert_eq!(refused.status(), 413, "{}", refused.body_text());
    assert_eq!(refused.header("x-frame-options"), Some("DENY"));
}

#[tokio::test]
async fn agent_005_client_endpoints_need_no_csrf_token() {
    // The world's registry runs the CSRF middleware on every request; the
    // client endpoints answer without a token, and set no cookie.
    let world = world().await;
    let client = &world.client;
    let registered = client
        .post("/oauth/register")
        .json(&json!({
            "client_name": "No Cookie",
            "redirect_uris": [LOOPBACK_REDIRECT],
            "token_endpoint_auth_method": "none",
        }))
        .send()
        .await;
    assert_eq!(registered.status(), 201, "{}", registered.body_text());
    assert!(registered.header("set-cookie").is_none());
    let token = client
        .post("/oauth/token")
        .form(&[("grant_type", "authorization_code"), ("code", "unknown")])
        .send()
        .await;
    assert_ne!(token.status(), 419, "{}", token.body_text());
    assert_eq!(token.json()["error"], "invalid_request");
    let revoked = client
        .post("/oauth/revoke")
        .form(&[("token", "mat_unknown")])
        .send()
        .await;
    assert_eq!(revoked.status(), 200, "{}", revoked.body_text());
}

#[tokio::test]
async fn agent_005_revoked_token_stops_working() {
    let world = world().await;
    let client = &world.client;
    let user = new_account("revoke");
    let tokens = connect(client, &user, "posts:read").await;
    assert_eq!(profile_status(client, &tokens.access).await, 200);
    let revoked = client
        .post("/oauth/revoke")
        .form(&[
            ("token", tokens.access.as_str()),
            ("client_id", tokens.client_id.as_str()),
        ])
        .send()
        .await;
    assert_eq!(revoked.status(), 200, "{}", revoked.body_text());
    assert_eq!(
        refusal_reason(client, &tokens.access).await,
        "The token was revoked."
    );
    // A token the server never issued is not this resolver's to refuse: no
    // resolver recognizes it.
    assert_eq!(
        refusal_reason(client, "mat_0000").await,
        "The token is not recognized."
    );
}

#[tokio::test]
async fn agent_005_flow_under_a_path_prefix() {
    if crate::own_process_async::delegate(module_path!(), "agent_005_flow_under_a_path_prefix")
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
    let world = world().await;
    let client = &world.client;
    let www = client
        .mcp()
        .call("users.profile", json!({}))
        .await
        .response()
        .header("www-authenticate")
        .unwrap()
        .to_owned();
    assert!(
        www.contains(
            "resource_metadata=\"https://example.test/billing/.well-known/oauth-protected-resource/mcp\""
        ),
        "{www}"
    );
    let resource = client
        .get("/.well-known/oauth-protected-resource/mcp")
        .send()
        .await
        .json();
    assert_eq!(resource["resource"], "https://example.test/billing/mcp");
    assert_eq!(
        resource["authorization_servers"],
        json!(["https://example.test/billing"])
    );
    let metadata = client
        .get("/.well-known/oauth-authorization-server")
        .send()
        .await
        .json();
    assert_eq!(metadata["issuer"], "https://example.test/billing");
    assert_eq!(
        metadata["token_endpoint"],
        "https://example.test/billing/oauth/token"
    );

    let user = new_account("prefix");
    let client_id = register(client, LOOPBACK_REDIRECT).await;
    sign_in(client, &user).await;
    let (verifier, challenge) = pkce();
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query
        .append_pair("response_type", "code")
        .append_pair("client_id", &client_id)
        .append_pair("redirect_uri", LOOPBACK_REDIRECT)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("scope", "posts:read")
        .append_pair("resource", "https://example.test/billing/mcp");
    let authorized = client
        .get(format!("/oauth/authorize?{}", query.finish()))
        .send()
        .await;
    let approval = location(&authorized);
    assert!(
        approval.starts_with("https://example.test/billing/oauth/approve?"),
        "{approval}"
    );
    let page = client
        .get(path_of(&approval).replacen("/billing", "", 1))
        .send()
        .await;
    assert_eq!(page.status(), 200, "{}", page.body_text());
    assert!(
        page.body_text()
            .contains("action=\"https://example.test/billing/oauth/authorizations/"),
        "{}",
        page.body_text()
    );
    let id = query_param(&approval, "authorization").unwrap();
    let back = location(&decide(client, &id, &page, "approve").await);
    assert_eq!(
        query_param(&back, "iss").as_deref(),
        Some("https://example.test/billing")
    );
    let code = query_param(&back, "code").unwrap();
    let tokens = client
        .post("/oauth/token")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", LOOPBACK_REDIRECT),
            ("client_id", client_id.as_str()),
            ("code_verifier", verifier.as_str()),
            ("resource", "https://example.test/billing/mcp"),
        ])
        .send()
        .await;
    assert_eq!(tokens.status(), 200, "{}", tokens.body_text());
    let access = tokens.json()["access_token"].as_str().unwrap().to_owned();
    assert_eq!(profile_status(client, &access).await, 200);
}

#[tokio::test]
async fn agent_010_user_lists_and_revokes_their_client_grant() {
    let world = world().await;
    let client = &world.client;
    let user = new_account("connected");
    let tokens = connect(client, &user, "posts:read").await;
    let page = client.get("/oauth/connected").send().await;
    assert_eq!(page.status(), 200, "{}", page.body_text());
    let html = page.body_text();
    for needle in ["Example Tool", "127.0.0.1", "Read your posts", "Revoke"] {
        assert!(html.contains(needle), "{needle} missing from {html}");
    }
    let action_start = html
        .find("action=\"http://localhost/oauth/connected/")
        .unwrap()
        + "action=\"".len();
    let action_end = html[action_start..].find('"').unwrap() + action_start;
    let action = path_of(&html[action_start..action_end]);

    // The revoke form needs its CSRF token.
    let forged = client
        .post(&action)
        .form(&[("_token", "forged")])
        .send()
        .await;
    assert_eq!(forged.status(), 419);
    assert_eq!(profile_status(client, &tokens.access).await, 200);

    let revoked = client
        .post(&action)
        .form(&[("_token", form_token(&html).as_str())])
        .send()
        .await;
    assert_eq!(revoked.status(), 303, "{}", revoked.body_text());
    assert_eq!(location(&revoked), "http://localhost/oauth/connected");
    assert_eq!(profile_status(client, &tokens.access).await, 401);
    let after = client.get("/oauth/connected").send().await.body_text();
    assert!(after.contains("No apps can act for you."), "{after}");
}

#[tokio::test]
async fn agent_010_another_users_grant_cannot_be_revoked() {
    let world = world().await;
    let owner = new_account("owner");
    let tokens = connect(&world.client, &owner, "posts:read").await;
    let grant = world
        .client
        .get("/oauth/connected")
        .send()
        .await
        .body_text();
    let start = grant.find("/oauth/connected/").unwrap() + "/oauth/connected/".len();
    let grant_id = grant[start..start + 32].to_owned();

    // Another user, in another browser, tries the owner's grant.
    let browser = TestClient::new(
        router(AgentsConfig::new().oauth_server(OAuthServer::new().connection(world.db.clone()))),
        registry(),
    );
    let other_user = new_account("intruder");
    sign_in(&browser, &other_user).await;
    let page = browser.get("/oauth/connected").send().await;
    assert!(page.body_text().contains("No apps can act for you."));
    // An empty page has no form; the intruder takes the token from the
    // cookie, as a script on the page would.
    let token = page.cookie("XSRF-TOKEN").expect("the CSRF cookie");
    let refused = browser
        .post(format!("/oauth/connected/{grant_id}/revoke"))
        .header("X-XSRF-TOKEN", token)
        .send()
        .await;
    assert_eq!(refused.status(), 404, "{}", refused.body_text());
    assert_eq!(profile_status(&world.client, &tokens.access).await, 200);
}

#[tokio::test]
async fn agent_010_connected_agents_facade_lists_and_revokes() {
    if crate::own_process_async::delegate(
        module_path!(),
        "agent_010_connected_agents_facade_lists_and_revokes",
    )
    .await
    {
        return;
    }
    let world = world().await;
    let user = new_account("facade");
    let tokens = connect(&world.client, &user, "posts:read posts:write").await;
    let listed = world.client.get("/test/facade/grants").send().await;
    assert_eq!(listed.status(), 200, "{}", listed.body_text());
    let grants = listed.json();
    assert_eq!(grants.as_array().map(Vec::len), Some(1), "{grants}");
    assert_eq!(grants[0]["client_name"], "Example Tool");
    assert_eq!(grants[0]["client_host"], "127.0.0.1");
    assert_eq!(
        grants[0]["scopes"],
        json!([
            { "name": "posts:read", "description": "Read your posts" },
            { "name": "posts:write", "description": "Create and edit your posts" },
        ])
    );
    let grant_id = grants[0]["id"].as_str().unwrap().to_owned();
    let revoked = world
        .client
        .get(format!("/test/facade/revoke?grant={grant_id}"))
        .send()
        .await;
    assert_eq!(revoked.json(), json!({ "revoked": true }));
    assert_eq!(profile_status(&world.client, &tokens.access).await, 401);
    let again = world
        .client
        .get(format!("/test/facade/revoke?grant={grant_id}"))
        .send()
        .await;
    assert_eq!(again.json(), json!({ "revoked": false }));
}

#[tokio::test]
async fn agent_017_default_pages_can_be_replaced() {
    let container = container();
    let db = database().await;
    let client = TestClient::new(
        router(
            AgentsConfig::new().oauth_server(
                OAuthServer::new()
                    .connection(db)
                    .custom_approval_page("/consent")
                    .custom_connected_agents_page(),
            ),
        ),
        registry(),
    );
    let user = new_account("custom");
    let client_id = register(&client, LOOPBACK_REDIRECT).await;
    sign_in(&client, &user).await;
    let (_, challenge) = pkce();
    let authorized = client
        .get(authorize_path(
            &client_id,
            LOOPBACK_REDIRECT,
            &challenge,
            "posts:read",
        ))
        .send()
        .await;
    let to = location(&authorized);
    assert!(
        to.starts_with("http://localhost/consent?authorization="),
        "{to}"
    );
    // The framework serves no page at either default path, and the JSON
    // route an application's own page reads answers the signed-in user.
    assert_eq!(client.get("/oauth/approve").send().await.status(), 404);
    assert_eq!(client.get("/oauth/connected").send().await.status(), 404);
    let id = query_param(&to, "authorization").unwrap();
    let pending = client
        .get(format!("/oauth/authorizations/{id}"))
        .send()
        .await;
    assert_eq!(pending.status(), 200, "{}", pending.body_text());
    let pending = pending.json();
    assert_eq!(pending["client"]["name"], "Example Tool");
    assert_eq!(pending["client"]["host"], "127.0.0.1");
    assert_eq!(pending["scopes"][0]["description"], "Read your posts");
    drop(container);
}
